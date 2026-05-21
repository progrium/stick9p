#![no_std]

extern crate alloc;

pub mod board;
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
