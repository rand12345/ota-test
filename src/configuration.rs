// nvs

#![allow(unused_imports)]
#![allow(dead_code)]

use crate::AP_PASS_KEY;
use crate::AP_SSID_KEY;
use crate::WIFI_PASS_KEY;
use crate::WIFI_SSID_KEY;
use anyhow::anyhow;
use anyhow::Context;
use embedded_svc::io::Read;
use serde_json::*;
use std::marker::PhantomData;
use std::sync::Arc;
use std::sync::RwLock;

use embedded_svc::storage::RawStorage;
// use embedded_svc::storage::Storage;
use embedded_svc::storage::StorageBase;
// use embedded_svc::storage::*;
use embedded_svc::{http::Headers, wifi::*};
use esp_idf_hal::mutex::{Condvar, Mutex};
use esp_idf_svc::nvs::EspDefaultNvs;
use esp_idf_svc::nvs::EspNvs;
use esp_idf_svc::nvs_storage::EspNvsStorage;
use esp_idf_sys::esp;
use log::info;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
type KeyPair = std::collections::HashMap<String, serde_json::Value>;

#[derive(Debug, Default, Serialize, Deserialize, Clone)]
pub struct Wifi {
    pub nvs: String,
    pub ssid: Option<String>,
    pub pass: Option<String>,
    pub channel: Option<u8>,
}

#[derive(Debug, Default, Serialize, Deserialize, Clone)]
pub struct BmsSettings {
    pub nvs: String,
}
#[derive(Debug, Default, Serialize, Deserialize, Clone)]
pub struct MqttSettings {
    pub nvs: String,
    pub address: String,
    pub username: String,
    pub password: String,
    pub client_id: String,
    pub base_topic: String,
    pub qos: u8,
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct AppConfiguration {
    name: &'static str,
    pub sta: Wifi,
    pub ap: Wifi,
    pub bms: BmsSettings,
    pub mqtt: MqttSettings,
    #[serde(skip_serializing, skip_deserializing)]
    nvs: Option<Arc<RwLock<EspNvsStorage>>>,
}

pub trait NvsStorage {
    fn get_val(&self, key: &str) -> anyhow::Result<Vec<u8>>;
    fn set_val(&mut self, key: &str, val: &[u8]) -> anyhow::Result<bool, esp_idf_sys::EspError>;
}

impl NvsStorage for EspNvsStorage {
    fn get_val(&self, key: &str) -> anyhow::Result<Vec<u8>> {
        if !self.contains(key)? {
            return Err(anyhow!("NVS Key:{key} Not found"));
        }
        let len = self
            .len(key)?
            .context(format!("NVS Key:{key} Bad len check"))?;
        // let mut buf = Vec::with_capacity(len + 1);
        let mut buf = [0u8; 512];

        self.get_raw(key, &mut buf)?;
        info!("NVS Read {} : {}", key, String::from_utf8_lossy(&buf));
        Ok(buf[0..len].to_owned())
    }

    fn set_val(&mut self, key: &str, val: &[u8]) -> anyhow::Result<bool, esp_idf_sys::EspError> {
        if key.is_empty() {
            panic!("set_val attempted to write to NVS with zero length key")
        }
        info!("NVS Write {} : {}", key, String::from_utf8_lossy(val));
        self.put_raw(key, val)
    }
}

impl AppConfiguration {
    pub fn erase_values_in_nvs(&mut self) -> anyhow::Result<()> {
        let store = self.nvs.as_ref().unwrap().clone();
        info!("Erasing old data in NVS");
        if let Ok(mut store) = store.write() {
            for val in ["ap", "sta", "bms", "mqtt"] {
                match store.remove(val) {
                    Ok(_) => info!("Removed {val}"),
                    Err(e) => info!("Removed {val} failed {}", e),
                };
            }
        }
        Ok(())
    }
    pub fn init(&mut self, nvs: Arc<RwLock<EspNvsStorage>>) -> anyhow::Result<()> {
        // self.store = Some(EspNvsStorage::new_default(default_nvs, namespace, true).unwrap());
        self.nvs = Some(nvs.clone());
        self.ap = Wifi::default();
        self.ap.set_nvs_key("ap".into());
        self.ap.channel = Some(1);
        self.ap.ssid = Some(AP_SSID_KEY.to_owned());
        self.ap.pass = Some(AP_PASS_KEY.to_owned());

        self.sta.set_nvs_key("sta".into());
        self.sta.ssid = Some(WIFI_SSID_KEY.to_owned());
        self.sta.pass = Some(WIFI_PASS_KEY.to_owned());
        self.bms.set_nvs_key("bms".into());
        self.mqtt.set_nvs_key("mqtt".into());
        let valid = if let Ok(store) = nvs.write() {
            store.contains(&self.ap.nvs)?
                && store.contains(&self.sta.nvs)?
                && store.contains(&self.bms.nvs)?
                && store.contains(&self.mqtt.nvs)?
        } else {
            false
        };

        let store = self.nvs.as_ref().unwrap().clone();
        if valid {
            self.ap.read_from_nvs(&store)?;
            self.sta.read_from_nvs(&store)?;
            self.bms.read_from_nvs(&store)?;
            self.mqtt.read_from_nvs(&store)?;
        } else {
            self.erase_values_in_nvs()?;
            self.store_values_to_nvs()?;
        }
        Ok(())
    }

