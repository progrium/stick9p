//! Soft-AP + captive portal WiFi setup (DESIGN.md §4.8).

use core::net::{Ipv4Addr, SocketAddrV4};

use embassy_executor::Spawner;
use embassy_net::{
    tcp::TcpSocket, IpListenEndpoint, Ipv4Address, Ipv4Cidr, Stack, StackResources,
    StaticConfigV4, udp::{PacketMetadata, UdpSocket},
};
use embassy_futures::select::select;
use embassy_time::{Duration, Instant, Timer, WithTimeout};
use embedded_io_async::Write;
use esp_hal::rng::Rng;
use esp_println::println;
use esp_radio::wifi::{ap::AccessPointConfig, AuthenticationMethod, Config};

#[cfg(not(feature = "board-sticks3"))]
use esp_radio::wifi::ControllerConfig;

#[cfg(feature = "board-sticks3")]
use crate::net::sticks3_wifi;

use devices::display;

use crate::mk_static;
use crate::net::buffers;
use crate::net::runner::net_task;
use crate::net::services::GW_PROVISION;
use crate::nvs::{self, WifiConfig};

const SETUP_HTML: &str = r#"<!DOCTYPE html><html><head><meta charset=utf-8><meta name=viewport content="width=device-width"><title>stick9p WiFi</title></head><body><h1>stick9p WiFi Setup</h1><form method=POST action="/save"><label>SSID <input name=ssid size=32 required></label><br><label>Password <input name=pass type=password size=64></label><br><button type=submit>Save &amp; Reboot</button></form></body></html>"#;

pub struct ProvisionInfo {
    pub ssid: heapless::String<24>,
    pub password: heapless::String<16>,
}

pub async fn run(spawner: &Spawner, wifi: esp_hal::peripherals::WIFI<'static>) -> ! {
    #[cfg(feature = "board-sticks3")]
    crate::boot_gate::set_provisioning(true);

    let info = ap_credentials();
    println!("provision: AP {} / {}", info.ssid.as_str(), info.password.as_str());
    println!("provision: join AP, open http://192.168.4.1/");
    display::splash_provision(info.ssid.as_str(), info.password.as_str());

    let ap_cfg = Config::AccessPoint(
        AccessPointConfig::default()
            .with_ssid(info.ssid.as_str())
            .with_auth_method(AuthenticationMethod::Wpa2Personal)
            .with_password(info.password.as_str().into()),
    );

    #[cfg(feature = "board-sticks3")]
    let (mut controller, interfaces) =
        esp_radio::wifi::new(wifi, sticks3_wifi::controller_config(ap_cfg)).expect("wifi ap");
    #[cfg(not(feature = "board-sticks3"))]
    let (mut controller, interfaces) = esp_radio::wifi::new(
        wifi,
        ControllerConfig::default().with_initial_config(ap_cfg),
    )
    .expect("wifi ap");

    #[cfg(feature = "board-sticks3")]
    {
        if let Err(e) = controller.set_max_tx_power(sticks3_wifi::TX_POWER_AP) {
            println!("provision: tx cap err {:?}", e);
        } else {
            println!("provision: tx capped for AP (lean buffers)");
        }
    }

    let gw_addr = Ipv4Address::new(192, 168, 4, 1);
    let mut dns_servers = heapless::Vec::<Ipv4Address, 3>::new();
    let _ = dns_servers.push(gw_addr);
    let config = embassy_net::Config::ipv4_static(StaticConfigV4 {
        address: Ipv4Cidr::new(gw_addr, 24),
        gateway: Some(gw_addr),
        dns_servers,
    });

    let rng = Rng::new();
    let seed = (rng.random() as u64) << 32 | rng.random() as u64;
    let (stack, runner) = embassy_net::new(
        interfaces.access_point,
        config,
        mk_static!(StackResources<7>, StackResources::<7>::new()),
        seed,
    );

    spawner.spawn(net_task(runner).unwrap());
    stack.wait_config_up().await;
    if let Some(cfg) = stack.config_v4() {
        println!("provision: net up {}", cfg.address);
    }
    #[cfg(feature = "board-sticks3")]
    {
        // Let the AP stack settle before L3B + ST7789 (shared brownout trigger with STA).
        println!("provision: settling before display rail…");
        Timer::after(Duration::from_millis(2000)).await;
        println!("boot: network ready (provision AP)");
        println!("boot: provision mode — amp/fanfare disabled (avoid brownout)");
        crate::boot_gate::signal_network_ready();
        // Framebuffer was filled before L3B; refresh now that the panel can flush.
        display::splash_provision(info.ssid.as_str(), info.password.as_str());
    }
    spawner.spawn(dhcp_task(stack).unwrap());
    spawner.spawn(captive_dns(stack).unwrap());
    spawner.spawn(http_portal(stack).unwrap());

    loop {
        Timer::after(Duration::from_secs(60)).await;
    }
}

