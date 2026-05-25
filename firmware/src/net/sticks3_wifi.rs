//! StickS3 WiFi bring-up tuned for USB-powered brownout avoidance.

use esp_radio::wifi::{Config, ControllerConfig};

/// Minimum `set_max_tx_power` (2 dBm) during association.
pub const TX_POWER_CONNECT: i8 = 8;
/// Nominal TX after link is up (~13 dBm).
pub const TX_POWER_RUN: i8 = 52;

/// Smaller buffer pools and no AMPDU during boot — less inrush when `esp_wifi_init` runs.
pub fn controller_config(initial: Config) -> ControllerConfig {
    ControllerConfig::default()
        .with_initial_config(initial)
        .with_static_rx_buf_num(6)
        .with_dynamic_rx_buf_num(16)
        .with_dynamic_tx_buf_num(16)
        .with_ampdu_rx_enable(false)
        .with_ampdu_tx_enable(false)
        .with_rx_ba_win(4)
}
