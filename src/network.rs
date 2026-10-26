use anyhow::{Context, Result};
use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::OwnedObjectPath;

const NETWORK_MANAGER: &str = "org.freedesktop.NetworkManager";
const NM_PATH: &str = "/org/freedesktop/NetworkManager";
const DEVICE_IFACE: &str = "org.freedesktop.NetworkManager.Device";
const WIFI_DEVICE_IFACE: &str = "org.freedesktop.NetworkManager.Device.Wireless";
const WIFI_DEVICE_TYPE: u32 = 2;

pub fn active_wifi_ssid() -> Result<Option<String>> {
    let connection = Connection::system().context("Could not connect to NetworkManager")?;
    let manager = Proxy::new(
        &connection,
        NETWORK_MANAGER,
        NM_PATH,
        "org.freedesktop.NetworkManager",
    )?;
    let devices: Vec<OwnedObjectPath> = manager
        .call("GetDevices", &())
        .context("Could not list NetworkManager devices")?;

    for path in devices {
        let path = path.to_string();
        let device = Proxy::new(&connection, NETWORK_MANAGER, path.as_str(), DEVICE_IFACE)?;
        let device_type: u32 = device.get_property("DeviceType")?;
        if device_type != WIFI_DEVICE_TYPE {
            continue;
        }

        let wireless = Proxy::new(
            &connection,
            NETWORK_MANAGER,
            path.as_str(),
            WIFI_DEVICE_IFACE,
        )?;
        let access_point: OwnedObjectPath = wireless
            .get_property("ActiveAccessPoint")
            .context("Could not read the active Wi-Fi access point")?;
        if access_point.as_str() == "/" {
            continue;
        }

        let access_point_path = access_point.to_string();
        let access_point_proxy = Proxy::new(
            &connection,
            NETWORK_MANAGER,
            access_point_path.as_str(),
            "org.freedesktop.NetworkManager.AccessPoint",
        )?;
        let ssid: Vec<u8> = access_point_proxy
            .get_property("Ssid")
            .context("Could not read the connected Wi-Fi name")?;
        if !ssid.is_empty() {
            return Ok(Some(String::from_utf8_lossy(&ssid).into_owned()));
        }
    }

    Ok(None)
}
