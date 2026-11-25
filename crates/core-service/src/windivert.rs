//! The Windows x64 C ABI from vendor/windivert/include/windivert.h.
use std::{
    ffi::{c_char, c_void, CStr},
    io,
    os::windows::ffi::OsStrExt,
    ptr,
    sync::Arc,
};

pub(crate) const FLOW_FILTER: &CStr = c"outbound and ip and (tcp or udp) and !loopback";
pub(crate) const NETWORK_FILTER: &CStr = c"((ip and (tcp or udp)) or (ipv6 and tcp)) and !loopback";
pub(crate) const NETWORK: i32 = 0;
pub(crate) const FLOW: i32 = 2;
pub(crate) const FLOW_FLAGS: u64 = 0x01 | 0x04;
pub(crate) const FLOW_ESTABLISHED: u8 = 1;
pub(crate) const FLOW_DELETED: u8 = 2;

type Handle = isize;

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct Flow {
    pub endpoint_id: u64,
    pub parent_endpoint_id: u64,
    pub process_id: u32,
    pub local_addr: [u32; 4],
    pub remote_addr: [u32; 4],
    pub local_port: u16,
    pub remote_port: u16,
    pub protocol: u8,
}

#[repr(C)]
#[derive(Clone, Copy)]
union LayerData {
    flow: Flow,
    network: [u32; 2],
    reserved: [u64; 8],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct Address {
    timestamp: i64,
    flags: u32,
    reserved: u32,
    data: LayerData,
}

const _: () = {
    assert!(std::mem::size_of::<Address>() == 80);
    assert!(std::mem::offset_of!(Address, data) == 16);
    assert!(std::mem::size_of::<Flow>() == 64);
    assert!(std::mem::offset_of!(Flow, local_port) == 52);
};

impl Address {
    pub fn outbound(self) -> bool {
        self.flags & (1 << 17) != 0
    }

    pub fn interface(self) -> [u32; 2] {
        // Used only for NETWORK addresses.
        unsafe { self.data.network }
    }

    pub fn make_inbound(&mut self, interface: [u32; 2]) {
        // Clear Outbound and checksum validity before recalculating; preserve IPv6 (bit 20).
        self.flags &= !((1 << 17) | (1 << 21) | (1 << 22) | (1 << 23));
        self.data.network = interface;
    }

    pub fn event(self) -> u8 {
        (self.flags >> 8) as u8
    }

    pub fn flow(self) -> Flow {
        // Used only for FLOW addresses.
        unsafe { self.data.flow }
    }
}

#[link(name = "kernel32")]
extern "system" {
    fn LoadLibraryExW(path: *const u16, file: Handle, flags: u32) -> Handle;
    fn GetProcAddress(module: Handle, name: *const c_char) -> *const c_void;
    fn FreeLibrary(module: Handle) -> i32;
}

struct Library(Handle);

impl Drop for Library {
    fn drop(&mut self) {
        unsafe { FreeLibrary(self.0) };
    }
}

pub(crate) struct Driver {
    _library: Library,
    open: unsafe extern "C" fn(*const c_char, i32, i16, u64) -> Handle,
    recv: unsafe extern "C" fn(Handle, *mut u8, u32, *mut u32, *mut Address) -> i32,
    send: unsafe extern "C" fn(Handle, *const u8, u32, *mut u32, *const Address) -> i32,
    shutdown: unsafe extern "C" fn(Handle, i32) -> i32,
    close: unsafe extern "C" fn(Handle) -> i32,
    checksums: unsafe extern "C" fn(*mut u8, u32, *mut Address, u64) -> i32,
}

impl Driver {
    pub fn load() -> io::Result<Arc<Self>> {
        Self::load_from(&std::env::current_exe()?.with_file_name("WinDivert.dll"))
    }

    fn load_from(dll: &std::path::Path) -> io::Result<Arc<Self>> {
        let path: Vec<_> = dll.as_os_str().encode_wide().chain([0]).collect();
        // Resolve only the executable's sibling DLL and system DLL dependencies.
        let module = unsafe { LoadLibraryExW(path.as_ptr(), 0, 0x0000_0800) };
        if module == 0 {
            return Err(io::Error::last_os_error());
        }
        let library = Library(module);
        macro_rules! symbol {
            ($name:literal) => {{
                let address = unsafe { GetProcAddress(module, $name.as_ptr()) };
                if address.is_null() {
                    return Err(io::Error::last_os_error());
                }
                // Each inferred function pointer has the exact signature declared above.
                unsafe { std::mem::transmute::<*const c_void, _>(address) }
            }};
        }
        Ok(Arc::new(Self {
            open: symbol!(c"WinDivertOpen"),
            recv: symbol!(c"WinDivertRecv"),
            send: symbol!(c"WinDivertSend"),
            shutdown: symbol!(c"WinDivertShutdown"),
            close: symbol!(c"WinDivertClose"),
            checksums: symbol!(c"WinDivertHelperCalcChecksums"),
            _library: library,
        }))
    }

    pub fn open(self: &Arc<Self>, filter: &CStr, layer: i32, flags: u64) -> io::Result<Divert> {
        let raw = unsafe { (self.open)(filter.as_ptr(), layer, 0, flags) };
        if raw == -1 {
            return Err(io::Error::last_os_error());
        }
        Ok(Divert {
            raw,
            api: Arc::clone(self),
        })
    }

    fn recalculate(&self, bytes: &mut [u8], address: &mut Address) -> io::Result<()> {
        if unsafe { (self.checksums)(bytes.as_mut_ptr(), bytes.len() as u32, address, 0) } == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "WinDivert checksum calculation failed",
            ));
        }
        Ok(())
    }
}

