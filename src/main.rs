use embedded_svc::http::server::registry::Registry;
use embedded_svc::http::server::{Request, Response};
// use embedded_svc::http::Headers;
use embedded_svc::io::adapters::ToStd;
use esp_idf_hal::mutex::{Condvar, Mutex};
use esp_idf_svc::http::server::EspHttpServer;
use esp_idf_svc::netif::EspNetifStack;
use esp_idf_svc::nvs::EspDefaultNvs;
use esp_idf_svc::nvs_storage::EspNvsStorage;
use esp_idf_svc::sysloop::EspSysLoopStack;
use esp_idf_svc::wifi::EspWifi;
use esp_idf_sys as _; // If using the `binstart` feature of `esp-idf-sys`, always keep this module imported
use esp_idf_sys::{self as _};

use crate::configuration::{AppConfiguration, NvsStruct};
use lazy_static::lazy_static;
use log::*;
use std::env;
use std::io::Read;
use std::sync::Arc;
use std::thread;
use std::time::Duration;
mod configuration;
mod ota;
mod wifi_init;
#[macro_use]
extern crate dotenv_codegen;

const VERSION: &str = env!("CARGO_PKG_VERSION");

const HTMLOTA: &str = include_str!("html/ota.html");

const HTMLSETTINGS: &str = include_str!("html/settings.html");

const HTMLINDEX: &str = include_str!("html/index.html");

const WIFI_SSID_KEY: &str = dotenv!("WSSID");
const WIFI_PASS_KEY: &str = dotenv!("WPASS");
const AP_SSID_KEY: &str = dotenv!("APSSID");
const AP_PASS_KEY: &str = dotenv!("APPASS");

use std::sync::RwLock;

lazy_static! {
    static ref APP_CONFIG: RwLock<AppConfiguration> = RwLock::new(AppConfiguration::default());
}

fn main() -> anyhow::Result<()> {
    // Temporary. Will disappear once ESP-IDF 4.4 is released, but for now it is necessary to call this function once,
    // or else some patches to the runtime implemented by esp-idf-sys might not link properly.
    esp_idf_sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();
    let nvs = Arc::new(EspDefaultNvs::new()?);
    let nvs_storage = Arc::new(RwLock::new(
        EspNvsStorage::new_default(nvs.clone(), "config", true).unwrap(),
    ));
    let request_restart = Arc::new(Mutex::new(false));

    if let Ok(mut app_config) = APP_CONFIG.write() {
        if let Err(e) = app_config.init(nvs_storage.clone()) {
            panic!("{e}");
        };
        if let Some(val) = app_config.ap.channel {
            app_config.ap.channel = Some(val + 1);
            app_config.ap.write_to_nvs(&nvs_storage.clone())?;
        } else {
            app_config.ap.channel = Some(1);
            app_config.ap.write_to_nvs(&nvs_storage.clone())?;
        }
    }
    #[allow(unused)]
    let netif_stack = Arc::new(EspNetifStack::new()?);
    #[allow(unused)]
    let sys_loop_stack = Arc::new(EspSysLoopStack::new()?);

    let (sta, ap) = if let Ok(app_config) = APP_CONFIG.read() {
        (app_config.sta.to_owned(), app_config.ap.to_owned())
    } else {
        panic!()
    };
    let mut wifi = Box::new(EspWifi::new(netif_stack, sys_loop_stack, nvs)?);
    let wifi_scan = wifi_init::scan(&mut wifi);
    wifi = wifi_init::wifi(wifi, sta, ap)?;

    let mutex = Arc::new((Mutex::new(None), Condvar::new()));
    let httpd = httpd(mutex, request_restart.clone())?;

    println!("FW version: {} testing", VERSION);
    println!("{:?}", wifi_scan);
    esp_ota::mark_app_valid();

    loop {
        if *request_restart.lock() {
            log::info!("Restart requested");
            thread::sleep(Duration::from_secs(2));
            break;
        }
        thread::sleep(Duration::from_secs(1));
    }
    drop(httpd);
    info!("Httpd stopped");

    {
        drop(wifi);
        info!("Wifi stopped");
    }

    if *request_restart.lock() {
        unsafe {
            info!("Restarting...");
            esp_idf_sys::esp_restart();
        }
    }
    Ok(())
}
fn httpd(
    _mutex: Arc<(Mutex<Option<u32>>, Condvar)>,
    request_restart: Arc<Mutex<bool>>,
) -> anyhow::Result<EspHttpServer> {
    let mut server = EspHttpServer::new(&Default::default())?;

    server
        .handle_get("/id", |_req, resp| {
            resp.send_str("OTATest")?;
            Ok(())
        })?
        .handle_get("/", move |_req, resp| {
            resp.send_str(HTMLINDEX)?;
            Ok(())
        })?
        .handle_get("/json", move |_req, resp| {
            let config = APP_CONFIG.read().unwrap().to_owned();
            if let Ok(payload) = serde_json::to_string(&config) {
                resp.send_bytes(payload.as_bytes())?;
            }
            Ok(())
        })?
        .handle_get("/restart", |_req, resp| {
            info!("Restart requested");
            resp.send_str("Rebooting")?;
            unsafe { esp_idf_sys::esp_restart() } // no execution beyond this point
            Ok(())
        })?
        .handle_get("/settings", |_req, resp| {
            resp.send_str(HTMLSETTINGS)?;
            Ok(())
        })?
        .handle_post(
            "/settings",
            |mut req, resp| -> Result<(), embedded_svc::http::server::HandlerError> {
                let mut body = Vec::new();

                ToStd::new(req.reader()).read_to_end(&mut body)?;

                let ssid = url::form_urlencoded::parse(&body)
                    .filter(|p| p.0 == "ssid")
                    .map(|p| p.1.to_string())
                    .next()
                    .unwrap();
                let pass = url::form_urlencoded::parse(&body)
                    .filter(|p| p.0 == "pass")
                    .map(|p| p.1.to_string())
                    .next()
                    .unwrap();
                if let Ok(mut app_config) = APP_CONFIG.write() {
                    app_config.sta.ssid = Some(ssid.to_owned());
                    app_config.sta.pass = Some(pass.to_owned());
                    app_config.store_values_to_nvs()?;
                }
                resp.send_str(&format!(
                    "Wifi setup completed for SSID: {} Password: {}, please reboot to connect",
                    ssid, pass
                ))?;

                Ok(())
            },
        )?
        .handle_get("/ota", |_req, resp| {
            resp.send_str(HTMLOTA)?;
            Ok(())
        })?
        // *********** OTA POST handler
        .handle_post(
            "/ota",
            move |req, resp| -> Result<(), embedded_svc::http::server::HandlerError> {
                let response = match ota::ota_processing(req) {
                    Ok(elapsed) => {
                        if let Some(time) = elapsed {
                            *request_restart.lock() = true;
                            format!(
                                "Flashed device in {:?} - Rebooting in 2 seconds",
                                time.elapsed()
                            )
                        } else {
                            "Error?".to_string()
                        }
                    }
                    Err(_) => todo!(),
                };
                resp.send_str(&response)?;
                Ok(())
            },
        )?;
    Ok(server)
}
