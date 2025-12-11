//! REST API implementation for the network monitoring central server
//!
//! This module provides the HTTP endpoints that agents use to communicate with
//! the central server, including metrics submission, configuration retrieval,
//! and bandwidth testing.
// This module uses the `axum` web framework to build the API. Each public
// function corresponds to an API endpoint and is responsible for handling
// incoming requests, interacting with other parts of the server (like the
// database), and returning appropriate responses.

use axum::{
    extract::{DefaultBodyLimit, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use base64::Engine;
use shared::{
    api::{
        // Importing the data structures for API requests and responses from the `shared` crate.
        endpoints,
        headers,
        BandwidthTestRequest,
        BandwidthTestResponse,
        ConfigErrorRequest,
        ConfigStatus,
        ConfigUploadRequest,
        ConfigUploadResponse,
        ConfigVerifyRequest,
        ConfigVerifyResponse,
        ConfigsResponse,
        MetricsRequest,
        MetricsResponse,
    },
    config::ServerConfig,
    utils::encode_base64,
};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

/// Simple rate limiter per agent ID
///
/// Tracks request timestamps per agent and enforces rate limits based on
/// a sliding window approach. Old requests outside the time window are
/// automatically cleaned up.
pub struct AgentRateLimiter {
    /// Map of agent ID to list of request timestamps
    limits: Arc<RwLock<HashMap<String, Vec<Instant>>>>,
    /// Time window for rate limiting
    window: Duration,
    /// Maximum number of requests allowed within the window
    max_requests: usize,
}

impl AgentRateLimiter {
    /// Create a new rate limiter
    ///
    /// # Parameters
    /// * `window` - Time window for rate limiting (e.g., 60 seconds)
    /// * `max_requests` - Maximum requests allowed within the window
    pub fn new(window: Duration, max_requests: usize) -> Self {
        Self {
            limits: Arc::new(RwLock::new(HashMap::new())),
            window,
            max_requests,
        }
    }

    /// Check if a request is allowed for the given agent
    ///
    /// Returns Ok(()) if allowed, Err(ApiError::TooManyRequests) if rate limit exceeded
    pub async fn check_rate_limit(&self, agent_id: &str) -> Result<(), ApiError> {
        let now = Instant::now();
        let mut limits = self.limits.write().await;

        let requests = limits.entry(agent_id.to_string()).or_insert_with(Vec::new);

        // Remove old requests outside the window
        requests.retain(|&time| now.duration_since(time) < self.window);

        if requests.len() >= self.max_requests {
            warn!(
                agent_id = %agent_id,
                count = requests.len(),
                max = self.max_requests,
                "Rate limit exceeded"
            );
            return Err(ApiError::TooManyRequests);
        }

        requests.push(now);
        Ok(())
    }

    /// Remove stale entries from agents that haven't sent requests recently.
    /// This prevents unbounded memory growth from agents that connect once and never return.
    pub async fn cleanup_stale_entries(&self) {
        let mut limits = self.limits.write().await;
        let now = Instant::now();

        // Remove entries where all timestamps are older than the window
        // (meaning the agent hasn't sent requests within the rate limit window)
        let before_count = limits.len();
        limits.retain(|_, timestamps| {
            timestamps.retain(|&time| now.duration_since(time) < self.window);
            !timestamps.is_empty()
        });
        let removed = before_count.saturating_sub(limits.len());

        if removed > 0 {
            debug!(
                removed_agents = removed,
                remaining_agents = limits.len(),
                "Cleaned up stale rate limiter entries"
            );
        }
    }

    /// Returns the number of agents currently tracked
    pub async fn tracked_agent_count(&self) -> usize {
        self.limits.read().await.len()
    }
}

impl Clone for AgentRateLimiter {
    fn clone(&self) -> Self {
        Self {
            limits: Arc::clone(&self.limits),
            window: self.window,
            max_requests: self.max_requests,
        }
    }
}

/// Application state shared across all API handlers
#[derive(Clone)]
pub struct AppState {
    /// Server configuration
    pub config: Arc<ServerConfig>,
    /// Agent-specific rate limiter
    pub rate_limiter: AgentRateLimiter,
    /// Database handle for storing metrics and agent data
    pub database: Arc<tokio::sync::Mutex<crate::database::ServerDatabase>>,
    /// Configuration manager for serving agent configs
    pub config_manager: Arc<tokio::sync::Mutex<crate::config::ConfigManager>>,
    /// Bandwidth test coordination manager
    pub bandwidth_manager: Arc<tokio::sync::Mutex<crate::bandwidth_state::BandwidthTestManager>>,
}

impl AppState {
    /// Create new application state from server configuration and dependencies
    pub fn new(
        config: ServerConfig,
        database: Arc<tokio::sync::Mutex<crate::database::ServerDatabase>>,
        config_manager: Arc<tokio::sync::Mutex<crate::config::ConfigManager>>,
        bandwidth_manager: crate::bandwidth_state::BandwidthTestManager,
    ) -> Self {
        // Create rate limiter with configured values
        let rate_limiter = AgentRateLimiter::new(
            Duration::from_secs(config.rate_limit_window_seconds as u64),
            config.rate_limit_max_requests,
        );

        Self {
            config: Arc::new(config),
            rate_limiter,
            database,
            config_manager,
            bandwidth_manager: Arc::new(tokio::sync::Mutex::new(bandwidth_manager)),
        }
    }
}

/// Creates the main API router and defines all the application's routes.
/// This function is called once at server startup to build the routing tree.
pub fn create_router(state: AppState) -> Router {
    // Maximum request body size: 10MB
    // Prevents memory exhaustion attacks from large request payloads
    const MAX_REQUEST_SIZE: usize = 10 * 1024 * 1024;

    Router::new()
        // A simple, unauthenticated health check endpoint. This is useful for
        // load balancers, container orchestrators (like Kubernetes), or monitoring
        // systems to verify that the server process is running and responsive.
        .route("/health", get(health_check))
        // These are the main API endpoints for agent communication.
        // They are defined as constants in `shared::api::endpoints` to ensure
        // consistency between the agent and the server.
        .route(endpoints::METRICS, post(handle_metrics))
        .route(endpoints::CONFIGS, get(handle_configs))
        .route(endpoints::CONFIG_ERROR, post(handle_config_error))
        .route(endpoints::CONFIG_VERIFY, post(handle_config_verify))
        .route(endpoints::CONFIG_UPLOAD, post(handle_config_upload))
        .route(endpoints::BANDWIDTH_TEST, post(handle_bandwidth_test))
        .route(
            endpoints::BANDWIDTH_DOWNLOAD,
            get(handle_bandwidth_download),
        )
        .layer(DefaultBodyLimit::max(MAX_REQUEST_SIZE))
        .with_state(state)
}

/// Helper function to validate API key from request headers
///
/// Uses constant-time comparison to prevent timing attacks that could
/// allow an attacker to deduce the API key character-by-character.
fn validate_api_key(headers: &HeaderMap, expected_key: &str) -> Result<(), ApiError> {
    use subtle::ConstantTimeEq;

    let provided_key = match headers.get(headers::API_KEY) {
        Some(key) => match key.to_str() {
            Ok(key_str) => key_str,
            Err(_) => {
                warn!("Invalid API key format in header");
                return Err(ApiError::Unauthorized);
            }
        },
        None => {
            warn!("Missing API key header");
            return Err(ApiError::Unauthorized);
        }
    };

    if provided_key.is_empty() {
        warn!("Empty API key provided");
        return Err(ApiError::Unauthorized);
    }

    // Use constant-time comparison to prevent timing attacks
    let provided_bytes = provided_key.as_bytes();
    let expected_bytes = expected_key.as_bytes();

    // First check lengths match (this leaks length info, but API keys should be fixed length)
    // Then do constant-time content comparison
    let keys_match = provided_bytes.len() == expected_bytes.len()
        && bool::from(provided_bytes.ct_eq(expected_bytes));

    if !keys_match {
        warn!("Invalid API key provided");
        return Err(ApiError::Unauthorized);
    }

    Ok(())
}

/// Helper function to validate agent ID from request
///
/// Agent IDs must:
/// - Not be empty
/// - Be between 1 and 128 characters
/// - Contain only alphanumeric characters, hyphens, and underscores
/// - Not start or end with special characters
fn validate_agent_id(agent_id: &str) -> Result<(), ApiError> {
    // Check if empty
    if agent_id.is_empty() {
        warn!("Empty agent ID provided");
        return Err(ApiError::BadRequest("Agent ID cannot be empty".to_string()));
    }

    // Check length (reasonable limits)
    if agent_id.len() > 128 {
        warn!("Agent ID too long: {} characters", agent_id.len());
        return Err(ApiError::BadRequest(format!(
            "Agent ID too long: {} characters (max 128)",
            agent_id.len()
        )));
    }

    // Check format: alphanumeric, hyphens, underscores only
    if !agent_id
        .chars()
        .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
    {
        warn!("Agent ID contains invalid characters: {}", agent_id);
        return Err(ApiError::BadRequest(
            "Agent ID must contain only alphanumeric characters, hyphens, and underscores"
                .to_string(),
        ));
    }

    // Check doesn't start or end with special characters
    if agent_id.starts_with('-')
        || agent_id.starts_with('_')
        || agent_id.ends_with('-')
        || agent_id.ends_with('_')
    {
        warn!(
            "Agent ID has invalid format (starts/ends with special char): {}",
            agent_id
        );
        return Err(ApiError::BadRequest(
            "Agent ID cannot start or end with hyphens or underscores".to_string(),
        ));
    }

    Ok(())
}

/// Helper function to validate agent ID against whitelist
///
/// Checks if the agent ID is allowed based on the server's whitelist configuration.
/// If whitelist is empty, all agents are allowed.
/// If whitelist is configured, only agents in the list are allowed.
///
/// # Parameters
/// * `agent_id` - The agent ID to validate
/// * `whitelist` - The configured whitelist from ServerConfig
///
/// # Returns
/// * `Ok(())` - Agent is allowed (either whitelist is empty or agent is in the list)
/// * `Err(ApiError::Forbidden)` - Agent is not in the whitelist
fn validate_agent_whitelist(agent_id: &str, whitelist: &[String]) -> Result<(), ApiError> {
    // If whitelist is empty, allow all agents
    if whitelist.is_empty() {
        return Ok(());
    }

    // Check if agent is in whitelist
    if whitelist.iter().any(|allowed_id| allowed_id == agent_id) {
        Ok(())
    } else {
        warn!(
            agent_id = %agent_id,
            "Agent ID not in whitelist"
        );
        Err(ApiError::Forbidden("Agent ID not in whitelist".to_string()))
    }
}

/// The handler for the `/health` endpoint.
/// It returns a simple JSON response indicating the server's status.
async fn health_check() -> impl IntoResponse {
    // Using `serde_json::json!` macro for a convenient way to create JSON values.
    Json(serde_json::json!({
        "status": "healthy",
        "service": "network-monitoring-server",
        "version": env!("CARGO_PKG_VERSION") // Includes the crate version from Cargo.toml.
    }))
}

/// The handler for the metrics submission endpoint.
/// Agents send their collected metrics to this endpoint.
// `Json(request)` is an `axum` extractor that deserializes the request body
// from JSON into a `MetricsRequest` struct.
async fn handle_metrics(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<MetricsRequest>,
) -> Result<Json<MetricsResponse>, ApiError> {
    // Validate API key against configured value
    validate_api_key(&headers, &state.config.api_key)?;

    // Validate agent ID
    validate_agent_id(&request.agent_id)?;

    // Validate agent against whitelist
    validate_agent_whitelist(&request.agent_id, &state.config.agent_id_whitelist)?;

    // Check rate limit for this agent (if enabled)
    if state.config.rate_limit_enabled {
        state
            .rate_limiter
            .check_rate_limit(&request.agent_id)
            .await?;
    }

    // Structured logging is used to record key information from the request.
    info!(
        agent_id = %request.agent_id,
        metric_count = request.metrics.len(),
        "Received metrics from agent"
    );

    // Upsert agent in database to track last seen time and config checksum
    {
        let mut db = state.database.lock().await;
        if let Err(e) = db
            .upsert_agent(
                &request.agent_id,
                &request.config_checksum,
                request.agent_version.as_deref(),
            )
            .await
        {
            error!(
                agent_id = %request.agent_id,
                error = %e,
                "Failed to upsert agent in database"
            );
            return Err(ApiError::Database(format!(
                "Failed to update agent status: {}",
                e
            )));
        }
    }

    // Store metrics in database
    if !request.metrics.is_empty() {
        let mut db = state.database.lock().await;
        if let Err(e) = db.store_metrics(&request.agent_id, &request.metrics).await {
            error!(
                agent_id = %request.agent_id,
                metric_count = request.metrics.len(),
                error = %e,
                "Failed to store metrics in database"
            );
            return Err(ApiError::Database(format!(
                "Failed to store metrics: {}",
                e
            )));
        }

        info!(
            agent_id = %request.agent_id,
            metric_count = request.metrics.len(),
            "Successfully stored metrics in database"
        );
    }

    // Compare config hash to detect if agent needs to update
    let config_status = {
        let config_manager = state.config_manager.lock().await;
        match config_manager.get_agent_config(&request.agent_id).await {
            Ok(cached) => {
                if cached.hash == request.config_checksum {
                    debug!(
                        agent_id = %request.agent_id,
                        "Agent config is up to date"
                    );
                    ConfigStatus::UpToDate
                } else {
                    info!(
                        agent_id = %request.agent_id,
                        agent_hash = %request.config_checksum,
                        server_hash = %cached.hash,
                        "Agent config is stale, needs update"
                    );
                    ConfigStatus::Stale
                }
            }
            Err(_) => {
                // Server doesn't have a config for this agent
                // Request that the agent upload its config
                warn!(
                    agent_id = %request.agent_id,
                    "Server has no config for agent, requesting agent to upload its configuration"
                );
                ConfigStatus::Stale
            }
        }
    };

    let response = if config_status == ConfigStatus::Stale {
        MetricsResponse::stale()
    } else {
        MetricsResponse::up_to_date()
    };

    Ok(Json(response))
}

/// The handler for the bandwidth download endpoint.
/// Agents download test data from this endpoint to measure their network throughput.
/// This should only be called after receiving permission via the bandwidth_test endpoint.
///
/// IMPORTANT: The download size is controlled ONLY by server configuration.
/// The agent cannot influence the size - any size_mb parameter is ignored.
///
/// This endpoint uses streaming to avoid allocating the entire response in memory,
/// which prevents memory exhaustion attacks with large bandwidth test sizes.
async fn handle_bandwidth_download(
    State(state): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<impl IntoResponse, ApiError> {
    use futures_util::stream;

    let agent_id = params.get("agent_id").map_or("", |s| s.as_str());

    // Validate agent ID
    validate_agent_id(agent_id)?;

    // Validate agent against whitelist
    validate_agent_whitelist(agent_id, &state.config.agent_id_whitelist)?;

    // Validate that this agent has an active bandwidth test
    {
        let bandwidth_manager = state.bandwidth_manager.lock().await;
        let status = bandwidth_manager.get_status().await;

        if let Some((current_agent, _start_time)) = status.current_test {
            if current_agent != agent_id {
                warn!(
                    agent_id = %agent_id,
                    current_agent = %current_agent,
                    "Agent attempted bandwidth download without permission"
                );
                return Err(ApiError::BadRequest(
                    "No active bandwidth test for this agent".to_string(),
                ));
            }
        } else {
            warn!(
                agent_id = %agent_id,
                "Agent attempted bandwidth download without active test"
            );
            return Err(ApiError::BadRequest("No active bandwidth test".to_string()));
        }
    }

    // Use server configuration for bandwidth test size
    // Agent cannot influence this value - it comes from server.toml
    let size_mb = state.config.bandwidth_test_size_mb;

    debug!(
        agent_id = %agent_id,
        size_mb = size_mb,
        "Serving bandwidth test data (server-controlled size)"
    );

    // Use streaming to avoid allocating entire response in memory
    // This prevents memory exhaustion with large bandwidth tests (e.g., 100MB+)
    const CHUNK_SIZE: usize = 64 * 1024; // 64 KB chunks - good balance for network efficiency
    let total_size = (size_mb as usize).saturating_mul(1024 * 1024);
    let chunks_needed = total_size.saturating_add(CHUNK_SIZE - 1) / CHUNK_SIZE;

    // Create a single zero-filled chunk that will be reused in the stream
    // This keeps memory usage constant at ~64KB regardless of total download size
    let zero_chunk = axum::body::Bytes::from(vec![0u8; CHUNK_SIZE]);

    // Create a stream that yields chunks without allocating the full response
    let byte_stream = stream::iter((0..chunks_needed).map(move |i| {
        let offset = i * CHUNK_SIZE;
        let remaining = total_size.saturating_sub(offset);
        let chunk_len = std::cmp::min(remaining, CHUNK_SIZE);

        if chunk_len == CHUNK_SIZE {
            // Full chunk - reuse the pre-allocated buffer
            Ok::<_, std::io::Error>(zero_chunk.clone())
        } else {
            // Last chunk may be smaller
            Ok(axum::body::Bytes::from(vec![0u8; chunk_len]))
        }
    }));

    // Mark the test as completed so queued tests can proceed
    // We do this before streaming starts so other agents don't wait unnecessarily
    {
        let bandwidth_manager = state.bandwidth_manager.lock().await;
        bandwidth_manager.complete_test(agent_id).await;
    }

    info!(
        agent_id = %agent_id,
        total_size = total_size,
        "Bandwidth test streaming started, queued agents can proceed"
    );

    let response = Response::builder()
        .status(StatusCode::OK)
        .header(axum::http::header::CONTENT_TYPE, "application/octet-stream")
        .header(axum::http::header::CONTENT_LENGTH, total_size)
        .body(axum::body::Body::from_stream(byte_stream))
        .map_err(|e| ApiError::Internal(format!("Failed to build response: {}", e)))?;

    Ok(response)
}

/// The handler for the configuration retrieval endpoint.
/// Agents call this endpoint to download their configuration files.
// `Query(params)` is an extractor for URL query parameters.
async fn handle_configs(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Json<ConfigsResponse>, ApiError> {
    // Validate API key - configs contain sensitive task definitions
    validate_api_key(&headers, &state.config.api_key)?;

    // The agent identifies itself via a query parameter.
    let agent_id = params.get("agent_id").map_or("", |s| s.as_str());

    // Validate agent ID
    validate_agent_id(agent_id)?;

    // Validate agent against whitelist
    validate_agent_whitelist(agent_id, &state.config.agent_id_whitelist)?;

    info!(agent_id = %agent_id, "Agent requesting configuration");

    // Get agent's tasks.toml configuration (from cache or disk)
    let tasks_toml_compressed = {
        let config_manager = state.config_manager.lock().await;
        match config_manager.get_agent_config(agent_id).await {
            Ok(cached) => cached.compressed,
            Err(_) => {
                warn!(
                    agent_id = %agent_id,
                    "Agent configuration not found"
                );
                return Err(ApiError::BadRequest(format!(
                    "Configuration not found for agent {}",
                    agent_id
                )));
            }
        }
    };

    // Build a minimal agent.toml with the agent_id
    // In a production system, this could also be read from a file
    let agent_toml_content = format!(
        r#"agent_id = "{}"
# Agent-specific configuration (auto-generated by server)
"#,
        agent_id
    );
    let agent_toml_encoded = encode_base64(&agent_toml_content);

    let response = ConfigsResponse {
        agent_toml: agent_toml_encoded,
        tasks_toml: tasks_toml_compressed,
    };

    info!(
        agent_id = %agent_id,
        "Successfully served configuration to agent"
    );

    Ok(Json(response))
}

/// The handler for agents to report errors they encounter while parsing configuration.
/// This is a "fire and forget" endpoint for the agent.
async fn handle_config_error(
    State(state): State<AppState>,
    Json(request): Json<ConfigErrorRequest>,
) -> Result<StatusCode, ApiError> {
    // Validate agent ID before processing
    if let Err(_err) = validate_agent_id(&request.agent_id) {
        // Still log the error but with warning about invalid agent ID
        warn!(
            agent_id = %request.agent_id,
            timestamp = %request.timestamp_utc,
            error = %request.error_message,
            "Agent reported configuration error (INVALID AGENT ID)"
        );
        return Err(ApiError::BadRequest("Invalid agent ID".to_string()));
    }

    // Validate agent against whitelist
    validate_agent_whitelist(&request.agent_id, &state.config.agent_id_whitelist)?;

    // It's important to log these errors on the server, as they might indicate
    // a problem with the configuration files being served.
    error!(
        agent_id = %request.agent_id,
        timestamp = %request.timestamp_utc,
        error = %request.error_message,
        "Agent reported configuration error"
    );

    // Store configuration error in database for tracking
    {
        let mut db = state.database.lock().await;
        if let Err(e) = db
            .log_config_error(
                &request.agent_id,
                &request.timestamp_utc,
                &request.error_message,
            )
            .await
        {
            // Log the error but don't fail the request - we already have the error in logs
            warn!(
                agent_id = %request.agent_id,
                error = %e,
                "Failed to store config error in database"
            );
        }
    }

    // The server responds with `202 Accepted` to indicate that it has received
    // the error report but has not necessarily taken any action yet.
    Ok(StatusCode::ACCEPTED)
}

/// The handler for the config verification endpoint.
/// Agents call this endpoint to check if their configuration is up to date.
async fn handle_config_verify(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<ConfigVerifyRequest>,
) -> Result<Json<ConfigVerifyResponse>, ApiError> {
    // Validate API key against configured value
    validate_api_key(&headers, &state.config.api_key)?;

    // Validate agent ID
    validate_agent_id(&request.agent_id)?;

    // Validate agent against whitelist
    validate_agent_whitelist(&request.agent_id, &state.config.agent_id_whitelist)?;

    // Check rate limit for this agent (if enabled)
    if state.config.rate_limit_enabled {
        state
            .rate_limiter
            .check_rate_limit(&request.agent_id)
            .await?;
    }

    info!(
        agent_id = %request.agent_id,
        tasks_hash = %request.tasks_config_hash,
        "Received config verification request from agent"
    );

    // Get agent config (from cache or disk)
    let cached_config = {
        let config_manager = state.config_manager.lock().await;
        match config_manager.get_agent_config(&request.agent_id).await {
            Ok(cached) => cached,
            Err(_) => {
                warn!(
                    agent_id = %request.agent_id,
                    "Server has no config for agent"
                );
                // If we can't get the config, tell the agent to update
                let response = ConfigVerifyResponse {
                    status: "error".to_string(),
                    config_status: shared::api::ConfigStatus::Stale,
                    tasks_toml: None,
                };
                return Ok(Json(response));
            }
        }
    };

    // Compare agent's hash with server's hash
    if request.tasks_config_hash == cached_config.hash {
        debug!(
            agent_id = %request.agent_id,
            hash = %cached_config.hash,
            "Agent config is up to date"
        );

        let response = ConfigVerifyResponse {
            status: "success".to_string(),
            config_status: shared::api::ConfigStatus::UpToDate,
            tasks_toml: None,
        };
        Ok(Json(response))
    } else {
        info!(
            agent_id = %request.agent_id,
            agent_hash = %request.tasks_config_hash,
            server_hash = %cached_config.hash,
            "Agent config is outdated, sending new config"
        );

        let response = ConfigVerifyResponse {
            status: "success".to_string(),
            config_status: shared::api::ConfigStatus::Stale,
            tasks_toml: Some(cached_config.compressed),
        };
        Ok(Json(response))
    }
}

/// The handler for the configuration upload endpoint.
/// Agents call this endpoint to upload their local configuration to the server
/// when the server doesn't have a configuration for them.
async fn handle_config_upload(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<ConfigUploadRequest>,
) -> Result<Json<ConfigUploadResponse>, ApiError> {
    // Validate API key against configured value
    validate_api_key(&headers, &state.config.api_key)?;

    // Validate agent ID
    validate_agent_id(&request.agent_id)?;

    // Validate agent against whitelist
    validate_agent_whitelist(&request.agent_id, &state.config.agent_id_whitelist)?;

    // Check rate limit for this agent (if enabled)
    if state.config.rate_limit_enabled {
        state
            .rate_limiter
            .check_rate_limit(&request.agent_id)
            .await?;
    }

    info!(
        agent_id = %request.agent_id,
        "Received config upload request from agent"
    );

    // Decode and decompress the tasks.toml content
    use flate2::read::GzDecoder;
    use std::io::Read;

    let compressed_data = base64::engine::general_purpose::STANDARD
        .decode(&request.tasks_toml)
        .map_err(|e| {
            warn!(
                agent_id = %request.agent_id,
                error = %e,
                "Failed to decode base64 config data"
            );
            ApiError::BadRequest(format!("Invalid base64 encoding: {}", e))
        })?;

    let mut decoder = GzDecoder::new(&compressed_data[..]);
    let mut tasks_toml_content = String::new();
    decoder
        .read_to_string(&mut tasks_toml_content)
        .map_err(|e| {
            warn!(
                agent_id = %request.agent_id,
                error = %e,
                "Failed to decompress config data"
            );
            ApiError::BadRequest(format!("Invalid gzip compression: {}", e))
        })?;

    // Validate the TOML syntax and structure
    let validation_result = shared::config::TasksConfig::validate_from_toml(&tasks_toml_content);
    if let Err(e) = validation_result {
        warn!(
            agent_id = %request.agent_id,
            error = %e,
            "Agent uploaded invalid tasks configuration"
        );
        return Ok(Json(ConfigUploadResponse {
            status: "error".to_string(),
            message: format!("Invalid tasks configuration: {}", e),
            accepted: false,
        }));
    }

    // Save the configuration to disk
    let agent_configs_dir = std::path::PathBuf::from(&state.config.agent_configs_dir);
    let agent_config_path = agent_configs_dir.join(format!("{}.toml", request.agent_id));

    // Check if config already exists (use async metadata check to avoid blocking)
    if tokio::fs::try_exists(&agent_config_path)
        .await
        .unwrap_or(false)
    {
        info!(
            agent_id = %request.agent_id,
            path = %agent_config_path.display(),
            "Configuration already exists for agent, not overwriting"
        );
        return Ok(Json(ConfigUploadResponse {
            status: "success".to_string(),
            message: "Configuration already exists on server, using existing config".to_string(),
            accepted: false,
        }));
    }

    // Write the configuration file using async I/O to avoid blocking the runtime
    if let Err(e) = tokio::fs::write(&agent_config_path, &tasks_toml_content).await {
        error!(
            agent_id = %request.agent_id,
            path = %agent_config_path.display(),
            error = %e,
            "Failed to write agent configuration file"
        );
        return Err(ApiError::Internal(format!(
            "Failed to save configuration: {}",
            e
        )));
    }

    info!(
        agent_id = %request.agent_id,
        path = %agent_config_path.display(),
        "Successfully saved agent configuration"
    );

    Ok(Json(ConfigUploadResponse {
        status: "success".to_string(),
        message: "Configuration uploaded and saved successfully".to_string(),
        accepted: true,
    }))
}

/// The handler for the bandwidth test coordination endpoint.
/// Agents call this endpoint to request permission to start a bandwidth test.
/// The server coordinates tests to ensure only one runs at a time.
async fn handle_bandwidth_test(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<BandwidthTestRequest>,
) -> Result<Json<BandwidthTestResponse>, ApiError> {
    // Validate API key against configured value
    validate_api_key(&headers, &state.config.api_key)?;

    // Validate agent ID
    validate_agent_id(&request.agent_id)?;

    // Validate agent against whitelist
    validate_agent_whitelist(&request.agent_id, &state.config.agent_id_whitelist)?;

    // Check rate limit for this agent (if enabled)
    if state.config.rate_limit_enabled {
        state
            .rate_limiter
            .check_rate_limit(&request.agent_id)
            .await?;
    }

    debug!(
        agent_id = %request.agent_id,
        "Received bandwidth test request from agent"
    );

    // Get data size from server configuration
    let data_size_bytes = (state.config.bandwidth_test_size_mb as u64) * 1024 * 1024;

    // Request bandwidth test from the manager
    let response = {
        let bandwidth_manager = state.bandwidth_manager.lock().await;
        bandwidth_manager
            .request_test(request.agent_id.clone(), data_size_bytes)
            .await
    };

    debug!(
        agent_id = %request.agent_id,
        action = ?response.action,
        "Bandwidth test request processed"
    );

    Ok(Json(response))
}

/// Custom error types for the API.
/// Using a dedicated enum for API errors allows for consistent error handling
/// and response formatting.
#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("Unauthorized")]
    Unauthorized,
    #[error("Forbidden: {0}")]
    Forbidden(String),
    #[error("Bad request: {0}")]
    BadRequest(String),
    #[error("Too many requests")]
    TooManyRequests,
    #[error("Internal server error: {0}")]
    Internal(String),
    #[error("Database error: {0}")]
    Database(String),
}

/// This implementation allows `ApiError` to be converted into an HTTP response.
/// This is a key part of `axum`'s error handling. If a handler returns a
/// `Result<_, ApiError>`, `axum` will automatically call this `into_response`
/// method when the `Err` variant is returned.
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, error_message) = match self {
            ApiError::Unauthorized => (StatusCode::UNAUTHORIZED, "Unauthorized"),
            ApiError::Forbidden(_) => (StatusCode::FORBIDDEN, "Forbidden"),
            ApiError::BadRequest(_) => (StatusCode::BAD_REQUEST, "Bad Request"),
            ApiError::TooManyRequests => (StatusCode::TOO_MANY_REQUESTS, "Too Many Requests"),
            ApiError::Internal(_) => (StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error"),
            ApiError::Database(_) => (StatusCode::INTERNAL_SERVER_ERROR, "Database Error"),
        };

        // The error response body is a JSON object with a consistent structure.
        let body = Json(serde_json::json!({
            "error": error_message,
            "details": self.to_string() // Includes the detailed error message.
        }));

        (status, body).into_response()
    }
}
