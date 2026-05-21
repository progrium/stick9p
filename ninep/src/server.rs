//! 9P2000 session handler.

use crate::buffers::{self, SessionStorage};
use crate::fs::{self, FsContext, Node};
use crate::wire::{
    self, decode_tmsg, put_u16, put_u32, put_u8, write_rerror, write_rversion_str,
    Tmsg, RATTACH, RCLUNK, RCREATE, ROPEN, RREAD, RSTAT, RWALK, RWSTAT, RWRITE,
    DEFAULT_MSIZE,
};
use devices::{buttons, mic};
use embedded_io_async::{Read, Write};
use embassy_time::{Duration, Instant, Timer};

#[cfg(feature = "log")]
macro_rules! ninep_log {
    ($($t:tt)*) => { esp_println::println!($($t)*); };
}
#[cfg(not(feature = "log"))]
macro_rules! ninep_log {
    ($($t:tt)*) => {};
}

const MAX_FIDS: usize = 32;
const EVENT_POLL_MS: u64 = 25;

struct PendingStreamRead {
    tag: u16,
    count: u32,
}

enum PendingKind {
    BtnEvent,
    MicPcm,
}

enum DispatchResult {
    Reply(usize),
    WaitStream { kind: PendingKind, tag: u16, count: u32 },
}

struct FidSlot {
    in_use: bool,
    node: Node,
    open: bool,
}

impl FidSlot {
    const EMPTY: Self = Self {
        in_use: false,
        node: Node::Root,
        open: false,
    };
}

pub struct Session<'a, S> {
    stream: S,
    ctx: FsContext<'a>,
    msize: u32,
    fids: [FidSlot; MAX_FIDS],
    storage: &'a mut SessionStorage,
    pending_stream: Option<(PendingKind, PendingStreamRead)>,
}

