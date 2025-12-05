//! SNMP query implementation using snmp2 crate
//!
//! This module provides async SNMP query functionality supporting SNMPv1, SNMPv2c,
//! and SNMPv3 (noAuthNoPriv and authNoPriv security levels).

use anyhow::{Context, Result};
use shared::config::{SnmpAuthProtocol, SnmpParams, SnmpSecurityLevel, SnmpVersion};
use shared::metrics::RawSnmpMetric;
use snmp2::{AsyncSession, Oid, Pdu, Value};
use std::net::{SocketAddr, ToSocketAddrs};
use std::time::{Duration, Instant};
use tracing::{debug, error};

/// Default SNMP port
const DEFAULT_SNMP_PORT: u16 = 161;

/// Execute an SNMP GET query and return the raw metric
pub async fn execute_snmp_task(params: &SnmpParams) -> Result<RawSnmpMetric> {
    let start_time = Instant::now();
    debug!(
        "Executing SNMP {} query for OID {} on host {}",
        version_str(&params.version),
        params.oid,
        params.host
    );

    // Parse the host address
    let addr = parse_host(&params.host)?;

    // Parse the OID
    let oid = parse_oid(&params.oid)?;

    let timeout = Duration::from_secs(params.timeout_seconds as u64);

    // Execute the query with timeout
    let result = tokio::time::timeout(timeout, execute_query(params, addr, &oid)).await;

    match result {
        Ok(Ok((value, value_type))) => {
            let response_time_ms = start_time.elapsed().as_secs_f64() * 1000.0;
            debug!(
                "SNMP query successful: {} = {} ({})",
                params.oid, value, value_type
            );
            Ok(RawSnmpMetric {
                response_time_ms: Some(response_time_ms),
                success: true,
                value: Some(value),
                value_type: Some(value_type),
                oid_queried: params.oid.clone(),
                error: None,
                target_id: params.target_id.clone(),
            })
        }
        Ok(Err(e)) => {
            error!("SNMP query for {} failed: {}", params.oid, e);
            Ok(RawSnmpMetric {
                response_time_ms: None,
                success: false,
                value: None,
                value_type: None,
                oid_queried: params.oid.clone(),
                error: Some(e.to_string()),
                target_id: params.target_id.clone(),
            })
        }
        Err(_) => {
            error!(
                "SNMP query for {} timed out after {}s",
                params.oid, params.timeout_seconds
            );
            Ok(RawSnmpMetric {
                response_time_ms: None,
                success: false,
                value: None,
                value_type: None,
                oid_queried: params.oid.clone(),
                error: Some(format!(
                    "Query timed out after {} seconds",
                    params.timeout_seconds
                )),
                target_id: params.target_id.clone(),
            })
        }
    }
}

/// Execute the actual SNMP query based on version
async fn execute_query(
    params: &SnmpParams,
    addr: SocketAddr,
    oid: &Oid<'_>,
) -> Result<(String, String)> {
    match params.version {
        SnmpVersion::V1 => execute_v1_query(addr, &params.community, oid).await,
        SnmpVersion::V2c => execute_v2c_query(addr, &params.community, oid).await,
        SnmpVersion::V3 => execute_v3_query(params, addr, oid).await,
    }
}

/// Execute SNMPv1 query
async fn execute_v1_query(
    addr: SocketAddr,
    community: &str,
    oid: &Oid<'_>,
) -> Result<(String, String)> {
    let mut session = AsyncSession::new_v1(addr, community.as_bytes(), 0)
        .await
        .context("Failed to create SNMPv1 session")?;

    let response = session
        .get(oid)
        .await
        .context("SNMPv1 GET request failed")?;

    extract_value_from_response(response)
}

