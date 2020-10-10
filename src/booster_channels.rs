//! Booster NGFW channel management control interface definitions.
//!
//! # Copyright
//! Copyright (C) 2020 QUARTIQ GmbH - All Rights Reserved
//! Unauthorized usage, editing, or copying is strictly prohibited.
//! Proprietary and confidential.

use enum_iterator::IntoEnumIterator;
use stm32f4xx_hal as hal;
use tca9548::{self, Tca9548};

use super::{I2cBusManager, I2cProxy};
use crate::error::Error;
use crate::rf_channel::{
    ChannelPins as RfChannelPins, ChannelState, Property as RfChannelProperty, PropertyId,
    RfChannel, ChannelStatus,
};
use embedded_hal::blocking::delay::DelayUs;

/// Represents a control structure for interfacing to booster RF channels.
pub struct BoosterChannels {
    channels: [Option<RfChannel>; 8],
    adc: hal::adc::Adc<hal::stm32::ADC3>,
    mux: Tca9548<I2cProxy>,
}

/// Indicates a booster RF channel.
#[derive(IntoEnumIterator, Copy, Clone, Debug, serde::Serialize, serde::Deserialize)]
pub enum Channel {
    Zero = 0,
    One = 1,
    Two = 2,
    Three = 3,
    Four = 4,
    Five = 5,
    Six = 6,
    Seven = 7,
}

impl Into<tca9548::Bus> for Channel {
    fn into(self) -> tca9548::Bus {
        match self {
            Channel::Zero => tca9548::Bus::Zero,
            Channel::One => tca9548::Bus::One,
            Channel::Two => tca9548::Bus::Two,
            Channel::Three => tca9548::Bus::Three,
            Channel::Four => tca9548::Bus::Four,
            Channel::Five => tca9548::Bus::Five,
            Channel::Six => tca9548::Bus::Six,
            Channel::Seven => tca9548::Bus::Seven,
        }
    }
}

impl BoosterChannels {
    /// Construct booster RF channels.
    ///
    /// # Note
    /// This function will scan channels to check if they are present.
    ///
    /// # Args
    /// * `mux` - The I2C mux used for switching between channel communications.
    /// * `adc` - The ADC used to measure analog channels.
    /// * `manager` - The I2C bus manager used for the shared I2C bus.
    /// * `pins` - An array of all RfChannel control/status pins.
    /// * `delay` - A means of delaying during setup.
    ///
    /// # Returns
    /// A `BoosterChannels` object that can be used to manage all available RF channels.
    pub fn new(
        mut mux: Tca9548<I2cProxy>,
        adc: hal::adc::Adc<hal::stm32::ADC3>,
        manager: &'static I2cBusManager,
        mut pins: [Option<RfChannelPins>; 8],
        delay: &mut impl DelayUs<u16>,
    ) -> Self {
        let mut rf_channels: [Option<RfChannel>; 8] =
            [None, None, None, None, None, None, None, None];

        for channel in Channel::into_enum_iter() {
            // Selecting an I2C bus should never fail.
            mux.select_bus(Some(channel.into()))
                .expect("Failed to select channel");

            let control_pins = pins[channel as usize]
                .take()
                .expect("Channel pins not available");

            match RfChannel::new(&manager, control_pins, delay) {
                Some(rf_channel) => {
                    rf_channels[channel as usize].replace(rf_channel);
                }
                None => {
                    info!("Channel {} did not enumerate", channel as usize);
                }
            }
        }

        BoosterChannels {
            channels: rf_channels,
            mux: mux,
            adc: adc,
        }
    }

    /// Perform an action on a channel.
    ///
    /// # Args
    /// * `channel` - The channel to perform the action on.
    /// * `func` - A function called with the channel selected and with the channel and the ADC3 peripheral passed as arguments.
    pub fn map_channel<F, R>(&mut self, channel: Channel, f: F) -> Result<R, Error>
    where
        F: FnOnce(&mut RfChannel, &mut hal::adc::Adc<hal::stm32::ADC3>) -> Result<R, Error>,
    {
        let mux = &mut self.mux;
        let adc = &mut self.adc;
        let ch = &mut self.channels[channel as usize];
        ch.as_mut().ok_or(Error::NotPresent).and_then(|ch| {
            mux.select_bus(Some(channel.into())).unwrap();
            f(ch, adc)
        })
    }

