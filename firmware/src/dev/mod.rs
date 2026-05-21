//! Board peripheral tasks (Stage 2 + Stage 3 mic).

#[cfg(feature = "board-plus2")]
mod mic;

#[cfg(feature = "board-plus2")]
mod plus2;

#[cfg(feature = "board-plus2")]
pub use plus2::spawn;
