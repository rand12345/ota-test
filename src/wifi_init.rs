use crate::AP_PASS_KEY;
use crate::AP_SSID_KEY;
use crate::WIFI_PASS_KEY;
use crate::WIFI_SSID_KEY;
use anyhow::{bail, Result};
use embedded_svc::ipv4::{self};
use embedded_svc::ping::Ping;
use embedded_svc::wifi::*;
// use esp_idf_svc::netif::EspNetifStack;
// use esp_idf_svc::nvs::EspDefaultNvs;
use esp_idf_svc::ping;
// use esp_idf_svc::sysloop::EspSysLoopStack;
use esp_idf_svc::wifi::EspWifi;
use log::info;
// use std::sync::Arc;
use std::time::Duration;

use crate::configuration;

pub fn wifi(
    wifi: Box<EspWifi>,
    sta: configuration::Wifi,
    ap: configuration::Wifi,
) -> Result<Box<EspWifi>> {
    match sta.ssid.is_some() && sta.pass.is_some() {
        true => wifimixed(wifi, sta, ap),
        false => wifiap(wifi, ap),
    }
}

pub fn scan(wifi: &mut Box<EspWifi>) -> anyhow::Result<Vec<AccessPointInfo>> {
    // let ap_infos = ;
    // let mut out = "".to_string();
    let mut ours: Vec<AccessPointInfo> = wifi.scan()?.into_iter().collect();
    // for our in ours.iter() {
    //     our.signal_strength
    // }
    ours.sort_by(|a, b| b.signal_strength.cmp(&a.signal_strength));
    for our in ours.iter() {
        println!("{:?}", our);
    }
    Ok(ours.to_owned())
}

fn wifimixed(
    mut wifi: Box<EspWifi>,
    sta: configuration::Wifi,
    ap: configuration::Wifi,
) -> Result<Box<EspWifi>> {
    info!("Wifi created, about to scan");
    let ssid: &str = &sta.ssid.unwrap_or_else(|| WIFI_SSID_KEY.to_string());
    let pass: &str = &sta.pass.unwrap_or_else(|| WIFI_PASS_KEY.to_string());
    let ap_ssid: &str = &ap.ssid.unwrap_or_else(|| AP_SSID_KEY.to_string());
    let ap_pass: &str = &ap.pass.unwrap_or_else(|| AP_PASS_KEY.to_string());

    let ap_infos = wifi.scan()?;

    let ours = ap_infos.into_iter().find(|a| a.ssid.contains(ssid));

    let channel = if let Some(ours) = ours {
        info!(
            "Found configured access point {} on channel {}",
            ssid, ours.channel
        );
        Some(ours.channel)
    } else {
        info!(
            "Configured access point {} not found during scanning, will go with unknown channel",
            ssid
        );
        None
    };

    wifi.set_configuration(&Configuration::Mixed(
        ClientConfiguration {
            ssid: ssid.into(),
            password: pass.into(),
            channel,
            ..Default::default()
        },
        AccessPointConfiguration {
            ssid: ap_ssid.into(),
            password: ap_pass.into(),
            channel: ap.channel.unwrap_or(1),
            ..Default::default()
        },
    ))?;

    info!("Wifi sta/ap configuration set, about to get status");

    wifi.wait_status_with_timeout(Duration::from_secs(60), |status| !status.is_transitional())
        .map_err(|e| anyhow::anyhow!("Unexpected Wifi status: {:?}", e))?;

    let status = wifi.get_status();

    if let Status(
        ClientStatus::Started(ClientConnectionStatus::Connected(ClientIpStatus::Done(ip_settings))),
        ApStatus::Started(ApIpStatus::Done),
    ) = status
    {
        info!("Wifi sta connected");

        ping_init(&ip_settings)?;
    } else {
        bail!("Unexpected sta Wifi status: {:?}", status);
    }

    Ok(wifi)
}

fn wifiap(mut wifi: Box<EspWifi>, ap: configuration::Wifi) -> Result<Box<EspWifi>> {
    // info!("Wifi created, about to scan");
    let ssid: &str = &ap.ssid.unwrap();
    let pass: &str = &ap.pass.unwrap();
    wifi.set_configuration(&Configuration::AccessPoint(AccessPointConfiguration {
        ssid: ssid.into(),
        password: pass.into(),
        channel: ap.channel.unwrap_or(1),
        ..Default::default()
    }))?;

    info!("Wifi ap configuration set, about to get status");

    wifi.wait_status_with_timeout(Duration::from_secs(60), |status| !status.is_transitional())
        .map_err(|e| anyhow::anyhow!("Unexpected Wifi status: {:?}", e))?;

    let status = wifi.get_status();

    if let Status(_, ApStatus::Started(ApIpStatus::Done)) = status {
        info!("Wifi ap connected");
    } else {
        bail!("Unexpected ap Wifi status: {:?}", status);
    }
    Ok(wifi)
}

fn ping_init(ip_settings: &ipv4::ClientSettings) -> Result<()> {
    info!("About to do some sta pings for {:?}", ip_settings);

    let ping_summary =
        ping::EspPing::default().ping(ip_settings.subnet.gateway, &Default::default())?;
    if ping_summary.transmitted != ping_summary.received {
        bail!(
            "Pinging sta gateway {} resulted in timeouts",
            ip_settings.subnet.gateway
        );
    }

    info!("Pinging done");

    Ok(())
}
