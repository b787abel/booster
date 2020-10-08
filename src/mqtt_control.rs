//! Booster NGFW Application
//!
//! # Copyright
//! Copyright (C) 2020 QUARTIQ GmbH - All Rights Reserved
//! Unauthorized usage, editing, or copying is strictly prohibited.
//! Proprietary and confidential.
use super::{idle::Resources, BoosterChannels, Channel, Error};
use core::fmt::Write;
use embedded_hal::blocking::delay::DelayUs;
use heapless::{consts, String};
use minimq::{Property, QoS};

use crate::rf_channel::{
    InterlockThresholds, Property as ChannelProperty, PropertyId as ChannelPropertyId,
};

use crate::linear_transformation::LinearTransformation;

#[derive(serde::Deserialize)]
struct PropertyReadRequest {
    pub channel: Channel,
    pub prop: ChannelPropertyId,
}

#[derive(serde::Serialize)]
struct PropertyReadResponse {
    code: u32,
    data: String<consts::U64>,
}

impl PropertyReadResponse {
    /// Indicate that a property read response was successful.
    ///
    /// # Args
    /// * `vgs` - The resulting gate voltage of the RF amplifier.
    /// * `ids` - The resulting drain current of the RF amplifier.
    pub fn okay(prop: ChannelProperty) -> String<consts::U256> {
        // Serialize the property.
        let data: String<consts::U64> = match prop {
            ChannelProperty::InterlockThresholds(thresholds) => {
                serde_json_core::to_string(&thresholds).unwrap()
            }
            ChannelProperty::InputPowerTransform(transform) => {
                serde_json_core::to_string(&transform).unwrap()
            }
            ChannelProperty::OutputPowerTransform(transform) => {
                serde_json_core::to_string(&transform).unwrap()
            }
            ChannelProperty::ReflectedPowerTransform(transform) => {
                serde_json_core::to_string(&transform).unwrap()
            }
        };

        let mut response = Self {
            code: 200,
            data: String::new(),
        };

        // Convert double quotes to single in the encoded property. This gets around string escape
        // sequences.
        for byte in data.as_str().chars() {
            if byte == '"' {
                response.data.push('\'').unwrap();
            } else {
                response.data.push(byte).unwrap();
            }
        }

        serde_json_core::to_string(&response).unwrap()
    }
}

#[derive(serde::Deserialize)]
struct PropertyWriteRequest {
    pub channel: Channel,
    prop: ChannelPropertyId,
    data: String<consts::U64>,
}

impl PropertyWriteRequest {
    pub fn property(&self) -> Result<ChannelProperty, Error> {
        // Convert single quotes to double in the property data.
        let mut data: String<consts::U64> = String::new();
        for byte in self.data.as_str().chars() {
            if byte == '\'' {
                data.push('"').unwrap();
            } else {
                data.push(byte).unwrap();
            }
        }

        // Convert the property
        let prop = match self.prop {
            ChannelPropertyId::InterlockThresholds => ChannelProperty::InterlockThresholds(
                serde_json_core::from_str::<InterlockThresholds>(&data)
                    .map_err(|_| Error::Invalid)?,
            ),
            ChannelPropertyId::OutputPowerTransform => ChannelProperty::OutputPowerTransform(
                serde_json_core::from_str::<LinearTransformation>(&data)
                    .map_err(|_| Error::Invalid)?,
            ),
            ChannelPropertyId::InputPowerTransform => ChannelProperty::InputPowerTransform(
                serde_json_core::from_str::<LinearTransformation>(&data)
                    .map_err(|_| Error::Invalid)?,
            ),
            ChannelPropertyId::ReflectedPowerTransform => ChannelProperty::ReflectedPowerTransform(
                serde_json_core::from_str::<LinearTransformation>(&data)
                    .map_err(|_| Error::Invalid)?,
            ),
        };

        Ok(prop)
    }
}

/// Specifies an action to take on a channel.
#[derive(serde::Deserialize)]
enum ChannelAction {
    Enable,
    Disable,
    Powerup,
    Save,
}

/// Specifies a generic request for a specific channel.
#[derive(serde::Deserialize)]
struct ChannelRequest {
    pub channel: Channel,
    pub action: ChannelAction,
}

/// Specifies the desired channel RF bias current.
#[derive(serde::Deserialize)]
struct ChannelTuneRequest {
    pub channel: Channel,
    pub current: f32,
}

