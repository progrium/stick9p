use core::convert::Infallible;

use embassy_net::tcp::TcpSocket;
use embedded_io_async::{ErrorType, Read, Write};

const WS_GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

pub struct WsIo<'a> {
    pub socket: TcpSocket<'a>,
    frame: &'static mut [u8; 2048],
    frame_len: usize,
}

impl<'a> WsIo<'a> {
    pub fn new(socket: TcpSocket<'a>, frame: &'static mut [u8; 2048]) -> Self {
        Self {
            socket,
            frame,
            frame_len: 0,
        }
    }

    pub async fn handshake(&mut self) -> Result<(), ()> {
        let mut buf = [0u8; 1024];
        let n = self.socket.read(&mut buf).await.map_err(|_| ())?;
        let req = core::str::from_utf8(&buf[..n]).map_err(|_| ())?;
        if !req.to_ascii_lowercase().contains("upgrade: websocket") {
            return Err(());
        }
        let key = extract_header(req, "sec-websocket-key:").ok_or(())?;
        let accept = accept_key(key)?;
        let mut resp = heapless::String::<256>::new();
        {
            use core::fmt::Write;
            write!(
                resp,
                "HTTP/1.1 101 Switching Protocols\r\n\
                 Upgrade: websocket\r\n\
                 Connection: Upgrade\r\n\
                 Sec-WebSocket-Accept: {}\r\n\r\n",
                accept.as_str()
            )
            .map_err(|_| ())?;
        }
        self.socket
            .write_all(resp.as_bytes())
            .await
            .map_err(|_| ())?;
        Ok(())
    }

    async fn read_frame(&mut self) -> Result<&[u8], ()> {
        let mut hdr = [0u8; 2];
        read_exact(&mut self.socket, &mut hdr).await?;
        let len = (hdr[1] & 0x7f) as usize;
        if len == 0 {
            self.frame_len = 0;
            return Ok(&[]);
        }
        if len > self.frame.len() {
            return Err(());
        }
        read_exact(&mut self.socket, &mut self.frame[..len]).await?;
        self.frame_len = len;
        Ok(&self.frame[..len])
    }

    async fn write_frame(&mut self, data: &[u8]) -> Result<(), ()> {
        let mut hdr = [0u8; 10];
        let mut n = 2usize;
        hdr[0] = 0x82;
        if data.len() < 126 {
            hdr[1] = data.len() as u8;
        } else if data.len() <= 65535 {
            hdr[1] = 126;
            hdr[2..4].copy_from_slice(&(data.len() as u16).to_be_bytes());
            n = 4;
        } else {
            return Err(());
        }
        write_all(&mut self.socket, &hdr[..n]).await?;
        write_all(&mut self.socket, data).await?;
        Ok(())
    }
}

impl<'a> ErrorType for WsIo<'a> {
    type Error = Infallible;
}

impl<'a> Read for WsIo<'a> {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        let frame = self.read_frame().await.unwrap_or(&[]);
        let n = frame.len().min(buf.len());
        buf[..n].copy_from_slice(&frame[..n]);
        Ok(n)
    }
}

impl<'a> Write for WsIo<'a> {
    async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        let _ = self.write_frame(buf).await;
        Ok(buf.len())
    }

    async fn flush(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}

async fn read_exact(socket: &mut TcpSocket<'_>, buf: &mut [u8]) -> Result<(), ()> {
    let mut pos = 0;
    while pos < buf.len() {
        let n = socket.read(&mut buf[pos..]).await.map_err(|_| ())?;
        if n == 0 {
            return Err(());
        }
        pos += n;
    }
    Ok(())
}

async fn write_all(socket: &mut TcpSocket<'_>, buf: &[u8]) -> Result<(), ()> {
    let mut pos = 0;
    while pos < buf.len() {
        let n = socket.write(&buf[pos..]).await.map_err(|_| ())?;
        if n == 0 {
            return Err(());
        }
        pos += n;
    }
    Ok(())
}

fn extract_header<'a>(req: &'a str, name: &str) -> Option<&'a str> {
    for line in req.lines() {
        let l = line.trim();
        if l.to_ascii_lowercase().starts_with(name) {
            return l.split_once(':').map(|(_, v)| v.trim());
        }
    }
    None
}

fn accept_key(key: &str) -> Result<heapless::String<64>, ()> {
    use sha1::{Digest, Sha1};
    let mut s = heapless::String::<128>::new();
    s.push_str(key).map_err(|_| ())?;
    s.push_str(WS_GUID).map_err(|_| ())?;
    let hash = Sha1::digest(s.as_bytes());
    Ok(base64_encode(&hash))
}

fn base64_encode(data: &[u8]) -> heapless::String<64> {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = heapless::String::new();
    let mut i = 0;
    while i + 3 <= data.len() {
        let n = ((data[i] as u32) << 16) | ((data[i + 1] as u32) << 8) | data[i + 2] as u32;
        let _ = out.push(TABLE[((n >> 18) & 63) as usize] as char);
        let _ = out.push(TABLE[((n >> 12) & 63) as usize] as char);
        let _ = out.push(TABLE[((n >> 6) & 63) as usize] as char);
        let _ = out.push(TABLE[(n & 63) as usize] as char);
        i += 3;
    }
    let rem = data.len() - i;
    if rem == 1 {
        let n = (data[i] as u32) << 16;
        let _ = out.push(TABLE[((n >> 18) & 63) as usize] as char);
        let _ = out.push(TABLE[((n >> 12) & 63) as usize] as char);
        let _ = out.push('=');
        let _ = out.push('=');
    } else if rem == 2 {
        let n = ((data[i] as u32) << 16) | ((data[i + 1] as u32) << 8);
        let _ = out.push(TABLE[((n >> 18) & 63) as usize] as char);
        let _ = out.push(TABLE[((n >> 12) & 63) as usize] as char);
        let _ = out.push(TABLE[((n >> 6) & 63) as usize] as char);
        let _ = out.push('=');
    }
    out
}
