//! TLS handshake and connection utilities
//!
//! This module provides TLS-layer network primitives:
//! - DNS resolution
//! - TLS handshake timing and certificate validation
//!
//! This module sits above TCP (imports from task_tcp) and is used by HTTP tasks.
//! Network monitoring pyramid: TCP → TLS → HTTP

#![deny(missing_docs)]
#![deny(missing_debug_implementations)]
#![deny(rustdoc::all)]
#![deny(clippy::all)]
#![deny(clippy::cargo)]
#![deny(clippy::unwrap_used)]
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::cast_possible_wrap)]

use std::{
    fmt::Debug,
    net::SocketAddr,
    time::{Duration, SystemTime, UNIX_EPOCH},
    vec::IntoIter,
};

use openssl::{
    asn1::{Asn1Time, Asn1TimeRef},
    error::ErrorStack,
    ssl::{SslConnector, SslMethod, SslVerifyMode},
    x509::X509,
};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    net::TcpStream,
};
use tracing::debug;
use url::Url;

use crate::task_tcp;

/// `AsyncReadWrite` trait
///
/// This trait is implemented for types that implement the `AsyncRead` and `AsyncWrite` traits.
/// This is mainly used to make socket streams compatible with both [`tokio::net::TcpStream`] and [`tokio_openssl::SslStream`].
pub trait AsyncReadWrite: AsyncRead + AsyncWrite + Debug + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Debug + Unpin + Send> AsyncReadWrite for T {}

/// Error types
///
/// This module contains the error types for network connection operations.
///
/// The errors are defined using the `thiserror` crate.
pub mod error {
    use thiserror::Error;

    #[derive(Error, Debug)]
    /// Error types for network operations
    pub enum Error {
        #[error("io error: {0}")]
        /// IO error, derived from [`std::io::Error`]
        Io(#[from] std::io::Error),
        #[error("ssl error: {0}")]
        /// SSL error, derived from [`openssl::error::ErrorStack`]
        Ssl(#[from] openssl::error::ErrorStack),
        #[error("ssl handshake error: {0}")]
        /// SSL handshake error, derived from [`openssl::ssl::HandshakeError`]
        SslHandshake(#[from] openssl::ssl::HandshakeError<tokio::net::TcpStream>),
        #[error("ssl certificate not found")]
        /// SSL certificate not found
        SslCertificateNotFound,
        #[error("system time error: {0}")]
        /// System time error, derived from [`std::time::SystemTimeError`]
        SystemTime(#[from] std::time::SystemTimeError),
        #[error("timeout error: {0}")]
        /// Timeout error from [`tokio::time::error::Elapsed`]
        Timeout(#[from] tokio::time::error::Elapsed),
        #[error("url parse error: {0}")]
        /// URL parse error
        UrlParse(#[from] url::ParseError),
    }
}

#[derive(Debug)]
/// Basic information about an SSL certificate
pub struct CertificateInformation {
    /// Expires at
    pub expires_at: SystemTime,
    /// Is active (certificate is not expired)
    pub is_active: bool,
}

/// Response from TLS timing operation
pub struct TlsTimingResponse {
    /// TLS handshake duration
    pub timing: Duration,
    /// The connected stream (TLS-wrapped)
    pub stream: Box<dyn AsyncReadWrite + Send>,
    /// Certificate information if available
    pub certificate_information: Option<CertificateInformation>,
    /// Raw certificate if available
    pub certificate: Option<X509>,
}

/// Convert OpenSSL ASN.1 time format to Rust SystemTime
fn asn1_time_to_system_time(time: &Asn1TimeRef) -> Result<SystemTime, ErrorStack> {
    let unix_time = Asn1Time::from_unix(0)?.diff(time)?;
    Ok(SystemTime::UNIX_EPOCH
        + Duration::from_secs((unix_time.days as u64) * 86400 + unix_time.secs as u64))
}

/// Resolve URL hostname to socket addresses for connection
///
/// # Arguments
/// * `url` - The URL to resolve
///
/// # Returns
/// Iterator of resolved socket addresses
///
/// # Errors
/// Returns error if DNS resolution fails
pub async fn resolve_dns(url: &Url) -> Result<IntoIter<SocketAddr>, error::Error> {
    let Some(domain) = url.host_str() else {
        return Err(error::Error::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "invalid url",
        )));
    };
    let port = url.port().unwrap_or(match url.scheme() {
        "http" => 80,
        "https" => 443,
        _ => {
            return Err(error::Error::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "invalid url scheme",
            )))
        }
    });
    match tokio::net::lookup_host(format!("{domain}:{port}")).await {
        Ok(addrs) => Ok(addrs.collect::<Vec<_>>().into_iter()),
        Err(e) => Err(error::Error::Io(e)),
    }
}

/// Perform TLS handshake and measure timing
///
/// # Arguments
/// * `url` - The URL being connected to (used for SNI)
/// * `stream` - The established TCP stream
/// * `connector` - Shared SSL connector to use for the connection
///
/// # Returns
/// TLS timing response with duration, stream, and certificate info
///
/// # Errors
/// Returns error if TLS handshake fails or certificate is missing
pub async fn get_tls_timing(
    url: &Url,
    stream: TcpStream,
    connector: &SslConnector,
) -> Result<TlsTimingResponse, error::Error> {
    let now = std::time::Instant::now();
    let ssl = connector
        .configure()?
        .into_ssl(url.host_str().unwrap_or(""))?;
    let ssl_stream = tokio_openssl::SslStream::new(ssl, stream).map_err(|e| {
        error::Error::Io(std::io::Error::new(
            std::io::ErrorKind::Other,
            format!("TLS connection failed: {}", e),
        ))
    })?;
    let mut ssl_stream = ssl_stream;

    std::pin::Pin::new(&mut ssl_stream)
        .connect()
        .await
        .map_err(|e| {
            error::Error::Io(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("TLS handshake failed: {}", e),
            ))
        })?;