    /// Enable an RF channel.
    ///
    /// # Args
    /// * `channel` - The channel to enable.
    pub fn enable_channel(&mut self, channel: Channel) -> Result<(), Error> {
        self.map_channel(channel, |ch, _| ch.start_powerup(true))
    }

    /// Power up an RF channel without enabling output.
    ///
    /// # Args
    /// * `channel` - The channel to power-up.
    pub fn power_channel(&mut self, channel: Channel) -> Result<(), Error> {
        self.map_channel(channel, |ch, _| ch.start_powerup(false))
    }

    /// Disable an RF channel.
    ///
    /// # Args
    /// * `channel` - The channel to disable.
    pub fn disable_channel(&mut self, channel: Channel) -> Result<(), Error> {
        self.map_channel(channel, |ch, _| Ok(ch.start_disable()))
    }

    /// Get the temperature of a channel.
    ///
    /// # Args
    /// * `channel` - The channel to get the temperature of.
    ///
    /// # Returns
    /// The temperature of the channel in degrees celsius.
    pub fn get_temperature(&mut self, channel: Channel) -> Result<f32, Error> {
        self.map_channel(channel, |ch, _| Ok(ch.get_temperature()))
    }

    /// Set the bias voltage of a channel.
    ///
    /// # Args
    /// * `channel` - The channel to set the bias voltage of.
    /// * `bias_voltage` - The desired bias voltage to apply to the RF amplification transistor.
    pub fn set_bias(
        &mut self,
        channel: Channel,
        bias_voltage: f32,
        delay: &mut impl DelayUs<u16>,
    ) -> Result<(f32, f32), Error> {
        self.map_channel(channel, |ch, _| {
            ch.set_bias(bias_voltage)?;

            // Settle the bias current and wait for an up-to-date measurement.
            delay.delay_us(11000);
            Ok((ch.get_bias_voltage(), ch.get_p28v_current()))
        })
    }

    /// Get the state of a channel.
    ///
    /// # Args
    /// * `channel` - The channel to get the state of.
    ///
    /// # Returns
    /// The channel state of the requested channel.
    pub fn get_channel_state(&mut self, channel: Channel) -> Result<ChannelState, Error> {
        self.map_channel(channel, |ch, _| Ok(ch.get_state()))
    }

    /// Get the current status of the channel.
    ///
    /// # Args
    /// * `channel` - The channel to get the status of.
    ///
    /// Returns
    /// A structure indicating all measurements on the channel.
    pub fn get_status(&mut self, channel: Channel) -> Result<ChannelStatus, Error> {
        self.map_channel(channel, |ch, adc| Ok(ch.get_status(adc)))
    }

    /// Save the current channel configuration in channel EEPROM.
    pub fn save_configuration(&mut self, channel: Channel) -> Result<(), Error> {
        self.map_channel(channel, |ch, _| Ok(ch.save_configuration()))
    }

    /// Update the states of RF channels as necessary.
    pub fn update(&mut self) {
        for channel in Channel::into_enum_iter() {
            self.map_channel(channel, |ch, _| Ok(ch.update().unwrap()))
                .ok();
        }
    }

    /// Read a property from an RF channel.
    ///
    /// # Args
    /// * `channel` - The channel to read the property of.
    /// * `property` - The identifier of the property to read.
    ///
    /// # Returns
    /// The requested property of the desired channel.
    pub fn read_property(
        &mut self,
        channel: Channel,
        property: PropertyId,
    ) -> Result<RfChannelProperty, Error> {
        self.map_channel(channel, |ch, _| Ok(ch.get_property(property)))
    }

    /// Write a property into an RF channel.
    ///
    /// # Args
    /// * `channel` - The channel to update the property of.
    /// * `property` - The property to set.
    pub fn write_property(
        &mut self,
        channel: Channel,
        property: RfChannelProperty,
    ) -> Result<(), Error> {
        self.map_channel(channel, |ch, _| ch.set_property(property))
    }
}
