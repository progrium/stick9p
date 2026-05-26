//! PSRAM-backed ramfs for `/tmp` (nested directories and files under the root).

use core::cell::RefCell;
use critical_section::Mutex;
use heapless::String;

/// Base for 9P qid.path values (`path = QID_PATH_BASE + ino`).
pub const QID_PATH_BASE: u64 = 0x0001_0000;
pub const ROOT_INO: u16 = 0;

const MAX_INODES: usize = 64;
const MAX_NAME: usize = 32;
const MAX_FREE: usize = 32;

/// Plan 9 `DMDIR` bit in `Tcreate` perm.
const DMDIR: u32 = 0x8000_0000;

#[derive(Clone, Copy, PartialEq, Eq)]
enum InodeType {
    Free,
    Dir,
    File,
}

struct Inode {
    typ: InodeType,
    parent: u16,
    name: String<MAX_NAME>,
    size: u32,
    data_off: u32,
    qid_vers: u32,
}

impl Inode {
    const FREE: Self = Self {
        typ: InodeType::Free,
        parent: 0,
        name: String::new(),
        size: 0,
        data_off: 0,
        qid_vers: 0,
    };
}

#[derive(Clone, Copy)]
struct FreeBlock {
    off: u32,
    len: u32,
}

struct MemFs {
    inodes: [Inode; MAX_INODES],
    /// Byte address of the PSRAM arena (`usize` so `MemFs` is `Send`).
    arena_ptr: usize,
    arena_len: usize,
    free: heapless::Vec<FreeBlock, MAX_FREE>,
}

static STATE: Mutex<RefCell<MemFs>> = Mutex::new(RefCell::new(MemFs {
    inodes: [Inode::FREE; MAX_INODES],
    arena_ptr: 0,
    arena_len: 0,
    free: heapless::Vec::new(),
}));

/// Reserve `arena_len` bytes at `arena_ptr` for file payloads. Remaining PSRAM
/// should be registered with `esp_alloc` separately.
pub fn init(arena_ptr: *mut u8, arena_len: usize) {
    if arena_ptr.is_null() || arena_len == 0 {
        return;
    }
    critical_section::with(|cs| {
        let mut st = STATE.borrow(cs).borrow_mut();
        st.arena_ptr = arena_ptr as usize;
        st.arena_len = arena_len;
        st.free.clear();
        let _ = st.free.push(FreeBlock {
            off: 0,
            len: arena_len as u32,
        });
        st.inodes = [Inode::FREE; MAX_INODES];
        st.inodes[ROOT_INO as usize] = Inode {
            typ: InodeType::Dir,
            parent: ROOT_INO,
            name: String::new(),
            size: 0,
            data_off: 0,
            qid_vers: 0,
        };
    });
}

pub fn is_ready() -> bool {
    critical_section::with(|cs| {
        let st = STATE.borrow(cs).borrow();
        st.arena_ptr != 0 && st.arena_len > 0
    })
}

/// PSRAM arena bytes: `(free, used, total)`. `None` if `/tmp` was not initialized.
pub fn arena_stats() -> Option<(usize, usize, usize)> {
    critical_section::with(|cs| {
        let st = STATE.borrow(cs).borrow();
        if st.arena_ptr == 0 || st.arena_len == 0 {
            return None;
        }
        let total = st.arena_len;
        let free_bytes: usize = st.free.iter().map(|b| b.len as usize).sum();
        let used = total.saturating_sub(free_bytes);
        Some((free_bytes, used, total))
    })
}

/// Inode slots excluding the fixed `/tmp` root: `(used, total)`.
pub fn inode_stats() -> (usize, usize) {
    const USER_INODES: usize = MAX_INODES - 1;
    critical_section::with(|cs| {
        let st = STATE.borrow(cs).borrow();
        let used = st
            .inodes
            .iter()
            .enumerate()
            .filter(|(i, ent)| *i != ROOT_INO as usize && ent.typ != InodeType::Free)
            .count();
        (used, USER_INODES)
    })
}

pub fn qid_path(ino: u16) -> u64 {
    QID_PATH_BASE + ino as u64
}

pub fn qid_typ(ino: u16) -> u8 {
    if is_dir(ino) {
        0x80 // QT_DIR
    } else {
        0x00 // QT_FILE
    }
}

pub fn qid_vers(ino: u16) -> u32 {
    critical_section::with(|cs| STATE.borrow(cs).borrow().inodes[ino as usize].qid_vers)
}

pub fn mode(ino: u16) -> u32 {
    if is_dir(ino) {
        0x8000_0000 | 0o777
    } else {
        0o100_666
    }
}

pub fn length(ino: u16) -> u64 {
    critical_section::with(|cs| STATE.borrow(cs).borrow().inodes[ino as usize].size as u64)
}

pub fn name(ino: u16) -> heapless::String<MAX_NAME> {
    if ino == ROOT_INO {
        let mut s = String::new();
        let _ = s.push_str("tmp");
        return s;
    }
    critical_section::with(|cs| STATE.borrow(cs).borrow().inodes[ino as usize].name.clone())
}

