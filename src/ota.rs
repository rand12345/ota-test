use anyhow::anyhow;
use anyhow::Result;
use embedded_svc::http::server::HandlerError;
use embedded_svc::http::Headers;
use embedded_svc::io::Write;
use embedded_svc::ota::{Ota, OtaSlot, OtaUpdate};
use esp_idf_svc::http::server::EspHttpRequest;
use esp_idf_svc::ota::*;
// use esp_ota::*;
use log::info;

use embedded_svc::io::Read;
use std::time::{Duration, Instant};

use embedded_svc::http::server::Request;

pub fn mark_app_valid(ok: bool) -> anyhow::Result<()> {
    let mut ota = EspOta::new()?;
    if ok {
        if let Err(e) = ota.mark_running_slot_valid() {
            return Err(anyhow!("{}", e));
        } else {
            let slot = ota.get_running_slot()?;
            info!(
                "Running slot: Firmware info = {:?}",
                slot.get_firmware_info()
            );
            info!("Running slot: Label info = {:?}", slot.get_label());
            info!("Running slot: State info = {:?}", slot.get_state());
            let slot = ota.get_update_slot()?;
            info!(
                "Updating slot: Firmware info = {:?}",
                slot.get_firmware_info()
            );
            info!("Updating slot: Label info = {:?}", slot.get_label());
            info!("Updating slot: State info = {:?}", slot.get_state());
            return Ok(());
        }
    }
    ota.mark_running_slot_invalid_and_reboot();
    Ok(())
}

pub fn ota_processing(
    mut req: EspHttpRequest,
    // resp: &EspHttpResponse,
) -> Result<Option<Instant>, HandlerError> {
    if req.content_len().is_none() || req.content_len() == Some(0) {
        return Err(anyhow!("Multipart POST len is None").into());
    };

    if req.get_boundary().is_none() {
        return Err(anyhow!("No boundary string, check multipart form POST").into());
    } else {
        info!("Using boundary: {}", req.get_boundary().unwrap());
    }
    let start_time = Instant::now();
    let mut ota = EspOta::new().unwrap();
    let mut ota_update = ota.initiate_update().unwrap();

    // let mut ota = OtaUpdate::begin()?;
    let mut ota_bytes_counter = 0;
    let mut multipart_bytes_counter = 0;
    let mut buf = Box::new([0u8; 1440 * 3]);
    while let Ok(bytelen) = req.reader().read(&mut *buf) {
        if start_time.elapsed() > Duration::from_millis(900) {
            std::thread::sleep(Duration::from_millis(10)) //wdt
        }
        if bytelen == 0 {
            break;
        }
        let payload = req.extract_payload(&buf[..bytelen]);
        multipart_bytes_counter += bytelen;
        ota_bytes_counter += &payload.len();

        if let Err(e) = ota_update.write_all(payload) {
            info!("failed to write update with: {:?}", e);
            ota_update.abort()?;
            return Err(anyhow!(
                "Flashed failed at {} bytes\n{}",
                multipart_bytes_counter,
                String::from_utf8_lossy(payload)
            )
            .into());
        } else {
            info!(
                "Recieved {bytelen}b, flashed {}b -> Progeress {}%",
                &payload.len(),
                (multipart_bytes_counter as f32 / req.content_len().unwrap() as f32) * 100.0
            );
        }
    }
    if let Err(e) = ota_update.complete() {
        eprintln!("OTA Error at completion {e}");
        return Err(anyhow!("Flashed failed at completion stage").into());
    };

    Ok(Some(start_time))
}

// fn finalise_ota(
//     ota: esp_ota::OtaUpdate,
//     ota_bytes_counter: usize,
//     // resp: &EspHttpResponse,
//     start_time: Instant,
// ) -> Result<Option<Instant>, HandlerError> {
// }

trait Multipart {
    fn get_boundary(&self) -> Option<&str>;
    fn extract_payload<'a>(&self, buf: &'a [u8]) -> &'a [u8];
}

impl Multipart for EspHttpRequest<'_> {
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
    fn extract_payload<'a>(&self, buf: &'a [u8]) -> &'a [u8] {
        if twoway::find_bytes(buf, &[13, 10]).is_none() {
            return buf;
        }
        if twoway::find_bytes(buf, self.get_boundary().unwrap().as_bytes()).is_none() {
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
}
