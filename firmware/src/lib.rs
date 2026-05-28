#![no_std]

#[cfg(all(feature = "board-plus2", feature = "board-sticks3"))]
compile_error!("enable only one board feature: use --no-default-features --features board-plus2 or board-sticks3");

extern crate alloc;

pub mod board;
pub mod boot_gate;
pub mod dev;
pub mod led_task;
pub mod net;
pub mod nvs;
pub mod transport;

#[macro_export]
macro_rules! mk_static {
    ($t:ty, $val:expr) => {{
        static STATIC_CELL: static_cell::StaticCell<$t> = static_cell::StaticCell::new();
        #[deny(unused_attributes)]
        STATIC_CELL.uninit().write(($val))
    }};
}
