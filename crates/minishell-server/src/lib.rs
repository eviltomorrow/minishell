pub mod config;
pub mod server;
pub mod shell;
pub mod sftp;

use std::sync::Arc;
use config::{load_config, load_or_generate_host_key};
use russh::server::Server as _;

pub async fn run_server(config_path: &str) -> anyhow::Result<()> {
    let config = load_config(config_path)?;
    let config = Arc::new(config);

    // Setup logging
    let log_level = config.log.level.parse::<tracing_subscriber::filter::LevelFilter>()
        .unwrap_or(tracing_subscriber::filter::LevelFilter::INFO);
    tracing_subscriber::fmt()
        .with_max_level(log_level)
        .init();

    tracing::info!("Starting minishell-server on {}:{}", config.server.bind, config.server.port);

    // Load or generate host key
    let host_key_path = config.expanded_host_key_path();
    let host_key = load_or_generate_host_key(&host_key_path)?;
    let host_keys = vec![host_key];

    // Create russh config
    let russh_config = russh::server::Config {
        keys: host_keys,
        auth_rejection_time: std::time::Duration::from_secs(3),
        ..Default::default()
    };

    // Create server
    let mut server = server::MinishellServer::new(config.clone());

    // Run
    let addr = format!("{}:{}", config.server.bind, config.server.port);
    server.run_on_address(Arc::new(russh_config), &addr).await?;

    Ok(())
}
