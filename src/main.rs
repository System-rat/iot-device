use std::{
    sync::mpsc::{channel, Sender},
    thread::JoinHandle,
};

use anyhow::{Context, Result};
use embedded_hal::delay::DelayNs;
use esp_idf_hal::{
    delay::FreeRtos,
    gpio::{AnyIOPin, Input, Output, PinDriver},
    peripherals::Peripherals,
};
use esp_idf_svc::{
    eventloop::EspSystemEventLoop,
    nvs::{EspDefaultNvsPartition, EspNvs},
    wifi::{ClientConfiguration, Configuration, EspWifi, WifiEvent},
};
use log::{error, info};

const WIFI_SSID: &str = env!("wifi_ssid");
const WIFI_PASSWORD: &str = env!("wifi_password");

const WEB_SOCKET_URL: &str = "";
const DEVICE_ID: &str = "";
const DEVICE_KEY: &str = "";

const RELAY_COUNT: usize = 2;
const BUTTON_COUNT: usize = 2;

#[derive(Copy, Clone, Debug)]
enum RelayMessage {
    Toggle,
    On,
    Off,
}

fn main() -> Result<()> {
    esp_idf_svc::sys::link_patches();

    esp_idf_svc::log::EspLogger::initialize_default();

    let mut nvs = EspNvs::new(
        EspDefaultNvsPartition::take().context("No NVS!")?,
        "config_stuff",
        true,
    )?;

    let p = Peripherals::take()?;

    let buttons: Vec<PinDriver<AnyIOPin, Input>> = vec![
        PinDriver::input(AnyIOPin::from(p.pins.gpio1))?,
        PinDriver::input(AnyIOPin::from(p.pins.gpio2))?,
    ];

    let relays: Vec<PinDriver<AnyIOPin, Output>> = vec![
        PinDriver::output(AnyIOPin::from(p.pins.gpio4))?,
        PinDriver::output(AnyIOPin::from(p.pins.gpio5))?,
    ];

    let event_loop = EspSystemEventLoop::take()?;

    let mut wifi = EspWifi::new(p.modem, event_loop.clone(), None)?;

    wifi.set_configuration(&Configuration::Client(
        ClientConfiguration {
            ssid: WIFI_SSID.try_into().unwrap(),
            password: WIFI_PASSWORD.try_into().unwrap(),
            ..Default::default()
        },
    ))?;

    wifi.start()?;
    wifi.connect()?;

    let _event = event_loop.subscribe::<WifiEvent, _>(|e| {
        info!("WiFi Event: {:?}", e);
    })?;

    let tx = relay_control_thread(relays);
    let _t = button_control(buttons, tx.clone());

    loop {
        FreeRtos.delay_ms(50);
    }
}

fn button_control(
    buttons: Vec<PinDriver<'static, AnyIOPin, Input>>,
    sender: Sender<(usize, RelayMessage)>,
) -> JoinHandle<()> {
    let mut states = [false; BUTTON_COUNT];
    std::thread::spawn(move || loop {
        for i in 0..BUTTON_COUNT {
            if states[i] && buttons[i].is_low() {
                info!("Sending for {}", i);
                if let Err(e) = sender.send((i, RelayMessage::Toggle)) {
                    error!("Error during send: {}", e);
                }
            }

            states[i] = buttons[i].is_high();

            FreeRtos.delay_ms(50)
        }
    })
}

fn relay_control_thread(
    relays: Vec<PinDriver<'static, AnyIOPin, Output>>,
) -> Sender<(usize, RelayMessage)> {
    let (tx, rx) = channel::<(usize, RelayMessage)>();

    info!("Relays: {}", relays.len());

    let mut relays = relays;

    std::thread::spawn(move || loop {
        if let Ok(msg) = rx.recv() {
            info!("Got msg: {:?}", msg);
            match msg {
                (id, RelayMessage::Toggle) => {
                    if let Err(e) = relays[id].toggle() {
                        error!("Error during relay toggle: {}", e);
                    }
                }
                (id, RelayMessage::On) => {
                    if let Err(e) = relays[id].set_high() {
                        error!("Error during relay set high: {}", e);
                    }
                }
                (id, RelayMessage::Off) => {
                    if let Err(e) = relays[id].set_low() {
                        error!("Error during relay set low: {}", e);
                    }
                }
            }
        }
    });

    tx
}
