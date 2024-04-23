use anyhow::Result;
use embedded_hal::{delay::DelayNs, digital::InputPin};
use embedded_svc::wifi::Wifi;
use esp_idf_hal::{delay::FreeRtos, gpio::PinDriver, peripherals::Peripherals};
use esp_idf_svc::{
    eventloop::EspSystemEventLoop,
    ipv4::ClientConfiguration,
    wifi::{Configuration, EspWifi},
};
use log::info;

fn main() -> Result<()> {
    esp_idf_svc::sys::link_patches();

    esp_idf_svc::log::EspLogger::initialize_default();

    let p = Peripherals::take()?;

    let button1 = PinDriver::input(p.pins.gpio1)?;

    info!("It works lol");

    let event_loop = EspSystemEventLoop::take()?;

    let mut wifi = EspWifi::new(p.modem, event_loop.clone(), None)?;

    loop {
        if button1.is_high() {
            info!("ON");
        }

        FreeRtos.delay_ms(50);
    }
}