pub(crate) struct Divert {
    raw: Handle,
    api: Arc<Driver>,
}

impl Divert {
    pub fn recv(&self, bytes: &mut [u8]) -> io::Result<(usize, Address)> {
        let mut address = Address {
            timestamp: 0,
            flags: 0,
            reserved: 0,
            data: LayerData { reserved: [0; 8] },
        };
        let mut received = 0;
        let buffer = if bytes.is_empty() {
            ptr::null_mut()
        } else {
            bytes.as_mut_ptr()
        };
        if unsafe {
            (self.api.recv)(
                self.raw,
                buffer,
                bytes.len() as u32,
                &mut received,
                &mut address,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok((received as usize, address))
    }

    pub fn send(&self, bytes: &[u8], address: &Address) -> io::Result<()> {
        if unsafe {
            (self.api.send)(
                self.raw,
                bytes.as_ptr(),
                bytes.len() as u32,
                ptr::null_mut(),
                address,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    pub fn reflect(
        &self,
        bytes: &mut [u8],
        mut address: Address,
        interface: [u32; 2],
    ) -> io::Result<()> {
        address.make_inbound(interface);
        self.api.recalculate(bytes, &mut address)?;
        self.send(bytes, &address)
    }

    pub fn stop_receiving(&self) {
        // Shutdown stops new captures and unblocks pending reads after queued packets drain.
        if unsafe { (self.api.shutdown)(self.raw, 1) } == 0 {
            tracing::debug!(error = %io::Error::last_os_error(), "WinDivert receive shutdown");
        }
    }
}

impl Drop for Divert {
    fn drop(&mut self) {
        unsafe { (self.api.close)(self.raw) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vendor_checksum_helper_matches_rust_and_sets_flags() {
        let driver = Driver::load_from(
            &std::path::Path::new(env!("MY_WIN_ANTIFY_DRIVER_TEST_DIR")).join("WinDivert.dll"),
        )
        .unwrap();
        let mut bytes = crate::packet::fixture(2, b"abc");
        let mut expected = bytes.clone();
        crate::packet::checksums(&mut expected);
        let mut address = Address {
            timestamp: 0,
            flags: 1 << 17,
            reserved: 0,
            data: LayerData { reserved: [0; 8] },
        };
        address.make_inbound([42, 0]);
        driver.recalculate(&mut bytes, &mut address).unwrap();
        assert_eq!(bytes, expected);
        assert_eq!(address.interface(), [42, 0]);
        assert!(!address.outbound());
        assert_eq!(
            address.flags & ((1 << 21) | (1 << 22)),
            (1 << 21) | (1 << 22)
        );
    }

    #[test]
    fn vendor_checksum_helper_preserves_ipv6_on_inbound_tcp_reset() {
        let driver = Driver::load_from(
            &std::path::Path::new(env!("MY_WIN_ANTIFY_DRIVER_TEST_DIR")).join("WinDivert.dll"),
        )
        .unwrap();
        let syn = crate::packet::fixture_v6(&[], 2, &[]);
        let mut reset = crate::packet::PacketV6::parse(&syn).unwrap().block_reset();
        let mut address = Address {
            timestamp: 0,
            flags: (1 << 17) | (1 << 20) | (1 << 21) | (1 << 22) | (1 << 23),
            reserved: 0,
            data: LayerData { reserved: [0; 8] },
        };
        address.make_inbound([1, 0]);
        assert_eq!(address.flags, 1 << 20);
        driver.recalculate(&mut reset, &mut address).unwrap();
        assert_eq!(address.interface(), [1, 0]);
        assert_eq!(address.flags, (1 << 20) | (1 << 22));
        assert_ne!(&reset[56..58], &[0, 0]);
    }

    #[test]
    fn vendor_checksum_helper_recalculates_inbound_ipv4_udp() {
        let driver = Driver::load_from(
            &std::path::Path::new(env!("MY_WIN_ANTIFY_DRIVER_TEST_DIR")).join("WinDivert.dll"),
        )
        .unwrap();
        let mut bytes = crate::packet::build_inbound_udp(
            "203.0.113.1:53".parse().unwrap(),
            "192.0.2.1:55000".parse().unwrap(),
            b"DNS",
        );
        let mut address = Address {
            timestamp: 0,
            flags: 1 << 17,
            reserved: 0,
            data: LayerData { reserved: [0; 8] },
        };
        address.make_inbound([5, 0]);
        driver.recalculate(&mut bytes, &mut address).unwrap();
        assert_eq!(address.flags, (1 << 21) | (1 << 23));
        assert_eq!(address.interface(), [5, 0]);
        assert_ne!(bytes[10..12], [0, 0]);
        assert_ne!(bytes[26..28], [0, 0]);
        assert_eq!(
            crate::packet::UdpPacket::parse(&bytes).unwrap().payload,
            b"DNS"
        );
    }

    #[test]
    #[ignore = "requires an elevated Windows console and the signed WinDivert driver"]
    fn opens_both_driver_layers() {
        let driver = Driver::load_from(
            &std::path::Path::new(env!("MY_WIN_ANTIFY_DRIVER_TEST_DIR")).join("WinDivert.dll"),
        )
        .unwrap();
        let _flow = driver.open(FLOW_FILTER, FLOW, FLOW_FLAGS).unwrap();
        let _network = driver.open(NETWORK_FILTER, NETWORK, 0).unwrap();
    }
}
