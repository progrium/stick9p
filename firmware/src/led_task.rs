use devices::led::LedState;
use embassy_time::{Duration, Timer};
use esp_hal::gpio::Output;

#[embassy_executor::task]
pub async fn run(mut led: Output<'static>) {
    let mut on = false;
    loop {
        let st = devices::led::get();
        match st {
            LedState::On => {
                led.set_high();
                Timer::after(Duration::from_millis(50)).await;
            }
            LedState::Off => {
                led.set_low();
                Timer::after(Duration::from_millis(50)).await;
            }
            LedState::Blink { hi_ms, lo_ms } => {
                if on {
                    led.set_high();
                    Timer::after(Duration::from_millis(hi_ms as u64)).await;
                } else {
                    led.set_low();
                    Timer::after(Duration::from_millis(lo_ms as u64)).await;
                }
                on = !on;
            }
        }
    }
}
