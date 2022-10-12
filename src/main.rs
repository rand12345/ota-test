#![allow(unused_imports)]
#![allow(clippy::single_component_path_imports)]
#![feature(slice_take)]
#![feature(never_type)]
use embedded_svc::{http::Headers, wifi::*};
use esp_idf_hal::mutex::{Condvar, Mutex};
use esp_idf_svc::{netif, nvs::EspDefaultNvs, sysloop, wifi::EspWifi};
use esp_idf_sys as _; // If using the `binstart` feature of `esp-idf-sys`, always keep this module imported
use esp_idf_sys::esp;
use esp_idf_sys::{self as _};
use std::env;

use log::*;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

mod http_static;

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[allow(dead_code)]
const WIFI_SSID_KEY: &str = env!("WSSID");
#[allow(dead_code)]
const WIFI_PASS_KEY: &str = env!("WPASS");

fn main() -> anyhow::Result<()> {
    // Temporary. Will disappear once ESP-IDF 4.4 is released, but for now it is necessary to call this function once,
    // or else some patches to the runtime implemented by esp-idf-sys might not link properly.
    esp_idf_sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();
    first_run_validate()?;
    println!("V2");
    let uptime = std::time::Instant::now();
    // first_run_validate()?;
    let nvs = Arc::new(EspDefaultNvs::new()?);
    let mut wifi = Box::new(EspWifi::new(
        Arc::new(netif::EspNetifStack::new()?),
        Arc::new(sysloop::EspSysLoopStack::new()?),
        nvs,
    )?);

    wifi.set_configuration(&Configuration::Client(ClientConfiguration {
        ssid: WIFI_SSID_KEY.into(),
        password: WIFI_PASS_KEY.into(),
        ..Default::default()
    }))?;
    // wait for wifi
    std::thread::sleep(std::time::Duration::from_secs(5));

    let mutex = Arc::new((Mutex::new(None), Condvar::new()));
    let _httpd = httpd(mutex)?;

    println!("FW version: {}", VERSION);
    loop {
        println!("Uptime! {:?}", uptime.elapsed().as_secs());
        thread::sleep(Duration::from_secs(60));
    }
}

fn httpd(
    _mutex: Arc<(Mutex<Option<u32>>, Condvar)>,
) -> anyhow::Result<esp_idf_svc::http::server::EspHttpServer> {
    use embedded_svc::http::server::registry::Registry;
    use embedded_svc::http::server::{Request, Response};
    use embedded_svc::io::Read;

    let mut server = esp_idf_svc::http::server::EspHttpServer::new(&Default::default())?;

    server
        .handle_get("/", |_req, resp| {
            resp.send_str("Hello from Rust!")?;
            Ok(())
        })?
        .handle_get("/ota", |_req, resp| {
            resp.send_str(http_static::SERVER)?;
            Ok(())
        })?
        // *********** OTA POST handler
        .handle_post(
            "/otaupload",
            |mut req, resp| -> Result<(), embedded_svc::http::server::HandlerError> {
                let mut ota = esp_ota::OtaUpdate::begin()?;
                let mut counter = 0;
                if let Some(len) = req.content_len() {
                    let mut buf = [0u8; 2048];

                    info!(
                        "Encoding: {:?} Len: {} bytes ",
                        req.reader().content_encoding(),
                        len
                    );

                    while let Ok(bytelen) = req.reader().read(&mut buf) {
                        if bytelen == 0 {
                            break;
                        }
                        let payload = extract_payload(&buf, bytelen);
                        counter += &buf.len();
                        info!("Rx: {} bytes", payload.len());

                        match ota.write(payload) {
                            Ok(_) => {
                                info!("Upload: {}%", (counter as f32 / len as f32) * 100.0)
                            }
                            Err(e) => {
                                println!("Error! {e}\n{:02x?}", payload);
                                resp.send_str(&format!(
                                    "<h1>Flashed {} bytes {}% FAILED</h1><br><br><p>{:02x?}</p>",
                                    counter,
                                    counter / len * 100,
                                    payload
                                ))?;
                                return Ok(());
                            }
                        }
                    }

                    match ota.finalize() {
                        Ok(mut completed_ota) => {
                            info!("Flashed {} bytes OK", len);
                            resp.send_str(&format!(
                                "<h1>Flashed {} bytes OK</h1><br><br><p>Restarting ESP32</p>",
                                len,
                            ))?;
                            completed_ota.set_as_boot_partition()?;
                            info!("Set as boot partition");
                            completed_ota.restart();
                        }
                        Err(e) => {
                            resp.send_str(&format!(
                                "<h1>Flashed {} bytes FAILED</h1><br><br><p>{e}</p>",
                                len,
                            ))?;
                        }
                    };
                    Ok(())
                } else {
                    resp.send_str("Len None!")?;
                    Ok(())
                }
            },
        )?
        .handle_get("/reboot", |_req, _resp| panic!("User requested a reboot!"))?;
    Ok(server)
}

fn extract_payload(buf: &[u8; 2048], bytelen: usize) -> &[u8] {
    // Broken - need to find a crate to extract data from multipart (rfc7578)
    let left_offset = match twoway::find_bytes(buf, &[13, 10, 13, 10]) {
        Some(v) => v + 4,
        None => 0,
    };
    let right_offset = match twoway::rfind_bytes(buf, &[45, 45, 13, 10]) {
        Some(v) => v,
        None => bytelen,
    };
    let mut fudge = 0;
    if (buf[right_offset - 2] == 13u8 && buf[right_offset - 1] == 10u8)
        || (buf[right_offset - 1] == 13u8 && buf[right_offset - 2] == 10u8)
    {
        println!(
            "PANIC -2 {} -1 {}",
            buf[right_offset - 2],
            buf[right_offset - 1]
        );
        fudge = 2
    }

    let new_buf = &buf[left_offset..right_offset - fudge];

    {
        // debugging
        if twoway::find_bytes(new_buf, "--".as_bytes()).is_some() {
            println!(
                "E1 {:?}\n{:02x?}\n{}",
                new_buf,
                new_buf,
                String::from_utf8_lossy(new_buf)
            );
            panic!()
        }
        if twoway::find_bytes(new_buf, &[13, 10]).is_some() {
            println!(
                "E2 {:?}\n{:02x?}\n{}",
                new_buf,
                new_buf,
                String::from_utf8_lossy(new_buf)
            );
            panic!()
        }
    }
    new_buf
}

fn first_run_validate() -> anyhow::Result<()> {
    unsafe {
        let cur_partition = esp_idf_sys::esp_ota_get_running_partition();
        let mut ota_state: esp_idf_sys::esp_ota_img_states_t = 0;
        if let Ok(()) = esp!(esp_idf_sys::esp_ota_get_state_partition(
            cur_partition,
            &mut ota_state
        )) {
            if ota_state == esp_idf_sys::esp_ota_img_states_t_ESP_OTA_IMG_PENDING_VERIFY {
                // Validate image
                esp!(esp_idf_sys::esp_ota_mark_app_valid_cancel_rollback())?;
            }
        }
    }
    Ok(())
}
