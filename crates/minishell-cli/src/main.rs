use std::io::IsTerminal;
use std::path::PathBuf;
use std::sync::Arc;
use anyhow::Result;
use clap::{Parser, Subcommand};
use minishell_core::Machine;
use minishell_store::Store;

#[derive(Parser)]
#[command(name = "minishell", version, about = "SSH Machine Management TUI Tool")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Query for quick login
    query: Option<String>,

    /// Skip TUI, use CLI mode
    #[arg(long)]
    no_tui: bool,
}

#[derive(Subcommand)]
enum Commands {
    /// Print version info
    Version,

    /// Generate import template
    Tpl {
        /// Output path
        path: Option<String>,
    },

    /// Import machines from Excel
    Import {
        /// Excel file path
        path: String,
    },

    /// Export machines to Excel
    Export {
        /// Output path
        path: Option<String>,
    },

    /// Show all machines
    Show,
}

fn db_path() -> PathBuf {
    PathBuf::from("/tmp/minishell")
}

fn open_db() -> Result<Store> {
    let path = db_path();
    let store = Store::open(&path)?;
    store.init()?;
    Ok(store)
}

fn can_use_tui() -> bool {
    if std::env::var("TERM").as_deref() == Ok("dumb") {
        return false;
    }
    std::io::stdout().is_terminal()
}

fn print_machines(machines: &[Machine]) {
    let header = format!("{:>4}  {:<15}  {:<15}  {:>5}  {:<10}  {:<15}  {:<20}  {:<10}  {:<20}",
        "#", "IP", "NAT-IP", "Port", "User", "Password", "Key", "Device", "Remark");
    println!("{}", header);
    println!("{}", "-".repeat(120));

    for (i, m) in machines.iter().enumerate() {
        let or_dash = |s: &str| if s.is_empty() || s == "-" { "-".to_string() } else { s.to_string() };
        println!("{:>4}  {:<15}  {:<15}  {:>5}  {:<10}  {:<15}  {:<20}  {:<10}  {:<20}",
            i + 1,
            m.ip,
            or_dash(&m.nat_ip),
            m.port,
            m.username,
            or_dash(&m.password),
            or_dash(&m.private_key_path),
            or_dash(&m.device),
            or_dash(&m.remark),
        );
    }
}

fn quick_login(query: &str) -> Result<()> {
    let store = open_db()?;

    // Try exact match by ID
    if let Ok(id) = query.parse::<i64>() {
        let machines = store.search("")?;
        if let Some(m) = machines.iter().find(|m| m.id == id) {
            minishell_ssh::login_to_machine(m)?;
            return Ok(());
        }
    }

    // Try exact match by IP
    let machines = store.search(query)?;
    if machines.len() == 1 {
        minishell_ssh::login_to_machine(&machines[0])?;
        return Ok(());
    }

    if machines.is_empty() {
        anyhow::bail!("No machines found matching '{}'", query);
    }

    // Multiple matches - try selector if TUI available
    if can_use_tui() {
        if let Some(selected) = minishell_tui::select_machine(machines)? {
            minishell_ssh::login_to_machine(&selected)?;
        }
    } else {
        println!("Multiple matches for '{}':", query);
        print_machines(&machines);
        anyhow::bail!("Please refine your search or use TUI mode");
    }

    Ok(())
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Version) => {
            println!("minishell {}", env!("CARGO_PKG_VERSION"));
            println!("git: {}", option_env!("GIT_SHA").unwrap_or("unknown"));
            println!("built: {}", option_env!("BUILD_TIME").unwrap_or("unknown"));
        }
        Some(Commands::Tpl { path }) => {
            let path = path.map(PathBuf::from).unwrap_or_else(|| {
                let bin_dir = std::env::current_exe().ok()
                    .and_then(|p| p.parent().map(|p| p.to_path_buf()))
                    .unwrap_or_else(|| PathBuf::from("."));
                bin_dir.join("machines-template.xlsx")
            });
            minishell_xlsx::generate_template(&path)?;
            println!("Template generated: {}", path.display());
        }
        Some(Commands::Import { path }) => {
            let store = open_db()?;
            let machines = minishell_xlsx::import_from(PathBuf::from(&path).as_path())?;
            let count = store.import_machines(&machines)?;
            println!("Imported {} machines ({} skipped)", count, machines.len() - count);
        }
        Some(Commands::Export { path }) => {
            let store = open_db()?;
            let machines = store.search("")?;
            let path = path.map(PathBuf::from).unwrap_or_else(|| {
                let bin_dir = std::env::current_exe().ok()
                    .and_then(|p| p.parent().map(|p| p.to_path_buf()))
                    .unwrap_or_else(|| PathBuf::from("."));
                bin_dir.join("machines-export.xlsx")
            });
            minishell_xlsx::export_to(&path, &machines)?;
            println!("Exported {} machines to {}", machines.len(), path.display());
        }
        Some(Commands::Show) => {
            let store = open_db()?;
            let machines = store.search("")?;
            print_machines(&machines);
        }
        None => {
            if let Some(query) = cli.query {
                quick_login(&query)?;
            } else if cli.no_tui || !can_use_tui() {
                let store = open_db()?;
                let machines = store.search("")?;
                print_machines(&machines);
            } else {
                let store = open_db()?;
                let store = Arc::new(store);
                if let Some(machine) = minishell_tui::run(store)? {
                    minishell_ssh::login_to_machine(&machine)?;
                }
            }
        }
    }

    Ok(())
}
