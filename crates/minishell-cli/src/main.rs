use std::path::PathBuf;
use std::sync::Arc;
use anyhow::Result;
use clap::{Parser, Subcommand};
use minishell_core::Machine;
use minishell_store::Store;
use unicode_width::UnicodeWidthStr;

#[derive(Parser)]
#[command(name = "minishell", version, about = "SSH Machine Management TUI Tool")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Query for quick login
    query: Option<String>,

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
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(home).join(".minishell")
}

fn open_db() -> Result<Store> {
    let path = db_path();
    let store = Store::open(&path)?;
    store.init()?;
    Ok(store)
}

fn pad_str(s: &str, width: usize, align_left: bool) -> String {
    let visible = UnicodeWidthStr::width(s);
    if visible >= width {
        return truncate_to_width(s, width);
    }
    let padding = " ".repeat(width - visible);
    if align_left {
        format!("{}{}", s, padding)
    } else {
        format!("{}{}", padding, s)
    }
}

fn truncate_to_width(s: &str, max_width: usize) -> String {
    let mut result = String::new();
    let mut current_width = 0;
    for c in s.chars() {
        let cw = unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
        if current_width + cw > max_width {
            break;
        }
        result.push(c);
        current_width += cw;
    }
    result
}

fn print_machines(machines: &[Machine]) {
    let cols = [
        ("#", 4, false),
        ("IP", 18, true),
        ("NAT-IP", 15, true),
        ("Port", 6, false),
        ("User", 10, true),
        ("Password", 12, true),
        ("Key", 28, true),
        ("Device", 8, true),
        ("Remark", 20, true),
    ];

    let header: String = cols.iter()
        .map(|(name, width, left)| pad_str(name, *width, *left))
        .collect::<Vec<_>>()
        .join("  ");
    println!("{}", header);
    println!("{}", "-".repeat(header.len()));

    for (i, m) in machines.iter().enumerate() {
        let or_dash = |s: &str| if s.is_empty() || s == "-" { "-".to_string() } else { s.to_string() };
        let values: Vec<String> = vec![
            format!("{}", i + 1),
            m.ip.clone(),
            or_dash(&m.nat_ip),
            format!("{}", m.port),
            m.username.clone(),
            or_dash(&m.password),
            or_dash(&m.private_key_path),
            or_dash(&m.device),
            or_dash(&m.remark),
        ];
        let row: String = values.iter().zip(cols.iter())
            .map(|(val, (_, width, left))| pad_str(val, *width, *left))
            .collect::<Vec<_>>()
            .join("  ");
        println!("{}", row);
    }
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
            let store = open_db()?;

            if let Some(ref query) = cli.query {
                if let Ok(id) = query.parse::<i64>() {
                    let machines = store.search("")?;
                    if let Some(m) = machines.iter().find(|m| m.id == id) {
                        minishell_ssh::login_to_machine(m)?;
                        return Ok(());
                    }
                }

                let machines = store.search(query)?;
                if machines.is_empty() {
                    println!("⚠ No machines found matching '{}'", query);
                    return Ok(());
                }
                if machines.len() == 1 {
                    minishell_ssh::login_to_machine(&machines[0])?;
                } else if let Some(selected) = minishell_tui::select_machine(machines)? {
                    minishell_ssh::login_to_machine(&selected)?;
                }
            } else {
                let store = Arc::new(store);
                minishell_tui::run(store)?;
            }
        }
    }

    Ok(())
}
