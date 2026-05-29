#![no_std]

extern crate alloc;

pub mod vfs;
pub mod wire;

#[cfg(feature = "server")]
pub mod buffers;

#[cfg(feature = "server")]
mod readme;

#[cfg(feature = "server")]
pub mod fs;

#[cfg(all(feature = "server", feature = "wamr"))]
pub mod vfs_ffi;

#[cfg(feature = "server")]
pub mod server;
