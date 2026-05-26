//! Minimal 9P2000 wire encoding/decoding (Stage 1).

pub const DEFAULT_MSIZE: u32 = 2048;
pub const VERSION: &str = "9P2000";

pub const RVERSION: u8 = 101;
pub const RERROR: u8 = 107;
pub const RATTACH: u8 = 105;
pub const RWALK: u8 = 111;
pub const ROPEN: u8 = 113;
pub const RCREATE: u8 = 115;
pub const RREAD: u8 = 117;
pub const RWRITE: u8 = 119;
pub const RCLUNK: u8 = 121;
pub const RREMOVE: u8 = 123;
pub const RSTAT: u8 = 125;
pub const RWSTAT: u8 = 127;

pub const TVERSION: u8 = 100;
pub const TAUTH: u8 = 98;
pub const TATTACH: u8 = 104;
pub const TWALK: u8 = 110;
pub const TOPEN: u8 = 112;
pub const TREAD: u8 = 116;
pub const TWRITE: u8 = 118;
pub const TCLUNK: u8 = 120;
pub const TSTAT: u8 = 124;
pub const TWSTAT: u8 = 126;
pub const TCREATE: u8 = 114;
pub const TREMOVE: u8 = 122;
pub const TFLUSH: u8 = 108;
pub const RFLUSH: u8 = 109;

pub const NOFID: u32 = 0xffff_ffff;

#[derive(Clone, Copy, Debug)]
pub struct QidWire {
    pub typ: u8,
    pub vers: u32,
    pub path: u64,
}