/// Indicates the result of a channel tuning request.
#[derive(serde::Serialize)]
struct ChannelTuneResponse {
    code: u32,
    pub vgs: f32,
    pub ids: f32,
}

impl ChannelTuneResponse {
    /// Indicate that a channel bias tuning command was successfully processed.
    ///
    /// # Args
    /// * `vgs` - The resulting gate voltage of the RF amplifier.
    /// * `ids` - The resulting drain current of the RF amplifier.
    pub fn okay(vgs: f32, ids: f32) -> String<consts::U256> {
        let response = Self {
            code: 200,
            vgs,
            ids,
        };

        serde_json_core::to_string(&response).unwrap()
    }
}

/// Represents a generic response to a command.
#[derive(serde::Serialize)]
struct Response {
    code: u32,
    msg: String<heapless::consts::U256>,
}

impl Response {
    /// Indicate that a command was successfully processed.
    ///
    /// # Args
    /// * `msg` - An additional user-readable message.
    pub fn okay<'a>(msg: &'a str) -> String<consts::U256> {
        let response = Response {
            code: 200,
            msg: String::from(msg),
        };

        serde_json_core::to_string(&response).unwrap()
    }

    /// Indicate that a command failed to be processed.
    ///
    /// # Args
    /// * `msg` - An additional user-readable message.
    pub fn error_msg<'a>(msg: &'a str) -> String<consts::U256> {
        let response = Response {
            code: 400,
            msg: String::from(msg),
        };

        serde_json_core::to_string(&response).unwrap()
    }

    /// Indicate that a command failed to be processed.
    ///
    /// # Args
    /// * `error` - The error that was encountered while the command was being processed.
    pub fn error(error: Error) -> String<consts::U256> {
        let mut msg = String::<consts::U256>::new();
        write!(&mut msg, "{:?}", error).unwrap();

        let response = Response { code: 400, msg };

        serde_json_core::to_string(&response).unwrap()
    }
}

/// Represents a means of handling MQTT-based control interface.
pub struct ControlState {
    subscribed: bool,
    id: String<heapless::consts::U32>,
}

impl ControlState {
    /// Construct the MQTT control state manager.
    pub fn new<'a>(id: &'a str) -> Self {
        Self {
            subscribed: false,
            id: String::from(id),
        }
    }

    fn generate_topic_string<'a>(&self, topic_postfix: &'a str) -> String<heapless::consts::U64> {
        let mut topic_string: String<heapless::consts::U64> = String::new();
        write!(&mut topic_string, "{}/{}", self.id, topic_postfix).unwrap();
        topic_string
    }

    /// Handle the MQTT-based control interface.
    ///
    /// # Args
    /// * `resources` - The `idle` resources containing the client and RF channels.
    pub fn update(&mut self, resources: &mut Resources) {
        use rtic::Mutex as _;
        // Subscribe to any control topics necessary.
        if !self.subscribed {
            resources.mqtt_client.lock(|client| {
                if client.is_connected().unwrap() {
                    client
                        .subscribe(&self.generate_topic_string("channel/state"), &[])
                        .unwrap();
                    client
                        .subscribe(&self.generate_topic_string("channel/tune"), &[])
                        .unwrap();
                    client
                        .subscribe(&self.generate_topic_string("channel/read"), &[])
                        .unwrap();
                    client
                        .subscribe(&self.generate_topic_string("channel/write"), &[])
                        .unwrap();
                    self.subscribed = true;
                }
            });
        }

        let main_bus = &mut resources.main_bus;
        let delay = &mut resources.delay;

        resources.mqtt_client.lock(|client| {
            match client.poll(|client, topic, message, properties| {
                main_bus.lock(|main_bus| {
                    let (id, route) = topic.split_at(topic.find('/').unwrap());
                    let route = &route[1..];

                    if id != self.id {
                        warn!("Ignoring topic for identifier: {}", id);
                        return;
                    }

                    let response = match route {
                        "channel/state" => handle_channel_update(message, &mut main_bus.channels),
                        "channel/tune" => {
                            handle_channel_tune(message, &mut main_bus.channels, *delay)
                        }
                        "channel/read" => {
                            handle_channel_property_read(message, &mut main_bus.channels)
                        }
                        "channel/write" => {
                            handle_channel_property_write(message, &mut main_bus.channels)
                        }
                        _ => Response::error_msg("Unexpected topic"),
                    };

                    if let Property::ResponseTopic(topic) = properties
                        .iter()
                        .find(|&prop| {
                            if let Property::ResponseTopic(_) = *prop {
                                true
                            } else {
                                false
                            }
                        })
                        .or(Some(&Property::ResponseTopic(
                            &self.generate_topic_string("log"),
                        )))
                        .unwrap()
                    {
                        client
                            .publish(topic, &response.into_bytes(), QoS::AtMostOnce, &[])
                            .unwrap();
                    }
                });
            }) {
                Ok(_) => {}

                // Whenever MQTT disconnects, we will lose our pending subscriptions. We will need
                // to re-establish them once we reconnect.
                Err(minimq::Error::Disconnected) => self.subscribed = false,

                Err(e) => error!("Unexpected error: {:?}", e),
            }
        });
    }
}

