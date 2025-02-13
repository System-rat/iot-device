use std::{
    sync::mpsc::{channel, Receiver, Sender},
    thread::JoinHandle,
    time::Duration,
};

use anyhow::{Context, Result};
use dht_sensor::DhtReading;
use embedded_hal::{delay::DelayNs, digital::StatefulOutputPin};
use esp_idf_hal::{
    delay::{Ets, FreeRtos},
    gpio::{AnyIOPin, IOPin, Input, Output, PinDriver},
    peripherals::Peripherals,
};
use esp_idf_svc::{
    eventloop::EspSystemEventLoop,
    nvs::{EspDefaultNvsPartition, EspNvs},
    wifi::{ClientConfiguration, Configuration, EspWifi, WifiEvent},
    ws::client::{EspWebSocketClient, EspWebSocketClientConfig, WebSocketEventType},
};
use log::{error, info};
use serde::{Deserialize, Serialize};

const WIFI_SSID: &str = env!("wifi_ssid");
const WIFI_PASSWORD: &str = env!("wifi_password");

const WEB_SOCKET_URL: &str = env!("url");
const DEVICE_ID: &str = env!("device_id");
const DEVICE_KEY: &str = env!("device_key");

const RELAY_COUNT: usize = 2;
const BUTTON_COUNT: usize = 2;

#[derive(Copy, Clone, Debug)]
enum RelayMessage {
    Toggle,
    On,
    Off,
    Status,
}

#[derive(Serialize)]
struct RelayStatus {
    id: usize,
    state: bool,
}

#[derive(Serialize)]
enum Telemetry {
    Relay(RelayStatus),
    Sensor((i8, u8)),
    Empty,
}

#[derive(Serialize)]
struct TelemetryMessage {
    device_id: String,
    telemetry: Telemetry,
}

#[derive(Deserialize)]
struct RelayRemoteMessage {
    id: usize,
    state: bool,
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

    wifi.set_configuration(&Configuration::Client(ClientConfiguration {
        ssid: WIFI_SSID.try_into().unwrap(),
        password: WIFI_PASSWORD.try_into().unwrap(),
        channel: None,
        auth_method: esp_idf_svc::wifi::AuthMethod::WPA2Personal,
        ..Default::default()
    }))?;

    wifi.start()?;
    wifi.connect()?;

    let _event = event_loop.subscribe::<WifiEvent, _>(move |e| {
        info!("WiFi Event: {:?}", e);
        if matches!(e, WifiEvent::StaDisconnected) {
            let _ = wifi.connect();
        }
    })?;

    let (telemetry_tx, telemetry_rx) = channel::<TelemetryMessage>();

    let tx = relay_control_thread(relays, telemetry_tx.clone());
    let _button_thread = button_control(buttons, tx.clone());
    let _sensor_thread = sensor_control(p.pins.gpio3.downgrade(), telemetry_tx.clone())?;
    let wifi_tx = tx.clone();

    let mut ws = EspWebSocketClient::new(
        &format!(
            "{}?device_id={}&auth_password={}",
            WEB_SOCKET_URL, DEVICE_ID, DEVICE_KEY
        ),
        &EspWebSocketClientConfig {
            network_timeout_ms: Duration::from_secs(2),
            reconnect_timeout_ms: Duration::from_secs(2),
            ..Default::default()
        },
        Duration::from_secs(2),
        move |e| {
            if let Ok(we) = e {
                if let WebSocketEventType::Text(txt) = we.event_type {
                    info!("Got message: {}", txt);
                    if let Ok(msg) = serde_json::from_str::<RelayRemoteMessage>(txt) {
                        if msg.state {
                            let _ = tx.send((msg.id, RelayMessage::Status));
                        } else {
                            let _ = tx.send((msg.id, RelayMessage::Toggle));
                        }
                    }
                }
            }
        },
    )?;

    while let Ok(tel) = telemetry_rx.recv() {
        let _ = ws.send(
            esp_idf_svc::ws::FrameType::Text(false),
            serde_json::to_string(&tel)
                .unwrap_or("".to_string())
                .as_bytes(),
        );
    }

    Ok(())
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
    telemetry_tx: Sender<TelemetryMessage>,
) -> Sender<(usize, RelayMessage)> {
    let (tx, rx) = channel::<(usize, RelayMessage)>();

    info!("Relays: {}", relays.len());

    let mut relays = relays;

    std::thread::spawn(move || loop {
        if let Ok(msg) = rx.recv() {
            info!("Got msg: {:?}", msg);
            let mut rid = 0;
            match msg {
                (id, RelayMessage::Toggle) => {
                    if let Err(e) = relays[id].toggle() {
                        error!("Error during relay toggle: {}", e);
                    }

                    rid = id;
                }
                (id, RelayMessage::On) => {
                    if let Err(e) = relays[id].set_high() {
                        error!("Error during relay set high: {}", e);
                    }

                    rid = id;
                }
                (id, RelayMessage::Off) => {
                    if let Err(e) = relays[id].set_low() {
                        error!("Error during relay set low: {}", e);
                    }

                    rid = id;
                }
                (id, RelayMessage::Status) => {
                    info!("Reporting status");
                    rid = id;
                }
            }

            let _ = telemetry_tx.send(TelemetryMessage {
                device_id: DEVICE_ID.to_string(),
                telemetry: Telemetry::Relay(RelayStatus {
                    id: rid,
                    state: relays[rid].is_set_high(),
                }),
            });
        }
    });

    tx
}

fn sensor_control(
    dht_pin: AnyIOPin,
    telemetry_tx: Sender<TelemetryMessage>,
) -> Result<JoinHandle<()>> {
    let mut driver = PinDriver::input_output(dht_pin)?;
    driver.set_pull(esp_idf_hal::gpio::Pull::Up)?;

    FreeRtos.delay_ms(1000);

    Ok(std::thread::spawn(move || loop {
        match dht_sensor::dht11::Reading::read(&mut Ets, &mut driver) {
            Ok(reading) => {
                info!(
                    "Sensor reading: temp = {} hum = {}",
                    reading.temperature, reading.relative_humidity
                );
                let _ = telemetry_tx.send(TelemetryMessage {
                    device_id: DEVICE_ID.to_string(),
                    telemetry: Telemetry::Sensor((reading.temperature, reading.relative_humidity)),
                });
            }
            Err(e) => {
                error!("Sensor reading error: {:?}", e);
            }
        }

        FreeRtos.delay_ms(3000);
    }))
}
