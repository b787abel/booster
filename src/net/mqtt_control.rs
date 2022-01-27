//! Booster NGFW Application
//!
//! # Copyright
//! Copyright (C) 2020 QUARTIQ GmbH - All Rights Reserved
//! Unauthorized usage, editing, or copying is strictly prohibited.
//! Proprietary and confidential.
use crate::{
    delay::AsmDelay,
    hardware::{booster_channels::BoosterChannels, clock::SystemTimer},
    Channel, Error, MainBus,
};

use super::NetworkStackProxy;

use core::fmt::Write;
use embedded_hal::blocking::delay::DelayUs;
use heapless::String;
use minimq::{Property, QoS};

/// Specifies an action to take on a channel.
#[derive(serde::Deserialize)]
enum ChannelAction {
    Save,
    ReadBiasCurrent,
}

/// Specifies a generic request for a specific channel.
#[derive(serde::Deserialize)]
struct ChannelRequest {
    pub channel: Channel,
    pub action: ChannelAction,
}

/// Indicates the result of a channel bias setting request.
#[derive(serde::Serialize)]
struct ChannelBiasResponse {
    code: u32,
    pub vgs: f32,
    pub ids: f32,
}

impl ChannelBiasResponse {
    /// Indicate that a channel bias setting command was successfully processed.
    ///
    /// # Args
    /// * `vgs` - The resulting gate voltage of the RF amplifier.
    /// * `ids` - The resulting drain current of the RF amplifier.
    pub fn okay(vgs: f32, ids: f32) -> String<256> {
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
    msg: String<256>,
}

impl Response {
    /// Indicate that a command was successfully processed.
    ///
    /// # Args
    /// * `msg` - An additional user-readable message.
    pub fn okay<'a>(msg: &'a str) -> String<256> {
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
    pub fn error_msg<'a>(msg: &'a str) -> String<256> {
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
    pub fn error(error: Error) -> String<256> {
        let mut msg = String::<256>::new();
        write!(&mut msg, "{:?}", error).unwrap();

        let response = Response { code: 400, msg };

        serde_json_core::to_string(&response).unwrap()
    }
}

/// Represents a means of handling MQTT-based control interface.
pub struct ControlState {
    mqtt: minimq::Minimq<NetworkStackProxy, SystemTimer, 128, 1>,
    subscribed: bool,
    id: String<32>,
    delay: AsmDelay,
}

impl ControlState {
    /// Construct the MQTT control state manager.
    pub fn new<'a>(
        broker: minimq::embedded_nal::IpAddr,
        stack: super::NetworkStackProxy,
        id: &'a str,
        delay: AsmDelay,
    ) -> Self {
        let mut client_id: String<64> = String::new();
        write!(&mut client_id, "{}-ctrl", id).unwrap();

        Self {
            mqtt: minimq::Minimq::new(broker, &client_id, stack, SystemTimer::default()).unwrap(),
            subscribed: false,
            id: String::from(id),
            delay,
        }
    }

    fn generate_topic_string<'a>(&self, topic_postfix: &'a str) -> String<64> {
        let mut topic_string: String<64> = String::new();
        write!(&mut topic_string, "{}/{}", self.id, topic_postfix).unwrap();
        topic_string
    }

    /// Handle the MQTT-based control interface.
    ///
    /// # Args
    /// * `resources` - The `idle` resources containing the client and RF channels.
    pub fn update(&mut self, main_bus: &mut impl rtic::Mutex<T = MainBus>) {
        // Subscribe to any control topics necessary.
        if !self.subscribed {
            if self.mqtt.client.is_connected() {
                for topic in [
                    "channel/state",
                    "channel/bias",
                    "channel/read",
                    "channel/write",
                ]
                .iter()
                {
                    self.mqtt
                        .client
                        .subscribe(&self.generate_topic_string(topic), &[])
                        .unwrap();
                }
                self.subscribed = true;
            }
        }

        let response_topic = self.generate_topic_string("log");
        let delay = &mut self.delay;
        let my_id = &self.id;

        match self.mqtt.poll(|client, topic, message, properties| {
            let (id, route) = topic.split_at(topic.find('/').unwrap());
            let route = &route[1..];

            if id != my_id {
                warn!("Ignoring topic for identifier: {}", id);
                return;
            }

            let response = main_bus.lock(|main_bus| match route {
                "channel/state" => handle_channel_update(message, &mut main_bus.channels, delay),
                _ => Response::error_msg("Unexpected topic"),
            });

            if let Property::ResponseTopic(topic) = properties
                .iter()
                .find(|&prop| {
                    if let Property::ResponseTopic(_) = *prop {
                        true
                    } else {
                        false
                    }
                })
                .or(Some(&Property::ResponseTopic(&response_topic)))
                .unwrap()
            {
                client
                    .publish(
                        topic,
                        &response.into_bytes(),
                        QoS::AtMostOnce,
                        minimq::Retain::NotRetained,
                        &[],
                    )
                    .unwrap();
            }
        }) {
            Ok(_) => {}

            // Whenever the MQTT broker stops maintaining the session,
            // this MQTT client will reset the session,
            // and we will lose our pending subscriptions.
            // We will need to re-establish them once we reconnect.
            Err(minimq::Error::SessionReset) => self.subscribed = false,

            // Note: There's a race condition where the W5500 may disconnect the socket
            // immediately before Minimq tries to use it. In these cases, a NotReady error is
            // returned to indicate the socket is no longer connected. On the next processing
            // cycle of Minimq, the device should detect and handle the broker disconnection.
            #[cfg(feature = "phy_w5500")]
            Err(minimq::Error::Network(w5500::tcp::TcpSocketError::NotReady)) => {}

            Err(e) => error!("Unexpected error: {:?}", e),
        }
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
fn handle_channel_update(
    message: &[u8],
    channels: &mut BoosterChannels,
    delay: &mut impl DelayUs<u16>,
) -> String<256> {
    let mut response: String<256> = String::new();

    let request = match serde_json_core::from_slice::<ChannelRequest>(message) {
        Ok((data, _)) => data,
        Err(_) => return Response::error_msg("Failed to decode data"),
    };
    channels
        .map(request.channel, |ch, _| match request.action {
            ChannelAction::Save => {
                ch.save_configuration();
                Ok("Channel saved")
            }
            ChannelAction::ReadBiasCurrent => {
                // Wait for 11 ms > 10.04 ms total cycle time to ensure an up-to-date
                // current measurement.
                delay.delay_us(11000);
                response =
                    ChannelBiasResponse::okay(ch.get_bias_voltage(), ch.get_p28v_current()).into();
                Ok(&response)
            }
        })
        .map_or_else(Response::error, Response::okay)
}
