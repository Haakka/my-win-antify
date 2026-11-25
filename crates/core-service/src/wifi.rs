use std::{
    ffi::{c_char, c_void},
    mem::{self, size_of},
    ptr::{self, NonNull},
    slice,
};

use shared::{AppConfig, CurrentWifiInfo};
use tokio::sync::Notify;

const LOAD_LIBRARY_SEARCH_SYSTEM32: u32 = 0x0800;
const CONNECTED: u32 = 1;
const CURRENT_CONNECTION: u32 = 7;
const NOTIFICATION_SOURCE_ACM: u32 = 0x08;

static CHANGED: Notify = Notify::const_new();

type OpenHandle = unsafe extern "system" fn(u32, *const c_void, *mut u32, *mut isize) -> u32;
type EnumInterfaces = unsafe extern "system" fn(isize, *const c_void, *mut *mut c_void) -> u32;
type QueryInterface = unsafe extern "system" fn(
    isize,
    *const Guid,
    u32,
    *const c_void,
    *mut u32,
    *mut *mut c_void,
    *mut u32,
) -> u32;
type Callback = unsafe extern "system" fn(*const c_void, *mut c_void);
type RegisterNotification = unsafe extern "system" fn(
    isize,
    u32,
    i32,
    Option<Callback>,
    *mut c_void,
    *const c_void,
    *mut u32,
) -> u32;
type FreeMemory = unsafe extern "system" fn(*mut c_void);
type CloseHandle = unsafe extern "system" fn(isize, *const c_void) -> u32;

#[repr(C)]
struct Guid {
    data1: u32,
    data2: u16,
    data3: u16,
    data4: [u8; 8],
}

#[repr(C)]
struct InterfaceInfo {
    guid: Guid,
    description: [u16; 256],
    state: u32,
}

#[repr(C)]
struct InterfaceList {
    count: u32,
    _index: u32,
}

// Prefix of WLAN_CONNECTION_ATTRIBUTES through wlanAssociationAttributes.dot11Ssid.
#[repr(C)]
struct ConnectionSsid {
    state: u32,
    _mode: u32,
    _profile_name: [u16; 256],
    length: u32,
    ssid: [u8; 32],
}

#[link(name = "kernel32")]
extern "system" {
    fn LoadLibraryExW(path: *const u16, file: isize, flags: u32) -> isize;
    fn GetProcAddress(module: isize, name: *const c_char) -> *const c_void;
    fn FreeLibrary(module: isize) -> i32;
}

struct Library(isize);

impl Drop for Library {
    fn drop(&mut self) {
        unsafe { FreeLibrary(self.0) };
    }
}

struct WlanMemory(NonNull<c_void>, FreeMemory);

impl Drop for WlanMemory {
    fn drop(&mut self) {
        unsafe { (self.1)(self.0.as_ptr()) };
    }
}

pub(crate) struct Session {
    handle: isize,
    enumerate: EnumInterfaces,
    query: QueryInterface,
    free: FreeMemory,
    close: CloseHandle,
    _library: Library,
}

impl Session {
    pub fn open(watch: bool) -> Option<Self> {
        let path: Vec<_> = "wlanapi.dll\0".encode_utf16().collect();
        let module = unsafe { LoadLibraryExW(path.as_ptr(), 0, LOAD_LIBRARY_SEARCH_SYSTEM32) };
        if module == 0 {
            return None;
        }
        let library = Library(module);
        macro_rules! resolve {
            ($name:literal, $ty:ty) => {{
                let symbol = unsafe { GetProcAddress(module, $name.as_ptr()) };
                if symbol.is_null() {
                    return None;
                }
                // Each symbol from the system DLL uses the signature declared above.
                unsafe { mem::transmute::<*const c_void, $ty>(symbol) }
            }};
        }

        let open = resolve!(c"WlanOpenHandle", OpenHandle);
        let enumerate = resolve!(c"WlanEnumInterfaces", EnumInterfaces);
        let query = resolve!(c"WlanQueryInterface", QueryInterface);
        let register = resolve!(c"WlanRegisterNotification", RegisterNotification);
        let free = resolve!(c"WlanFreeMemory", FreeMemory);
        let close = resolve!(c"WlanCloseHandle", CloseHandle);
        let (mut version, mut handle) = (0, 0);
        if unsafe { open(2, ptr::null(), &mut version, &mut handle) } != 0 {
            return None;
        }
        if watch {
            // Registration can fail independently of queries. Polling still runs.
            unsafe {
                register(
                    handle,
                    NOTIFICATION_SOURCE_ACM,
                    1,
                    Some(changed),
                    ptr::null_mut(),
                    ptr::null(),
                    ptr::null_mut(),
                )
            };
        }
        Some(Self {
            handle,
            enumerate,
            query,
            free,
            close,
            _library: library,
        })
    }