fn ap_credentials() -> ProvisionInfo {
    // Derive stable SSID + password from the efuse MAC so the credentials don't
    // change across reboots. SSID suffix = last 2 MAC bytes; password = 8 chars
    // mixed from all 6 MAC bytes.
    let mac = esp_hal::efuse::base_mac_address();
    let bytes = mac.as_bytes();

    let mut ssid = heapless::String::<24>::new();
    let _ = ssid.push_str("Stick9p-");
    let suffix = ((bytes[4] as u16) << 8) | bytes[5] as u16;
    push_hex4(suffix, &mut ssid);

    const CHARSET: &[u8] = b"abcdefghjkmnpqrstuvwxyz23456789";
    let mut password = heapless::String::<16>::new();
    // Simple deterministic mixer: rolling FNV-like hash seeded with a constant.
    let mut h: u32 = 0x9e37_79b9;
    for _ in 0..8 {
        for &b in bytes {
            h = h.wrapping_mul(0x0100_0193) ^ b as u32;
        }
        let idx = (h as usize) % CHARSET.len();
        let _ = password.push(CHARSET[idx] as char);
        h = h.wrapping_add(0x6c07_8965);
    }

    ProvisionInfo { ssid, password }
}

fn push_hex4(v: u16, s: &mut heapless::String<24>) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let _ = s.push(HEX[(v >> 12) as usize] as char);
    let _ = s.push(HEX[((v >> 8) & 0xf) as usize] as char);
    let _ = s.push(HEX[((v >> 4) & 0xf) as usize] as char);
    let _ = s.push(HEX[(v & 0xf) as usize] as char);
}

#[embassy_executor::task]
async fn dhcp_task(stack: Stack<'static>) {
    use edge_dhcp::io;
    use edge_dhcp::server::{Server, ServerOptions};
    use edge_nal::UdpBind;
    use edge_nal_embassy::Udp;

    let ip = GW_PROVISION;
    let gw_buf = buffers::dhcp_gw();
    let opts = ServerOptions::new(ip, Some(gw_buf));

    let buf = buffers::dhcp_packet();
    let socket = Udp::new(stack, buffers::edge_udp_buffers());
    let mut bound = socket
        .bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, io::DEFAULT_SERVER_PORT).into())
        .await
        .unwrap();
    println!("provision: dhcp server on :{}", io::DEFAULT_SERVER_PORT);
    loop {
        let _ = io::server::run(
            &mut Server::<_, 64>::new_with_et(ip),
            &opts,
            &mut bound,
            &mut buf[..],
        )
        .await;
        Timer::after(Duration::from_millis(200)).await;
    }
}

#[embassy_executor::task]
async fn captive_dns(stack: Stack<'static>) {
    let mut rx_meta = [PacketMetadata::EMPTY; 2];
    let mut tx_meta = [PacketMetadata::EMPTY; 2];
    let rx = buffers::dns_rx();
    let tx = buffers::dns_tx();
    let mut sock = UdpSocket::new(stack, &mut rx_meta, rx, &mut tx_meta, tx);
    sock.bind(53).ok();
    let gw = GW_PROVISION.octets();
    loop {
        let mut buf = [0u8; 512];
        if let Ok((n, ep)) = sock.recv_from(&mut buf).await {
            if n < 12 {
                continue;
            }
            let mut out = [0u8; 512];
            out[..n.min(512)].copy_from_slice(&buf[..n.min(512)]);
            out[2] = 0x81;
            out[3] = 0x80;
            if n > 12 {
                out[7] = buf[7];
            }
            let qend = 12 + query_len(&buf[12..n]) + 4;
            if qend + 16 <= out.len() {
                out[qend] = 0xc0;
                out[qend + 1] = 0x0c;
                out[qend + 2] = 0x00;
                out[qend + 3] = 0x01;
                out[qend + 4] = 0x00;
                out[qend + 5] = 0x01;
                out[qend + 6] = 0x00;
                out[qend + 7] = 0x3c;
                out[qend + 8] = 0x00;
                out[qend + 9] = 0x04;
                out[qend + 10] = gw[0];
                out[qend + 11] = gw[1];
                out[qend + 12] = gw[2];
                out[qend + 13] = gw[3];
                let _ = sock.send_to(&out[..qend + 14], ep).await;
            }
        }
    }
}

fn query_len(q: &[u8]) -> usize {
    let mut i = 0;
    while i < q.len() {
        let l = q[i] as usize;
        if l == 0 {
            return i + 1;
        }
        i += 1 + l;
    }
    0
}

