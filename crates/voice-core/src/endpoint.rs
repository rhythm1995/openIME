//! R3:用户自填 endpoint 的 SSRF 校验（纯函数，无网络 I/O）。
//!
//! 策略：**字面 host/IP 分类 + 公网强制 TLS**。fail-closed：
//! - 拒绝云元数据（EC2/ECS/阿里云/Azure IMDS）、link-local、CGNAT、0.0.0.0/8、组播/广播、
//!   IPv6 mapped 到上述、IPv6 fd00:ec2::254。
//! - 放行 loopback、RFC1918、IPv6 ULA（fc00::/7）——允许 http/ws（自托管 ollama/Whisper）。
//! - 其余（公网 IP/hostname）强制 https/wss。
//!
//! 请求期 DNS 重绑定闭环留到 P2；本模块不 resolve host（离线/CI 友好）。

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use thiserror::Error;
use url::Url;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum EndpointError {
    #[error("URL 格式无效：{0}")]
    InvalidUrl(String),
    #[error("禁止的云元数据服务地址（IMDS）")]
    BlockedMetadata,
    #[error("禁止的 link-local 地址")]
    BlockedLinkLocal,
    #[error("禁止的 CGNAT 地址（100.64.0.0/10）")]
    BlockedCgnat,
    #[error("禁止的保留地址")]
    BlockedReserved,
    #[error("公网地址必须使用 https / wss")]
    PublicRequiresTls,
}

const EC2_V4: Ipv4Addr = Ipv4Addr::new(169, 254, 169, 254);
const ECS_IMDS: Ipv4Addr = Ipv4Addr::new(169, 254, 170, 2);
const ALIYUN_IMDS: Ipv4Addr = Ipv4Addr::new(100, 100, 100, 200);
const AZURE_IMDS: Ipv4Addr = Ipv4Addr::new(168, 63, 129, 16);

/// 校验用户填写的 endpoint。空串 =「用默认」，直接放行（FR-3.7）。
pub fn validate_endpoint(raw: &str) -> Result<(), EndpointError> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(());
    }
    let parsed = Url::parse(raw).map_err(|e| EndpointError::InvalidUrl(e.to_string()))?;
    let scheme = parsed.scheme();
    if !matches!(scheme, "http" | "https" | "ws" | "wss") {
        return Err(EndpointError::InvalidUrl(format!("不支持的 scheme: {scheme}")));
    }
    let raw_host = parsed
        .host_str()
        .ok_or_else(|| EndpointError::InvalidUrl("URL 缺少 host".into()))?;
    // url crate 对 IPv6 字面 host 返回带方括号（如 "[::1]"），且会归一化（含 decimal→点分 IPv4）。
    let host = raw_host.trim_start_matches('[').trim_end_matches(']');
    let host_lower = host.to_ascii_lowercase();

    // 元数据 hostname（GCP）。
    if matches!(host_lower.as_str(), "metadata.google.internal" | "metadata.goog") {
        return Err(EndpointError::BlockedMetadata);
    }

    // 字面 IP（含 IPv4-mapped IPv6 / 纯十进制）。
    if let Some(ip) = parse_host_ip(host) {
        return classify_ip(ip, scheme);
    }

    // localhost 放行（http 可）。
    if host_lower == "localhost" {
        return Ok(());
    }

    // 其余视作公网 hostname：强制 TLS。
    require_tls(scheme)
}

/// 把 host 解析为 IP：直接 parse、或纯十进制 u32 → IPv4（大端）。
fn parse_host_ip(host: &str) -> Option<IpAddr> {
    if let Ok(ip) = host.parse::<IpAddr>() {
        return Some(ip);
    }
    if let Ok(n) = host.parse::<u32>() {
        return Some(IpAddr::V4(Ipv4Addr::from(n)));
    }
    None
}

fn classify_ip(ip: IpAddr, scheme: &str) -> Result<(), EndpointError> {
    match ip {
        IpAddr::V6(v) => {
            // IPv4-mapped IPv6 → 走 IPv4 分类。
            if let Some(mapped) = v.to_ipv4_mapped() {
                return classify_ip(IpAddr::V4(mapped), scheme);
            }
            // AWS EC2 IPv6 IMDS。
            if v.segments() == [0xfd00, 0xec2, 0, 0, 0, 0, 0, 0x254] {
                return Err(EndpointError::BlockedMetadata);
            }
            if is_unicast_link_local_v6(v) {
                return Err(EndpointError::BlockedLinkLocal);
            }
            if v.is_multicast() || v.is_unspecified() {
                return Err(EndpointError::BlockedReserved);
            }
            if v.is_loopback() || is_ula(v) {
                return Ok(());
            }
            require_tls(scheme)
        }
        IpAddr::V4(v4) => {
            if v4 == EC2_V4 || v4 == ECS_IMDS || v4 == ALIYUN_IMDS || v4 == AZURE_IMDS {
                return Err(EndpointError::BlockedMetadata);
            }
            if v4.is_link_local() {
                return Err(EndpointError::BlockedLinkLocal);
            }
            if is_cgnat(v4) {
                return Err(EndpointError::BlockedCgnat);
            }
            if is_ipv4_this_net(v4) || v4.is_broadcast() || v4.is_multicast() {
                return Err(EndpointError::BlockedReserved);
            }
            if v4.is_loopback() || v4.is_private() {
                return Ok(());
            }
            require_tls(scheme)
        }
    }
}

