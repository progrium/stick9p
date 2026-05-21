//! Root `/README` — embedded from workspace `USAGE.md` at compile time.

/// Full usage guide served at the 9P root as `README`.
pub const TEXT: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../USAGE.md"));
