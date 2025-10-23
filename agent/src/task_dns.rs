//! DNS query implementations using hickory-client
//!
//! This module provides async DNS query functionality for both regular DNS
//! and DNS over HTTPS (DoH) using hickory-client directly to bypass system caching.

use anyhow::{Context, Result};
use hickory_client::client::{Client, ClientHandle};
use hickory_client::proto::rr::Name as DomainName;
use hickory_client::proto::rr::{DNSClass, RData, RecordType as ClientRecordType};
use hickory_client::proto::runtime::TokioRuntimeProvider;
use hickory_client::proto::udp::UdpClientStream;
use hickory_client::proto::xfer::DnsResponse;
use shared::config::{DnsQueryDohParams, DnsQueryParams, DnsRecordType};
use shared::metrics::RawDnsMetric;
use std::net::SocketAddr;
use std::time::{Duration, Instant};
use tracing::{debug, error};

/// Helper function to create DNS metric from query result
fn create_dns_metric(
    result: Result<DnsResponse>,
    start_time: Instant,
    domain: &str,
    record_type: &DnsRecordType,
    expected_ip: Option<&str>,
    target_id: Option<&str>,
) -> RawDnsMetric {
    match result {
        Ok(response) => {
            let query_time_ms = start_time.elapsed().as_secs_f64() * 1000.0;
            let (record_count, resolved_addresses) = parse_dns_response(&response, record_type);

            // Get first resolved IP for validation
            let resolved_ip = resolved_addresses.first().cloned();

            // Check if resolution matches expected IP (check if expected IP is in any of the resolved addresses)
            let correct_resolution = match expected_ip {
                Some(expected) => resolved_addresses.iter().any(|ip| ip == expected),
                None => true, // No expected IP means always correct
            };

            RawDnsMetric {
                query_time_ms: Some(query_time_ms),
                success: true,
                record_count: Some(record_count),
                resolved_addresses: Some(resolved_addresses),
                domain_queried: domain.to_string(),
                error: None,
                expected_ip: expected_ip.map(|s| s.to_string()),
                resolved_ip,
                correct_resolution,
                target_id: target_id.map(|s| s.to_string()),
            }
        }
        Err(e) => {
            error!("DNS query for {} failed: {}", domain, e);
            RawDnsMetric {
                query_time_ms: None, // Don't report time on error - query didn't complete
                success: false,
                record_count: None,
                resolved_addresses: None,
                domain_queried: domain.to_string(),
                error: Some(e.to_string()),
                expected_ip: expected_ip.map(|s| s.to_string()),
                resolved_ip: None,
                correct_resolution: false, // Failed queries are always incorrect
                target_id: target_id.map(|s| s.to_string()),
            }
        }
    }
}

/// Execute a regular DNS query using UDP with hickory-client
pub async fn execute_dns_query(params: &DnsQueryParams) -> Result<RawDnsMetric> {
    let start_time = Instant::now();
    debug!(
        "Executing DNS query for {} on server {}",
        params.domain, params.server
    );

    let domain_name = DomainName::from_ascii(&params.domain)
        .with_context(|| format!("Invalid domain name: {}", params.domain))?;

    let record_type = convert_record_type(&params.record_type);

    // Parse server address, defaulting to port 53 if not specified
    let server_addr: SocketAddr = if params.server.contains(':') {
        // Address already includes port
        params
            .server
            .parse()
            .with_context(|| format!("Invalid DNS server address: {}", params.server))?
    } else {
        // No port specified, default to standard DNS port 53
        format!("{}:53", params.server)
            .parse()
            .with_context(|| format!("Invalid DNS server address: {}", params.server))?
    };

    let result = query_via_udp(
        server_addr,
        domain_name,
        record_type,
        params.timeout_seconds,
        &params.domain,
        &params.server,
    )
    .await;

    Ok(create_dns_metric(
        result,
        start_time,
        &params.domain,
        &params.record_type,
        params.expected_ip.as_deref(),
        params.target_id.as_deref(),
    ))
}

/// Execute a DNS over HTTPS query using hickory-client
pub async fn execute_dns_over_https_query(params: &DnsQueryDohParams) -> Result<RawDnsMetric> {
    let start_time = Instant::now();
    debug!(
        "Executing DNS over HTTPS query for {} on server {}",
        params.domain, params.server_url
    );

    let domain_name = DomainName::from_ascii(&params.domain)
        .with_context(|| format!("Invalid domain name: {}", params.domain))?;

    let record_type = convert_record_type(&params.record_type);

    let result = query_via_https(
        &params.server_url,
        domain_name,
        record_type,
        params.timeout_seconds,
        &params.domain,
    )
    .await;

    Ok(create_dns_metric(
        result,
        start_time,
        &params.domain,
        &params.record_type,
        params.expected_ip.as_deref(),
        params.target_id.as_deref(),
    ))
}

/// Query DNS via UDP using hickory-client directly
async fn query_via_udp(
    server_addr: SocketAddr,
    domain_name: DomainName,
    record_type: ClientRecordType,
    timeout_seconds: u32,
    domain: &str,
    server: &str,
) -> Result<DnsResponse> {
    // Create UDP client stream using the builder pattern
    let connection = UdpClientStream::builder(server_addr, TokioRuntimeProvider::default()).build();
    let (mut client, bg) = Client::connect(connection).await.with_context(|| {
        format!(
            "Failed to connect to DNS server {} for domain {}",
            server, domain
        )
    })?;

    // Spawn background task with proper join handle tracking
    let bg_handle = tokio::spawn(bg);

    let timeout = Duration::from_secs(timeout_seconds as u64);
    let query_result = tokio::time::timeout(
        timeout,
        client.query(domain_name.clone(), DNSClass::IN, record_type),
    )
    .await;

    // Ensure background task is cleaned up
    bg_handle.abort();

    let response = query_result
        .with_context(|| {
            format!(
                "DNS query timed out after {}s for {} on {}",
                timeout_seconds, domain, server
            )
        })?
        .with_context(|| format!("DNS query failed for {} on {}", domain, server))?;

    Ok(response)
}