pub fn is_dir(ino: u16) -> bool {
    critical_section::with(|cs| {
        matches!(
            STATE.borrow(cs).borrow().inodes[ino as usize].typ,
            InodeType::Dir
        )
    })
}

/// Resolve a single path component under `parent`.
pub fn walk(parent: u16, name: &str) -> Option<u16> {
    if name.is_empty() || name == "." {
        return Some(parent);
    }
    if name == ".." {
        return critical_section::with(|cs| {
            let st = STATE.borrow(cs).borrow();
            let ent = &st.inodes[parent as usize];
            if !matches!(ent.typ, InodeType::Dir) {
                return None;
            }
            if parent == ROOT_INO {
                return None;
            }
            Some(ent.parent)
        });
    }
    critical_section::with(|cs| {
        let st = STATE.borrow(cs).borrow();
        if !matches!(st.inodes[parent as usize].typ, InodeType::Dir) {
            return None;
        }
        for (i, ent) in st.inodes.iter().enumerate() {
            if ent.typ == InodeType::Free || i == ROOT_INO as usize {
                continue;
            }
            if ent.parent == parent && ent.name.as_str() == name {
                return Some(i as u16);
            }
        }
        None
    })
}

pub fn child_count(parent: u16) -> usize {
    critical_section::with(|cs| {
        let st = STATE.borrow(cs).borrow();
        st.inodes
            .iter()
            .enumerate()
            .filter(|(i, ent)| {
                *i != ROOT_INO as usize
                    && ent.typ != InodeType::Free
                    && ent.parent == parent
            })
            .count()
    })
}

pub fn child_ino_at(parent: u16, index: usize) -> Option<u16> {
    critical_section::with(|cs| {
        let st = STATE.borrow(cs).borrow();
        let mut n = 0usize;
        for (i, ent) in st.inodes.iter().enumerate() {
            if i == ROOT_INO as usize || ent.typ == InodeType::Free || ent.parent != parent {
                continue;
            }
            if n == index {
                return Some(i as u16);
            }
            n += 1;
        }
        None
    })
}

/// `Tcreate` entry: `perm & DMDIR` creates a directory, otherwise a file.
pub fn create(parent: u16, name: &str, perm: u32) -> Result<u16, &'static str> {
    if !is_ready() {
        return Err("tmp not available");
    }
    validate_name(name)?;
    let is_dir = perm & DMDIR != 0;
    critical_section::with(|cs| {
        let mut st = STATE.borrow(cs).borrow_mut();
        if !matches!(st.inodes[parent as usize].typ, InodeType::Dir) {
            return Err("not a directory");
        }
        if lookup_child(&st.inodes, parent, name).is_some() {
            return Err("file exists");
        }
        let ino = alloc_inode(&mut st.inodes)?;
        let ent = &mut st.inodes[ino as usize];
        ent.typ = if is_dir {
            InodeType::Dir
        } else {
            InodeType::File
        };
        ent.parent = parent;
        ent.name.clear();
        let _ = ent.name.push_str(name);
        ent.size = 0;
        ent.data_off = 0;
        ent.qid_vers = ent.qid_vers.wrapping_add(1);
        Ok(ino)
    })
}

pub fn remove(ino: u16) -> Result<(), &'static str> {
    if ino == ROOT_INO {
        return Err("is a directory");
    }
    critical_section::with(|cs| {
        let mut st = STATE.borrow(cs).borrow_mut();
        let typ = st.inodes[ino as usize].typ;
        if typ == InodeType::Free {
            return Err("not found");
        }
        if typ == InodeType::Dir {
            if dir_has_children(&st.inodes, ino) {
                return Err("directory not empty");
            }
            st.inodes[ino as usize] = Inode::FREE;
            return Ok(());
        }
        let (data_off, size) = {
            let ent = &st.inodes[ino as usize];
            (ent.data_off, ent.size)
        };
        if size > 0 {
            free_block(&mut st.free, data_off, size);
        }
        st.inodes[ino as usize] = Inode::FREE;
        Ok(())
    })
}

pub fn truncate(ino: u16) {
    critical_section::with(|cs| {
        let mut st = STATE.borrow(cs).borrow_mut();
        let (data_off, size, vers) = {
            let ent = &st.inodes[ino as usize];
            if ent.typ != InodeType::File {
                return;
            }
            (ent.data_off, ent.size, ent.qid_vers)
        };
        if size > 0 {
            free_block(&mut st.free, data_off, size);
        }
        let ent = &mut st.inodes[ino as usize];
        ent.size = 0;
        ent.data_off = 0;
        ent.qid_vers = vers.wrapping_add(1);
    })
}

pub fn read(ino: u16, off: u64, buf: &mut [u8]) -> usize {
    critical_section::with(|cs| {
        let st = STATE.borrow(cs).borrow();
        let ent = &st.inodes[ino as usize];
        if ent.typ != InodeType::File {
            return 0;
        }
        if off >= ent.size as u64 || ent.size == 0 || st.arena_ptr == 0 {
            return 0;
        }
        let arena =
            unsafe { core::slice::from_raw_parts(st.arena_ptr as *const u8, st.arena_len) };
        let start = off as usize;
        let avail = (ent.size as usize).saturating_sub(start);
        let n = avail.min(buf.len());
        if n == 0 {
            return 0;
        }
        let src = &arena[ent.data_off as usize..][start..start + n];
        buf[..n].copy_from_slice(src);
        n
    })
}

