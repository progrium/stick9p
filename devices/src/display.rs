//! Display framebuffer + ctl (Stage 2).

use core::cell::RefCell;
use critical_section::Mutex;
use heapless::String;

use crate::font8x8;

pub const WIDTH: usize = 135;
pub const HEIGHT: usize = 240;
pub const FB_LEN: usize = WIDTH * HEIGHT * 2;

struct DisplayState {
    fb: Option<&'static mut [u8; FB_LEN]>,
    brightness: u8,
    on: bool,
    dirty: bool,
    scale: u8,
}

static STATE: Mutex<RefCell<DisplayState>> = Mutex::new(RefCell::new(DisplayState {
    fb: None,
    brightness: 128,
    on: true,
    dirty: false,
    scale: 1,
}));

pub fn init(fb: &'static mut [u8; FB_LEN]) {
    fb.fill(0);
    critical_section::with(|cs| {
        let mut st = STATE.borrow(cs).borrow_mut();
        st.fb = Some(fb);
        st.brightness = 128;
        st.on = true;
        st.dirty = true;
        st.scale = 1;
    });
}

pub fn is_on() -> bool {
    critical_section::with(|cs| STATE.borrow(cs).borrow().on)
}

pub fn brightness() -> u8 {
    critical_section::with(|cs| STATE.borrow(cs).borrow().brightness)
}

pub fn scale() -> u8 {
    critical_section::with(|cs| STATE.borrow(cs).borrow().scale)
}

/// Run `f` with a view of the framebuffer (9P and SPI flush must not nest locks).
pub fn with_fb<R>(f: impl FnOnce(&[u8; FB_LEN]) -> R) -> Option<R> {
    critical_section::with(|cs| {
        let st = STATE.borrow(cs).borrow();
        st.fb.as_deref().map(f)
    })
}

pub fn take_dirty() -> bool {
    critical_section::with(|cs| {
        let mut st = STATE.borrow(cs).borrow_mut();
        let d = st.dirty;
        st.dirty = false;
        d
    })
}

pub fn read_fb(off: u64, buf: &mut [u8]) -> usize {
    critical_section::with(|cs| {
        let st = STATE.borrow(cs).borrow();
        let Some(fb) = st.fb.as_deref() else {
            return 0;
        };
        let off = off as usize;
        if off >= FB_LEN {
            return 0;
        }
        let n = (FB_LEN - off).min(buf.len());
        buf[..n].copy_from_slice(&fb[off..off + n]);
        n
    })
}

pub fn write_fb(off: u64, data: &[u8]) -> usize {
    critical_section::with(|cs| {
        let mut st = STATE.borrow(cs).borrow_mut();
        let Some(fb) = st.fb.as_deref_mut() else {
            return 0;
        };
        let off = off as usize;
        if off >= FB_LEN {
            return 0;
        }
        let n = (FB_LEN - off).min(data.len());
        fb[off..off + n].copy_from_slice(&data[..n]);
        // DESIGN: buffered until `flush`, except a write that reaches end-of-fb.
        if off + n >= FB_LEN {
            st.dirty = true;
        }
        n
    })
}

pub fn brightness_line() -> String<8> {
    critical_section::with(|cs| {
        let b = STATE.borrow(cs).borrow().brightness;
        u8_to_str(b)
    })
}

pub fn ctl_status_line() -> String<32> {
    let mut s = String::new();
    let _ = s.push_str("font=builtin\n");
    let _ = s.push_str("scale=");
    let scale = scale();
    let _ = s.push(if scale == 2 { '2' } else { '1' });
    let _ = s.push('\n');
    s
}

pub fn handle_brightness(s: &str) -> Result<(), &'static str> {
    let v: u8 = s.trim().parse().map_err(|_| "bad brightness")?;
    critical_section::with(|cs| {
        STATE.borrow(cs).borrow_mut().brightness = v.min(255);
    });
    Ok(())
}

