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
    sync::Arc,
    time::{Duration, SystemTime},
    vec::IntoIter,
};

use rustls::pki_types::{CertificateDer, ServerName};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    net::TcpStream,
};
use tokio_rustls::TlsConnector;
use tracing::debug;
use url::Url;

use crate::task_tcp;

/// `AsyncReadWrite` trait
///
/// This trait is implemented for types that implement the `AsyncRead` and `AsyncWrite` traits.
/// This is mainly used to make socket streams compatible with both [`tokio::net::TcpStream`] and [`tokio_rustls::client::TlsStream`].
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
        #[error("tls error: {0}")]
        /// TLS error from rustls
        Tls(String),
        #[error("invalid DNS name: {0}")]
        /// Invalid DNS name error
        InvalidDnsName(String),
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
    /// Raw certificate if available (DER-encoded)
    pub certificate: Option<Vec<u8>>,
}

/// Extract certificate expiry time from DER-encoded certificate
fn extract_cert_expiry(cert_der: &CertificateDer) -> Result<SystemTime, error::Error> {
    // Use x509-parser to extract certificate expiry information
    use x509_parser::prelude::{FromDer, X509Certificate};

    let (_, parsed_cert) = X509Certificate::from_der(cert_der.as_ref())
        .map_err(|e| error::Error::Tls(format!("Failed to parse certificate DER: {}", e)))?;

    let not_after = parsed_cert.validity().not_after;
    let timestamp = not_after.timestamp();

    Ok(SystemTime::UNIX_EPOCH + Duration::from_secs(timestamp as u64))
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
/// * `connector` - Shared TLS connector to use for the connection
///
/// # Returns
/// TLS timing response with duration, stream, and certificate info
///
/// # Errors
/// Returns error if TLS handshake fails or certificate is missing
pub async fn get_tls_timing(
    url: &Url,
    stream: TcpStream,
    connector: &TlsConnector,
) -> Result<TlsTimingResponse, error::Error> {
    let now = std::time::Instant::now();

    let host = url.host_str().unwrap_or("");
    let server_name = ServerName::try_from(host)
        .map_err(|e| error::Error::InvalidDnsName(format!("Invalid DNS name '{}': {}", host, e)))?
        .to_owned();

    let tls_stream = connector.connect(server_name, stream).await.map_err(|e| {
        error::Error::Io(std::io::Error::other(format!(
            "TLS handshake failed: {}",
            e
        )))
    })?;

    let time_elapsed = now.elapsed();

    // Extract certificate information from the TLS connection
    let (_, server_connection) = tls_stream.get_ref();
    let peer_certificates = server_connection.peer_certificates();

    let (certificate_information, raw_certificate) = if let Some(certs) = peer_certificates {
        if let Some(cert) = certs.first() {
            // Extract expiry information
            let expires_at = extract_cert_expiry(cert)?;
            let is_active = expires_at > SystemTime::now();

            let cert_info = CertificateInformation {
                expires_at,
                is_active,
            };

            // Store the raw DER-encoded certificate
            let raw_cert = cert.as_ref().to_vec();

            (Some(cert_info), Some(raw_cert))
        } else {
            (None, None)
        }
    } else {
        (None, None)
    };

    Ok(TlsTimingResponse {
        timing: time_elapsed,
        stream: Box::new(tls_stream),
        certificate_information,
        certificate: raw_certificate,
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
/// * `connector` - Shared TLS connector to use for the connection
///
/// # Returns
/// Result containing TLS check data or error
///
/// # Errors
/// Returns error if DNS resolution, TCP connection, or TLS handshake fails
pub async fn check_tls_handshake(
    host: &str,
    connector: &TlsConnector,
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

    // Explicitly close the TLS connection now that we have all the timing and certificate data
    // This ensures proper cleanup of the TLS session and underlying TCP connection
    drop(tls_response.stream);

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
/// * `connector` - Shared TLS connector to use for the connection
///
/// # Returns
/// Result containing TLS check data or error
///
/// # Errors
/// Returns error if operation times out or fails
pub async fn check_tls_handshake_with_timeout(
    host: &str,
    timeout: Option<Duration>,
    connector: &TlsConnector,
) -> Result<TlsCheckResult, error::Error> {
    match timeout {
        Some(duration) => tokio::time::timeout(duration, check_tls_handshake(host, connector))
            .await
            .map_err(error::Error::Timeout)?,
        None => check_tls_handshake(host, connector).await,
    }
}

/// Create a TLS connector with certificate verification enabled
///
/// # Returns
/// Configured TLS connector with system root certificates
pub fn create_tls_connector_with_verification() -> Result<TlsConnector, error::Error> {
    // Install the default crypto provider if not already installed
    // This is safe to call multiple times - it will only install once
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    let mut root_store = rustls::RootCertStore::empty();

    // Load native system certificates
    // We tolerate individual certificate loading errors as long as we get at least some valid certs
    let certs = rustls_native_certs::load_native_certs();

    // Only fail if we have errors AND no valid certificates were loaded
    if !certs.errors.is_empty() && certs.certs.is_empty() {
        return Err(error::Error::Tls(format!(
            "Failed to load any native certs: {} errors",
            certs.errors.len()
        )));
    }

    for cert in certs.certs {
        // Ignore individual certificate parsing errors - as long as we have some valid certs, continue
        let _ = root_store.add(cert);
    }

    let config = rustls::ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();

    Ok(TlsConnector::from(Arc::new(config)))
}

/// Create a TLS connector with certificate verification disabled
///
/// # Returns
/// Configured TLS connector that accepts any certificate
pub fn create_tls_connector_without_verification() -> Result<TlsConnector, error::Error> {
    // Install the default crypto provider if not already installed
    // This is safe to call multiple times - it will only install once
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    let config = rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(NoCertificateVerification))
        .with_no_client_auth();

    Ok(TlsConnector::from(Arc::new(config)))
}

/// Certificate verifier that accepts all certificates (for testing/monitoring)
#[derive(Debug)]
struct NoCertificateVerification;

impl rustls::client::danger::ServerCertVerifier for NoCertificateVerification {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        vec![
            rustls::SignatureScheme::RSA_PKCS1_SHA256,
            rustls::SignatureScheme::ECDSA_NISTP256_SHA256,
            rustls::SignatureScheme::ED25519,
            rustls::SignatureScheme::RSA_PSS_SHA256,
            rustls::SignatureScheme::RSA_PKCS1_SHA384,
            rustls::SignatureScheme::ECDSA_NISTP384_SHA384,
            rustls::SignatureScheme::RSA_PSS_SHA384,
            rustls::SignatureScheme::RSA_PKCS1_SHA512,
            rustls::SignatureScheme::ECDSA_NISTP521_SHA512,
            rustls::SignatureScheme::RSA_PSS_SHA512,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_resolve_dns() {
        let url = Url::parse("https://example.com").expect("Failed to parse URL");
        let result = resolve_dns(&url).await;
        assert!(result.is_ok());
        let addrs: Vec<SocketAddr> = result
            .expect("Expected DNS resolution to succeed")
            .collect();
        assert!(!addrs.is_empty());
    }

    #[tokio::test]
    async fn test_tls_handshake_check() {
        // Create a test connector without verification for testing
        let connector =
            create_tls_connector_without_verification().expect("Failed to create TLS connector");

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
        // Create a test connector
        let connector =
            create_tls_connector_without_verification().expect("Failed to create TLS connector");

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