#[embassy_executor::task]
async fn http_portal(stack: Stack<'static>) {
    stack.wait_config_up().await;
    println!("provision: http listening on :80 (2 slots)");
    #[cfg(feature = "board-sticks3")]
    crate::boot_gate::mark_subsystem_ready(crate::boot_gate::SUBSYS_NET9P);

    loop {
        select(
            http_listen_once(stack, 0),
            http_listen_once(stack, 1),
        )
        .await;
    }
}

async fn http_listen_once(stack: Stack<'static>, slot: usize) {
    let req_buf = buffers::http_req(slot);
    let bufs = buffers::tcp_http(slot);
    let mut socket = TcpSocket::new(stack, &mut bufs.rx, &mut bufs.tx);
    socket.set_timeout(Some(Duration::from_secs(30)));

    if socket
        .accept(IpListenEndpoint {
            addr: None,
            port: 80,
        })
        .await
        .is_err()
    {
        Timer::after(Duration::from_millis(200)).await;
        return;
    }

    let res = buffers::http_res();
    serve_http(&mut socket, res, req_buf).await;
}

async fn serve_http(socket: &mut TcpSocket<'_>, tx: &mut [u8], req_buf: &mut [u8]) {
    let n = read_http_request(socket, req_buf).await;
    let req = core::str::from_utf8(&req_buf[..n]).unwrap_or("");

    if is_post_save(req) {
        if let Some(body) = request_body(req) {
            if let Some(cfg) = parse_form(body) {
                match nvs::save(&cfg) {
                    Ok(()) => {
                        println!("provision: saved ssid={}", cfg.ssid.as_str());
                        let body = b"Saved. Rebooting...\n";
                        let _ = send_http_response(socket, tx, 200, "text/plain", body).await;
                        let _ = socket.flush().await;
                        display::splash_booting("rebooting");
                        Timer::after(Duration::from_millis(500)).await;
                        println!("provision: software reset");
                        esp_hal::system::software_reset();
                    }
                    Err(()) => {
                        println!("provision: nvs save failed");
                        let body = b"Save failed - try again\n";
                        let _ = send_http_response(socket, tx, 500, "text/plain", body).await;
                    }
                }
                return;
            }
            println!("provision: bad form body");
        } else {
            println!("provision: POST /save missing body");
        }
    }

    if req.starts_with("HEAD ") {
        let _ = write_html_headers_only(socket, tx, SETUP_HTML.len()).await;
    } else if req.contains("favicon.ico") {
        let empty = b"";
        let _ = send_http_response(socket, tx, 204, "text/plain", empty).await;
    } else {
        let body = SETUP_HTML.as_bytes();
        let _ = send_html_page(socket, tx, body).await;
    }

    let _ = socket.flush().await;
    drain_socket(socket).await;
    Timer::after(Duration::from_millis(50)).await;
}

async fn read_http_request(socket: &mut TcpSocket<'_>, buf: &mut [u8]) -> usize {
    let mut n = 0usize;
    let deadline = Instant::now() + Duration::from_millis(2500);
    while n < buf.len() && Instant::now() < deadline {
        let chunk = match socket
            .read(&mut buf[n..])
            .with_timeout(Duration::from_millis(300))
            .await
        {
            Ok(Ok(k)) => k,
            _ => break,
        };
        if chunk == 0 {
            break;
        }
        n += chunk;
        if request_complete(&buf[..n]) {
            break;
        }
    }
    n
}

fn is_post_save(req: &str) -> bool {
    let Some(line) = req.split("\r\n").next() else {
        return false;
    };
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or("");
    let target = parts.next().unwrap_or("");
    if method != "POST" {
        return false;
    }
    target == "/save"
        || target.starts_with("/save?")
        || target.ends_with("/save")
        || target.contains("/save")
}

fn request_body<'a>(req: &'a str) -> Option<&'a str> {
    req.split("\r\n\r\n").nth(1).filter(|b| !b.is_empty())
}

fn request_complete(buf: &[u8]) -> bool {
    let Ok(req) = core::str::from_utf8(buf) else {
        return false;
    };
    let Some(headers_end) = req.find("\r\n\r\n") else {
        return false;
    };
    let headers = &req[..headers_end];
    let body_start = headers_end + 4;
    let body_len = req.len().saturating_sub(body_start);
    if let Some(cl) = header_value(headers, "content-length") {
        if let Ok(need) = cl.trim().parse::<usize>() {
            return body_len >= need;
        }
    }
    // No Content-Length: enough for GET/HEAD once headers arrived.
    !is_post_save(req) || body_len > 0
}

fn header_value<'a>(headers: &'a str, name: &str) -> Option<&'a str> {
    for line in headers.lines().skip(1) {
        if let Some((k, v)) = line.split_once(':') {
            if k.trim().eq_ignore_ascii_case(name) {
                return Some(v.trim());
            }
        }
    }
    None
}

