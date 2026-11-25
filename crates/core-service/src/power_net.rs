use std::{
    ffi::{c_char, c_void},
    io, mem, ptr,
    time::Duration,
};

use tokio::{
    sync::Notify,
    time::{self, Instant},
};

const AF_INET: u16 = 2;
const DEVICE_NOTIFY_CALLBACK: u32 = 2;
const PBT_APMRESUMESUSPEND: u32 = 7;
const PBT_APMRESUMEAUTOMATIC: u32 = 18;
const LOAD_LIBRARY_SEARCH_SYSTEM32: u32 = 0x0800;
const DEBOUNCE: Duration = Duration::from_millis(900);
const MAX_DEBOUNCE: Duration = Duration::from_secs(3);

// The process has one service monitor. A late Windows callback can safely
// signal this static even if its registration is being torn down.
static CHANGED: Notify = Notify::const_new();

type NetworkCallback = unsafe extern "system" fn(*mut c_void, *const c_void, i32);
type UnregisterPower = unsafe extern "system" fn(isize) -> u32;
type RegisterPower = unsafe extern "system" fn(u32, *const Subscription, *mut isize) -> u32;

#[repr(C)]
struct Subscription {
    callback: unsafe extern "system" fn(*mut c_void, u32, *mut c_void) -> u32,
    context: *mut c_void,
}

#[link(name = "iphlpapi")]
extern "system" {
    fn NotifyIpInterfaceChange(
        family: u16,
        callback: NetworkCallback,
        context: *mut c_void,
        initial: u8,
        handle: *mut isize,
    ) -> u32;
    fn NotifyUnicastIpAddressChange(
        family: u16,
        callback: NetworkCallback,
        context: *mut c_void,
        initial: u8,
        handle: *mut isize,
    ) -> u32;
    fn CancelMibChangeNotify2(handle: isize) -> u32;
}

#[link(name = "kernel32")]
extern "system" {
    fn LoadLibraryExW(path: *const u16, file: isize, flags: u32) -> isize;
    fn GetProcAddress(module: isize, name: *const c_char) -> *const c_void;
    fn FreeLibrary(module: isize) -> i32;
}

pub(crate) struct Monitor {
    interfaces: Option<NetworkRegistration>,
    addresses: Option<NetworkRegistration>,
    power: Option<PowerRegistration>,
}

impl Monitor {
    pub fn new() -> Self {
        Self {
            interfaces: register("IP interface changes", || {
                NetworkRegistration::new(NotifyIpInterfaceChange)
            }),
            addresses: register("unicast IP address changes", || {
                NetworkRegistration::new(NotifyUnicastIpAddressChange)
            }),
            power: register("power resume notifications", PowerRegistration::new),
        }
    }

    pub fn has_network_notifications(&self) -> bool {
        self.interfaces.is_some() || self.addresses.is_some()
    }

    pub fn is_registered(&self) -> bool {
        self.has_network_notifications() || self.power.is_some()
    }

    pub async fn changed(&self) {
        CHANGED.notified().await;
    }

    pub async fn debounce(&self) {
        let limit = Instant::now() + MAX_DEBOUNCE;
        while time::timeout_at((Instant::now() + DEBOUNCE).min(limit), CHANGED.notified())
            .await
            .is_ok()
        {
            if Instant::now() >= limit {
                break;
            }
        }
    }
}

fn register<T>(name: &str, register: impl FnOnce() -> io::Result<T>) -> Option<T> {
    match register() {
        Ok(registration) => Some(registration),
        Err(error) => {
            tracing::warn!(notification = name, %error, "Windows notification registration failed");
            None
        }
    }
}

struct NetworkRegistration(isize);

impl NetworkRegistration {
    fn new(
        register: unsafe extern "system" fn(
            u16,
            NetworkCallback,
            *mut c_void,
            u8,
            *mut isize,
        ) -> u32,
    ) -> io::Result<Self> {
        let mut handle = 0;
        let status = unsafe { register(AF_INET, network_changed, ptr::null_mut(), 0, &mut handle) };
        check_status(status)?;
        Ok(Self(handle))
    }
}

impl Drop for NetworkRegistration {
    fn drop(&mut self) {
        unsafe { CancelMibChangeNotify2(self.0) };
    }
}

unsafe extern "system" fn network_changed(_: *mut c_void, _: *const c_void, _: i32) {
    CHANGED.notify_one();
}

struct Library(isize);

impl Drop for Library {
    fn drop(&mut self) {
        unsafe { FreeLibrary(self.0) };
    }
}

struct PowerRegistration {
    handle: isize,
    unregister: UnregisterPower,
    _library: Library,
}

impl PowerRegistration {
    fn new() -> io::Result<Self> {
        let path: Vec<_> = "powrprof.dll\0".encode_utf16().collect();
        let module = unsafe { LoadLibraryExW(path.as_ptr(), 0, LOAD_LIBRARY_SEARCH_SYSTEM32) };
        if module == 0 {
            return Err(io::Error::last_os_error());
        }
        let library = Library(module);
        let register =
            unsafe { GetProcAddress(module, c"PowerRegisterSuspendResumeNotification".as_ptr()) };
        let unregister =
            unsafe { GetProcAddress(module, c"PowerUnregisterSuspendResumeNotification".as_ptr()) };
        if register.is_null() || unregister.is_null() {
            return Err(io::Error::last_os_error());
        }
        // Both symbols come from the system PowrProf DLL and have the signatures above.
        let register: RegisterPower = unsafe { mem::transmute(register) };
        let unregister: UnregisterPower = unsafe { mem::transmute(unregister) };
        let subscription = Subscription {
            callback: power_changed,
            context: ptr::null_mut(),
        };
        let mut handle = 0;
        check_status(unsafe { register(DEVICE_NOTIFY_CALLBACK, &subscription, &mut handle) })?;
        Ok(Self {
            handle,
            unregister,
            _library: library,
        })
    }
}

impl Drop for PowerRegistration {
    fn drop(&mut self) {
        unsafe { (self.unregister)(self.handle) };
    }
}

unsafe extern "system" fn power_changed(_: *mut c_void, event: u32, _: *mut c_void) -> u32 {
    if matches!(event, PBT_APMRESUMEAUTOMATIC | PBT_APMRESUMESUSPEND) {
        CHANGED.notify_one();
    }
    0
}

fn check_status(status: u32) -> io::Result<()> {
    if status == 0 {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(status as i32))
    }
}
