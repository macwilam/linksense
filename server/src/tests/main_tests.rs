//! Tests for the main server module

use crate::Server;
use std::io::Write;
use tempfile::NamedTempFile;

#[tokio::test]
async fn test_server_creation() {
    // Create a temporary configuration file for the test.
    let mut temp_file = NamedTempFile::new().unwrap();
    writeln!(
        temp_file,
        r#"
listen_address = "127.0.0.1:8787"
api_key = "test-api-key"
data_retention_days = 30
agent_configs_dir = "/tmp/configs"
bandwidth_test_size_mb = 10
"#
    )
    .unwrap();

    let config_path = temp_file.path().to_path_buf();

    // In the current development phase, this test might have different
    // expectations. Here, we are just checking if `Server::new` can be
    // called and if it returns a `Result`.
    let result = Server::new(config_path);

    // As of Phase 1.2, with the `config` module implemented, server creation
    // should succeed if a valid config file is provided.
    assert!(result.is_ok());
}