pub fn handle_ctl(s: &str) -> Result<(), &'static str> {
    let s = s.trim();
    let mut pos = 0;
    while pos < s.len() {
        pos = skip_ascii_whitespace(s, pos);
        if pos >= s.len() {
            break;
        }
        if s[pos..].starts_with("text ") {
            pos = dispatch_text_cmd(s, pos)?;
        } else {
            let end = s[pos..].find('\n').map(|i| pos + i).unwrap_or(s.len());
            let line = s[pos..end].trim();
            if !line.is_empty() {
                handle_ctl_line(line)?;
            }
            pos = if end < s.len() { end + 1 } else { end };
        }
    }
    Ok(())
}

/// `text` payloads may contain `\n` (wrapped lines). Other ctl verbs are one line each.
fn dispatch_text_cmd(s: &str, start: usize) -> Result<usize, &'static str> {
    let after_verb = &s[start + 5..];
    let (x, y, r, g, b, payload_off) = parse_text_header(after_verb)?;
    let payload_start = start + 5 + payload_off;
    let payload_end = text_payload_end(s, payload_start);
    draw_text_parsed(x, y, r, g, b, &s[payload_start..payload_end])?;
    Ok(skip_ascii_whitespace(s, payload_end))
}

fn parse_text_header(rest: &str) -> Result<(i32, i32, u8, u8, u8, usize), &'static str> {
    let consumed_base = rest.as_ptr() as usize;
    let (x_str, rest) = rest.split_once(' ').ok_or("bad text")?;
    let (y_str, rest) = rest.split_once(' ').ok_or("bad text")?;
    let (color, rest) = rest.split_once(' ').ok_or("bad text")?;
    if color.len() != 6 {
        return Err("bad text");
    }
    let x: i32 = x_str.parse().map_err(|_| "bad text")?;
    let y: i32 = y_str.parse().map_err(|_| "bad text")?;
    let r = u8::from_str_radix(&color[0..2], 16).map_err(|_| "bad text")?;
    let g = u8::from_str_radix(&color[2..4], 16).map_err(|_| "bad text")?;
    let b = u8::from_str_radix(&color[4..6], 16).map_err(|_| "bad text")?;
    let payload_off = rest.as_ptr() as usize - consumed_base;
    Ok((x, y, r, g, b, payload_off))
}

/// End of a `text` string: next newline that begins another ctl verb, or EOF.
fn text_payload_end(s: &str, payload_start: usize) -> usize {
    let bytes = s.as_bytes();
    let mut i = payload_start;
    while i < bytes.len() {
        if bytes[i] == b'\n' {
            let mut j = i + 1;
            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            if j >= bytes.len() || line_starts_ctl(&s[j..]) {
                return i;
            }
        }
        i += 1;
    }
    s.len()
}

fn line_starts_ctl(s: &str) -> bool {
    let s = s.trim_start();
    if s.is_empty() {
        return false;
    }
    matches!(
        s,
        "on" | "off" | "flush" | "font builtin" | "scale 1" | "scale 2"
    ) || s.starts_with("fill ")
        || s.starts_with("text ")
        || s.starts_with("scale ")
        || s.starts_with("font ")
}

fn skip_ascii_whitespace(s: &str, mut pos: usize) -> usize {
    let bytes = s.as_bytes();
    while pos < bytes.len() && bytes[pos].is_ascii_whitespace() {
        pos += 1;
    }
    pos
}

