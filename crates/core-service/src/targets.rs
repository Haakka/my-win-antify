use std::{net::Ipv4Addr, ops::RangeInclusive, str::FromStr, sync::Arc};

use shared::Rule;

use crate::sniffer::hostname;

pub(crate) struct Targets {
    ports: Patterns<RangeInclusive<u16>>,
    hosts: Arc<HostPatterns>,
}

impl Targets {
    pub fn parse(rule: &Rule) -> Result<Self, String> {
        Ok(Self {
            ports: Patterns::parse(&rule.target_ports, "port", port_range)?,
            hosts: Arc::new(HostPatterns(Patterns::parse(
                &rule.target_hosts,
                "host",
                Host::parse,
            )?)),
        })
    }

    pub fn check_ipv6(&self, port: u16) -> bool {
        self.ports.matches(|range| range.contains(&port))
            && (self.hosts.0.allowed.is_empty() || self.hosts.0.allowed.iter().any(Host::is_domain))
    }

    // None rejects the rule. Some(None) confirms it, and Some(Some(_)) needs a peek.
    pub fn check(&self, ip: Ipv4Addr, port: u16) -> Option<Option<Arc<HostPatterns>>> {
        if !self.ports.matches(|range| range.contains(&port)) {
            return None;
        }
        match self.hosts.check_ip(ip) {
            IpMatch::Rejected => None,
            IpMatch::Matched => Some(None),
            IpMatch::Inspect => Some(Some(Arc::clone(&self.hosts))),
        }
    }
}

#[derive(Debug)]
struct Patterns<T> {
    allowed: Vec<T>,
    excluded: Vec<T>,
}

impl<T> Patterns<T> {
    fn parse(
        input: &[String],
        kind: &str,
        parse: impl Fn(&str) -> Option<T>,
    ) -> Result<Self, String> {
        let mut patterns = Self {
            allowed: Vec::new(),
            excluded: Vec::new(),
        };
        for entry in input {
            let entry = entry.trim();
            let (pattern, list) = match entry.strip_prefix('-') {
                Some(pattern) => (pattern.trim(), &mut patterns.excluded),
                None => (entry, &mut patterns.allowed),
            };
            list.push(
                parse(pattern).ok_or_else(|| format!("Invalid target {kind} pattern {entry:?}"))?,
            );
        }
        Ok(patterns)
    }

    fn includes(&self, matches: impl Fn(&T) -> bool) -> bool {
        self.allowed.is_empty() || self.allowed.iter().any(matches)
    }

    fn excludes(&self, matches: impl Fn(&T) -> bool) -> bool {
        self.excluded.iter().any(matches)
    }

    fn matches(&self, matches: impl Fn(&T) -> bool) -> bool {
        !self.excludes(&matches) && self.includes(matches)
    }
}

fn range<T: FromStr + Ord + Copy>(input: &str) -> Option<RangeInclusive<T>> {
    let (start, end) = if let Some((first, last)) = input.split_once('-') {
        (first.trim().parse().ok()?, last.trim().parse().ok()?)
    } else {
        let value = input.parse().ok()?;
        (value, value)
    };
    (start <= end).then_some(start..=end)
}