impl<'a, S> Session<'a, S>
where
    S: Read + Write,
{
    pub fn new(stream: S, ctx: FsContext<'a>, storage: &'a mut SessionStorage) -> Self {
        Self {
            stream,
            ctx,
            msize: DEFAULT_MSIZE,
            fids: [FidSlot::EMPTY; MAX_FIDS],
            storage,
            pending_stream: None,
        }
    }

    pub async fn run(mut self) {
        loop {
            if let Some((kind, p)) = self.pending_stream.as_ref() {
                let max_count = max_read_count(p.count, self.msize);
                let buf = &mut self.storage.rx[..max_count];
                let n = match kind {
                    PendingKind::BtnEvent => buttons::try_read_event(0, buf),
                    PendingKind::MicPcm => mic::try_read_pcm(0, buf),
                };
                if n > 0 {
                    let reply_len = build_read_reply(&mut self.storage.tx, p.tag, &buf[..n]);
                    if write_all(&mut self.stream, &self.storage.tx[..reply_len])
                        .await
                        .is_err()
                    {
                        break;
                    }
                    let _ = flush_stream(&mut self.stream).await;
                    self.pending_stream = None;
                    continue;
                }
            }

            let timeout_ms = if self.pending_stream.is_some() {
                EVENT_POLL_MS
            } else {
                0
            };
            let size = match read_packet(&mut self.stream, &mut self.storage.work, self.msize, timeout_ms)
                .await
            {
                Ok(Some(size)) => size,
                Ok(None) => continue,
                Err(()) => break,
            };

            let outcome = match self.dispatch(size) {
                Ok(r) => r,
                Err(()) => break,
            };
            match outcome {
                DispatchResult::Reply(n) => {
                    if write_all(&mut self.stream, &self.storage.tx[..n])
                        .await
                        .is_err()
                    {
                        break;
                    }
                    let _ = flush_stream(&mut self.stream).await;
                }
                DispatchResult::WaitStream { kind, tag, count } => {
                    self.pending_stream = Some((kind, PendingStreamRead { tag, count }));
                }
            }
        }
    }

    fn dispatch(&mut self, size: usize) -> Result<DispatchResult, ()> {
        let tag = u16::from_le_bytes(self.storage.work[5..7].try_into().unwrap_or([0, 0]));
        let typ = self.storage.work[4];
        let msg = match decode_tmsg(&self.storage.work[..size]) {
            Some(m) => m,
            None => {
                ninep_log!("9p: decode fail typ={} size={}", typ, size);
                return Ok(DispatchResult::Reply(write_rerror(
                    &mut self.storage.tx,
                    tag,
                    "bad message",
                )));
            }
        };
        match msg {
            Tmsg::Version { tag, msize, version } => {
                ninep_log!("9p: Tversion msize={} ver={}", msize, version);
                self.msize = msize.min(buffers::MSG_CAP as u32);
                Ok(DispatchResult::Reply(write_rversion_str(
                    &mut self.storage.tx,
                    tag,
                    self.msize,
                    wire::VERSION,
                )))
            }
            Tmsg::Flush { tag, oldtag } => {
                if self
                    .pending_stream
                    .as_ref()
                    .is_some_and(|(_, p)| p.tag == oldtag)
                {
                    self.pending_stream = None;
                }
                let out = &mut self.storage.tx;
                let mut o = 0usize;
                put_u32(out, &mut o, 7);
                put_u8(out, &mut o, 103);
                put_u16(out, &mut o, tag);
                Ok(DispatchResult::Reply(o))
            }
            Tmsg::Attach { tag, fid, aname, .. } => {
                ninep_log!("9p: Tattach fid={} aname={}", fid, aname);
                if !self.take_fid(fid, Node::Root) {
                    return Ok(DispatchResult::Reply(write_rerror(
                        &mut self.storage.tx,
                        tag,
                        "bad fid",
                    )));
                }
                Ok(DispatchResult::Reply(self.reply_attach(tag, fid)))
            }
            Tmsg::Walk {
                tag,
                fid,
                newfid,
                nwname,
            } => {
                ninep_log!("9p: Twalk fid={} newfid={} nwname={}", fid, newfid, nwname);
                self.handle_walk(tag, fid, newfid, nwname, size)
            }
            Tmsg::Auth { tag } => Ok(DispatchResult::Reply(write_rerror(
                &mut self.storage.tx,
                tag,
                "authentication not required",
            ))),
            Tmsg::Open { tag, fid, mode } => {
                ninep_log!("9p: Topen fid={} mode={}", fid, mode);
                self.handle_open(tag, fid)
            }
            Tmsg::Create {
                tag,
                fid,
                name,
                perm,
                mode,
            } => {
                ninep_log!("9p: Tcreate fid={} name={} perm={:#o} mode={}", fid, name, perm, mode);
                let n = name.len().min(self.storage.rx.len());
                self.storage.rx[..n].copy_from_slice(name.as_bytes());
                self.handle_create(tag, fid, n)
            }
            Tmsg::Read {
                tag,
                fid,
                offset,
                count,
            } => self.handle_read(tag, fid, offset, count),
            Tmsg::Write {
                tag,
                fid,
                offset,
                data,
            } => {
                let dlen = data.len().min(self.storage.rx.len());
                self.storage.rx[..dlen].copy_from_slice(&data[..dlen]);
                self.handle_write(tag, fid, offset, dlen)
            }
            Tmsg::Clunk { tag, fid } => {
                ninep_log!("9p: Tclunk fid={}", fid);
                self.handle_clunk(tag, fid)
            }
            Tmsg::Stat { tag, fid } => {
                ninep_log!("9p: Tstat fid={}", fid);
                self.handle_stat(tag, fid)
            }
            Tmsg::Wstat { tag, fid } => {
                ninep_log!("9p: Twstat fid={}", fid);
                self.handle_wstat(tag, fid)
            }
            Tmsg::Remove { tag, .. } => Ok(DispatchResult::Reply(write_rerror(
                &mut self.storage.tx,
                tag,
                "not supported",
            ))),
            Tmsg::Unknown { tag, typ } => {
                ninep_log!("9p: unknown typ={}", typ);
                Ok(DispatchResult::Reply(write_rerror(
                    &mut self.storage.tx,
                    tag,
                    &unknown_typ(typ),
                )))
            }
        }
    }

    /// Assign `fid` to `node`, replacing any previous binding (Linux often re-walks without Tclunk).
    fn take_fid(&mut self, fid: u32, node: Node) -> bool {
        if fid as usize >= MAX_FIDS {
            return false;
        }
        self.fids[fid as usize] = FidSlot {
            in_use: true,
            node,
            open: false,
        };
        true
    }

    fn fid_node(&self, fid: u32) -> Option<Node> {
        self.fids.get(fid as usize).filter(|s| s.in_use).map(|s| s.node)
    }

    fn reply_attach(&mut self, tag: u16, fid: u32) -> usize {
        let out = &mut self.storage.tx;
        let mut o = 0usize;
        put_u32(out, &mut o, 0);
        put_u8(out, &mut o, RATTACH);
        put_u16(out, &mut o, tag);
        let q = Node::Root.qid();
        let qw = wire::QidWire {
            typ: q.typ,
            vers: q.vers,
            path: q.path,
        };
        o += qw.encode(&mut out[o..]);
        let size = o as u32;
        out[0..4].copy_from_slice(&size.to_le_bytes());
        let _ = fid;
        o
    }

    fn handle_walk(
        &mut self,
        tag: u16,
        fid: u32,
        newfid: u32,
        nwname: u16,
        msg_len: usize,
    ) -> Result<DispatchResult, ()> {
        let Some(mut node) = self.fid_node(fid) else {
            return Ok(DispatchResult::Reply(write_rerror(
                &mut self.storage.tx,
                tag,
                "fid not found",
            )));
        };
        if nwname == 0 {
            if !self.take_fid(newfid, node) {
                return Ok(DispatchResult::Reply(write_rerror(
                    &mut self.storage.tx,
                    tag,
                    "bad fid",
                )));
            }
            return Ok(DispatchResult::Reply(self.reply_walk(tag, newfid, &[])));
        }

        let mut name_lens = [0usize; 16];
        let name_count = match walk_name_lens(
            &self.storage.work[..msg_len],
            &mut name_lens,
            &mut self.storage.rx,
        ) {
            Some(n) => n,
            None => {
                return Ok(DispatchResult::Reply(write_rerror(
                    &mut self.storage.tx,
                    tag,
                    "bad walk",
                )))
            }
        };

        let mut qids = heapless::Vec::<crate::vfs::Qid, 16>::new();
        let mut walked = 0u16;
        let mut off = 0usize;
        for i in 0..name_count.min(nwname as usize) {
            let len = name_lens[i];
            let name = match core::str::from_utf8(&self.storage.rx[off..off + len]) {
                Ok(s) => s,
                Err(_) => {
                    return Ok(DispatchResult::Reply(write_rerror(
                        &mut self.storage.tx,
                        tag,
                        "bad name",
                    )))
                }
            };
            off += len;
            let Some(next) = node.walk(name) else {
                break;
            };
            node = next;
            let _ = qids.push(node.qid());
            walked += 1;
        }

        if walked == nwname {
            if !self.take_fid(newfid, node) {
                return Ok(DispatchResult::Reply(write_rerror(
                    &mut self.storage.tx,
                    tag,
                    "bad fid",
                )));
            }
        }

        Ok(DispatchResult::Reply(self.reply_walk(tag, newfid, qids.as_slice())))
    }

    fn reply_walk(&mut self, tag: u16, newfid: u32, qids: &[crate::vfs::Qid]) -> usize {
        let out = &mut self.storage.tx;
        let mut o = 0usize;
        put_u32(out, &mut o, 0);
        put_u8(out, &mut o, RWALK);
        put_u16(out, &mut o, tag);
        put_u16(out, &mut o, qids.len() as u16);
        for q in qids {
            let qw = wire::QidWire {
                typ: q.typ,
                vers: q.vers,
                path: q.path,
            };
            o += qw.encode(&mut out[o..]);
        }
        let size = o as u32;
        out[0..4].copy_from_slice(&size.to_le_bytes());
        let _ = newfid;
        o
    }

    fn handle_open(&mut self, tag: u16, fid: u32) -> Result<DispatchResult, ()> {
        let Some(node) = self.fid_node(fid) else {
            return Ok(DispatchResult::Reply(write_rerror(
                &mut self.storage.tx,
                tag,
                "fid not found",
            )));
        };
        if node == Node::DevBtnEvent {
            buttons::flush_events();
        }
        Ok(DispatchResult::Reply(self.reply_open(tag, fid, node, ROPEN)))
    }

    fn handle_create(&mut self, tag: u16, fid: u32, name_len: usize) -> Result<DispatchResult, ()> {
        let name = core::str::from_utf8(&self.storage.rx[..name_len]).unwrap_or("");
        let Some(parent) = self.fid_node(fid) else {
            return Ok(DispatchResult::Reply(write_rerror(
                &mut self.storage.tx,
                tag,
                "fid not found",
            )));
        };
        let node = if fs::is_writable(parent) && (name.is_empty() || name == parent.name()) {
            parent
        } else if let Some(child) = parent.walk(name) {
            if !fs::is_writable(child) {
                return Ok(DispatchResult::Reply(write_rerror(
                    &mut self.storage.tx,
                    tag,
                    "permission denied",
                )));
            }
            child
        } else {
            return Ok(DispatchResult::Reply(write_rerror(
                &mut self.storage.tx,
                tag,
                "not found",
            )));
        };
        self.take_fid(fid, node);
        Ok(DispatchResult::Reply(self.reply_open(tag, fid, node, RCREATE)))
    }

    fn reply_open(&mut self, tag: u16, fid: u32, node: Node, rtype: u8) -> usize {
        if let Some(slot) = self.fids.get_mut(fid as usize) {
            slot.open = true;
        }
        let out = &mut self.storage.tx;
        let mut o = 0usize;
        put_u32(out, &mut o, 0);
        put_u8(out, &mut o, rtype);
        put_u16(out, &mut o, tag);
        let qw = wire::QidWire {
            typ: node.qid().typ,
            vers: 0,
            path: node.path(),
        };
        o += qw.encode(&mut out[o..]);
        put_u32(out, &mut o, 0);
        let size = o as u32;
        out[0..4].copy_from_slice(&size.to_le_bytes());
        o
    }

    fn handle_read(
        &mut self,
        tag: u16,
        fid: u32,
        offset: u64,
        count: u32,
    ) -> Result<DispatchResult, ()> {
        let Some(node) = self.fid_node(fid) else {
            return Ok(DispatchResult::Reply(write_rerror(
                &mut self.storage.tx,
                tag,
                "fid not found",
            )));
        };
        let max_count = max_read_count(count, self.msize);
        let data = &mut self.storage.rx[..max_count];
        let n = if node == Node::DevBtnEvent {
            buttons::try_read_event(offset, data)
        } else if node == Node::DevMicPcm {
            mic::try_read_pcm(offset, data)
        } else if !node.children().is_empty() {
            fs::pack_dir_list(node, offset, data)
        } else {
            fs::read_file(node, &self.ctx, offset, data)
        };
        let n = n.min(max_count);
        if node == Node::DevBtnEvent && offset == 0 && n == 0 {
            return Ok(DispatchResult::WaitStream {
                kind: PendingKind::BtnEvent,
                tag,
                count,
            });
        }
        if node == Node::DevMicPcm && n == 0 && mic::is_running() {
            return Ok(DispatchResult::WaitStream {
                kind: PendingKind::MicPcm,
                tag,
                count,
            });
        }
        let payload = &self.storage.rx[..n];
        Ok(DispatchResult::Reply(build_read_reply(
            &mut self.storage.tx,
            tag,
            payload,
        )))
    }
}

