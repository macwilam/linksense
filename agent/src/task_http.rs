//! HTTP timing implementation for network monitoring
//!
//! This module is inspired / based on the excellent work from the http-timings crate:
//! Original code: https://github.com/metrixweb/http-timings
//! Author: metrixweb
//!
//! The code has been adapted for async usage in the network monitoring agent.
//! All credit for the core HTTP timing logic goes to the original author.
//!
//! License: The original http-timings crate is licensed under the MIT license.

#![deny(missing_docs)]
#![deny(missing_debug_implementations)]
#![deny(rustdoc::all)]
#![deny(clippy::all)]
#![deny(clippy::cargo)]
#![deny(clippy::unwrap_used)]
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::cast_possible_wrap)]

use std::time::Duration;

use openssl::ssl::SslConnector;
use openssl::x509::X509;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use url::Url;

// Import connection primitives from appropriate modules
// Network monitoring pyramid: TCP → TLS → HTTP
use crate::task_tcp::get_tcp_timing;
use crate::task_tls::{error, get_tls_timing, resolve_dns, AsyncReadWrite, CertificateInformation};

#[derive(Debug)]
/// The response timings for any given request. The response timings can be found
/// [here](https://developer.chrome.com/docs/devtools/network/reference/?utm_source=devtools#timing-explanation).
///
/// Note: DNS timing is NOT measured here as it uses the system's local DNS resolver which is typically cached.
/// For accurate DNS performance measurements, use the dedicated DNS task which queries specific DNS servers.
pub struct ResponseTimings {
    /// TCP connection time
    pub tcp: Duration,
    /// TLS handshake time
    pub tls: Option<Duration>,
    /// Time To First Byte
    pub ttfb: Duration,
    /// Content download time
    pub content_download: Duration,
}

impl ResponseTimings {
    fn new(
        tcp: Duration,
        tls: Option<Duration>,
        ttfb: Duration,
        content_download: Duration,
    ) -> Self {
        Self {
            tcp,
            tls,
            ttfb,
            content_download,
        }
    }
}

#[derive(Debug)]
/// The response from a given request
pub struct Response {
    /// The timings of the response
    pub timings: ResponseTimings,
    /// The certificate information
    pub certificate_information: Option<CertificateInformation>,
    /// The raw certificate
    pub certificate: Option<X509>,
    /// The status of the response
    pub status: u16,
}

async fn get_http_send_timing(
    url: &Url,
    stream: &mut Box<dyn AsyncReadWrite + Send>,
) -> Result<Duration, error::Error> {
    let now = std::time::Instant::now();
    let path = if url.path().is_empty() {
        "/"
    } else {
        url.path()
    };
    let request = format!(
        "GET {} HTTP/1.1\r\nHost: {}\r\nUser-Agent: curl/8.7.1\r\nConnection: close\r\nAccept: */*\r\n\r\n",
        path,
        match url.host_str() {
            Some(host) => host,
            None => return Err(error::Error::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "invalid url host",
            ))),
        }
    );

    if let Err(err) = stream.write_all(request.as_bytes()).await {
        return Err(error::Error::Io(err));
    }
    Ok(now.elapsed())
}

async fn get_ttfb_timing(
    stream: &mut Box<dyn AsyncReadWrite + Send>,
) -> Result<(Duration, u8), error::Error> {
    let mut one_byte = [0_u8; 1];
    let now = std::time::Instant::now();
    if let Err(err) = stream.read_exact(&mut one_byte).await {
        return Err(error::Error::Io(err));
    }
    Ok((now.elapsed(), one_byte[0]))
}

