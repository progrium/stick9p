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

#[cfg(feature = "server")]
pub mod server;
