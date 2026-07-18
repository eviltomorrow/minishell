use clap::Parser;

#[derive(Parser)]
#[command(name = "minishell-server", version, about = "SSH/SFTP server for minishell")]
struct Cli {
    /// Config file path
    #[arg(short, long)]
    config: Option<String>,

    /// Log level override
    #[arg(long)]
    log_level: Option<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let config_path = cli.config.unwrap_or_else(|| {
        "config.toml".to_string()
    });
    minishell_server::run_server(&config_path).await
}