/// Execute SNMPv2c query
async fn execute_v2c_query(
    addr: SocketAddr,
    community: &str,
    oid: &Oid<'_>,
) -> Result<(String, String)> {
    let mut session = AsyncSession::new_v2c(addr, community.as_bytes(), 0)
        .await
        .context("Failed to create SNMPv2c session")?;

    let response = session
        .get(oid)
        .await
        .context("SNMPv2c GET request failed")?;

    extract_value_from_response(response)
}

/// Execute SNMPv3 query
async fn execute_v3_query(
    params: &SnmpParams,
    addr: SocketAddr,
    oid: &Oid<'_>,
) -> Result<(String, String)> {
    use snmp2::v3::{Auth, AuthProtocol, Security};

    let username = params
        .username
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("SNMPv3 requires username"))?;

    let security = match params.security_level {
        SnmpSecurityLevel::NoAuthNoPriv => {
            Security::new(username.as_bytes(), &[]).with_auth(Auth::NoAuthNoPriv)
        }
        SnmpSecurityLevel::AuthNoPriv => {
            let auth_password = params
                .auth_password
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("authNoPriv requires auth_password"))?;

            let auth_proto = match params.auth_protocol {
                SnmpAuthProtocol::None => {
                    return Err(anyhow::anyhow!(
                        "authNoPriv requires an authentication protocol"
                    ))
                }
                SnmpAuthProtocol::Md5 => AuthProtocol::Md5,
                SnmpAuthProtocol::Sha1 => AuthProtocol::Sha1,
                SnmpAuthProtocol::Sha224 => AuthProtocol::Sha224,
                SnmpAuthProtocol::Sha256 => AuthProtocol::Sha256,
                SnmpAuthProtocol::Sha384 => AuthProtocol::Sha384,
                SnmpAuthProtocol::Sha512 => AuthProtocol::Sha512,
            };

            Security::new(username.as_bytes(), auth_password.as_bytes())
                .with_auth(Auth::AuthNoPriv)
                .with_auth_protocol(auth_proto)
        }
    };

    let mut session = AsyncSession::new_v3(addr, 0, security)
        .await
        .context("Failed to create SNMPv3 session")?;

    // Initialize the session (performs engine discovery)
    session
        .init()
        .await
        .context("SNMPv3 initialization failed")?;

    let response = session
        .get(oid)
        .await
        .context("SNMPv3 GET request failed")?;

    extract_value_from_response(response)
}

/// Extract the value from SNMP response
fn extract_value_from_response(response: Pdu<'_>) -> Result<(String, String)> {
    let mut varbinds = response.varbinds;

    if let Some((_oid, value)) = varbinds.next() {
        let value_str = value_to_string(&value);
        let type_str = value_type_name(&value).to_string();
        Ok((value_str, type_str))
    } else {
        Err(anyhow::anyhow!("No value returned in SNMP response"))
    }
}

/// Convert SNMP Value to string representation
fn value_to_string(value: &Value<'_>) -> String {
    match value {
        Value::Boolean(b) => b.to_string(),
        Value::Null => "null".to_string(),
        Value::Integer(i) => i.to_string(),
        Value::OctetString(bytes) => {
            // Try UTF-8 first, fall back to hex representation
            match std::str::from_utf8(bytes) {
                Ok(s) if s.chars().all(|c| !c.is_control() || c == '\n' || c == '\t') => {
                    s.to_string()
                }
                _ => bytes_to_hex(bytes),
            }
        }
        Value::ObjectIdentifier(oid) => format!("{}", oid),
        Value::IpAddress(ip) => format!("{}.{}.{}.{}", ip[0], ip[1], ip[2], ip[3]),
        Value::Counter32(c) => c.to_string(),
        Value::Unsigned32(u) => u.to_string(),
        Value::Timeticks(t) => {
            // Convert to human-readable format (days, hours, minutes, seconds)
            let hundredths = *t as u64;
            let seconds = hundredths / 100;
            let days = seconds / 86400;
            let hours = (seconds % 86400) / 3600;
            let minutes = (seconds % 3600) / 60;
            let secs = seconds % 60;
            format!("{}d {}h {}m {}s ({} ticks)", days, hours, minutes, secs, t)
        }
        Value::Opaque(bytes) => bytes_to_hex(bytes),
        Value::Counter64(c) => c.to_string(),
        Value::EndOfMibView => "endOfMibView".to_string(),
        Value::NoSuchObject => "noSuchObject".to_string(),
        Value::NoSuchInstance => "noSuchInstance".to_string(),
        // For complex types, provide a basic representation
        _ => format!("{:?}", value),
    }
}