async fn get_content_download_timing(
    stream: Box<dyn AsyncReadWrite + Send>,
    first_byte: u8,
) -> Result<(Duration, u16), error::Error> {
    let mut reader = BufReader::new(stream);

    // Security limits
    const MAX_HEADER_SIZE: usize = 64 * 1024; // 64KB for all headers
    const MAX_BODY_SIZE: usize = 100 * 1024 * 1024; // 100MB max response

    // Start with the first byte we already read for TTFB timing
    // Pre-allocate with a reasonable initial capacity, but never more than MAX_HEADER_SIZE
    let mut header_buf = String::with_capacity(4096.min(MAX_HEADER_SIZE));
    header_buf.push(first_byte as char);

    let now = std::time::Instant::now();
    loop {
        // Check BEFORE reading to prevent over-allocation attacks
        // Reserve space for the next line (typically < 1KB, but we check against a safe buffer)
        if header_buf.len() + 1024 > MAX_HEADER_SIZE {
            return Err(error::Error::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "HTTP headers exceed maximum size of {} bytes",
                    MAX_HEADER_SIZE
                ),
            )));
        }

        let bytes_read = match reader.read_line(&mut header_buf).await {
            Ok(bytes_read) => bytes_read,
            Err(err) => return Err(error::Error::Io(err)),
        };

        // Double-check after reading (defense in depth)
        if header_buf.len() > MAX_HEADER_SIZE {
            return Err(error::Error::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "HTTP headers exceed maximum size of {} bytes",
                    MAX_HEADER_SIZE
                ),
            )));
        }

        // Empty line signifies end of headers
        if bytes_read == 2 && header_buf.ends_with("\n") {
            break;
        }
        // Connection closed prematurely
        if bytes_read == 0 {
            break;
        }
    }

    // Parse headers efficiently - avoid cloning iterator and repeated lowercase conversions
    let mut content_length: Option<usize> = None;
    let mut is_chunked = false;
    let mut status_code: Option<u16> = None;

    for line in header_buf.lines() {
        // Convert to lowercase once per line for comparison
        let line_lower = line.to_ascii_lowercase();

        // Parse status line (first line: "HTTP/1.1 200 OK")
        if status_code.is_none() && line_lower.starts_with("http/") {
            status_code = line
                .split_whitespace()
                .nth(1)
                .and_then(|code| code.parse::<u16>().ok());
        }

        // Parse Content-Length header
        if content_length.is_none() && line_lower.starts_with("content-length:") {
            content_length = line
                .split(':')
                .nth(1)
                .and_then(|value| value.trim().parse::<usize>().ok());
        }

        // Check for chunked transfer encoding
        if !is_chunked && line_lower.contains("transfer-encoding") && line_lower.contains("chunked")
        {
            is_chunked = true;
        }
    }

    let status = match status_code {
        Some(code) => code,
        None => {
            // Drain remaining data from the stream to ensure proper cleanup
            // Use a limited buffer to prevent unbounded memory consumption
            let mut discard_buf = vec![0u8; 8192];
            let mut total_discarded = 0;
            const MAX_DISCARD: usize = 1024 * 1024; // Limit to 1MB drainage

            while total_discarded < MAX_DISCARD {
                match reader.read(&mut discard_buf).await {
                    Ok(0) => break, // EOF
                    Ok(n) => total_discarded += n,
                    Err(_) => break, // Error while draining, give up
                }
            }

            // Explicitly drop reader to close connection before returning error
            drop(reader);

            return Err(error::Error::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "invalid http status line",
            )));
        }
    };

    // Enforce maximum body size
    let content_length_value = content_length.unwrap_or(0);
    if content_length_value > MAX_BODY_SIZE {
        return Err(error::Error::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!(
                "Content-Length ({} bytes) exceeds maximum size of {} bytes",
                content_length_value, MAX_BODY_SIZE
            ),
        )));
    }

    // We only measure timing - don't buffer the response body
    // Use a small reusable buffer to read and discard data without allocating for entire response
    let mut discard_buf = [0u8; 8192]; // 8KB reusable buffer

    if content_length_value > 0 {
        // Read content_length bytes without storing them
        let mut remaining = content_length_value;
        while remaining > 0 {
            let to_read = remaining.min(discard_buf.len());
            if let Err(err) = reader.read_exact(&mut discard_buf[..to_read]).await {
                return Err(error::Error::Io(err));
            }
            remaining -= to_read;
        }
    } else if is_chunked {
        // Handle chunked transfer encoding with size limit
        let mut total_read = 0;
        loop {
            let mut chunk_size_line = String::new();
            if let Err(err) = reader.read_line(&mut chunk_size_line).await {
                return Err(error::Error::Io(err));
            }

            let chunk_size = match usize::from_str_radix(chunk_size_line.trim(), 16) {
                Ok(size) => size,
                Err(e) => {
                    return Err(error::Error::Io(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!("Invalid chunk size in chunked encoding: {}", e),
                    )));
                }
            };

            if chunk_size == 0 {
                // Read final chunk trailer (empty line)
                let mut trailer = String::new();
                let _ = reader.read_line(&mut trailer).await;
                break;
            }

            // Check if adding this chunk would exceed size limit
            if total_read + chunk_size > MAX_BODY_SIZE {
                return Err(error::Error::Io(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!(
                        "Chunked response size ({} bytes) exceeds maximum size of {} bytes",
                        total_read + chunk_size,
                        MAX_BODY_SIZE
                    ),
                )));
            }

            // Read and discard chunk data without buffering
            let mut remaining = chunk_size;
            while remaining > 0 {
                let to_read = remaining.min(discard_buf.len());
                if let Err(err) = reader.read_exact(&mut discard_buf[..to_read]).await {
                    return Err(error::Error::Io(err));
                }
                remaining -= to_read;
                total_read += to_read;
            }

            // Read trailing CRLF after chunk data
            let mut trailing_crlf = String::new();
            if let Err(err) = reader.read_line(&mut trailing_crlf).await {
                return Err(error::Error::Io(err));
            }
        }
    } else {
        // For responses without Content-Length and not chunked, read until connection closes
        let mut total_read = 0;

        loop {
            match reader.read(&mut discard_buf).await {
                Ok(0) => break, // Connection closed
                Ok(n) => {
                    if total_read + n > MAX_BODY_SIZE {
                        return Err(error::Error::Io(std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            format!(
                                "Response size ({} bytes) exceeds maximum size of {} bytes",
                                total_read + n,
                                MAX_BODY_SIZE
                            ),
                        )));
                    }
                    // Data is read into discard_buf but not stored - just count bytes
                    total_read += n;
                }
                Err(err) => {
                    return Err(error::Error::Io(err));
                }
            }
        }
    }

    // The timing measurement is complete once reading is done.
    let time_elapsed = now.elapsed();

    // Explicitly drop the BufReader which drops the underlying stream
    // This ensures the connection is closed properly
    drop(reader);

    Ok((time_elapsed, status))
}
/// Measures the HTTP timings from the given URL asynchronously.
///
/// # Errors
///
/// This function will return an error if the URL is invalid or the URL is not reachable.
/// It could also error under any scenario in the [`error::Error`] enum.
pub async fn from_url(url: &Url, connector: &SslConnector) -> Result<Response, error::Error> {
    let mut socket_addrs = resolve_dns(url).await?;
    let Some(url_ip) = socket_addrs.next() else {
        return Err(error::Error::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "invalid url ip",
        )));
    };

    let mut ssl_certificate = None;
    let mut ssl_certificate_information = None;
    let mut tls_timing = None;

    let (tcp_timing, mut stream) = if url.scheme() == "https" {
        let (tcp_timing, tcp_stream) = get_tcp_timing(&url_ip).await.map_err(error::Error::Io)?;
        let timing_response = get_tls_timing(url, tcp_stream, connector).await?;
        tls_timing = Some(timing_response.timing);
        ssl_certificate = timing_response.certificate;
        ssl_certificate_information = timing_response.certificate_information;
        (tcp_timing, timing_response.stream)
    } else {
        let (tcp_timing, tcp_stream) = get_tcp_timing(&url_ip).await.map_err(error::Error::Io)?;
        (
            tcp_timing,
            Box::new(tcp_stream) as Box<dyn AsyncReadWrite + Send>,
        )
    };

    // Send HTTP request - we don't use the send timing for metrics, but we measure it for completeness
    let _http_send_timing = get_http_send_timing(url, &mut stream).await?;

    let (ttfb_timing, first_byte) = get_ttfb_timing(&mut stream).await?;

    // get_content_download_timing consumes the stream and handles its cleanup internally
    // The stream will be properly dropped (and connection closed) when BufReader goes out of scope
    let (content_download_timing, status) = get_content_download_timing(stream, first_byte).await?;

    let response = Response {
        timings: ResponseTimings::new(tcp_timing, tls_timing, ttfb_timing, content_download_timing),
        certificate_information: ssl_certificate_information,
        certificate: ssl_certificate,
        status,
    };

    Ok(response)
}