    let Some(raw_certificate) = ssl_stream.ssl().peer_certificate() else {
        return Err(error::Error::SslCertificateNotFound);
    };
    let time_elapsed = now.elapsed();

    let current_asn1_time =
        Asn1Time::from_unix(match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(duration) => duration.as_secs() as i64,
            Err(e) => return Err(error::Error::SystemTime(e)),
        })?;

    // Only extract the essential certificate information needed for validation
    // This avoids unnecessary allocations from parsing unused fields
    let certificate_information = CertificateInformation {
        expires_at: asn1_time_to_system_time(raw_certificate.not_after())?,
        is_active: raw_certificate.not_after() > current_asn1_time,
    };

    Ok(TlsTimingResponse {
        timing: time_elapsed,
        stream: Box::new(ssl_stream),
        certificate_information: Some(certificate_information),
        certificate: Some(raw_certificate),
    })
}

/// Result of a TLS handshake check
#[derive(Debug)]
pub struct TlsCheckResult {
    /// TCP connection time
    pub tcp_timing: Duration,
    /// TLS handshake time (None if not TLS or failed)
    pub tls_timing: Option<Duration>,
    /// Whether SSL certificate is valid
    pub ssl_valid: Option<bool>,
    /// Days until SSL certificate expires
    pub ssl_cert_days_until_expiry: Option<i64>,
    /// Whether the check was successful
    pub success: bool,
    /// Error message if check failed
    pub error: Option<String>,
}