    pub fn current(&self) -> Option<CurrentWifiInfo> {
        let mut data = ptr::null_mut();
        if unsafe { (self.enumerate)(self.handle, ptr::null(), &mut data) } != 0 {
            return None;
        }
        let memory = WlanMemory(NonNull::new(data)?, self.free);
        let list = unsafe { memory.0.cast::<InterfaceList>().as_ref() };
        if list.count == 0 {
            return Some(unavailable());
        }
        // Windows allocates count trailing WLAN_INTERFACE_INFO records after the header.
        let first = unsafe {
            memory
                .0
                .cast::<InterfaceList>()
                .as_ptr()
                .add(1)
                .cast::<InterfaceInfo>()
        };
        let interfaces = unsafe { slice::from_raw_parts(first, list.count as usize) };
        let mut info = CurrentWifiInfo {
            supported: true,
            ..unavailable()
        };
        for interface in interfaces
            .iter()
            .filter(|interface| interface.state == CONNECTED)
        {
            if info.interface_name.is_none() {
                info.connected = true;
                info.interface_name = Some(description(interface));
            }
            if let Some(ssid) = self.ssid(interface) {
                info.ssid = Some(ssid);
                info.interface_name = Some(description(interface));
                break;
            }
        }
        Some(info)
    }

    fn ssid(&self, interface: &InterfaceInfo) -> Option<String> {
        let (mut size, mut data) = (0, ptr::null_mut());
        if unsafe {
            (self.query)(
                self.handle,
                &interface.guid,
                CURRENT_CONNECTION,
                ptr::null(),
                &mut size,
                &mut data,
                ptr::null_mut(),
            )
        } != 0
        {
            return None;
        }
        let memory = WlanMemory(NonNull::new(data)?, self.free);
        if (size as usize) < size_of::<ConnectionSsid>() {
            return None;
        }
        let connection = unsafe { memory.0.cast::<ConnectionSsid>().as_ref() };
        if connection.state != CONNECTED {
            return None;
        }
        let ssid = connection.ssid.get(..connection.length as usize)?;
        (!ssid.is_empty()).then(|| String::from_utf8_lossy(ssid).into_owned())
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        unsafe { (self.close)(self.handle, ptr::null()) };
    }
}

unsafe extern "system" fn changed(_: *const c_void, _: *mut c_void) {
    CHANGED.notify_one();
}

pub(crate) async fn notification() {
    CHANGED.notified().await;
}

fn description(interface: &InterfaceInfo) -> String {
    let end = interface
        .description
        .iter()
        .position(|&ch| ch == 0)
        .unwrap_or(256);
    String::from_utf16_lossy(&interface.description[..end])
}

pub(crate) fn unavailable() -> CurrentWifiInfo {
    CurrentWifiInfo {
        supported: false,
        connected: false,
        ssid: None,
        interface_name: None,
    }
}

pub(crate) fn profile_to_activate<'a>(
    config: &'a AppConfig,
    info: &CurrentWifiInfo,
) -> Option<&'a str> {
    let ssid = info.ssid.as_deref()?.trim();
    if !config.automation.enabled || !info.connected {
        return None;
    }
    // Config order breaks ties between enabled rules for the same SSID.
    let rule = config.automation.wifi_rules.iter().find(|rule| {
        rule.enabled
            && rule.ssid.trim() == ssid
            && config
                .profiles
                .iter()
                .any(|profile| profile.id == rule.profile_id)
    })?;
    (rule.profile_id != config.active_profile_id).then_some(rule.profile_id.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::WifiProfileRule;

    #[test]
    fn switches_only_for_an_enabled_matching_rule_and_an_existing_profile() {
        let mut config = AppConfig::default();
        let mut profile = config.profiles[0].clone();
        profile.id = "work".into();
        config.profiles.push(profile);
        config.automation.wifi_rules.push(WifiProfileRule {
            id: "office".into(),
            ssid: "Office".into(),
            profile_id: "work".into(),
            enabled: true,
        });
        let mut wifi = CurrentWifiInfo {
            supported: true,
            connected: true,
            ssid: Some(" Office ".into()),
            interface_name: None,
        };
        assert_eq!(profile_to_activate(&config, &wifi), None);
        config.automation.enabled = true;
        assert_eq!(profile_to_activate(&config, &wifi), Some("work"));
        config.automation.wifi_rules.insert(
            0,
            WifiProfileRule {
                id: "invalid".into(),
                ssid: "Office".into(),
                profile_id: "removed".into(),
                enabled: true,
            },
        );
        assert_eq!(profile_to_activate(&config, &wifi), Some("work"));
        config.automation.wifi_rules[1].enabled = false;
        assert_eq!(profile_to_activate(&config, &wifi), None);
        config.automation.wifi_rules[1].enabled = true;
        wifi.ssid = Some("office".into());
        assert_eq!(profile_to_activate(&config, &wifi), None);
        wifi.ssid = Some("Office".into());
        wifi.connected = false;
        assert_eq!(profile_to_activate(&config, &wifi), None);
        wifi.connected = true;
        config.active_profile_id = "work".into();
        assert_eq!(profile_to_activate(&config, &wifi), None);
    }
}