/// Max bytes returned in one Tread/Rread (fits in `SessionStorage` and negotiated msize).
fn max_read_count(count: u32, msize: u32) -> usize {
    let cap = msize.saturating_sub(24).min(buffers::MSG_CAP.saturating_sub(24) as u32);
    count.min(cap) as usize
}

fn build_read_reply(out: &mut [u8], tag: u16, data: &[u8]) -> usize {
    let mut o = 0usize;
    put_u32(out, &mut o, 0);
    put_u8(out, &mut o, RREAD);
    put_u16(out, &mut o, tag);
    put_u32(out, &mut o, data.len() as u32);
    out[o..o + data.len()].copy_from_slice(data);
    o += data.len();
    let size = o as u32;
    out[0..4].copy_from_slice(&size.to_le_bytes());
    o
}

impl<'a, S> Session<'a, S>
where
    S: Read + Write,
{
    fn handle_write(
        &mut self,
        tag: u16,
        fid: u32,
        offset: u64,
        dlen: usize,
    ) -> Result<DispatchResult, ()> {
        let _ = offset;
        let Some(node) = self.fid_node(fid) else {
            return Ok(DispatchResult::Reply(write_rerror(
                &mut self.storage.tx,
                tag,
                "fid not found",
            )));
        };
        let data = &self.storage.rx[..dlen];
        let n = match fs::write_file(node, &self.ctx, offset, data) {
            Ok(n) => n,
            Err(e) => {
                return Ok(DispatchResult::Reply(write_rerror(
                    &mut self.storage.tx,
                    tag,
                    e,
                )))
            }
        };
        let out = &mut self.storage.tx;
        let mut o = 0usize;
        put_u32(out, &mut o, 0);
        put_u8(out, &mut o, RWRITE);
        put_u16(out, &mut o, tag);
        put_u32(out, &mut o, n as u32);
        let size = o as u32;
        out[0..4].copy_from_slice(&size.to_le_bytes());
        Ok(DispatchResult::Reply(o))
    }

    fn handle_clunk(&mut self, tag: u16, fid: u32) -> Result<DispatchResult, ()> {
        if let Some(slot) = self.fids.get_mut(fid as usize) {
            *slot = FidSlot::EMPTY;
        }
        let out = &mut self.storage.tx;
        let mut o = 0usize;
        put_u32(out, &mut o, 0);
        put_u8(out, &mut o, RCLUNK);
        put_u16(out, &mut o, tag);
        let size = o as u32;
        out[0..4].copy_from_slice(&size.to_le_bytes());
        Ok(DispatchResult::Reply(o))
    }

    fn handle_wstat(&mut self, tag: u16, fid: u32) -> Result<DispatchResult, ()> {
        if self.fid_node(fid).is_none() {
            return Ok(DispatchResult::Reply(write_rerror(
                &mut self.storage.tx,
                tag,
                "fid not found",
            )));
        }
        // Linux sends Twstat after O_TRUNC open (e.g. `echo x > file`) to set length=0.
        // Synthetic files have no persistent content; accept as no-op.
        let out = &mut self.storage.tx;
        let mut o = 0usize;
        put_u32(out, &mut o, 7);
        put_u8(out, &mut o, RWSTAT);
        put_u16(out, &mut o, tag);
        Ok(DispatchResult::Reply(o))
    }

    fn handle_stat(&mut self, tag: u16, fid: u32) -> Result<DispatchResult, ()> {
        let Some(node) = self.fid_node(fid) else {
            return Ok(DispatchResult::Reply(write_rerror(
                &mut self.storage.tx,
                tag,
                "fid not found",
            )));
        };
        let out = &mut self.storage.tx;
        let mut o = 0usize;
        put_u32(out, &mut o, 0);
        put_u8(out, &mut o, RSTAT);
        put_u16(out, &mut o, tag);
        let qw = wire::QidWire {
            typ: node.qid().typ,
            vers: 0,
            path: node.path(),
        };
        wire::encode_stat_rstat(out, &mut o, qw, node.mode(), node.length(), node.name());
        let size = o as u32;
        out[0..4].copy_from_slice(&size.to_le_bytes());
        ninep_log!("9p: Rstat fid={} bytes={}", fid, size);
        Ok(DispatchResult::Reply(o))
    }
}

