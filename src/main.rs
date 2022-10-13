use embedded_svc::{http::Headers, wifi::*};
use esp_idf_hal::mutex::{Condvar, Mutex};
use esp_idf_svc::{netif, nvs::EspDefaultNvs, sysloop, wifi::EspWifi};
use esp_idf_sys as _; // If using the `binstart` feature of `esp-idf-sys`, always keep this module imported
use esp_idf_sys::{self as _};

use log::*;
use std::env;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

mod http_static;

#[macro_use]
extern crate dotenv_codegen;

const WIFI_SSID_KEY: &str = dotenv!("WSSID");
const WIFI_PASS_KEY: &str = dotenv!("WPASS");
const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() -> anyhow::Result<()> {
    // Temporary. Will disappear once ESP-IDF 4.4 is released, but for now it is necessary to call this function once,
    // or else some patches to the runtime implemented by esp-idf-sys might not link properly.
    esp_idf_sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

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
        thread::sleep(Duration::from_secs(1));
    }
}

trait Multipart {
    fn get_boundary(&self) -> Option<&str>;
}
impl Multipart for esp_idf_svc::http::server::EspHttpRequest<'_> {
    fn get_boundary(&self) -> Option<&str> {
        match self.header("Content-Type") {
            Some(b) => {
                let r: Vec<&str> = b.split('=').collect();
                if r.len() == 2 {
                    Some(r[1])
                } else {
                    eprint!("Error: Boundary string = {b}");
                    None
                }
            }
            None => {
                eprint!("Error: No Boundary string");
                None
            }
        }
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
            // resp.send_str(http_static::_SERVER_OFFLINE)?; // Use for AP mode
            resp.send_str(http_static::_SERVER_ONLINE)?; // Use for STA mode
            Ok(())
        })?
        // *********** OTA POST handler
        .handle_post(
            "/otaupload",
            |mut req, resp| -> Result<(), embedded_svc::http::server::HandlerError> {
                let mut ota = esp_ota::OtaUpdate::begin()?;
                let start_time = std::time::Instant::now();
                let mut ota_bytes_counter = 0;
                let mut multipart_bytes_counter = 0;

                let len = {
                    match req.content_len() {
                        Some(val) => {
                            if val == 0 {
                                return Err(anyhow::anyhow!("Multipart POST len = 0").into());
                            } else {
                                val.to_owned()
                            }
                        }
                        None => return Err(anyhow::anyhow!("Multipart POST len = 0").into()),
                    }
                };
                let boundary = {
                    match req.get_boundary() {
                        Some(val) => val.to_owned(),
                        None => return Err(anyhow::anyhow!("No boundary string").into()),
                    }
                };

                let mut buf = [0u8; 2048];

                info!("Type: {:?} Len: {} bytes", &boundary, len);

                while let Ok(bytelen) = req.reader().read(&mut buf) {
                    if start_time.elapsed() > Duration::from_millis(900) {
                        std::thread::sleep(Duration::from_millis(10)) //wdt
                    }
                    if bytelen == 0 {
                        break;
                    }
                    multipart_bytes_counter += bytelen;

                    let payload = extract_payload(&buf[..bytelen], &boundary);

                    ota_bytes_counter += &payload.len();

                    match ota.write(payload) {
                        Ok(_) => {
                            info!(
                                "Upload: {}%",
                                (multipart_bytes_counter as f32 / len as f32) * 100.0
                            )
                        }
                        Err(e) => {
                            println!("Error! {e}\n{:02x?}", payload);
                            resp.send_str(&format!(
                                "<h1>Flashed {} bytes {}% FAILED</h1><br><br><p>{:02x?}</p>",
                                multipart_bytes_counter,
                                multipart_bytes_counter as f32 / len as f32 * 100.0,
                                payload
                            ))?;
                            return Ok(());
                        }
                    }
                }

                match ota.finalize() {
                    Ok(mut completed_ota) => {
                        info!("Flashed {} bytes OK", ota_bytes_counter);

                        completed_ota.set_as_boot_partition()?;
                        info!("Set as boot partition - restarting");

                        // this panics due to WIFI issue - also hangs browser in AP mode
                        // thread '<unnamed>' panicked at 'ESP-IDF ERROR: ESP_ERR_WIFI_NOT_STARTED',
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
            },
        )?;
    Ok(server)
}

fn extract_payload<'a>(buf: &'a [u8], boundary: &'a str) -> &'a [u8] {
    if twoway::find_bytes(buf, boundary.as_bytes()).is_none() {
        return buf;
    }
    let left_offset = match twoway::find_bytes(buf, &[13, 10, 13, 10]) {
        Some(v) => v + 4,
        None => 0,
    };

    let right_offset = match twoway::rfind_bytes(buf, &[13, 10, 45, 45]) {
        Some(v) => v,
        None => buf.len(),
    };
    &buf[left_offset..right_offset]
}
