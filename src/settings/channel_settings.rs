//! Booster NGFW NVM channel settings
//!
//! # Copyright
//! Copyright (C) 2020 QUARTIQ GmbH - All Rights Reserved
//! Unauthorized usage, editing, or copying is strictly prohibited.
//! Proprietary and confidential.

use super::{SinaraConfiguration, SinaraBoardId};
use crate::{linear_transformation::LinearTransformation, Error, I2cProxy};
use microchip_24aa02e48::Microchip24AA02E48;

/// Represents booster channel-specific configuration values.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct BoosterChannelData {
    reflected_interlock_threshold: f32,
    output_interlock_threshold: f32,
    bias_voltage: f32,
    enabled: bool,
    input_power_transform: LinearTransformation,
    output_power_transform: LinearTransformation,
    reflected_power_transform: LinearTransformation,
}

impl BoosterChannelData {
    /// Generate default booster channel data.
    pub fn default() -> Self {
        Self {
            reflected_interlock_threshold: f32::NAN,
            output_interlock_threshold: f32::NAN,
            bias_voltage: -3.2,
            enabled: false,

            // When operating at 100MHz, the power detectors specify the following output
            // characteristics for -10 dBm to 10 dBm (the equation uses slightly different coefficients
            // for different power levels and frequencies):
            //
            // dBm = V(Vout) / .035 V/dB - 35.6 dBm
            //
            // All of the power meters are preceded by attenuators which are incorporated in
            // the offset.
            output_power_transform: LinearTransformation::new(1.0 / 0.035, -35.6 + 19.8 + 10.0),
            reflected_power_transform: LinearTransformation::new(1.5 / 0.035, -35.6 + 19.8 + 10.0),

            // The input power and reflected power detectors are then passed through an
            // op-amp with gain 1.5x - this modifies the slope from 35mV/dB to 52.5mV/dB
            input_power_transform: LinearTransformation::new(1.5 / 0.035, -35.6 + 8.9),
        }
    }

    /// Construct booster configuration data from serialized `board_data` from a
    /// SinaraConfiguration.
    ///
    /// # Args
    /// * `data` - The data to deserialize from.
    ///
    /// # Returns
    /// The configuration if deserialization was successful. Otherwise, returns an error.
    pub fn deserialize(data: &[u8; 64]) -> Result<Self, Error> {
        let config: BoosterChannelData = postcard::from_bytes(data).unwrap();

        // Validate configuration parameters.
        if config.bias_voltage < -3.3 || config.bias_voltage > 0.0 {
            return Err(Error::Invalid);
        }

        Ok(config)
    }

    /// Serialize the booster config into a sinara configuration for storage into EEPROM.
    ///
    /// # Args
    /// * `config` - The sinara configuration to serialize the booster configuration into.
    pub fn serialize_into(&self, config: &mut SinaraConfiguration) {
        let mut buffer: [u8; 64] = [0; 64];
        let serialized = postcard::to_slice(self, &mut buffer).unwrap();
        config.board_data[..serialized.len()].copy_from_slice(serialized);
    }
}

pub struct BoosterChannelSettings {
    eeprom: Microchip24AA02E48<I2cProxy>,
    pub data: BoosterChannelData,
}

impl BoosterChannelSettings {

    pub fn new(eeprom: Microchip24AA02E48<I2cProxy>) -> Self {
        let mut settings = Self {
            eeprom,
            data: BoosterChannelData::default(),
        };

        match settings.load_config() {
            Ok(config) => {
                // If we loaded sinara configuration, deserialize the board data.
                match BoosterChannelData::deserialize(&config.board_data) {
                    Ok(data) => settings.data = data,

                    Err(_) => {
                        settings.data = BoosterChannelData::default();
                        settings.save();
                    }
                }

            },

            // If we failed to load configuration, use a default config.
            Err(_) => {
                settings.data = BoosterChannelData::default();
                settings.save();
            }
        };

        settings
    }

    /// Save the configuration settings to EEPROM for retrieval.
    pub fn save(&mut self) {
        let mut config = match self.load_config() {
            Err(_) => SinaraConfiguration::default(SinaraBoardId::RfChannel),
            Ok(config) => config,
        };

        self.data.serialize_into(&mut config);
        config.update_crc32();
        self.save_config(&config);
    }

    /// Load device settings from EEPROM.
    ///
    /// # Returns
    /// Ok(settings) if the settings loaded successfully. Otherwise, Err(settings), where `settings`
    /// are default values.
    fn load_config(&mut self) -> Result<SinaraConfiguration, Error> {
        // Read the sinara-config from memory.
        let mut sinara_config: [u8; 256] = [0; 256];
        self.eeprom.read(0, &mut sinara_config).unwrap();

        SinaraConfiguration::try_deserialize(sinara_config)
    }

    fn save_config(&mut self, config: &SinaraConfiguration) {
        // Save the updated configuration to EEPROM.
        let mut serialized = [0u8; 128];
        config.serialize_into(&mut serialized);
        self.eeprom.write(0, &serialized).unwrap();
    }
}
