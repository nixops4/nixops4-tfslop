//! go-plugin handshake line parsing and validation.
//!
//! Providers write a single pipe-delimited line to stdout right after
//! launch:
//!
//! ```text
//! CORE-PROTOCOL-VERSION | APP-PROTOCOL-VERSION | NETWORK-TYPE | NETWORK-ADDR | PROTOCOL | [CERT]
//! ```
//!
//! We accept 5 or 6 fields (the 6th, if present, is a base64 server cert
//! we do not currently use).

use anyhow::{bail, Result};

/// Parsed handshake line from a provider process.
#[derive(Debug, Clone)]
pub(crate) struct HandshakeInfo {
    pub core_protocol_version: String,
    pub app_protocol_version: String,
    pub network_type: String,
    pub network_address: String,
    pub protocol: String,
}

pub(crate) fn parse(line: &str) -> Result<HandshakeInfo> {
    let parts: Vec<&str> = line.trim().split('|').collect();
    if parts.len() < 5 || parts.len() > 6 {
        bail!(
            "invalid handshake line: expected 5 or 6 pipe-delimited fields, got {}: {:?}",
            parts.len(),
            line
        );
    }
    Ok(HandshakeInfo {
        core_protocol_version: parts[0].to_string(),
        app_protocol_version: parts[1].to_string(),
        network_type: parts[2].to_string(),
        network_address: parts[3].to_string(),
        protocol: parts[4].to_string(),
    })
}

pub(crate) fn validate(h: &HandshakeInfo) -> Result<()> {
    if h.core_protocol_version != "1" {
        bail!(
            "unsupported core protocol version: {}",
            h.core_protocol_version
        );
    }
    if h.protocol != "grpc" {
        bail!("unsupported protocol: {} (expected grpc)", h.protocol);
    }
    match h.network_type.as_str() {
        "tcp" | "unix" => {}
        other => bail!("unsupported network type: {}", other),
    }
    // v6 only, for now.
    if h.app_protocol_version != "6" {
        bail!(
            "unsupported app protocol version: {} (this build speaks v6 only)",
            h.app_protocol_version
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_five_field_line() {
        let h = parse("1|6|unix|/tmp/plug|grpc").unwrap();
        assert_eq!(h.core_protocol_version, "1");
        assert_eq!(h.app_protocol_version, "6");
        assert_eq!(h.network_type, "unix");
        assert_eq!(h.network_address, "/tmp/plug");
        assert_eq!(h.protocol, "grpc");
    }

    #[test]
    fn parses_six_field_line_with_empty_cert() {
        let h = parse("1|6|unix|/tmp/plug|grpc|").unwrap();
        assert_eq!(h.protocol, "grpc");
    }

    #[test]
    fn rejects_wrong_field_count() {
        assert!(parse("1|6|unix|/tmp/plug").is_err());
        assert!(parse("1|6|unix|/tmp/plug|grpc|extra|more").is_err());
    }

    #[test]
    fn validate_accepts_v6_grpc_unix() {
        validate(&parse("1|6|unix|/tmp/plug|grpc").unwrap()).unwrap();
    }

    #[test]
    fn validate_rejects_v5() {
        assert!(validate(&parse("1|5|unix|/tmp/plug|grpc").unwrap()).is_err());
    }
}
