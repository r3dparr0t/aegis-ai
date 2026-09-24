// src/executor/ip.rs

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IpFormat {
    Decimal,
    Hex,
    Octal,
}

impl IpFormat {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "decimal" | "dec" | "int" | "integer" => Some(Self::Decimal),
            "hex" | "hexadecimal" => Some(Self::Hex),
            "octal" | "oct" => Some(Self::Octal),
            _ => None,
        }
    }
}

pub fn ipv4_to_u32(ip: &str) -> Option<u32> {
    let octets: Vec<u32> = ip.split('.')
        .map(|p| p.parse().ok())
        .collect::<Option<Vec<_>>>()?;
    if octets.len() != 4 || octets.iter().any(|&o| o > 255) {
        return None;
    }
    Some((octets[0] << 24) | (octets[1] << 16) | (octets[2] << 8) | octets[3])
}

pub fn format_ipv4_as(ip: &str, fmt: IpFormat) -> Option<String> {
    let n = ipv4_to_u32(ip)?;
    Some(match fmt {
        IpFormat::Decimal => format!("{}", n),
        IpFormat::Hex     => format!("0x{:08X}", n),
        IpFormat::Octal   => format!("0{:o}", n),
    })
}

/// تو یه URL اگه host یه IPv4 dotted-quad باشه، با فرمت داده‌شده جایش می‌ذاره.
pub fn rewrite_url_host(url: &str, fmt: IpFormat) -> Option<String> {
    let (scheme, rest) = url.split_once("://")?;
    let host_end = rest.find('/').unwrap_or(rest.len());
    let host_port = &rest[..host_end];
    let path = &rest[host_end..];

    let (host, port) = match host_port.split_once(':') {
        Some((h, p)) => (h, Some(p)),
        None => (host_port, None),
    };

    let encoded = format_ipv4_as(host, fmt)?;
    Some(match port {
        Some(p) => format!("{}://{}:{}{}", scheme, encoded, p, path),
        None    => format!("{}://{}{}", scheme, encoded, path),
    })
}
