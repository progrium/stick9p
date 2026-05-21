//! Minimal 9P wire types shared by the static filesystem (`fs.rs`) and server.
//!
//! See DESIGN.md §5 — the tree is a fixed `fs::Node` enum plus `FsContext` callbacks,
//! not async `Node` / `Handle` traits.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Qid {
    pub typ: u8,
    pub vers: u32,
    pub path: u64,
}

pub const QT_DIR: u8 = 0x80;
pub const QT_FILE: u8 = 0x00;