    pub fn store_values_to_nvs(&mut self) -> anyhow::Result<()> {
        let store = self.nvs.as_ref().unwrap().clone();
        info!("Storing all settings to NVS");
        if self.ap.nvs.is_empty() {
            eprintln!("Attempted to call store on an empty");
            self.ap.nvs = "ap".to_string();
        }
        self.ap.write_to_nvs(&store)?;

        if self.sta.nvs.is_empty() {
            eprintln!("Attempted to call store on an empty");
            self.sta.nvs = "sta".to_string();
        }
        self.sta.write_to_nvs(&store)?;

        if self.bms.nvs.is_empty() {
            eprintln!("Attempted to call store on an empty");
            self.bms.nvs = "bms".to_string();
        }
        self.bms.write_to_nvs(&store)?;

        if self.mqtt.nvs.is_empty() {
            eprintln!("Attempted to call store on an empty");
            self.mqtt.nvs = "mqtt".to_string();
        }
        self.mqtt.write_to_nvs(&store)?;
        Ok(())
    }
}
pub trait NvsStruct {
    fn set_nvs_key(&mut self, key: String) -> &mut Self;
    fn read_from_nvs(
        &mut self,
        store: &RwLock<EspNvsStorage>,
    ) -> anyhow::Result<Self, anyhow::Error>
    where
        Self: std::marker::Sized;
    fn write_to_nvs(
        &mut self,
        store: &RwLock<EspNvsStorage>,
    ) -> anyhow::Result<&mut Self, anyhow::Error>
    where
        Self: std::marker::Sized;
}

impl NvsStruct for Wifi {
    fn set_nvs_key(&mut self, key: String) -> &mut Self {
        info!("Setting nvs key to {key}");
        self.nvs = key;
        self
    }
    fn read_from_nvs(
        &mut self,
        store: &RwLock<EspNvsStorage>,
    ) -> anyhow::Result<Self, anyhow::Error> {
        if let Ok(store) = store.read() {
            match store.get_val(&self.nvs) {
                Ok(val) => Ok(serde_json::from_slice(&val)?),
                Err(e) => {
                    eprintln!("{} - Using defaults - Error {}", self.nvs, e);
                    Err(anyhow!("{} Error {}", self.nvs, e))
                }
            }
        } else {
            Err(anyhow!("Failed to get read lock"))
        }
    }

    fn write_to_nvs(
        &mut self,
        store: &RwLock<EspNvsStorage>,
    ) -> anyhow::Result<&mut Self, anyhow::Error> {
        let message = serde_json::to_string(&self)?;
        if let Ok(mut store) = store.write() {
            store.set_val(&self.nvs, message.as_bytes())?;
            Ok(self)
        } else {
            Err(anyhow!("Failed to get write lock"))
        }
    }
}
impl NvsStruct for BmsSettings {
    fn set_nvs_key(&mut self, key: String) -> &mut Self {
        info!("Setting nvs key to {key}");
        self.nvs = key;
        self
    }
    fn read_from_nvs(
        &mut self,
        store: &RwLock<EspNvsStorage>,
    ) -> anyhow::Result<Self, anyhow::Error> {
        if let Ok(store) = store.read() {
            match store.get_val(&self.nvs) {
                Ok(val) => Ok(serde_json::from_slice(&val)?),
                Err(e) => {
                    eprintln!("{} - Using defaults - Error {}", self.nvs, e);
                    Err(anyhow!("{} Error {}", self.nvs, e))
                }
            }
        } else {
            Err(anyhow!("Failed to get read lock"))
        }
    }

    fn write_to_nvs(
        &mut self,
        store: &RwLock<EspNvsStorage>,
    ) -> anyhow::Result<&mut Self, anyhow::Error> {
        let message = serde_json::to_string(&self)?;
        if let Ok(mut store) = store.write() {
            store.set_val(&self.nvs, message.as_bytes())?;
            Ok(self)
        } else {
            Err(anyhow!("Failed to get write lock"))
        }
    }
}
impl NvsStruct for MqttSettings {
    fn set_nvs_key(&mut self, key: String) -> &mut Self {
        info!("Setting nvs key to {key}");
        self.nvs = key;
        self
    }
    fn read_from_nvs(
        &mut self,
        store: &RwLock<EspNvsStorage>,
    ) -> anyhow::Result<Self, anyhow::Error> {
        if let Ok(store) = store.read() {
            match store.get_val(&self.nvs) {
                Ok(val) => Ok(serde_json::from_slice(&val)?),
                Err(e) => {
                    eprintln!("{} - Using defaults - Error {}", self.nvs, e);
                    Err(anyhow!("{} Error {}", self.nvs, e))
                }
            }
        } else {
            Err(anyhow!("Failed to get read lock"))
        }
    }

    fn write_to_nvs(
        &mut self,
        store: &RwLock<EspNvsStorage>,
    ) -> anyhow::Result<&mut Self, anyhow::Error> {
        let message = serde_json::to_string(&self)?;
        if let Ok(mut store) = store.write() {
            store.set_val(&self.nvs, message.as_bytes())?;
            Ok(self)
        } else {
            Err(anyhow!("Failed to get write lock"))
        }
    }
}