/// Copy Twalk wname strings into `out`, returning count and per-name lengths.
fn walk_name_lens(buf: &[u8], lens: &mut [usize; 16], out: &mut [u8]) -> Option<usize> {
    if buf.len() < 17 || buf[4] != wire::TWALK {
        return None;
    }
    let size = u32::from_le_bytes(buf[0..4].try_into().ok()?) as usize;
    if size != buf.len() {
        return None;
    }
    let nwname = u16::from_le_bytes(buf[15..17].try_into().ok()?) as usize;
    let mut p = 17usize;
    let mut off = 0usize;
    for i in 0..nwname {
        if i >= 16 || p + 2 > buf.len() {
            return None;
        }
        let n = u16::from_le_bytes(buf[p..p + 2].try_into().ok()?) as usize;
        p += 2;
        let s = buf.get(p..p + n)?;
        core::str::from_utf8(s).ok()?;
        out[off..off + n].copy_from_slice(s);
        lens[i] = n;
        off += n;
        p += n;
    }
    Some(nwname.min(16))
}

fn unknown_typ(typ: u8) -> heapless::String<24> {
    let mut s = heapless::String::new();
    let _ = s.push_str("unknown ");
    let _ = s.push(char::from(b'0' + (typ / 100) % 10));
    let _ = s.push(char::from(b'0' + (typ / 10) % 10));
    let _ = s.push(char::from(b'0' + typ % 10));
    s
}