fn require_tls(scheme: &str) -> Result<(), EndpointError> {
    if matches!(scheme, "https" | "wss") {
        Ok(())
    } else {
        Err(EndpointError::PublicRequiresTls)
    }
}

// ── 1.75 兼容辅助：不调用 1.84+ 标准库同名稳定方法 ──

fn is_cgnat(v: Ipv4Addr) -> bool {
    let o = v.octets();
    o[0] == 100 && (o[1] & 0xc0) == 64 // 100.64.0.0/10
}
fn is_ula(v: Ipv6Addr) -> bool {
    (v.segments()[0] & 0xfe00) == 0xfc00 // fc00::/7
}
fn is_unicast_link_local_v6(v: Ipv6Addr) -> bool {
    (v.segments()[0] & 0xffc0) == 0xfe80 // fe80::/10
}
fn is_ipv4_this_net(v: Ipv4Addr) -> bool {
    v.octets()[0] == 0 // 0.0.0.0/8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_reject() {
        let reject = [
            "http://169.254.169.254",
            "http://169.254.1.1",
            "http://100.64.0.1",
            "http://api.openai.com/v1",
            "ws://example.com/ws",
            "ftp://x.example",
            "metadata.google.internal",
            "http://192.168.1.1.nip.io",
            "https://[::ffff:169.254.169.254]/",
            "https://0.0.0.1/",
            "http://100.100.100.200/",
            "http://168.63.129.16/",
        ];
        for u in reject {
            assert!(validate_endpoint(u).is_err(), "应拒绝 {u:?}（得到 {:?}）", validate_endpoint(u));
        }
    }

    #[test]
    fn table_allow() {
        let allow = [
            "http://192.168.0.5:1234",
            "http://10.0.0.2",
            "http://127.0.0.1:9000",
            "http://localhost:11434",
            "https://api.openai.com/v1",
            "wss://example.com/ws",
            "ws://192.168.1.2/ws",
            "https://[::1]/",
            "",
        ];
        for u in allow {
            assert!(validate_endpoint(u).is_ok(), "应放行 {u:?}（得到 {:?}）", validate_endpoint(u));
        }
    }

    #[test]
    fn metadata_host_class() {
        assert_eq!(
            validate_endpoint("https://metadata.google.internal/"),
            Err(EndpointError::BlockedMetadata)
        );
    }

    #[test]
    fn link_local_class() {
        assert_eq!(
            validate_endpoint("http://169.254.1.1/"),
            Err(EndpointError::BlockedLinkLocal)
        );
    }

    #[test]
    fn cgnat_class() {
        assert_eq!(
            validate_endpoint("http://100.64.0.1/"),
            Err(EndpointError::BlockedCgnat)
        );
    }

    #[test]
    fn public_http_requires_tls_class() {
        assert_eq!(
            validate_endpoint("http://api.openai.com/v1"),
            Err(EndpointError::PublicRequiresTls)
        );
    }

    #[test]
    fn mapped_ipv6_metadata() {
        assert_eq!(
            validate_endpoint("https://[::ffff:169.254.169.254]/"),
            Err(EndpointError::BlockedMetadata)
        );
    }

    #[test]
    fn mapped_ipv6_rfc1918_ok() {
        assert!(validate_endpoint("https://[::ffff:192.168.1.1]/").is_ok());
    }

    #[test]
    fn rfc1918_http_ok() {
        assert!(validate_endpoint("http://192.168.1.20:8080/v1").is_ok());
    }

    #[test]
    fn loopback_ipv6_http_ok() {
        assert!(validate_endpoint("http://[::1]:11434/").is_ok());
    }

    #[test]
    fn decimal_public_ipv4() {
        // 134744072 = 8.8.8.8（公网）→ url 归一化为点分；http 拒、https 放行。
        assert!(validate_endpoint("https://134744072/").is_ok());
        assert!(validate_endpoint("http://134744072/").is_err());
    }

    #[test]
    fn empty_and_whitespace_ok() {
        assert!(validate_endpoint("").is_ok());
        assert!(validate_endpoint("   ").is_ok());
    }

    #[test]
    fn reserved_zero_network() {
        assert_eq!(
            validate_endpoint("https://0.0.0.1/"),
            Err(EndpointError::BlockedReserved)
        );
    }
}