/// Given a string, it will be parsed as a URL and the HTTP timings will be measured asynchronously.
/// An optional timeout can be applied to the entire operation.
///
/// # Errors
///
/// This function will return an error if the URL is invalid or the URL is not reachable.
/// It could also error under any scenario in the [`error::Error`] enum, or if the operation times out.
pub async fn from_string(
    url: &str,
    timeout: Option<Duration>,
    connector: &SslConnector,
) -> Result<Response, error::Error> {
    let input = if !url.starts_with("http://") && !url.starts_with("https://") {
        format!("https://{url}") // Default to https for safety
    } else {
        url.to_string()
    };

    let url = Url::parse(&input).map_err(|e| {
        error::Error::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("invalid url: {e}"),
        ))
    })?;

    match timeout {
        Some(t) => Ok(tokio::time::timeout(t, from_url(&url, connector)).await??),
        None => Ok(from_url(&url, connector).await?),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use openssl::ssl::{SslMethod, SslVerifyMode};
    const TIMEOUT: Duration = Duration::from_secs(5);

    fn create_test_connector() -> SslConnector {
        let mut builder = SslConnector::builder(SslMethod::tls()).unwrap();
        builder.set_verify(SslVerifyMode::NONE);
        builder.build()
    }

    #[tokio::test]
    async fn test_non_tls_connection() {
        let connector = create_test_connector();
        // Note: neverssl.com sometimes redirects to http.
        // We will use a more stable http-only target.
        let url = "http://info.cern.ch"; // The first website
        let result = from_string(url, Some(TIMEOUT), &connector).await;
        assert!(result.is_ok());
        let response = result.unwrap();
        assert_eq!(response.status, 200);
        assert!(response.timings.tls.is_none());
        assert!(response.timings.content_download < TIMEOUT);
    }

    #[tokio::test]
    async fn test_popular_tls_connection() {
        let connector = create_test_connector();
        let url = "https://www.google.com";
        let result = from_string(url, Some(TIMEOUT), &connector).await;
        assert!(result.is_ok());
        let response = result.unwrap();
        // Google might return 301/302 for redirection based on location
        assert!(response.status >= 200 && response.status < 400);
        assert!(response.certificate_information.is_some());
        assert!(response.timings.tls.is_some());
        assert!(response.timings.content_download < TIMEOUT);
    }

    #[tokio::test]
    async fn test_ip() {
        let connector = create_test_connector();
        let url = "1.1.1.1"; // This will default to https://1.1.1.1
        let result = from_string(url, Some(TIMEOUT), &connector).await;
        assert!(result.is_ok());
        let response = result.unwrap();
        // Expect a redirect to the hostname
        assert!(response.status >= 300 && response.status < 400);
        assert!(response.timings.tls.is_some());
        assert!(response.timings.content_download < TIMEOUT);
    }

    #[tokio::test]
    async fn test_timeout() {
        let connector = create_test_connector();
        // Use a non-routable address to force a timeout
        let url = "http://10.255.255.1";
        let result = from_string(url, Some(Duration::from_secs(1)), &connector).await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), error::Error::Timeout(_)));
    }
}