/// Get the type name of an SNMP Value (using Rust enum variant names)
fn value_type_name(value: &Value<'_>) -> &'static str {
    match value {
        Value::Boolean(_) => "Boolean",
        Value::Null => "Null",
        Value::Integer(_) => "Integer",
        Value::OctetString(_) => "OctetString",
        Value::ObjectIdentifier(_) => "ObjectIdentifier",
        Value::IpAddress(_) => "IpAddress",
        Value::Counter32(_) => "Counter32",
        Value::Unsigned32(_) => "Unsigned32",
        Value::Timeticks(_) => "Timeticks",
        Value::Opaque(_) => "Opaque",
        Value::Counter64(_) => "Counter64",
        Value::EndOfMibView => "EndOfMibView",
        Value::NoSuchObject => "NoSuchObject",
        Value::NoSuchInstance => "NoSuchInstance",
        Value::Sequence(_) => "Sequence",
        Value::Set(_) => "Set",
        Value::Constructed(_, _) => "Constructed",
        Value::GetRequest(_) => "GetRequest",
        Value::GetNextRequest(_) => "GetNextRequest",
        Value::GetBulkRequest(_) => "GetBulkRequest",
        Value::SetRequest(_) => "SetRequest",
        Value::Response(_) => "Response",
        Value::InformRequest(_) => "InformRequest",
        Value::Trap(_) => "Trap",
        Value::Report(_) => "Report",
    }
}

/// Convert bytes to hexadecimal string
fn bytes_to_hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect::<Vec<_>>()
        .join(":")
}

/// Parse host string to SocketAddr, adding default port if not specified
fn parse_host(host: &str) -> Result<SocketAddr> {
    // Check if port is already specified
    let host_with_port = if host.contains(':') {
        // Check if it's an IPv6 address without port (contains multiple colons but no brackets)
        if host.matches(':').count() > 1 && !host.contains('[') {
            // IPv6 address without port
            format!("[{}]:{}", host, DEFAULT_SNMP_PORT)
        } else {
            host.to_string()
        }
    } else {
        format!("{}:{}", host, DEFAULT_SNMP_PORT)
    };

    host_with_port
        .to_socket_addrs()
        .with_context(|| format!("Failed to resolve host: {}", host))?
        .next()
        .ok_or_else(|| anyhow::anyhow!("No addresses found for host: {}", host))
}

/// Parse OID string to snmp2::Oid
fn parse_oid(oid_str: &str) -> Result<Oid<'static>> {
    // Remove leading dot if present
    let oid_str = oid_str.strip_prefix('.').unwrap_or(oid_str);

    // Parse the OID components as u64 (required by asn1_rs::Oid)
    let components: Vec<u64> = oid_str
        .split('.')
        .map(|s| {
            s.parse::<u64>()
                .with_context(|| format!("Invalid OID component: {}", s))
        })
        .collect::<Result<Vec<_>>>()?;

    if components.is_empty() {
        return Err(anyhow::anyhow!("OID cannot be empty"));
    }

    Oid::from(&components)
        .map_err(|e| anyhow::anyhow!("Failed to create OID from {}: {:?}", oid_str, e))
}

/// Get version string for logging
fn version_str(version: &SnmpVersion) -> &'static str {
    match version {
        SnmpVersion::V1 => "v1",
        SnmpVersion::V2c => "v2c",
        SnmpVersion::V3 => "v3",
    }
}
