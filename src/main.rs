#![no_std]
#![no_main]
#![feature(type_alias_impl_trait)]
#![feature(async_fn_in_trait)]
#![allow(incomplete_features)]

mod pio;

use defmt::*;
use embassy_executor::Spawner;
use embassy_rp::gpio::{Level, Output};
use embassy_rp::peripherals::{DMA_CH0, PIN_23, PIN_25};
use embassy_rp::pio::{Pio0, PioPeripherial, PioStateMachineInstance, Sm0};
use embassy_time::{Duration, Timer};
use static_cell::StaticCell;
use {defmt_rtt as _, panic_probe as _};

use crate::pio::PioSpi;

macro_rules! singleton {
    ($val:expr) => {{
        type T = impl Sized;
        static STATIC_CELL: StaticCell<T> = StaticCell::new();
        STATIC_CELL.init_with(move || $val)
    }};
}

//  Runs the background task for managing the cyw43 chip. This has to be done in a separate task because it never finishes, so it would block the main loop. It needs to be constantly run in the background in order to support the wifi chip.
// see https://github.com/embassy-rs/cyw43/issues/32 for more info
#[embassy_executor::task]
async fn wifi_task(
    runner: cyw43::Runner<
        'static,
        Output<'static, PIN_23>,
        PioSpi<PIN_25, PioStateMachineInstance<Pio0, Sm0>, DMA_CH0>,
    >,
) -> ! {
    runner.run().await
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = embassy_rp::init(Default::default());

    let cs = Output::new(p.PIN_25, Level::High);
    let (_, sm, _, _, _) = p.PIO0.split();

    let dma = p.DMA_CH0;
    let spi = PioSpi::new(sm, cs, p.PIN_24, p.PIN_29, dma);

    // Include the WiFi firmware and Country Locale Matrix (CLM) blobs.
    let fw = include_bytes!("../firmware/43439A0.bin");
    let clm = include_bytes!("../firmware/43439A0_clm.bin");

    // To make flashing faster for development, you may want to flash the firmwares independently
    // at hardcoded addresses, instead of baking them into the program with `include_bytes!`:
    //     probe-rs-cli download 43439A0.bin --format bin --chip RP2040 --base-address 0x10100000
    //     probe-rs-cli download 43439A0_clm.bin --format bin --chip RP2040 --base-address 0x10140000
    // let fw = unsafe { core::slice::from_raw_parts(0x10100000 as *const u8, 224190) };
    // let clm = unsafe { core::slice::from_raw_parts(0x10140000 as *const u8, 4752) };

    let pwr = Output::new(p.PIN_23, Level::Low);

    let state = singleton!(cyw43::State::new());
    let (_, mut control, runner) = cyw43::new(state, pwr, spi, fw).await;

    unwrap!(spawner.spawn(wifi_task(runner)));
    control.init(clm).await;
    // Since we're just using the gpio, we can make sure that we're in low power mode for the WiFi chip. Note we could probably use the experimental aggressive mode, but power save is likely more reliable

    control
        .set_power_management(cyw43::PowerManagementMode::PowerSave)
        .await;

    loop {
        Timer::after(Duration::from_secs(1)).await;
        control.gpio_set(0, true).await;
        Timer::after(Duration::from_secs(1)).await;
        control.gpio_set(0, false).await;
    }
}
