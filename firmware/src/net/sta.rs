//! WiFi station mode + 9P services.

use embassy_executor::Spawner;
use embassy_net::StackResources;
use embassy_time::Timer;
use esp_hal::rng::Rng;
use esp_println::println;
use esp_radio::wifi::{sta::StationConfig, Config, ControllerConfig};

use crate::mk_static;
use crate::nvs::WifiConfig;
use crate::net::runner::{net_task, wifi_connection_task};
use crate::net::services::{log_ip, spawn_sta_services};

pub async fn run(
    spawner: &Spawner,
    wifi: esp_hal::peripherals::WIFI<'static>,
    cfg: WifiConfig,
) -> ! {
    println!("sta: connecting to {}", cfg.ssid.as_str());

    let station = Config::Station(
        StationConfig::default()
            .with_ssid(cfg.ssid.as_str())
            .with_password(cfg.password.as_str().into()),
    );

    let (controller, interfaces) = esp_radio::wifi::new(
        wifi,
        ControllerConfig::default().with_initial_config(station),
    )
    .expect("wifi sta");

    let net_cfg = embassy_net::Config::dhcpv4(Default::default());
    let rng = Rng::new();
    let seed = (rng.random() as u64) << 32 | rng.random() as u64;
    let (stack, runner) = embassy_net::new(
        interfaces.station,
        net_cfg,
        mk_static!(StackResources<6>, StackResources::<6>::new()),
        seed,
    );

    spawner.spawn(net_task(runner).unwrap());
    spawner.spawn(wifi_connection_task(controller).unwrap());

    println!("sta: waiting for DHCP...");
    stack.wait_config_up().await;
    log_ip(&stack);
    println!("sta: 9P tcp/564  ws/8080/9p");

    spawn_sta_services(spawner, stack);

    loop {
        Timer::after(embassy_time::Duration::from_secs(3600)).await;
    }
}
