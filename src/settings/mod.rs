//! Booster NGFW NVM settings
//!
//! # Copyright
//! Copyright (C) 2020 QUARTIQ GmbH - All Rights Reserved
//! Unauthorized usage, editing, or copying is strictly prohibited.
//! Proprietary and confidential.

mod channel_settings;
mod global_settings;
mod sinara;

use sinara::{BoardId as SinaraBoardId, SinaraConfiguration};

pub use channel_settings::BoosterChannelSettings;
pub use global_settings::BoosterSettings;

#[derive(serde::Serialize, serde::Deserialize, PartialEq)]
pub struct SemVersion {
    major: u8,
    minor: u8,
    patch: u8,
}