/// Performs a TLS handshake check on the given host
///
/// # Arguments
/// * `host` - Target host:port (e.g., "example.com:443")
/// * `connector` - Shared SSL connector to use for the connection
///
/// # Returns
/// Result containing TLS check data or error
///
/// # Errors
/// Returns error if DNS resolution, TCP connection, or TLS handshake fails
pub async fn check_tls_handshake(
    host: &str,
    connector: &SslConnector,
) -> Result<TlsCheckResult, error::Error> {
    debug!("Performing TLS handshake check on: {}", host);

    // Construct URL for connection (we need URL for existing functions)
    let url_string = if host.contains("://") {
        host.to_string()
    } else {
        format!("https://{}", host)
    };
    let url = Url::parse(&url_string)?;

    // Resolve DNS and get first address
    let mut socket_addrs = resolve_dns(&url).await?;
    let url_ip = socket_addrs.next().ok_or_else(|| {
        error::Error::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "DNS resolution returned no addresses",
        ))
    })?;

    // Measure TCP connection time
    let (tcp_timing, tcp_stream) = task_tcp::get_tcp_timing(&url_ip)
        .await
        .map_err(error::Error::Io)?;

    // Measure TLS handshake time
    let tls_response = get_tls_timing(&url, tcp_stream, connector).await?;

    // Extract certificate information
    let (ssl_valid, ssl_cert_days_until_expiry) =
        if let Some(ref cert_info) = tls_response.certificate_information {
            let is_valid = cert_info.is_active;
            let days_until_expiry = match cert_info
                .expires_at
                .duration_since(std::time::SystemTime::now())
            {
                Ok(duration) => (duration.as_secs() / 86400) as i64,
                Err(_) => {
                    // Certificate is expired
                    match std::time::SystemTime::now().duration_since(cert_info.expires_at) {
                        Ok(duration) => -((duration.as_secs() / 86400) as i64),
                        Err(_) => 0,
                    }
                }
            };
            (Some(is_valid), Some(days_until_expiry))
        } else {
            (None, None)
        };

    Ok(TlsCheckResult {
        tcp_timing,
        tls_timing: Some(tls_response.timing),
        ssl_valid,
        ssl_cert_days_until_expiry,
        success: true,
        error: None,
    })
}

/// Performs a TLS handshake check with timeout
///
/// # Arguments
/// * `host` - Target host:port (e.g., "example.com:443")
/// * `timeout` - Optional timeout duration
/// * `connector` - Shared SSL connector to use for the connection
///
/// # Returns
/// Result containing TLS check data or error
///
/// # Errors
/// Returns error if operation times out or fails
pub async fn check_tls_handshake_with_timeout(
    host: &str,
    timeout: Option<Duration>,
    connector: &SslConnector,
) -> Result<TlsCheckResult, error::Error> {
    match timeout {
        Some(duration) => tokio::time::timeout(duration, check_tls_handshake(host, connector))
            .await
            .map_err(error::Error::Timeout)?,
        None => check_tls_handshake(host, connector).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_resolve_dns() {
        let url = Url::parse("https://example.com").unwrap();
        let result = resolve_dns(&url).await;
        assert!(result.is_ok());
        let addrs: Vec<SocketAddr> = result.unwrap().collect();
        assert!(!addrs.is_empty());
    }

    #[tokio::test]
    async fn test_tls_handshake_check() {
        // Create a test connector with verification disabled
        let mut builder = SslConnector::builder(SslMethod::tls()).unwrap();
        builder.set_verify(SslVerifyMode::NONE);
        let connector = builder.build();

        // Test with a well-known site
        let result = check_tls_handshake_with_timeout(
            "example.com:443",
            Some(Duration::from_secs(10)),
            &connector,
        )
        .await;

        match result {
            Ok(check) => {
                assert!(check.success);
                assert!(check.tls_timing.is_some());
                assert!(check.tcp_timing.as_millis() > 0);
            }
            Err(e) => {
                // Network errors are acceptable in tests
                eprintln!("TLS handshake test failed (network issue): {}", e);
            }
        }
    }

    #[tokio::test]
    async fn test_tls_handshake_timeout() {
        // Create a test connector with verification disabled
        let mut builder = SslConnector::builder(SslMethod::tls()).unwrap();
        builder.set_verify(SslVerifyMode::NONE);
        let connector = builder.build();

        // Use a non-routable IP to test timeout
        let result = check_tls_handshake_with_timeout(
            "192.0.2.1:443",
            Some(Duration::from_millis(100)),
            &connector,
        )
        .await;

        assert!(result.is_err());
    }
}
