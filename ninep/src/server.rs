//! 9P2000 session handler.

use crate::buffers::{self, SessionStorage};
use crate::fs::{self, FidTarget, FsContext, Node};
use crate::wire::{
    self, decode_tmsg, put_u16, put_u32, put_u8, write_rerror, write_rversion_str,
    Tmsg, RATTACH, RCLUNK, RCREATE, ROPEN, RREAD, RREMOVE, RSTAT, RWALK, RWSTAT, RWRITE,
    DEFAULT_MSIZE,
};
use devices::{buttons, mic};
use embedded_io_async::{Read, Write};
use embassy_time::{Duration, Timer};

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
    fid: u32,
    /// 9P `Tread.offset` carried verbatim for any future stream kinds that need it.
    /// `BtnEvent` and `MicPcm` always restart from 0; kept here so the field name
    /// stays parallel with `PendingExec` and `WaitStream { offset, .. }`.
    #[allow(dead_code)]
    offset: u64,
}

enum PendingKind {
    BtnEvent,
    MicPcm,
}

enum DispatchResult {
    Reply(usize),
    WaitStream {
        kind: PendingKind,
        tag: u16,
        count: u32,
        fid: u32,
        offset: u64,
    },
}

struct FidSlot {
    in_use: bool,
    target: FidTarget,
    open: bool,
}

