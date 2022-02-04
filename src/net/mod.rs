//! Booster network management definitions
//!
//! # Copyright
//! Copyright (C) 2020 QUARTIQ GmbH - All Rights Reserved
//! Unauthorized usage, editing, or copying is strictly prohibited.
//! Proprietary and confidential.
use crate::hardware::{clock::SystemTimer, NetworkStack};

use core::fmt::Write;
use heapless::String;

pub mod mqtt_control;
mod shared;

use shared::NetworkManager;

type NetworkStackProxy = shared::NetworkStackProxy<'static, NetworkStack>;

/// Container structure for holding all network devices.
///
/// # Note
/// All devices accessing the shared stack must be contained within a single structure to prevent
/// potential pre-emption when using the `shared` network stack.
pub struct NetworkDevices {
    pub control: mqtt_control::ControlClient,
    pub settings: miniconf::MqttClient<crate::RuntimeSettings, NetworkStackProxy, SystemTimer, 256>,

    // The stack reference is only used if the ENC424J600 PHY is used.
    #[allow(dead_code)]
    stack: NetworkStackProxy,
}

impl NetworkDevices {
    /// Construct all of Booster's Network devices.
    ///
    /// # Args
    /// * `broker` - The broker IP address for MQTT.
    /// * `stack` - The network stack to use for communications.
    /// * `identifier` - The unique identifier of this device.
    pub fn new(
        broker: minimq::embedded_nal::IpAddr,
        stack: NetworkStack,
        identifier: &str,
        settings: crate::RuntimeSettings,
    ) -> Self {
        let shared =
            cortex_m::singleton!(: NetworkManager<NetworkStack> = NetworkManager::new(stack))
                .unwrap();

        let mut miniconf_client: String<128> = String::new();
        write!(&mut miniconf_client, "booster-{}-settings", identifier).unwrap();

        let mut miniconf_prefix: String<128> = String::new();
        write!(&mut miniconf_prefix, "dt/sinara/booster/{}", identifier).unwrap();

        Self {
            control: mqtt_control::ControlClient::new(broker, shared.acquire_stack(), identifier),
            settings: miniconf::MqttClient::new(
                shared.acquire_stack(),
                &miniconf_client,
                &miniconf_prefix,
                broker,
                SystemTimer::default(),
                settings,
            )
            .unwrap(),
            stack: shared.acquire_stack(),
        }
    }

    /// Process the network stack.
    ///
    /// # Note
    /// This function must be called periodically to handle ingress/egress of packets and update
    /// state management.
    pub fn process(&mut self) -> bool {
        #[cfg(feature = "phy_enc424j600")]
        return self
            .stack
            .lock(|stack| stack.poll())
            .map_err(|_| Ok(true))
            .unwrap();

        false
    }
}
