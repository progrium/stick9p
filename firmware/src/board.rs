//! Board pin definitions.

#[cfg(feature = "board-plus2")]
pub mod pins {
    use esp_hal::gpio::{Level, Output, OutputConfig};

    pub fn init_hold(pin: esp_hal::peripherals::GPIO4<'_>) -> Output<'_> {
        let mut hold = Output::new(pin, Level::Low, OutputConfig::default());
        hold.set_high();
        hold
    }

    pub fn init_led(pin: esp_hal::peripherals::GPIO19<'_>) -> Output<'_> {
        Output::new(pin, Level::Low, OutputConfig::default())
    }
}

pub const BOARD_NAME: &str = {
    #[cfg(feature = "board-plus2")]
    {
        "plus2"
    }
    #[cfg(feature = "board-sticks3")]
    {
        "sticks3"
    }
};

pub const FW_VERSION: &str = "stick9p-0.5.1-stage4-i2c-gpio";