impl FidSlot {
    const EMPTY: Self = Self {
        in_use: false,
        target: FidTarget::Static(Node::Root),
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
    btn_poll_count: u32,
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
            btn_poll_count: 0,
            pending_stream: None,
        }
    }

    pub async fn run(mut self) {
        loop {
            if let Some((kind, p)) = self.pending_stream.as_ref() {
                let max_count = max_read_count(p.count, self.msize);
                let buf = &mut self.storage.rx[..max_count];
                let reply_n = match kind {
                    PendingKind::BtnEvent => {
                        let n = buttons::try_read_event(0, buf);
                        self.btn_poll_count += 1;
                        if n > 0 || self.btn_poll_count % 40 == 1 {
                            ninep_log!("9p: btn poll n={} total={}", n, self.btn_poll_count);
                        }
                        if n > 0 && n < buf.len() {
                            buf[n..].fill(0);
                            Some(buf.len())
                        } else if n > 0 {
                            Some(n)
                        } else {
                            None
                        }
                    }
                    PendingKind::MicPcm => {
                        let n = mic::try_read_pcm(0, buf);
                        if n > 0 {
                            Some(n)
                        } else if mic::is_running() {
                            None
                        } else {
                            Some(0)
                        }
                    }
                };
                if let Some(n) = reply_n {
                    ninep_log!("9p: pend-drain {} bytes (tag={})", n, p.tag);
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
                DispatchResult::WaitStream {
                    kind,
                    tag,
                    count,
                    fid,
                    offset,
                } => {
                    self.pending_stream =
                        Some((kind, PendingStreamRead { tag, count, fid, offset }));
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
                put_u8(out, &mut o, wire::RFLUSH);
                put_u16(out, &mut o, tag);
                Ok(DispatchResult::Reply(o))
            }
            Tmsg::Attach { tag, fid, aname, .. } => {
                ninep_log!("9p: Tattach fid={} aname={}", fid, aname);
                if !self.take_fid(fid, FidTarget::Static(Node::Root)) {
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
                self.handle_open(tag, fid, mode)
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
                self.handle_create(tag, fid, n, perm)
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
            Tmsg::Remove { tag, fid } => self.handle_remove(tag, fid),
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

    /// Assign `fid` to `target`, replacing any previous binding (Linux often re-walks without Tclunk).
    fn take_fid(&mut self, fid: u32, target: FidTarget) -> bool {
        if fid as usize >= MAX_FIDS {
            return false;
        }
        self.fids[fid as usize] = FidSlot {
            in_use: true,
            target: normalize_target(target),
            open: false,
        };
        true
    }

    fn fid_target(&self, fid: u32) -> Option<FidTarget> {
        self.fids
            .get(fid as usize)
            .filter(|s| s.in_use)
            .map(|s| s.target)
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
        let Some(mut target) = self.fid_target(fid) else {
            return Ok(DispatchResult::Reply(write_rerror(
                &mut self.storage.tx,
                tag,
                "fid not found",
            )));
        };
        if nwname == 0 {
            if !self.take_fid(newfid, target) {
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
            let Some(next) = target.walk(name) else {
                break;
            };
            target = next;
            let _ = qids.push(target.qid());
            walked += 1;
        }

        if walked == nwname {
            if !self.take_fid(newfid, target) {
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

    fn handle_open(&mut self, tag: u16, fid: u32, mode: u8) -> Result<DispatchResult, ()> {
        let Some(target) = self.fid_target(fid) else {
            return Ok(DispatchResult::Reply(write_rerror(
                &mut self.storage.tx,
                tag,
                "fid not found",
            )));
        };
        if target == FidTarget::Static(Node::DevBtnEvent) {
            buttons::flush_events();
        }
        if let FidTarget::Mem(ino) = target {
            if mode & 0x10 != 0 {
                devices::memfs::truncate(ino);
            }
        }
        Ok(DispatchResult::Reply(self.reply_open(tag, fid, target, ROPEN)))
    }

    fn handle_create(
        &mut self,
        tag: u16,
        fid: u32,
        name_len: usize,
        perm: u32,
    ) -> Result<DispatchResult, ()> {
        let name = core::str::from_utf8(&self.storage.rx[..name_len]).unwrap_or("");
        let Some(parent) = self.fid_target(fid) else {
            return Ok(DispatchResult::Reply(write_rerror(
                &mut self.storage.tx,
                tag,
                "fid not found",
            )));
        };
        let target = match parent {
            FidTarget::Mem(parent_ino) => {
                if !devices::memfs::is_dir(parent_ino) {
                    return Ok(DispatchResult::Reply(write_rerror(
                        &mut self.storage.tx,
                        tag,
                        "not a directory",
                    )));
                }
                match devices::memfs::create(parent_ino, name, perm) {
                    Ok(ino) => FidTarget::Mem(ino),
                    Err(e) => {
                        return Ok(DispatchResult::Reply(write_rerror(
                            &mut self.storage.tx,
                            tag,
                            e,
                        )))
                    }
                }
            }
            FidTarget::Static(parent) => {
                if fs::is_writable(parent) && (name.is_empty() || name == parent.name()) {
                    fs::static_target(parent)
                } else if let Some(child) = parent.walk(name) {
                    if !fs::is_writable(child) {
                        return Ok(DispatchResult::Reply(write_rerror(
                            &mut self.storage.tx,
                            tag,
                            "permission denied",
                        )));
                    }
                    fs::static_target(child)
                } else {
                    return Ok(DispatchResult::Reply(write_rerror(
                        &mut self.storage.tx,
                        tag,
                        "not found",
                    )));
                }
            }
        };
        self.take_fid(fid, target);
        Ok(DispatchResult::Reply(self.reply_open(tag, fid, target, RCREATE)))
    }

    fn handle_remove(&mut self, tag: u16, fid: u32) -> Result<DispatchResult, ()> {
        let Some(target) = self.fid_target(fid) else {
            return Ok(DispatchResult::Reply(write_rerror(
                &mut self.storage.tx,
                tag,
                "fid not found",
            )));
        };
        let FidTarget::Mem(ino) = target else {
            return Ok(DispatchResult::Reply(write_rerror(
                &mut self.storage.tx,
                tag,
                "not supported",
            )));
        };
        if let Err(e) = devices::memfs::remove(ino) {
            return Ok(DispatchResult::Reply(write_rerror(
                &mut self.storage.tx,
                tag,
                e,
            )));
        }
        if let Some(slot) = self.fids.get_mut(fid as usize) {
            *slot = FidSlot::EMPTY;
        }
        let out = &mut self.storage.tx;
        let mut o = 0usize;
        put_u32(out, &mut o, 0);
        put_u8(out, &mut o, RREMOVE);
        put_u16(out, &mut o, tag);
        let size = o as u32;
        out[0..4].copy_from_slice(&size.to_le_bytes());
        Ok(DispatchResult::Reply(o))
    }

    fn reply_open(&mut self, tag: u16, fid: u32, target: FidTarget, rtype: u8) -> usize {
        if let Some(slot) = self.fids.get_mut(fid as usize) {
            slot.open = true;
        }
        let q = target.qid();
        let out = &mut self.storage.tx;
        let mut o = 0usize;
        put_u32(out, &mut o, 0);
        put_u8(out, &mut o, rtype);
        put_u16(out, &mut o, tag);
        let qw = wire::QidWire {
            typ: q.typ,
            vers: q.vers,
            path: q.path,
        };
        o += qw.encode(&mut out[o..]);
        put_u32(out, &mut o, 0);
        let size = o as u32;
        out[0..4].copy_from_slice(&size.to_le_bytes());
        let _ = fid;
        o
    }

    fn handle_read(
        &mut self,
        tag: u16,
        fid: u32,
        offset: u64,
        count: u32,
    ) -> Result<DispatchResult, ()> {
        let Some(target) = self.fid_target(fid) else {
            return Ok(DispatchResult::Reply(write_rerror(
                &mut self.storage.tx,
                tag,
                "fid not found",
            )));
        };
        let max_count = max_read_count(count, self.msize);
        let data = &mut self.storage.rx[..max_count];
        let n = match target {
            FidTarget::Static(Node::DevBtnEvent) => buttons::try_read_event(offset, data),
            FidTarget::Static(Node::DevMicPcm) => mic::try_read_pcm(offset, data),
            FidTarget::Static(Node::MemRoot) => fs::pack_mem_dir_list(devices::memfs::ROOT_INO, offset, data),
            #[cfg(feature = "wamr")]
            FidTarget::Static(Node::Task) => fs::pack_task_root_dir_list(offset, data),
            #[cfg(feature = "wamr")]
            FidTarget::Static(Node::TaskDir(rid)) => fs::pack_task_dir_list(rid, offset, data),
            #[cfg(feature = "wamr")]
            FidTarget::Static(Node::TaskFile(rid, fs::TaskFileKind::Data)) => {
                devices::task::read_data(rid, offset, data).min(max_count)
            }
            FidTarget::Static(node) if !node.children().is_empty() && !node.uses_custom_dir_list() => {
                fs::pack_dir_list(node, offset, data)
            }
            FidTarget::Static(node) => fs::read_file(node, &self.ctx, offset, data),
            FidTarget::Mem(ino) if devices::memfs::is_dir(ino) => {
                fs::pack_mem_dir_list(ino, offset, data)
            }
            FidTarget::Mem(ino) => devices::memfs::read(ino, offset, data),
        };
        let n = n.min(max_count);
        if target == FidTarget::Static(Node::DevMicPcm) {
            ninep_log!(
                "9p: Tread mic/pcm tag={} off={} count={} max={} got={} running={}",
                tag, offset, count, max_count, n, mic::is_running()
            );
        }
        if target == FidTarget::Static(Node::DevBtnEvent) {
            let n = if n > 0 && n < max_count {
                // Zero-pad to fill the requested buffer so p9_client_read's fill-loop
                // is satisfied in one Tread, no follow-up 0-byte or error response needed.
                // (This kernel neither breaks on Rread(0) nor tolerates Rerror mid-stream.)
                data[n..max_count].fill(0);
                max_count
            } else {
                n
            };
            if n == 0 {
                ninep_log!("9p: WaitStream BtnEvent tag={} off={}", tag, offset);
                return Ok(DispatchResult::WaitStream {
                    kind: PendingKind::BtnEvent,
                    tag,
                    count,
                    fid,
                    offset,
                });
            }
            let payload = &self.storage.rx[..n];
            return Ok(DispatchResult::Reply(build_read_reply(
                &mut self.storage.tx,
                tag,
                payload,
            )));
        }
        if target == FidTarget::Static(Node::DevMicPcm) && n == 0 && mic::is_running() {
            ninep_log!("9p: WaitStream MicPcm tag={} count={}", tag, count);
            return Ok(DispatchResult::WaitStream {
                kind: PendingKind::MicPcm,
                tag,
                count,
                fid,
                offset,
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

fn build_write_reply(out: &mut [u8], tag: u16, write_len: u32) -> usize {
    let mut o = 0usize;
    put_u32(out, &mut o, 0);
    put_u8(out, &mut o, RWRITE);
    put_u16(out, &mut o, tag);
    put_u32(out, &mut o, write_len);
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
        let Some(target) = self.fid_target(fid) else {
            return Ok(DispatchResult::Reply(write_rerror(
                &mut self.storage.tx,
                tag,
                "fid not found",
            )));
        };
        let data = &self.storage.rx[..dlen];
        let n = match target {
            FidTarget::Mem(ino) => match devices::memfs::write(ino, offset, data) {
                Ok(n) => n,
                Err(e) => {
                    return Ok(DispatchResult::Reply(write_rerror(
                        &mut self.storage.tx,
                        tag,
                        e,
                    )))
                }
            },
            #[cfg(feature = "wamr")]
            FidTarget::Static(Node::TaskFile(rid, fs::TaskFileKind::Data)) => {
                match devices::task::write_data(rid, offset, data) {
                    Ok(n) => n,
                    Err(e) => {
                        return Ok(DispatchResult::Reply(write_rerror(
                            &mut self.storage.tx,
                            tag,
                            e,
                        )))
                    }
                }
            }
            FidTarget::Static(node) => match fs::write_file(node, &self.ctx, offset, data) {
                Ok(n) => n,
                Err(e) => {
                    return Ok(DispatchResult::Reply(write_rerror(
                        &mut self.storage.tx,
                        tag,
                        e,
                    )))
                }
            },
        };
        Ok(DispatchResult::Reply(build_write_reply(
            &mut self.storage.tx,
            tag,
            n as u32,
        )))
    }

    fn handle_clunk(&mut self, tag: u16, fid: u32) -> Result<DispatchResult, ()> {
        if let Some(slot) = self.fids.get_mut(fid as usize) {
            *slot = FidSlot::EMPTY;
        }
        // If this fid had an active WaitStream, cancel it so stale pend-drains
        // don't fire after the file is closed.
        if self.pending_stream.as_ref().map_or(false, |(_, p)| p.fid == fid) {
            ninep_log!("9p: cancel pending stream fid={}", fid);
            self.pending_stream = None;
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
        let Some(target) = self.fid_target(fid) else {
            return Ok(DispatchResult::Reply(write_rerror(
                &mut self.storage.tx,
                tag,
                "fid not found",
            )));
        };
        if let FidTarget::Mem(ino) = target {
            devices::memfs::truncate(ino);
        }
        // Linux sends Twstat after O_TRUNC open (e.g. `echo x > file`) to set length=0.
        // Other static files have no persistent content; accept as no-op.
        let out = &mut self.storage.tx;
        let mut o = 0usize;
        put_u32(out, &mut o, 7);
        put_u8(out, &mut o, RWSTAT);
        put_u16(out, &mut o, tag);
        Ok(DispatchResult::Reply(o))
    }

    fn handle_stat(&mut self, tag: u16, fid: u32) -> Result<DispatchResult, ()> {
        let Some(target) = self.fid_target(fid) else {
            return Ok(DispatchResult::Reply(write_rerror(
                &mut self.storage.tx,
                tag,
                "fid not found",
            )));
        };
        let q = target.qid();
        let name = target.name();
        let out = &mut self.storage.tx;
        let mut o = 0usize;
        put_u32(out, &mut o, 0);
        put_u8(out, &mut o, RSTAT);
        put_u16(out, &mut o, tag);
        let qw = wire::QidWire {
            typ: q.typ,
            vers: q.vers,
            path: q.path,
        };
        wire::encode_stat_rstat(out, &mut o, qw, target.mode(), target.length(), name.as_str());
        let size = o as u32;
        out[0..4].copy_from_slice(&size.to_le_bytes());
        ninep_log!("9p: Rstat fid={} bytes={}", fid, size);
        Ok(DispatchResult::Reply(o))
    }
}

fn normalize_target(target: FidTarget) -> FidTarget {
    match target {
        FidTarget::Static(Node::MemRoot) => FidTarget::Mem(devices::memfs::ROOT_INO),
        other => other,
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

/// Read one 9P message.
///
/// `timeout_ms == 0` blocks until a full packet arrives.
/// Non-zero `timeout_ms` *only* bounds the wait for the message's first byte
/// (the header); once a header arrives, the body is read with no timeout so a
/// streaming write from the client doesn't desynchronise us. The header-only
/// timeout is what `Session::run` uses to periodically re-poll
/// `pending_stream` while waiting on slow producers like the mic ring.
async fn read_packet<S: Read>(
    s: &mut S,
    work: &mut [u8],
    msize: u32,
    timeout_ms: u64,
) -> Result<Option<usize>, ()> {
    let mut hdr = [0u8; 4];
    if timeout_ms == 0 {
        read_exact(s, &mut hdr).await?;
    } else if !read_first_byte_until_deadline(s, &mut hdr, timeout_ms).await? {
        return Ok(None);
    }
    let size = u32::from_le_bytes(hdr) as usize;
    if size < 7 || size > msize as usize || size > work.len() {
        return Err(());
    }
    work[..4].copy_from_slice(&hdr);
    read_exact(s, &mut work[4..size]).await?;
    Ok(Some(size))
}

/// Wait up to `timeout_ms` for the first byte of a new 9P message, then drain
/// the rest of `buf` without further timeouts.
///
/// The old "check deadline → await s.read()" loop deadlocked streaming reads
/// (e.g. `dd if=/dev/mic/pcm`): once the session task was sleeping inside
/// `s.read().await` waiting for the next Tread, the audio task could keep
/// calling `mic::push_pcm` indefinitely without ever waking us, so the
/// pending Rread was never sent. Racing the *first* read against a real
/// timer lets `Session::run` re-poll the ring (or any other synthetic
/// stream) on a wall-clock schedule instead of "next inbound TCP byte".
///
/// Returns `Ok(true)` if the full buffer was filled, `Ok(false)` if the
/// deadline expired before any bytes arrived (no partial read), or `Err(())`
/// on socket error / EOF.
async fn read_first_byte_until_deadline<S: Read>(
    s: &mut S,
    buf: &mut [u8],
    timeout_ms: u64,
) -> Result<bool, ()> {
    use embassy_futures::select::{select, Either};
    let timer_fut = Timer::after(Duration::from_millis(timeout_ms));
    let read_fut = s.read(buf);
    let n = match select(read_fut, timer_fut).await {
        Either::First(Ok(0)) => return Err(()),
        Either::First(Ok(n)) => n,
        Either::First(Err(_)) => return Err(()),
        // Timer beat the read. Critically, embedded-io-async's contract for
        // dropping a pending read is implementation-defined; for embassy-net
        // TcpSocket it's safe (the future just stops being polled). No bytes
        // were consumed because the kernel pump only delivered them on poll.
        Either::Second(()) => return Ok(false),
    };
    if n < buf.len() {
        read_exact(s, &mut buf[n..]).await?;
    }
    Ok(true)
}