async fn read_exact<S: Read>(s: &mut S, buf: &mut [u8]) -> Result<(), ()> {
    let mut pos = 0;
    while pos < buf.len() {
        let n = s.read(&mut buf[pos..]).await.map_err(|_| ())?;
        if n == 0 {
            return Err(());
        }
        pos += n;
    }
    Ok(())
}

async fn write_all<S: Write>(s: &mut S, buf: &[u8]) -> Result<(), ()> {
    let mut pos = 0;
    while pos < buf.len() {
        let n = s.write(&buf[pos..]).await.map_err(|_| ())?;
        if n == 0 {
            return Err(());
        }
        pos += n;
    }
    Ok(())
}

async fn flush_stream<S: Write>(s: &mut S) -> Result<(), ()> {
    s.flush().await.map_err(|_| ())
}

/// Read one 9P message. `timeout_ms == 0` blocks until a full packet arrives.
async fn read_packet<S: Read>(
    s: &mut S,
    work: &mut [u8],
    msize: u32,
    timeout_ms: u64,
) -> Result<Option<usize>, ()> {
    let mut hdr = [0u8; 4];
    if timeout_ms == 0 {
        read_exact(s, &mut hdr).await?;
    } else if !read_until_deadline(s, &mut hdr, timeout_ms).await? {
        return Ok(None);
    }
    let size = u32::from_le_bytes(hdr) as usize;
    if size < 7 || size > msize as usize || size > work.len() {
        return Err(());
    }
    work[..4].copy_from_slice(&hdr);
    if timeout_ms == 0 {
        read_exact(s, &mut work[4..size]).await?;
    } else if !read_until_deadline(s, &mut work[4..size], timeout_ms).await? {
        return Err(());
    }
    Ok(Some(size))
}

async fn read_until_deadline<S: Read>(s: &mut S, buf: &mut [u8], timeout_ms: u64) -> Result<bool, ()> {
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    let mut pos = 0usize;
    while pos < buf.len() {
        if Instant::now() >= deadline {
            return Ok(false);
        }
        let n = s.read(&mut buf[pos..]).await.map_err(|_| ())?;
        if n == 0 {
            return Err(());
        }
        pos += n;
        if pos < buf.len() {
            Timer::after(Duration::from_millis(2)).await;
        }
    }
    Ok(true)
}
