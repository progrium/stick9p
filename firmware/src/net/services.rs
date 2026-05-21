use embassy_executor::Spawner;
use embassy_net::{IpListenEndpoint, Stack, tcp::TcpSocket};
use embassy_time::{Duration, Timer};
use esp_println::println;
use ninep::fs::FsContext;
use ninep::server::Session;

use crate::board::{BOARD_NAME, FW_VERSION};
use crate::net::buffers;
use devices::{buttons, buzzer, display, imu, led, mic, power};

pub const TCP_9P_PORT: u16 = 564;
pub const WS_PORT: u16 = 8080;

pub const GW_PROVISION: core::net::Ipv4Addr = core::net::Ipv4Addr::new(192, 168, 4, 1);

fn fs_context() -> FsContext<'static> {
    FsContext {
        board_name: BOARD_NAME,
        version: FW_VERSION,
        uptime_ms: || embassy_time::Instant::now().as_millis(),
        led_state_line: led::state_line,
        on_led_ctl: |s| led::handle_ctl(s).map_err(|_| "bad ctl"),
        request_reboot: || {
            println!("sys: reboot");
            esp_hal::system::software_reset();
        },
        read_display_fb: display::read_fb,
        write_display_fb: display::write_fb,
        on_display_ctl: display::handle_ctl,
        on_display_brightness: display::handle_brightness,
        read_imu_accel: imu::read_accel,
        read_imu_gyro: imu::read_gyro,
        on_imu_ctl: imu::handle_ctl,
        read_btn_a: buttons::read_a,
        read_btn_b: buttons::read_b,
        read_btn_event: buttons::try_read_event,
        on_buttons_ctl: buttons::handle_ctl,
        read_power_battery: |off, buf| {
            copy_line(off, buf, &power::battery_line())
        },
        read_power_vbat: |off, buf| copy_line(off, buf, &power::vbat_line()),
        on_power_ctl: power::handle_ctl,
        on_buzzer_ctl: buzzer::handle_ctl,
        read_mic_pcm: mic::try_read_pcm,
        on_mic_ctl: mic::handle_ctl,
    }
}

fn copy_line(off: u64, buf: &mut [u8], line: &str) -> usize {
    if off >= line.len() as u64 {
        return 0;
    }
    let start = off as usize;
    let n = (line.len() - start).min(buf.len());
    buf[..n].copy_from_slice(&line.as_bytes()[start..start + n]);
    n
}

#[embassy_executor::task]
pub async fn ninep_tcp_server(stack: Stack<'static>) {
    let bufs = buffers::tcp_9p();
    loop {
        let mut socket = TcpSocket::new(stack, &mut bufs.rx, &mut bufs.tx);
        socket.set_timeout(None);
        println!("9p: waiting tcp/{}", TCP_9P_PORT);
        match socket
            .accept(IpListenEndpoint {
                addr: None,
                port: TCP_9P_PORT,
            })
            .await
        {
            Ok(()) => println!("9p: tcp connected"),
            Err(e) => {
                println!("9p: accept err {:?}", e);
                Timer::after(Duration::from_secs(1)).await;
                continue;
            }
        }
        let storage = buffers::ninep_tcp_storage();
        Session::new(socket, fs_context(), storage).run().await;
        println!("9p: tcp session ended (client closed or protocol error)");
    }
}

#[embassy_executor::task]
pub async fn ninep_ws_server(stack: Stack<'static>) {
    let bufs = buffers::tcp_ws();
    loop {
        let mut socket = TcpSocket::new(stack, &mut bufs.rx, &mut bufs.tx);
        socket.set_timeout(None);
        if socket
            .accept(IpListenEndpoint {
                addr: None,
                port: WS_PORT,
            })
            .await
            .is_err()
        {
            Timer::after(Duration::from_secs(1)).await;
            continue;
        }
        println!("9p: ws connected");
        let frame = buffers::ws_frame();
        let mut ws = crate::transport::ws::WsIo::new(socket, frame);
        if ws.handshake().await.is_err() {
            ws.socket.abort();
            continue;
        }
        let storage = buffers::ninep_ws_storage();
        Session::new(ws, fs_context(), storage).run().await;
    }
}

pub fn spawn_sta_services(spawner: &Spawner, stack: Stack<'static>) {
    spawner.spawn(ninep_tcp_server(stack).unwrap());
    spawner.spawn(ninep_ws_server(stack).unwrap());
}

pub fn log_ip(stack: &Stack<'static>) {
    if let Some(cfg) = stack.config_v4() {
        println!("net: ip {}", cfg.address);
    }
}
