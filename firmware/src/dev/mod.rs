//! Board peripheral tasks (Stage 2 + Stage 3 mic).

pub mod gpio;
pub mod i2c1;
pub mod wasm_core;

#[cfg(feature = "board-plus2")]
mod mic;

#[cfg(feature = "board-plus2")]
mod plus2;

#[cfg(feature = "board-plus2")]
pub use plus2::spawn;

#[cfg(feature = "board-sticks3")]
mod sticks3;

#[cfg(feature = "board-sticks3")]
pub use sticks3::spawn;