pub fn write(ino: u16, off: u64, data: &[u8]) -> Result<usize, &'static str> {
    if data.is_empty() {
        return Ok(0);
    }
    critical_section::with(|cs| {
        let mut st = STATE.borrow(cs).borrow_mut();
        if st.arena_ptr == 0 {
            return Err("tmp not available");
        }
        let arena =
            unsafe { core::slice::from_raw_parts_mut(st.arena_ptr as *mut u8, st.arena_len) };
        let end = off.saturating_add(data.len() as u64);
        if end > u32::MAX as u64 {
            return Err("file too large");
        }
        let new_size = {
            let ent = &st.inodes[ino as usize];
            if ent.typ != InodeType::File {
                return Err("not a file");
            }
            end.max(ent.size as u64) as u32
        };
        if new_size > st.inodes[ino as usize].size {
            let (old_off, old_size) = {
                let ent = &st.inodes[ino as usize];
                (ent.data_off, ent.size)
            };
            let new_off = alloc_block(&mut st.free, new_size)?;
            if old_size > 0 {
                let old_len = old_size as usize;
                let src = old_off as usize;
                let dst = new_off as usize;
                arena.copy_within(src..src + old_len, dst);
                free_block(&mut st.free, old_off, old_size);
            }
            let ent = &mut st.inodes[ino as usize];
            ent.data_off = new_off;
            ent.size = new_size;
        }
        let (data_off, size) = {
            let ent = &st.inodes[ino as usize];
            (ent.data_off, ent.size)
        };
        let start = off as usize;
        let end_usize = start + data.len();
        if end_usize > size as usize {
            return Err("write past size");
        }
        arena[data_off as usize + start..][..data.len()].copy_from_slice(data);
        Ok(data.len())
    })
}

fn validate_name(name: &str) -> Result<(), &'static str> {
    if name.is_empty() || name == "." || name == ".." {
        return Err("invalid name");
    }
    if name.len() > MAX_NAME {
        return Err("name too long");
    }
    if name.bytes().any(|b| b == b'/') {
        return Err("invalid name");
    }
    Ok(())
}

fn dir_has_children(inodes: &[Inode; MAX_INODES], dir: u16) -> bool {
    inodes.iter().enumerate().any(|(i, ent)| {
        i != ROOT_INO as usize && ent.typ != InodeType::Free && ent.parent == dir
    })
}

fn lookup_child(inodes: &[Inode; MAX_INODES], parent: u16, name: &str) -> Option<u16> {
    for (i, ent) in inodes.iter().enumerate() {
        if ent.typ == InodeType::Free || i == ROOT_INO as usize {
            continue;
        }
        if ent.parent == parent && ent.name.as_str() == name {
            return Some(i as u16);
        }
    }
    None
}

fn alloc_inode(inodes: &mut [Inode; MAX_INODES]) -> Result<u16, &'static str> {
    for (i, ent) in inodes.iter_mut().enumerate() {
        if i == ROOT_INO as usize {
            continue;
        }
        if ent.typ == InodeType::Free {
            *ent = Inode::FREE;
            return Ok(i as u16);
        }
    }
    Err("no inodes")
}

fn alloc_block(free: &mut heapless::Vec<FreeBlock, MAX_FREE>, need: u32) -> Result<u32, &'static str> {
    let mut best_idx = None;
    let mut best_len = u32::MAX;
    for (i, blk) in free.iter().enumerate() {
        if blk.len >= need && blk.len < best_len {
            best_len = blk.len;
            best_idx = Some(i);
        }
    }
    let idx = best_idx.ok_or("no space")?;
    let blk = free[idx];
    if blk.len == need {
        let _ = free.remove(idx);
    } else {
        free[idx].off = blk.off + need;
        free[idx].len = blk.len - need;
    }
    Ok(blk.off)
}

fn free_block(free: &mut heapless::Vec<FreeBlock, MAX_FREE>, off: u32, len: u32) {
    if len == 0 {
        return;
    }
    let _ = free.push(FreeBlock { off, len });
    coalesce(free);
}

fn coalesce(free: &mut heapless::Vec<FreeBlock, MAX_FREE>) {
    if free.len() < 2 {
        return;
    }
    let mut items: heapless::Vec<FreeBlock, MAX_FREE> = heapless::Vec::new();
    for blk in free.iter().copied() {
        let _ = items.push(blk);
    }
    free.clear();
    let _ = items.sort_unstable_by_key(|b| b.off);
    let mut cur = items[0];
    for blk in items.iter().skip(1) {
        if cur.off + cur.len == blk.off {
            cur.len += blk.len;
        } else {
            let _ = free.push(cur);
            cur = *blk;
        }
    }
    let _ = free.push(cur);
}