/// Provisioning splash (firmware calls directly; same visuals as `text` ctl).
pub fn splash_provision(ssid: &str, password: &str) {
    critical_section::with(|cs| {
        let mut st = STATE.borrow(cs).borrow_mut();
        st.scale = 1;
        let Some(fb) = st.fb.as_deref_mut() else {
            return;
        };
        fill_fb(fb, 0, 0, 0);
        draw_text_scaled(fb, 4, 8, 0xff, 0xff, 0xff, "WiFi Setup", 2);
        let mut y = 40i32;
        let mut line = heapless::String::<40>::new();
        let _ = line.push_str("Network: ");
        let _ = line.push_str(ssid);
        draw_text_scaled(fb, 8, y, 0xff, 0xff, 0xff, line.as_str(), 1);
        y += line_height(1);
        line.clear();
        let _ = line.push_str("Password: ");
        let _ = line.push_str(password);
        draw_text_scaled(fb, 8, y, 0xff, 0xff, 0xff, line.as_str(), 1);
        y += line_height(1);
        draw_text_scaled(fb, 8, y, 0xff, 0xff, 0xff, "Open: http://192.168.4.1/", 1);
        st.dirty = true;
    });
}

/// Pre-WiFi banner shown immediately after the panel comes up.
pub fn splash_booting(board: &str) {
    critical_section::with(|cs| {
        let mut st = STATE.borrow(cs).borrow_mut();
        st.scale = 1;
        let Some(fb) = st.fb.as_deref_mut() else {
            return;
        };
        fill_fb(fb, 0, 0, 0);
        draw_text_scaled(fb, 4, 8, 0xff, 0xff, 0xff, "stick9p", 2);
        draw_text_scaled(fb, 4, 32, 0x80, 0xc0, 0xff, "booting...", 1);
        let mut line = heapless::String::<24>::new();
        let _ = line.push_str("board: ");
        let _ = line.push_str(board);
        draw_text_scaled(fb, 4, 48, 0xa0, 0xa0, 0xa0, line.as_str(), 1);
        st.dirty = true;
    });
}

/// STA-mode "ready" banner: green title + IP.
pub fn splash_ready(ip: &str) {
    critical_section::with(|cs| {
        let mut st = STATE.borrow(cs).borrow_mut();
        st.scale = 1;
        let Some(fb) = st.fb.as_deref_mut() else {
            return;
        };
        fill_fb(fb, 0, 0, 0);
        draw_text_scaled(fb, 4, 8, 0x00, 0xff, 0x00, "READY", 2);
        let mut line = heapless::String::<48>::new();
        let _ = line.push_str(ip);
        draw_text_scaled(fb, 4, 40, 0xff, 0xff, 0xff, line.as_str(), 1);
        draw_text_scaled(fb, 4, 72, 0xa0, 0xa0, 0xa0, "9p tcp/564", 1);
        draw_text_scaled(fb, 4, 88, 0xa0, 0xa0, 0xa0, "ws  /9p:8080", 1);
        st.dirty = true;
    });
}

fn handle_ctl_line(cmd: &str) -> Result<(), &'static str> {
    if cmd == "on" {
        critical_section::with(|cs| STATE.borrow(cs).borrow_mut().on = true);
        return Ok(());
    }
    if cmd == "off" {
        critical_section::with(|cs| STATE.borrow(cs).borrow_mut().on = false);
        return Ok(());
    }
    if cmd == "flush" {
        critical_section::with(|cs| STATE.borrow(cs).borrow_mut().dirty = true);
        return Ok(());
    }
    if cmd == "font builtin" {
        return Ok(());
    }
    if cmd == "scale 1" {
        critical_section::with(|cs| STATE.borrow(cs).borrow_mut().scale = 1);
        return Ok(());
    }
    if cmd == "scale 2" {
        critical_section::with(|cs| STATE.borrow(cs).borrow_mut().scale = 2);
        return Ok(());
    }
    if let Some(hex) = cmd.strip_prefix("fill ") {
        return handle_fill(hex.trim());
    }
    Err("unknown ctl")
}

