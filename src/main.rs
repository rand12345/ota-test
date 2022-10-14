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

#[macro_use]
extern crate dotenv_codegen;

const WIFI_SSID_KEY: &str = dotenv!("WSSID");
const WIFI_PASS_KEY: &str = dotenv!("WPASS");
const VERSION: &str = env!("CARGO_PKG_VERSION");
const SERVER: &str = include_str!("server.html");

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
        .handle_get("/restart", |_req, _resp| {
            info!("Restart requested");
            unsafe { esp_idf_sys::esp_restart() } // no execution beyond this point
            Ok(())
        })?
        .handle_get("/ota", |_req, resp| {
            resp.send_str(SERVER)?;
            Ok(())
        })?
        // *********** OTA POST handler
        .handle_post(
            "/ota",
            |mut req, resp| -> Result<(), embedded_svc::http::server::HandlerError> {
                let mut ota = esp_ota::OtaUpdate::begin()?;
                let start_time = std::time::Instant::now();
                let mut ota_bytes_counter = 0;
                let mut multipart_bytes_counter = 0;

                let len = {
                    if let Some(val) = req.content_len() {
                        if val == 0 {
                            return Err(anyhow::anyhow!("Multipart POST len = 0").into());
                        } else {
                            val.to_owned()
                        }
                    } else {
                        return Err(anyhow::anyhow!("Multipart POST len is None").into());
                    }
                };
                let boundary = {
                    match req.get_boundary() {
                        Some(val) => val.to_owned(),
                        None => {
                            return Err(anyhow::anyhow!(
                                "No boundary string - check multipart form POST"
                            )
                            .into());
                        }
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
                            drop(ota);
                            return Err(anyhow::anyhow!(
                                "Flashed failed at {} bytes",
                                multipart_bytes_counter,
                            )
                            .into());
                        }
                    }
                }

                if let Ok(mut completed_ota) = ota.finalize() {
                    info!("Flashed {} bytes OK", ota_bytes_counter);

                    completed_ota.set_as_boot_partition()?;
                    info!("Set as boot partition - restart required");

                    // send plin string back for html alert box
                    resp.send_str(&format!(
                        "Flashed {multipart_bytes_counter} bytes in {:?} - Reboot",
                        start_time.elapsed()
                    ))?;
                } else {
                    // drop(ota);
                    return Err(anyhow::anyhow!("Flashed {} bytes failed", len).into());
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
