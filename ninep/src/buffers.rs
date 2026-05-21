//! Static session buffers (lives in `.bss`, not on the executor stack).

/// Max 9P message size (Plus2 uses msize=4096; headroom for attach).
pub const MSG_CAP: usize = 4096;

/// Per-connection 9P encode/decode buffers.
pub struct SessionStorage {
    pub rx: [u8; MSG_CAP],
    pub tx: [u8; MSG_CAP],
    pub work: [u8; MSG_CAP],
}

impl SessionStorage {
    pub const fn new() -> Self {
        Self {
            rx: [0; MSG_CAP],
            tx: [0; MSG_CAP],
            work: [0; MSG_CAP],
        }
    }
}
