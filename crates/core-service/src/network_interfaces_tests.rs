use std::net::Ipv4Addr;

use crate::network_interfaces::{interface_info, list, sort_interfaces, IF_TYPE_SOFTWARE_LOOPBACK};

#[test]
fn filters_loopback_and_missing_addresses_and_sorts_up_first() {
    let mut interfaces: Vec<_> = [
        ("A offline", 6, 2, Some(Ipv4Addr::new(192, 0, 2, 1))),
        ("Z online", 71, 1, Some(Ipv4Addr::new(192, 0, 2, 2))),
        (
            "Loopback",
            IF_TYPE_SOFTWARE_LOOPBACK,
            1,
            Some(Ipv4Addr::LOCALHOST),
        ),
        ("B online", 6, 1, Some(Ipv4Addr::new(192, 0, 2, 3))),
        ("No address", 6, 1, None),
        ("Unspecified", 6, 1, Some(Ipv4Addr::UNSPECIFIED)),
    ]
    .into_iter()
    .filter_map(|(name, if_type, status, ip)| {
        interface_info(if_type, status, ip, || (name.into(), "Description".into()))
    })
    .collect();
    sort_interfaces(&mut interfaces);
    assert_eq!(
        interfaces
            .iter()
            .map(|info| (info.name.as_str(), info.is_up))
            .collect::<Vec<_>>(),
        [("B online", true), ("Z online", true), ("A offline", false)]
    );
    assert_eq!(interfaces[0].description, "Description");
    assert_eq!(interfaces[0].ipv4, "192.0.2.3");
}

#[test]
fn windows_adapter_enumeration_returns_ipv4_addresses_without_admin() {
    for interface in list().unwrap() {
        assert!(!interface.name.is_empty());
        assert!(!interface.ipv4.parse::<Ipv4Addr>().unwrap().is_loopback());
    }
}