impl QidWire {
    pub fn encode(self, buf: &mut [u8]) -> usize {
        buf[0] = self.typ;
        buf[1..5].copy_from_slice(&self.vers.to_le_bytes());
        buf[5..13].copy_from_slice(&self.path.to_le_bytes());
        13
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Tmsg<'a> {
    Version { tag: u16, msize: u32, version: &'a str },
    Attach { tag: u16, fid: u32, afid: u32, uname: &'a str, aname: &'a str },
    Walk {
        tag: u16,
        fid: u32,
        newfid: u32,
        nwname: u16,
    },
    Auth { tag: u16 },
    Open { tag: u16, fid: u32, mode: u8 },
    Read { tag: u16, fid: u32, offset: u64, count: u32 },
    Write { tag: u16, fid: u32, offset: u64, data: &'a [u8] },
    Clunk { tag: u16, fid: u32 },
    Stat { tag: u16, fid: u32 },
    Wstat { tag: u16, fid: u32 },
    Create {
        tag: u16,
        fid: u32,
        name: &'a str,
        perm: u32,
        mode: u8,
    },
    Remove { tag: u16, fid: u32 },
    Flush { tag: u16, oldtag: u16 },
    Unknown { tag: u16, typ: u8 },
}

pub fn decode_tmsg(buf: &[u8]) -> Option<Tmsg<'_>> {
    if buf.len() < 7 {
        return None;
    }
    let size = u32::from_le_bytes(buf[0..4].try_into().ok()?);
    if size as usize != buf.len() || size < 7 {
        return None;
    }
    let typ = buf[4];
    let tag = u16::from_le_bytes(buf[5..7].try_into().ok()?);
    let mut p = 7usize;
    let read_u32 = |p: &mut usize| -> Option<u32> {
        let v = u32::from_le_bytes(buf.get(*p..*p + 4)?.try_into().ok()?);
        *p += 4;
        Some(v)
    };
    let read_u64 = |p: &mut usize| -> Option<u64> {
        let v = u64::from_le_bytes(buf.get(*p..*p + 8)?.try_into().ok()?);
        *p += 8;
        Some(v)
    };
    let read_u16 = |p: &mut usize| -> Option<u16> {
        let v = u16::from_le_bytes(buf.get(*p..*p + 2)?.try_into().ok()?);
        *p += 2;
        Some(v)
    };
    let read_u8 = |p: &mut usize| -> Option<u8> {
        let v = *buf.get(*p)?;
        *p += 1;
        Some(v)
    };
    let read_str = |p: &mut usize| -> Option<&str> {
        let n = read_u16(p)? as usize;
        let s = core::str::from_utf8(buf.get(*p..*p + n)?).ok()?;
        *p += n;
        Some(s)
    };

    match typ {
        TVERSION => {
            let msize = read_u32(&mut p)?;
            let version = read_str(&mut p)?;
            Some(Tmsg::Version { tag, msize, version })
        }
        TATTACH => {
            let fid = read_u32(&mut p)?;
            let afid = read_u32(&mut p)?;
            let uname = read_str(&mut p)?;
            let aname = read_str(&mut p)?;
            // 9p2000.u / .L append n_uname[4] after aname
            if p + 4 <= buf.len() {
                let _ = read_u32(&mut p);
            }
            Some(Tmsg::Attach { tag, fid, afid, uname, aname })
        }
        TWALK => {
            let fid = read_u32(&mut p)?;
            let newfid = read_u32(&mut p)?;
            let nwname = read_u16(&mut p)?;
            for _ in 0..nwname {
                read_str(&mut p)?;
            }
            Some(Tmsg::Walk {
                tag,
                fid,
                newfid,
                nwname,
            })
        }
        TAUTH => {
            let _ = read_u32(&mut p)?;
            let _ = read_str(&mut p)?;
            let _ = read_str(&mut p)?;
            Some(Tmsg::Auth { tag })
        }
        TOPEN => {
            let fid = read_u32(&mut p)?;
            let mode = read_u8(&mut p)?;
            Some(Tmsg::Open { tag, fid, mode })
        }
        TREAD => {
            let fid = read_u32(&mut p)?;
            let offset = read_u64(&mut p)?;
            let count = read_u32(&mut p)?;
            Some(Tmsg::Read { tag, fid, offset, count })
        }
        TWRITE => {
            let fid = read_u32(&mut p)?;
            let offset = read_u64(&mut p)?;
            let count = read_u32(&mut p)?;
            let data = buf.get(p..p + count as usize)?;
            Some(Tmsg::Write {
                tag,
                fid,
                offset,
                data,
            })
        }
        TCLUNK => {
            let fid = read_u32(&mut p)?;
            Some(Tmsg::Clunk { tag, fid })
        }
        TSTAT => {
            let fid = read_u32(&mut p)?;
            Some(Tmsg::Stat { tag, fid })
        }
        TWSTAT => {
            let fid = read_u32(&mut p)?;
            if p + 2 <= buf.len() {
                let wrap = read_u16(&mut p)?;
                if p + wrap as usize <= buf.len() {
                    p += wrap as usize;
                }
            }
            // Linux may append 9p2000.u extensions after the stat blob.
            Some(Tmsg::Wstat { tag, fid })
        }
        TCREATE => {
            let fid = read_u32(&mut p)?;
            let name = read_str(&mut p)?;
            let perm = read_u32(&mut p)?;
            let mode = read_u8(&mut p)?;
            // 9p2000.u appends extension[s] after mode
            if p < buf.len() {
                let _ = read_str(&mut p);
            }
            Some(Tmsg::Create {
                tag,
                fid,
                name,
                perm,
                mode,
            })
        }
        TREMOVE => {
            let fid = read_u32(&mut p)?;
            Some(Tmsg::Remove { tag, fid })
        }
        TFLUSH => {
            let oldtag = read_u16(&mut p)?;
            Some(Tmsg::Flush { tag, oldtag })
        }
        _ => Some(Tmsg::Unknown { tag, typ }),
    }
}

pub fn put_u32(buf: &mut [u8], off: &mut usize, v: u32) {
    buf[*off..*off + 4].copy_from_slice(&v.to_le_bytes());
    *off += 4;
}

pub fn put_u16(buf: &mut [u8], off: &mut usize, v: u16) {
    buf[*off..*off + 2].copy_from_slice(&v.to_le_bytes());
    *off += 2;
}

pub fn put_u8(buf: &mut [u8], off: &mut usize, v: u8) {
    buf[*off] = v;
    *off += 1;
}

pub fn put_u64(buf: &mut [u8], off: &mut usize, v: u64) {
    buf[*off..*off + 8].copy_from_slice(&v.to_le_bytes());
    *off += 8;
}

pub fn put_str(buf: &mut [u8], off: &mut usize, s: &str) {
    put_u16(buf, off, s.len() as u16);
    buf[*off..*off + s.len()].copy_from_slice(s.as_bytes());
    *off += s.len();
}

pub fn encode_stat(buf: &mut [u8], off: &mut usize, qid: QidWire, mode: u32, length: u64, name: &str) {
    let start = *off;
    put_u16(buf, off, 0); // size placeholder
    put_u16(buf, off, 0); // type
    put_u32(buf, off, 0); // dev
    *off += qid.encode(&mut buf[*off..]);
    put_u32(buf, off, mode);
    put_u32(buf, off, 0); // atime
    put_u32(buf, off, 0); // mtime
    put_u64(buf, off, length);
    put_str(buf, off, name);
    put_str(buf, off, "sys");
    put_str(buf, off, "sys");
    put_str(buf, off, "sys");
    // Plan 9: size is byte count of fields *after* the 16-bit size field itself.
    let stat_size = (*off - start - 2) as u16;
    buf[start..start + 2].copy_from_slice(&stat_size.to_le_bytes());
}

/// Stat blob for [Rstat]: Linux `p9pdu_readf(..., "wS", ...)` expects a u16 length prefix
/// before the self-sized stat encoding used in directory listings and elsewhere.
pub fn encode_stat_rstat(
    buf: &mut [u8],
    off: &mut usize,
    qid: QidWire,
    mode: u32,
    length: u64,
    name: &str,
) {
    let wrap = *off;
    put_u16(buf, off, 0);
    let blob = *off;
    encode_stat(buf, off, qid, mode, length, name);
    let blob_len = (*off - blob) as u16;
    buf[wrap..wrap + 2].copy_from_slice(&blob_len.to_le_bytes());
}

pub fn write_rerror(out: &mut [u8], tag: u16, ename: &str) -> usize {
    let mut o = 0usize;
    put_u32(out, &mut o, 0);
    put_u8(out, &mut o, RERROR);
    put_u16(out, &mut o, tag);
    put_str(out, &mut o, ename);
    let size = o as u32;
    out[0..4].copy_from_slice(&size.to_le_bytes());
    o
}

pub fn write_rversion(out: &mut [u8], tag: u16, msize: u32) -> usize {
    write_rversion_str(out, tag, msize, VERSION)
}

pub fn write_rversion_str(out: &mut [u8], tag: u16, msize: u32, version: &str) -> usize {
    let mut o = 0usize;
    put_u32(out, &mut o, 0);
    put_u8(out, &mut o, RVERSION);
    put_u16(out, &mut o, tag);
    put_u32(out, &mut o, msize);
    put_str(out, &mut o, version);
    let size = o as u32;
    out[0..4].copy_from_slice(&size.to_le_bytes());
    o
}

pub fn rerror(tag: u16, ename: &str) -> heapless::Vec<u8, 256> {
    let mut out = heapless::Vec::new();
    let mut tmp = [0u8; 256];
    let mut o = 0usize;
    put_u32(&mut tmp, &mut o, 0);
    put_u8(&mut tmp, &mut o, RERROR);
    put_u16(&mut tmp, &mut o, tag);
    put_str(&mut tmp, &mut o, ename);
    let size = o as u32;
    tmp[0..4].copy_from_slice(&size.to_le_bytes());
    out.extend_from_slice(&tmp[..o]).ok();
    out
}

pub fn rversion(tag: u16, msize: u32) -> heapless::Vec<u8, 128> {
    let mut out = heapless::Vec::new();
    let mut tmp = [0u8; 128];
    let n = write_rversion(&mut tmp, tag, msize);
    let _ = out.extend_from_slice(&tmp[..n]);
    out
}
