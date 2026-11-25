use std::{ffi::c_void, io, mem::size_of, net::Ipv4Addr, ptr, slice};

use shared::NetworkInterfaceInfo;

const AF_INET: u16 = 2;
const SKIP_NON_UNICAST_AND_DNS: u32 = 0x02 | 0x04 | 0x08;
pub(super) const IF_TYPE_SOFTWARE_LOOPBACK: u32 = 24;
const IF_OPER_STATUS_UP: u32 = 1;
const ERROR_BUFFER_OVERFLOW: u32 = 111;
const ERROR_NO_DATA: u32 = 232;

// Prefixes of IP_ADAPTER_ADDRESSES_LH and IP_ADAPTER_UNICAST_ADDRESS_LH.
// Windows fills trailing fields in the caller's byte buffer.
#[repr(C)]
struct AdapterAddresses {
    _alignment: u64,
    next: *const Self,
    _adapter_name: *const u8,
    first_unicast: *const UnicastAddress,
    _first_anycast: *const c_void,
    _first_multicast: *const c_void,
    _first_dns_server: *const c_void,
    _dns_suffix: *const u16,
    description: *const u16,
    friendly_name: *const u16,
    _physical_address: [u8; 8],
    _physical_address_length: u32,
    _flags: u32,
    _mtu: u32,
    if_type: u32,
    oper_status: u32,
}

#[repr(C)]
struct UnicastAddress {
    _alignment: u64,
    next: *const Self,
    address: *const SockAddrIn,
    address_len: i32,
}

#[repr(C)]
struct SockAddrIn {
    family: u16,
    _port: u16,
    address: [u8; 4],
    _padding: [u8; 8],
}

#[link(name = "iphlpapi")]
extern "system" {
    fn GetAdaptersAddresses(
        family: u32,
        flags: u32,
        reserved: *mut c_void,
        addresses: *mut AdapterAddresses,
        size: *mut u32,
    ) -> u32;
}

#[link(name = "kernel32")]
extern "system" {
    fn lstrlenW(string: *const u16) -> i32;
}

pub(crate) fn list() -> io::Result<Vec<NetworkInterfaceInfo>> {
    let mut size = 15 * 1024;
    for _ in 0..3 {
        // The native linked-list nodes need eight-byte alignment.
        let mut buffer = vec![0u64; (size as usize).div_ceil(size_of::<u64>())];
        let addresses = buffer.as_mut_ptr().cast::<AdapterAddresses>();
        let status = unsafe {
            GetAdaptersAddresses(
                u32::from(AF_INET),
                SKIP_NON_UNICAST_AND_DNS,
                ptr::null_mut(),
                addresses,
                &mut size,
            )
        };
        match status {
            // The API owns the layout of all nodes and pointers until buffer is dropped.
            0 => return Ok(unsafe { read_adapters(addresses) }),
            ERROR_NO_DATA => return Ok(Vec::new()),
            ERROR_BUFFER_OVERFLOW => continue,
            error => return Err(io::Error::from_raw_os_error(error as i32)),
        }
    }
    Err(io::Error::from_raw_os_error(ERROR_BUFFER_OVERFLOW as i32))
}

unsafe fn read_adapters(mut next: *const AdapterAddresses) -> Vec<NetworkInterfaceInfo> {
    let mut interfaces = Vec::new();
    while let Some(adapter) = next.as_ref() {
        next = adapter.next;
        if let Some(info) = interface_info(
            adapter.if_type,
            adapter.oper_status,
            first_ipv4(adapter.first_unicast),
            || {
                (
                    wide_string(adapter.friendly_name),
                    wide_string(adapter.description),
                )
            },
        ) {
            interfaces.push(info);
        }
    }
    sort_interfaces(&mut interfaces);
    interfaces
}

unsafe fn first_ipv4(mut next: *const UnicastAddress) -> Option<Ipv4Addr> {
    while let Some(unicast) = next.as_ref() {
        next = unicast.next;
        if unicast.address_len < size_of::<SockAddrIn>() as i32 {
            continue;
        }
        if let Some(address) = unicast
            .address
            .as_ref()
            .filter(|address| address.family == AF_INET)
        {
            return Some(Ipv4Addr::from(address.address));
        }
    }
    None
}

unsafe fn wide_string(value: *const u16) -> String {
    if value.is_null() {
        return String::new();
    }
    String::from_utf16_lossy(slice::from_raw_parts(value, lstrlenW(value) as usize))
}

pub(super) fn interface_info(
    if_type: u32,
    oper_status: u32,
    ipv4: Option<Ipv4Addr>,
    names: impl FnOnce() -> (String, String),
) -> Option<NetworkInterfaceInfo> {
    let ipv4 = ipv4.filter(|ip| !ip.is_unspecified() && if_type != IF_TYPE_SOFTWARE_LOOPBACK)?;
    let (name, description) = names();
    (!name.is_empty()).then(|| NetworkInterfaceInfo {
        name,
        description,
        ipv4: ipv4.to_string(),
        is_up: oper_status == IF_OPER_STATUS_UP,
    })
}

pub(super) fn sort_interfaces(interfaces: &mut [NetworkInterfaceInfo]) {
    interfaces.sort_by(|a, b| b.is_up.cmp(&a.is_up).then_with(|| a.name.cmp(&b.name)));
}