fn port_range(input: &str) -> Option<RangeInclusive<u16>> {
    range(input).filter(|ports| *ports.start() != 0)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum IpMatch {
    Rejected,
    Matched,
    Inspect,
}

#[derive(Debug)]
pub(crate) struct HostPatterns(Patterns<Host>);

impl HostPatterns {
    fn check_ip(&self, ip: Ipv4Addr) -> IpMatch {
        let matches = |host: &Host| host.matches_ip(ip);
        if self.0.excludes(matches) {
            return IpMatch::Rejected;
        }
        let includes_ip = self.0.includes(matches);
        if !includes_ip && !self.0.allowed.iter().any(Host::is_domain) {
            return IpMatch::Rejected;
        }
        if !includes_ip || self.0.excluded.iter().any(Host::is_domain) {
            return IpMatch::Inspect;
        }
        IpMatch::Matched
    }

    pub fn matches(&self, ip: Ipv4Addr, domain: Option<&str>) -> bool {
        let domain = domain.map(|domain| domain.trim_end_matches('.').to_ascii_lowercase());
        self.0.matches(|host| {
            host.matches_ip(ip)
                || domain
                    .as_deref()
                    .is_some_and(|name| host.matches_domain(name))
        })
    }
}

#[derive(Debug)]
enum Host {
    Ip(RangeInclusive<Ipv4Addr>),
    Domain { name: String, wildcard: bool },
}

impl Host {
    fn parse(input: &str) -> Option<Self> {
        if let Some((base, prefix)) = input.split_once('/') {
            let address = u32::from(base.trim().parse::<Ipv4Addr>().ok()?);
            let prefix = prefix.trim().parse::<u32>().ok()?;
            if prefix > 32 {
                return None;
            }
            let mask = u32::MAX.checked_shl(32 - prefix).unwrap_or(0);
            return Some(Self::Ip(
                Ipv4Addr::from(address & mask)..=Ipv4Addr::from(address | !mask),
            ));
        }
        if let Ok(ip) = input.parse::<Ipv4Addr>() {
            return Some(Self::Ip(ip..=ip));
        }
        if let Some((first, last)) = input.split_once('-') {
            if let (Ok(first), Ok(last)) = (
                first.trim().parse::<Ipv4Addr>(),
                last.trim().parse::<Ipv4Addr>(),
            ) {
                return (first <= last).then_some(Self::Ip(first..=last));
            }
        }
        let name = input.strip_prefix("*.");
        let domain = name.unwrap_or(input).trim_end_matches('.');
        if domain
            .bytes()
            .all(|byte| byte.is_ascii_digit() || byte == b'.')
        {
            return None;
        }
        Some(Self::Domain {
            name: hostname(domain)?,
            wildcard: name.is_some(),
        })
    }

    fn is_domain(&self) -> bool {
        matches!(self, Self::Domain { .. })
    }

    fn matches_ip(&self, ip: Ipv4Addr) -> bool {
        matches!(self, Self::Ip(range) if range.contains(&ip))
    }

    fn matches_domain(&self, domain: &str) -> bool {
        match self {
            Self::Ip(_) => false,
            Self::Domain {
                name,
                wildcard: false,
            } => domain == name,
            Self::Domain {
                name,
                wildcard: true,
            } => domain
                .strip_suffix(name.as_str())
                .is_some_and(|prefix| prefix.len() > 1 && prefix.ends_with('.')),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hosts(entries: &[&str]) -> HostPatterns {
        let entries = entries
            .iter()
            .map(|entry| (*entry).into())
            .collect::<Vec<_>>();
        HostPatterns(Patterns::parse(&entries, "host", Host::parse).unwrap())
    }

    #[test]
    fn port_lists_match_exact_ranges_and_exclusions() {
        let entries = ["80", "8000-9000", "-8123", "-8500-8501"].map(String::from);
        let ports = Patterns::parse(&entries, "port", port_range).unwrap();
        for port in [80, 8000, 8124, 8499, 8502, 9000] {
            assert!(ports.matches(|range| range.contains(&port)), "{port}");
        }
        for port in [79, 81, 3124, 7999, 8123, 8500, 8501, 9001] {
            assert!(!ports.matches(|range| range.contains(&port)), "{port}");
        }
        let ports = Patterns::parse(&["-3124".into()], "port", port_range).unwrap();
        assert!(ports.matches(|range| range.contains(&443)));
        assert!(!ports.matches(|range| range.contains(&3124)));
        for entry in ["0", "65536", "10-1", "abc"] {
            assert!(port_range(entry).is_none());
        }
    }

    #[test]
    fn ip_lists_match_single_addresses_cidr_ranges_and_exclusions() {
        let hosts = hosts(&[
            "192.0.2.1",
            "10.0.0.0/8",
            "172.16.1.0-172.16.5.255",
            "-10.1.0.0/16",
        ]);
        for ip in [
            "192.0.2.1",
            "10.0.0.0",
            "10.255.255.255",
            "172.16.1.0",
            "172.16.5.255",
        ] {
            assert_eq!(
                hosts.check_ip(ip.parse().unwrap()),
                IpMatch::Matched,
                "{ip}"
            );
        }
        for ip in [
            "192.0.2.2",
            "11.0.0.0",
            "10.1.255.255",
            "172.16.0.255",
            "172.16.6.0",
        ] {
            assert_eq!(
                hosts.check_ip(ip.parse().unwrap()),
                IpMatch::Rejected,
                "{ip}"
            );
        }
        assert!(Host::parse("0.0.0.0/0")
            .unwrap()
            .matches_ip(Ipv4Addr::BROADCAST));
        assert!(Host::parse("10.0.0.0/33").is_none());
        assert!(Host::parse("10.0.0.2-10.0.0.1").is_none());
    }

    #[test]
    fn wildcard_domains_defer_and_exclusions_override_ip_and_domain_matches() {
        let hosts = hosts(&[
            "*.google.com",
            "192.0.2.1",
            "-ads.google.com",
            "-198.51.100.1",
        ]);
        let ip = "203.0.113.1".parse().unwrap();
        assert_eq!(hosts.check_ip(ip), IpMatch::Inspect);
        assert!(hosts.matches(ip, Some("WWW.Google.COM.")));
        assert!(hosts.matches(ip, Some("sub.www.google.com")));
        for domain in [
            Some("google.com"),
            Some("notgoogle.com"),
            Some("ads.google.com"),
            None,
        ] {
            assert!(!hosts.matches(ip, domain), "{domain:?}");
        }
        let allowed_ip = "192.0.2.1".parse().unwrap();
        assert_eq!(hosts.check_ip(allowed_ip), IpMatch::Inspect);
        assert!(hosts.matches(allowed_ip, None));
        assert!(!hosts.matches(allowed_ip, Some("ads.google.com")));
        let excluded_ip = "198.51.100.1".parse().unwrap();
        assert_eq!(hosts.check_ip(excluded_ip), IpMatch::Rejected);
        assert!(!hosts.matches(excluded_ip, Some("www.google.com")));
    }

    #[test]
    fn exclusion_only_hosts_allow_every_other_destination() {
        let hosts = hosts(&["-*.example.org"]);
        let ip = "203.0.113.1".parse().unwrap();
        assert_eq!(hosts.check_ip(ip), IpMatch::Inspect);
        assert!(!hosts.matches(ip, Some("www.example.org")));
        assert!(hosts.matches(ip, Some("other.org")));
        assert!(hosts.matches(ip, None));
    }
}