async fn drain_socket(socket: &mut TcpSocket<'_>) {
    let mut junk = [0u8; 64];
    loop {
        match socket
            .read(&mut junk)
            .with_timeout(Duration::from_millis(50))
            .await
        {
            Ok(Ok(0)) | Ok(Err(_)) | Err(_) => break,
            Ok(Ok(_)) => {}
        }
    }
}

fn write_usize(out: &mut [u8], mut val: usize) -> usize {
    let mut tmp = [0u8; 10];
    let mut i = 0;
    if val == 0 {
        tmp[0] = b'0';
        i = 1;
    } else {
        while val > 0 {
            tmp[i] = b'0' + (val % 10) as u8;
            val /= 10;
            i += 1;
        }
        tmp[..i].reverse();
    }
    out[..i].copy_from_slice(&tmp[..i]);
    i
}

async fn send_http_response(
    socket: &mut TcpSocket<'_>,
    tx: &mut [u8],
    code: u16,
    content_type: &str,
    body: &[u8],
) -> Result<(), ()> {
    let status = match code {
        200 => "200 OK",
        204 => "204 No Content",
        _ => "200 OK",
    };
    let mut pos = 0usize;
    let prefix = b"HTTP/1.1 ";
    tx[pos..pos + prefix.len()].copy_from_slice(prefix);
    pos += prefix.len();
    let st = status.as_bytes();
    tx[pos..pos + st.len()].copy_from_slice(st);
    pos += st.len();
    let ct = b"\r\nContent-Type: ";
    tx[pos..pos + ct.len()].copy_from_slice(ct);
    pos += ct.len();
    let ctb = content_type.as_bytes();
    tx[pos..pos + ctb.len()].copy_from_slice(ctb);
    pos += ctb.len();
    let cl = b"\r\nConnection: close\r\nContent-Length: ";
    tx[pos..pos + cl.len()].copy_from_slice(cl);
    pos += cl.len();
    pos += write_usize(&mut tx[pos..], body.len());
    tx[pos..pos + 4].copy_from_slice(b"\r\n\r\n");
    pos += 4;
    if pos + body.len() > tx.len() {
        return Err(());
    }
    if !body.is_empty() {
        tx[pos..pos + body.len()].copy_from_slice(body);
        pos += body.len();
    }
    socket.write_all(&tx[..pos]).await.map_err(|_| ())
}

async fn send_html_page(socket: &mut TcpSocket<'_>, tx: &mut [u8], body: &[u8]) -> Result<(), ()> {
    send_http_response(socket, tx, 200, "text/html; charset=utf-8", body).await
}

async fn write_html_headers_only(
    socket: &mut TcpSocket<'_>,
    tx: &mut [u8],
    body_len: usize,
) -> Result<(), ()> {
    let mut pos = 0usize;
    let head =
        b"HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nConnection: close\r\nContent-Length: ";
    tx[pos..pos + head.len()].copy_from_slice(head);
    pos += head.len();
    pos += write_usize(&mut tx[pos..], body_len);
    tx[pos..pos + 4].copy_from_slice(b"\r\n\r\n");
    pos += 4;
    socket.write_all(&tx[..pos]).await.map_err(|_| ())
}

fn parse_form(body: &str) -> Option<WifiConfig> {
    let mut ssid = heapless::String::<32>::new();
    let mut password = heapless::String::<64>::new();
    for part in body.split('&') {
        if let Some(v) = part.strip_prefix("ssid=") {
            let _ = urldecode_form(v.trim(), &mut ssid);
        } else if let Some(v) = part.strip_prefix("pass=") {
            let _ = urldecode_form(v.trim(), &mut password);
        }
    }
    if ssid.is_empty() {
        return None;
    }
    Some(WifiConfig { ssid, password })
}

/// Decode `application/x-www-form-urlencoded` values (`+` → space, `%XX` hex).
fn urldecode_form<const N: usize>(
    src: &str,
    out: &mut heapless::String<N>,
) -> Result<(), ()> {
    let bytes = src.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(' ').map_err(|_| ())?;
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                let hi = hex_nibble(bytes[i + 1])?;
                let lo = hex_nibble(bytes[i + 2])?;
                out.push(char::from(hi << 4 | lo)).map_err(|_| ())?;
                i += 3;
            }
            b => {
                out.push(b as char).map_err(|_| ())?;
                i += 1;
            }
        }
    }
    Ok(())
}

fn hex_nibble(b: u8) -> Result<u8, ()> {
    match b {
        b'0'..=b'9' => Ok(b - b'0'),
        b'a'..=b'f' => Ok(b - b'a' + 10),
        b'A'..=b'F' => Ok(b - b'A' + 10),
        _ => Err(()),
    }
}
