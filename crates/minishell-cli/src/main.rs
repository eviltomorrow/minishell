use std::path::PathBuf;
use std::sync::Arc;
use anyhow::Result;
use clap::{Parser, Subcommand};
use minishell_core::Machine;
use minishell_store::Store;
use minishell_utils::{pad_left, pad_right};
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

    /// Reset num column sequentially from 1
    Resetnum,
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
    if align_left {
        pad_right(s, width)
    } else {
        pad_left(s, width)
    }
}

fn print_machines(machines: &[Machine]) {
    let col_meta: &[(&str, bool)] = &[
        ("#", false),
        ("IP", true),
        ("NAT-IP", true),
        ("Port", false),
        ("User", true),
        ("Password", true),
        ("Key", true),
        ("Device", true),
        ("Remark", true),
    ];

    let or_dash = |s: &str| if s.is_empty() || s == "-" { "-".to_string() } else { s.to_string() };

    let rows: Vec<Vec<String>> = machines.iter().map(|m| {
        vec![
            format!("{}", m.num),
            m.ip.clone(),
            or_dash(&m.nat_ip),
            format!("{}", m.port),
            m.username.clone(),
            or_dash(&m.password),
            or_dash(&m.private_key_path),
            or_dash(&m.device),
            or_dash(&m.remark),
        ]
    }).collect();

    let widths: Vec<usize> = col_meta.iter().enumerate().map(|(ci, (name, _))| {
        let tw = UnicodeWidthStr::width(*name);
        let dw = rows.iter().filter_map(|r| r.get(ci))
            .map(|v| UnicodeWidthStr::width(v.as_str())).max().unwrap_or(0);
        tw.max(dw).max(3)
    }).collect();

    let header: String = col_meta.iter().zip(&widths)
        .map(|((name, left), w)| pad_str(name, *w, *left))
        .collect::<Vec<_>>()
        .join("  ");
    println!("{}", header);
    println!("{}", "-".repeat(header.len()));

    for row in &rows {
        let line: String = row.iter().zip(col_meta.iter().zip(&widths))
            .map(|(val, ((_, left), w))| pad_str(val, *w, *left))
            .collect::<Vec<_>>()
            .join("  ");
        println!("{}", line);
    }

    println!("{}", "-".repeat(header.len()));
    let total_text = format!("Total: {} machines", machines.len());
    let total_width = UnicodeWidthStr::width(total_text.as_str());
    let padding = header.len().saturating_sub(total_width);
    println!("{}{}", " ".repeat(padding), total_text);
}

fn default_output_path(filename: &str) -> PathBuf {
    let bin_dir = std::env::current_exe().ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."));
    bin_dir.join(filename)
}

fn main() {
    if let Err(e) = run() {
        eprintln!("Error: {e}");
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Version) => {
            println!("minishell {}", env!("CARGO_PKG_VERSION"));
            println!("git: {}", option_env!("GIT_SHA").unwrap_or("unknown"));
            println!("built: {}", option_env!("BUILD_TIME").unwrap_or("unknown"));
        }
        Some(Commands::Tpl { path }) => {
            let path = path.map(PathBuf::from).unwrap_or_else(|| default_output_path("machines-template.xlsx"));
            minishell_xlsx::generate_template(&path)?;
            println!("Template generated: {}", path.display());
        }
        Some(Commands::Import { path }) => {
            let store = open_db()?;
            let mut machines = minishell_xlsx::import_from(PathBuf::from(&path).as_path())?;
            let mut next = store.max_num()? + 1;
            for m in &mut machines {
                m.num = next;
                next += 1;
            }
            let count = store.import_machines(&machines)?;
            println!("Imported {} machines ({} skipped)", count, machines.len() - count);
        }
        Some(Commands::Export { path }) => {
            let store = open_db()?;
            let machines = store.search("")?;
            let path = path.map(PathBuf::from).unwrap_or_else(|| default_output_path("machines-export.xlsx"));
            minishell_xlsx::export_to(&path, &machines)?;
            println!("Exported {} machines to {}", machines.len(), path.display());
        }
        Some(Commands::Show) => {
            let store = open_db()?;
            let machines = store.search("")?;
            print_machines(&machines);
        }
        Some(Commands::Resetnum) => {
            let store = open_db()?;
            let count = store.reset_num()?;
            println!("Reset num for {} machines", count);
        }
        None => {
            let store = open_db()?;

            if let Some(ref query) = cli.query {
                if let Ok(num) = query.parse::<i32>() {
                    let machines = store.search("")?;
                    if let Some(m) = machines.iter().find(|m| m.num == num) {
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