/// Handle a request to update a booster RF channel state.
///
/// # Args
/// * `message` - The serialized message request.
/// * `channels` - The booster RF channels to configure.
///
/// # Returns
/// A String response indicating the result of the request.
fn handle_channel_update(message: &[u8], channels: &mut BoosterChannels) -> String<consts::U256> {
    let request = match serde_json_core::from_slice::<ChannelRequest>(message) {
        Ok(data) => data,
        Err(_) => return Response::error_msg("Failed to decode data"),
    };

    match request.action {
        ChannelAction::Enable => channels.enable_channel(request.channel).map_or_else(
            |e| Response::error(e),
            |_| Response::okay("Channel enabled"),
        ),
        ChannelAction::Disable => channels.disable_channel(request.channel).map_or_else(
            |e| Response::error(e),
            |_| Response::okay("Channel disabled"),
        ),
        ChannelAction::Powerup => channels.power_channel(request.channel).map_or_else(
            |e| Response::error(e),
            |_| Response::okay("Channel powered"),
        ),
        ChannelAction::Save => channels.save_configuration(request.channel).map_or_else(
            |e| Response::error(e),
            |_| Response::okay("Configuration saved"),
        ),
    }
}

/// Handle a request to read a property of an RF channel.
///
/// # Args
/// * `message` - The serialized message request.
/// * `channels` - The booster RF channels to read.
///
/// # Returns
/// A String response indicating the result of the request.
fn handle_channel_property_read(
    message: &[u8],
    channels: &mut BoosterChannels,
) -> String<consts::U256> {
    let request = match serde_json_core::from_slice::<PropertyReadRequest>(message) {
        Ok(data) => data,
        Err(_) => return Response::error_msg("Failed to decode read request"),
    };

    match channels.read_property(request.channel, request.prop) {
        Ok(prop) => PropertyReadResponse::okay(prop),
        Err(error) => Response::error(error),
    }
}

/// Handle a request to write a property of an RF channel.
///
/// # Args
/// * `message` - The serialized message request.
/// * `channels` - The booster RF channels to write.
///
/// # Returns
/// A String response indicating the result of the request.
fn handle_channel_property_write(
    message: &[u8],
    channels: &mut BoosterChannels,
) -> String<consts::U256> {
    let request = match serde_json_core::from_slice::<PropertyWriteRequest>(message) {
        Ok(data) => data,
        Err(_) => return Response::error_msg("Failed to decode read request"),
    };

    let property = match request.property() {
        Ok(property) => property,
        Err(_) => return Response::error_msg("Failed to decode property"),
    };

    match channels.write_property(request.channel, property) {
        Ok(_) => Response::okay("Property update successful"),
        Err(error) => Response::error(error),
    }
}

/// Handle a request to tune the bias current of a channel.
///
/// # Args
/// * `message` - The serialized message request.
/// * `channels` - The booster RF channels to configure.
/// * `delay` - A means of delaying during tuning.
///
/// # Returns
/// A String response indicating the result of the request.
fn handle_channel_tune(
    message: &[u8],
    channels: &mut BoosterChannels,
    delay: &mut impl DelayUs<u16>,
) -> String<consts::U256> {
    let request = match serde_json_core::from_slice::<ChannelTuneRequest>(message) {
        Ok(data) => data,
        Err(_) => return Response::error_msg("Failed to decode data"),
    };

    match channels.tune_channel(request.channel, request.current, delay) {
        Ok((vgs, ids)) => ChannelTuneResponse::okay(vgs, ids),
        Err(error) => Response::error(error),
    }
}
