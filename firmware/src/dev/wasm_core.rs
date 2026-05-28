//! Core-1 blocking loop that runs WAMR so core-0 Embassy tasks (audio, 9P) keep polling.

use core::sync::atomic::{AtomicBool, Ordering};

use esp_hal::delay::Delay;
use esp_hal::interrupt::software::SoftwareInterrupt;
use esp_hal::peripherals::CPU_CTRL;
use esp_hal::system::Stack;
use esp_println::println;

/// Core-1 RTOS thread stack (WAMR `wasm_runtime_init_wasi` + interpreter frames).
///
/// Must live in internal `.dram2_uninit` (~72 KiB total region). Do not allocate the stack
/// from PSRAM — AppCpu cannot use external RAM as a stack without special IDF options.
const CORE1_STACK_BYTES: usize = 32 * 1024;

#[unsafe(link_section = ".dram2_uninit")]
static mut CORE1_STACK: Stack<CORE1_STACK_BYTES> = Stack::new();

static CORE1_STARTED: AtomicBool = AtomicBool::new(false);

/// Start the core-1 scheduler thread that polls [`devices::wasm::run_pending`].
///
/// Call once after [`esp_rtos::start`] on core 0.
pub fn start(cpu_ctrl: CPU_CTRL<'static>, int1: SoftwareInterrupt<'static, 1>) {
    if CORE1_STARTED.swap(true, Ordering::AcqRel) {
        return;
    }
    let stack = unsafe { &mut *core::ptr::addr_of_mut!(CORE1_STACK) };
    println!(
        "wasm: starting core-1 worker (stack {} KiB dram2)",
        CORE1_STACK_BYTES / 1024
    );
    esp_rtos::start_second_core(cpu_ctrl, int1, stack, || {
        println!("wasm: core-1 worker running");
        let delay = Delay::new();
        loop {
            if devices::wasm::run_pending() {
                continue;
            }
            delay.delay_millis(5);
        }
    });
}