/// Query DNS via HTTPS using wire-format DNS over HTTPS (RFC 8484)
async fn query_via_https(
    server_url: &str,
    domain_name: DomainName,
    record_type: ClientRecordType,
    timeout_seconds: u32,
    domain: &str,
) -> Result<DnsResponse> {
    use hickory_client::proto::op::{Message, Query};
    use hickory_client::proto::serialize::binary::BinEncodable;

    // Build DNS query message
    let mut message = Message::new();
    message.add_query(Query::query(domain_name, record_type));
    message.set_recursion_desired(true);

    // Encode to wire format
    let query_bytes = message
        .to_bytes()
        .with_context(|| format!("Failed to encode DNS query for {}", domain))?;

    // Send DoH request
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(timeout_seconds as u64))
        .build()
        .with_context(|| "Failed to create HTTP client for DoH")?;

    let response = client
        .post(server_url)
        .header("Content-Type", "application/dns-message")
        .header("Accept", "application/dns-message")
        .body(query_bytes)
        .send()
        .await
        .with_context(|| format!("DoH request failed for {} to {}", domain, server_url))?;

    if !response.status().is_success() {
        return Err(anyhow::anyhow!(
            "DoH server returned error status {} for {} on {}",
            response.status(),
            domain,
            server_url
        ));
    }

    // Parse DNS response
    // DNS responses are small (typically < 512 bytes, max ~64KB for DNSSEC), so buffering is acceptable
    let response_bytes = response
        .bytes()
        .await
        .with_context(|| format!("Failed to read DoH response for {}", domain))?;

    let dns_message = Message::from_vec(&response_bytes)
        .with_context(|| format!("Failed to parse DNS response for {}", domain))?;

    // Convert Message to DnsResponse
    DnsResponse::from_message(dns_message)
        .with_context(|| format!("Failed to create DnsResponse for {}", domain))
}

/// Convert shared DNS record type to hickory-client record type
fn convert_record_type(record_type: &DnsRecordType) -> ClientRecordType {
    match record_type {
        DnsRecordType::A => ClientRecordType::A,
        DnsRecordType::AAAA => ClientRecordType::AAAA,
        DnsRecordType::MX => ClientRecordType::MX,
        DnsRecordType::CNAME => ClientRecordType::CNAME,
        DnsRecordType::TXT => ClientRecordType::TXT,
        DnsRecordType::NS => ClientRecordType::NS,
    }
}

/// Parse DNS response and extract record count and resolved addresses
fn parse_dns_response(response: &DnsResponse, _record_type: &DnsRecordType) -> (u32, Vec<String>) {
    let answers = response.answers();
    let record_count = answers.len() as u32;
    let mut resolved_addresses = Vec::new();

    for record in answers {
        let rdata = record.data();
        match rdata {
            RData::A(ipv4) => {
                resolved_addresses.push(ipv4.to_string());
            }
            RData::AAAA(ipv6) => {
                resolved_addresses.push(ipv6.to_string());
            }
            RData::CNAME(cname) => {
                resolved_addresses.push(cname.to_string());
            }
            RData::MX(mx) => {
                resolved_addresses.push(format!("{} {}", mx.preference(), mx.exchange()));
            }
            RData::TXT(txt) => {
                let txt_data = txt
                    .iter()
                    .map(|bytes| String::from_utf8_lossy(bytes))
                    .collect::<Vec<_>>()
                    .join(" ");
                resolved_addresses.push(txt_data);
            }
            RData::NS(ns) => {
                resolved_addresses.push(ns.to_string());
            }
            _ => {
                // Handle other record types as needed
                resolved_addresses.push(format!("Unsupported record type: {:?}", rdata));
            }
        }
    }

    (record_count, resolved_addresses)
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::config::{DnsQueryParams, DnsRecordType};

    #[tokio::test]
    async fn test_dns_query_google() {
        let params = DnsQueryParams {
            server: "8.8.8.8:53".to_string(),
            domain: "google.com".to_string(),
            record_type: DnsRecordType::A,
            timeout_seconds: 5,
            expected_ip: None,
            target_id: None,
        };

        let result = execute_dns_query(&params).await;
        assert!(result.is_ok());

        let metric = result.unwrap();
        assert_eq!(metric.domain_queried, "google.com");
        if metric.success {
            assert!(metric.query_time_ms.is_some());
            assert!(metric.record_count.is_some());
            assert!(metric.resolved_addresses.is_some());
        }
    }

    #[test]
    fn test_record_type_conversion() {
        assert_eq!(convert_record_type(&DnsRecordType::A), ClientRecordType::A);
        assert_eq!(
            convert_record_type(&DnsRecordType::AAAA),
            ClientRecordType::AAAA
        );
        assert_eq!(
            convert_record_type(&DnsRecordType::MX),
            ClientRecordType::MX
        );
        assert_eq!(
            convert_record_type(&DnsRecordType::CNAME),
            ClientRecordType::CNAME
        );
        assert_eq!(
            convert_record_type(&DnsRecordType::TXT),
            ClientRecordType::TXT
        );
        assert_eq!(
            convert_record_type(&DnsRecordType::NS),
            ClientRecordType::NS
        );
    }
}