fn handle_fill(hex: &str) -> Result<(), &'static str> {
    if hex.len() != 6 {
        return Err("bad fill");
    }
    let r = u8::from_str_radix(&hex[0..2], 16).map_err(|_| "bad fill")?;
    let g = u8::from_str_radix(&hex[2..4], 16).map_err(|_| "bad fill")?;
    let b = u8::from_str_radix(&hex[4..6], 16).map_err(|_| "bad fill")?;
    critical_section::with(|cs| {
        let mut st = STATE.borrow(cs).borrow_mut();
        if let Some(fb) = st.fb.as_deref_mut() {
            fill_fb(fb, r, g, b);
            st.dirty = true;
        }
        Ok(())
    })
}

fn draw_text_parsed(x: i32, y: i32, r: u8, g: u8, b: u8, text: &str) -> Result<(), &'static str> {
    critical_section::with(|cs| {
        let mut st = STATE.borrow(cs).borrow_mut();
        let scale = if st.scale == 2 { 2 } else { 1 };
        let Some(fb) = st.fb.as_deref_mut() else {
            return Err("no fb");
        };
        draw_text_scaled(fb, x, y, r, g, b, text, scale);
        st.dirty = true;
        Ok(())
    })
}

fn fill_fb(fb: &mut [u8; FB_LEN], r: u8, g: u8, b: u8) {
    let pixel = rgb565(r, g, b);
    for chunk in fb.chunks_exact_mut(2) {
        chunk.copy_from_slice(&pixel);
    }
}

fn line_height(scale: u8) -> i32 {
    (font8x8::GLYPH_H as i32) * (scale as i32)
}

fn draw_text_scaled(
    fb: &mut [u8; FB_LEN],
    mut x: i32,
    mut y: i32,
    r: u8,
    g: u8,
    b: u8,
    text: &str,
    scale: u8,
) {
    let scale = if scale == 2 { 2u8 } else { 1u8 };
    let origin_x = x;
    let color = rgb565(r, g, b);
    let advance = (font8x8::GLYPH_W as i32) * (scale as i32);

    for ch in text.chars() {
        if ch == '\n' {
            x = origin_x;
            y += line_height(scale);
            continue;
        }
        let b = ch as u8;
        if !(font8x8::FIRST..=font8x8::LAST).contains(&b) {
            continue;
        }
        if let Some(glyph) = font8x8::glyph(b) {
            blit_glyph(fb, x, y, glyph, color, scale);
        }
        x += advance;
    }
}

fn blit_glyph(fb: &mut [u8; FB_LEN], x: i32, y: i32, glyph: &[u8; 8], color: [u8; 2], scale: u8) {
    let scale = scale as i32;
    for (row, bits) in glyph.iter().enumerate() {
        for col in 0..8 {
            if bits & (1 << (7 - col)) == 0 {
                continue;
            }
            let px = x + col * scale;
            let py = y + (row as i32) * scale;
            for dy in 0..scale {
                for dx in 0..scale {
                    put_pixel(fb, px + dx, py + dy, color);
                }
            }
        }
    }
}

fn put_pixel(fb: &mut [u8; FB_LEN], x: i32, y: i32, color: [u8; 2]) {
    if x < 0 || y < 0 {
        return;
    }
    let x = x as usize;
    let y = y as usize;
    if x >= WIDTH || y >= HEIGHT {
        return;
    }
    let off = (y * WIDTH + x) * 2;
    fb[off..off + 2].copy_from_slice(&color);
}

fn rgb565(r: u8, g: u8, b: u8) -> [u8; 2] {
    let v = (((r as u16) >> 3) << 11) | (((g as u16) >> 2) << 5) | ((b as u16) >> 3);
    v.to_le_bytes()
}

fn u8_to_str(mut n: u8) -> String<8> {
    let mut s = String::new();
    if n == 0 {
        let _ = s.push('0');
        return s;
    }
    let mut digits = heapless::Vec::<u8, 4>::new();
    while n > 0 {
        let _ = digits.push((n % 10) as u8 + b'0');
        n /= 10;
    }
    while let Some(d) = digits.pop() {
        let _ = s.push(d as char);
    }
    s
}
