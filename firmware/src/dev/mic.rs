//! Plus2 SPM1423 PDM mic — GPIO0 CLK (I2S WS out), GPIO34 DATA (I2S DIN).

use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use esp_hal::dma_buffers;
use esp_hal::i2s::master::{Channels, Config, I2s, WsWidth};
use esp_println::println;

use devices::mic;

const RX_DMA_BYTES: usize = 4096;
const TX_DMA_BYTES: usize = 512;
const SCRATCH: usize = 1024;

/// ESP32 PDM RX: enable PDM decoder (esp-hal TDM path leaves `rx_pdm_en` cleared).
fn enable_pdm_rx() {
    use esp_hal::peripherals::I2S0;
    let regs = I2S0::regs();
    regs.pdm_conf().modify(|_, w| {
        w.rx_pdm_en().set_bit();
        w.pdm2pcm_conv_en().set_bit();
        w
    });
    regs.conf().modify(|_, w| w.rx_mono().set_bit());
}

pub fn spawn(
    spawner: &Spawner,
    i2s0: esp_hal::peripherals::I2S0<'static>,
    dma: esp_hal::peripherals::DMA_I2S0<'static>,
    pin_clk: esp_hal::peripherals::GPIO0<'static>,
    pin_data: esp_hal::peripherals::GPIO34<'static>,
) {
    spawner.spawn(mic_task(i2s0, dma, pin_clk, pin_data).unwrap());
}

#[embassy_executor::task]
async fn mic_task(
    i2s0: esp_hal::peripherals::I2S0<'static>,
    dma: esp_hal::peripherals::DMA_I2S0<'static>,
    pin_clk: esp_hal::peripherals::GPIO0<'static>,
    pin_data: esp_hal::peripherals::GPIO34<'static>,
) {
    let _ = mic::rate_hz();
    let config = Config::new_tdm_philips()
        .with_channels(Channels::MONO)
        .with_ws_width(WsWidth::Bit);

    let i2s = match I2s::new(i2s0, dma, config) {
        Ok(i) => i.into_async(),
        Err(e) => {
            println!("mic: I2S init err {:?}", e);
            loop {
                Timer::after(Duration::from_secs(30)).await;
            }
        }
    };

    let (mut rx_buffer, rx_descriptors, mut tx_buffer, tx_descriptors) =
        dma_buffers!(RX_DMA_BYTES, TX_DMA_BYTES);

    // PDM clock is I2S WS on GPIO0 (M5). Master clocks are driven from the TX unit.
    let mut i2s_tx = i2s.i2s_tx.with_ws(pin_clk).build(tx_descriptors);
    let mut i2s_rx = i2s.i2s_rx.with_din(pin_data).build(rx_descriptors);

    enable_pdm_rx();

    let mut tx_xfer = match i2s_tx.write_dma_circular_async(tx_buffer) {
        Ok(t) => t,
        Err(e) => {
            println!("mic: TX DMA (clock) err {:?}", e);
            loop {
                Timer::after(Duration::from_secs(30)).await;
            }
        }
    };

    let mut rx_xfer = match i2s_rx.read_dma_circular_async(rx_buffer) {
        Ok(t) => t,
        Err(e) => {
            println!("mic: RX DMA err {:?}", e);
            loop {
                Timer::after(Duration::from_secs(30)).await;
            }
        }
    };

    println!("mic: PDM ready (ws=GPIO0 din=GPIO34 44100Hz mono s16)");

    let mut scratch = [0u8; SCRATCH];
    let silence = [0u8; 256];

    loop {
        // Keep WS/BCLK running (feed silence into TX circular DMA).
        match tx_xfer.available().await {
            Ok(avail) if avail > 0 => {
                let n = avail.min(silence.len());
                let _ = tx_xfer.push(&silence[..n]).await;
            }
            Ok(_) => {}
            Err(e) => {
                println!("mic: tx avail err {:?}", e);
            }
        }

        match rx_xfer.available().await {
            Ok(avail) if avail > 0 => {
                let n = avail.min(scratch.len());
                match rx_xfer.pop(&mut scratch[..n]).await {
                    Ok(got) if got > 0 && mic::is_running() => mic::push_pcm(&scratch[..got]),
                    Ok(_) => {}
                    Err(e) => println!("mic: rx pop err {:?}", e),
                }
            }
            Ok(_) => {}
            Err(e) => println!("mic: rx avail err {:?}", e),
        }
    }
}
