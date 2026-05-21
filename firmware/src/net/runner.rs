use embassy_net::{Runner, Stack};
use embassy_time::{Duration, Timer};
use esp_println::println;
use esp_radio::wifi::{DisconnectReason, Interface, WifiController, WifiError};

use crate::nvs;

#[embassy_executor::task]
pub async fn net_task(mut runner: Runner<'static, Interface<'static>>) {
    runner.run().await
}

#[embassy_executor::task]
pub async fn wifi_connection_task(mut controller: WifiController<'static>) {
    println!("wifi: connection task");
    let mut ap_not_found = 0u8;
    loop {
        match controller.connect_async().await {
            Ok(info) => {
                ap_not_found = 0;
                println!("wifi: connected {:?}", info);
                let _ = controller.wait_for_disconnect_async().await;
                println!("wifi: disconnected");
            }
            Err(WifiError::Disconnected(info))
                if info.reason == DisconnectReason::NoAccessPointFound =>
            {
                ap_not_found = ap_not_found.saturating_add(1);
                println!(
                    "wifi: AP not found ({}/5) ssid={:?}",
                    ap_not_found,
                    info.ssid
                );
                if ap_not_found >= 5 {
                    println!("wifi: clearing stored credentials, rebooting to provision");
                    let _ = nvs::erase();
                    esp_hal::system::software_reset();
                }
            }
            Err(e) => {
                ap_not_found = 0;
                println!("wifi: connect error {:?}", e);
            }
        }
        Timer::after(Duration::from_secs(5)).await;
    }
}
