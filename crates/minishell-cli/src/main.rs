use std::path::{Path, PathBuf};
use std::sync::Arc;
use anyhow::Result;
use clap::{Parser, Subcommand};
use minishell_core::Machine;
use minishell_store::Store;
use minishell_ssh::sftp;
use minishell_ssh::sftp::Sftp;
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

    /// Upload files to remote machine (SCP-style)
    Push {
        /// Target machine (IP/remark/ID)
        query: String,
        /// Local source path
        local: String,
        /// Remote destination path
        remote: String,
        /// Recursive directory transfer
        #[arg(short)]
        recursive: bool,
    },

    /// Download files from remote machine (SCP-style)
    Pull {
        /// Target machine (IP/remark/ID)
        query: String,
        /// Remote source path
        remote: String,
        /// Local destination path
        local: String,
        /// Recursive directory transfer
        #[arg(short)]
        recursive: bool,
    },
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

#[derive(Clone, PartialEq)]
enum PathType { File, Dir, NotFound }

fn check_remote_type(sftp: &Sftp, path: &str) -> PathType {
    match sftp.stat(Path::new(path)) {
        Ok(stat) => {
            if stat.is_dir() { PathType::Dir } else { PathType::File }
        }
        Err(_) => PathType::NotFound,
    }
}

fn check_local_type(path: &Path) -> PathType {
    if path.is_dir() { PathType::Dir }
    else if path.exists() { PathType::File }
    else { PathType::NotFound }
}

fn print_progress(p: &sftp::TransferProgress, done: bool) {
    let pct = if p.total_bytes > 0 {
        p.bytes_written * 100 / p.total_bytes
    } else { 100 };
    let size_str = sftp::format_size(p.bytes_written);
    let total_str = sftp::format_size(p.total_bytes);
    if done {
        println!("  {}  100%  {}  {}", p.file_name, total_str, size_str);
    } else if pct > 0 {
        print!("\r  {}  {}%  {}/{}", p.file_name, pct, size_str, total_str);
    }
}

fn resolve_machine(store: &Store, query: &str) -> Result<Vec<Machine>> {
    if let Ok(num) = query.parse::<i32>() {
        let machines = store.search("")?;
        if let Some(m) = machines.iter().find(|m| m.num == num) {
            return Ok(vec![m.clone()]);
        }
    }
    let machines = store.search(query)?;
    if machines.is_empty() {
        anyhow::bail!("No machines found matching '{}'", query);
    }
    Ok(machines)
}

fn build_config(machine: &Machine) -> minishell_ssh::ConnectConfig {
    minishell_ssh::ConnectConfig {
        username: machine.username.clone(),
        password: if machine.password == "-" { String::new() } else { machine.password.clone() },
        private_key_path: if machine.private_key_path == "-" { String::new() } else { machine.private_key_path.clone() },
        host: machine.effective_host().to_string(),
        port: machine.port,
        timeout: std::time::Duration::from_secs(10),
        device: machine.device.clone(),
    }
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
        Some(Commands::Push { query, local, remote, recursive }) => {
            let store = open_db()?;
            let machines = resolve_machine(&store, &query)?;
            let machine = if machines.len() == 1 {
                machines.into_iter().next().unwrap()
            } else {
                let m = minishell_tui::select_machine(machines)?.ok_or_else(|| anyhow::anyhow!("No machine selected"))?;
                println!();
                m
            };

            let config = build_config(&machine);
            let session = minishell_ssh::create_session(&config)?;
            let ssh_sftp = session.sftp()?;

            let local_path = Path::new(&local);
            let local_type = check_local_type(local_path);
            let remote_type = check_remote_type(&ssh_sftp, &remote);
            let remote_ends_with_slash = remote.ends_with('/');

            let (actual_local, actual_remote) = match (local_type.clone(), remote_type.clone(), recursive, remote_ends_with_slash) {
                (PathType::File, PathType::Dir, _, _) => {
                    let fname = local_path.file_name().unwrap().to_string_lossy().to_string();
                    (local_path.to_path_buf(), format!("{}/{}", remote.trim_end_matches('/'), fname))
                }
                (PathType::File, PathType::NotFound, _, true) => {
                    let fname = local_path.file_name().unwrap().to_string_lossy().to_string();
                    (local_path.to_path_buf(), format!("{}/{}", remote.trim_end_matches('/'), fname))
                }
                (PathType::File, _, _, _) => {
                    (local_path.to_path_buf(), remote.trim_end_matches('/').to_string())
                }
                (PathType::Dir, PathType::File, _, _) => {
                    anyhow::bail!("Cannot overwrite remote file '{}' with local directory '{}'", remote, local);
                }
                (PathType::Dir, _, true, true) => {
                    (local_path.to_path_buf(), remote.trim_end_matches('/').to_string())
                }
                (PathType::Dir, PathType::Dir, true, _) => {
                    let dirname = local_path.file_name().unwrap().to_string_lossy().to_string();
                    (local_path.to_path_buf(), format!("{}/{}", remote.trim_end_matches('/'), dirname))
                }
                (PathType::Dir, PathType::NotFound, true, _) => {
                    (local_path.to_path_buf(), remote.trim_end_matches('/').to_string())
                }
                (PathType::Dir, _, false, _) => {
                    anyhow::bail!("'{}' is a directory. Use -r to transfer directories.", local);
                }
                (PathType::NotFound, _, _, _) => {
                    anyhow::bail!("Local path '{}' not found", local);
                }
            };

            if local_type == PathType::File {
                let total = std::fs::metadata(&actual_local)?.len();
                let fname = actual_local.file_name().unwrap().to_string_lossy().to_string();
                let cb = |written: u64, total: u64| {
                    let p = sftp::TransferProgress {
                        file_name: fname.clone(),
                        bytes_written: written,
                        total_bytes: total,
                        file_index: 1,
                        total_files: 1,
                    };
                    print_progress(&p, false);
                };
                print_progress(&sftp::TransferProgress {
                    file_name: fname.clone(), bytes_written: 0, total_bytes: total,
                    file_index: 1, total_files: 1,
                }, false);
                sftp::upload_file(&ssh_sftp, &actual_local, &actual_remote, &cb)?;
                let p = sftp::TransferProgress {
                    file_name: fname, bytes_written: total, total_bytes: total,
                    file_index: 1, total_files: 1,
                };
                print_progress(&p, true);
                #[cfg(unix)]
                if let Ok(meta) = std::fs::metadata(&actual_local) {
                    use std::os::unix::fs::PermissionsExt;
                    let _ = sftp::set_perm_remote(&ssh_sftp, &actual_remote, meta.permissions().mode() & 0o777);
                }
                println!("\nTransferred: 1 file, {}", sftp::format_size(total));
            } else {
                let total_count = std::sync::atomic::AtomicUsize::new(0);
                let total_count_ref = &total_count;
                let cb = |p: &sftp::TransferProgress| {
                    total_count_ref.store(p.file_index, std::sync::atomic::Ordering::Relaxed);
                    print_progress(p, false);
                };
                let errors = sftp::upload_recursive(&ssh_sftp, &actual_local, &actual_remote, &cb)?;
                println!();
                let count = total_count.load(std::sync::atomic::Ordering::Relaxed);
                if errors.is_empty() {
                    println!("Transferred: {} files", count);
                } else {
                    let succeeded = count.saturating_sub(errors.len());
                    println!("Transferred: {}/{} files ({} errors)", succeeded, count, errors.len());
                    for e in &errors {
                        println!("  {}", e);
                    }
                }
            }
        }
        Some(Commands::Pull { query, remote, local, recursive }) => {
            let store = open_db()?;
            let machines = resolve_machine(&store, &query)?;
            let machine = if machines.len() == 1 {
                machines.into_iter().next().unwrap()
            } else {
                let m = minishell_tui::select_machine(machines)?.ok_or_else(|| anyhow::anyhow!("No machine selected"))?;
                println!();
                m
            };

            let config = build_config(&machine);
            let session = minishell_ssh::create_session(&config)?;
            let ssh_sftp = session.sftp()?;

            let local_path = Path::new(&local);
            let remote_type = check_remote_type(&ssh_sftp, &remote);
            let local_type = check_local_type(local_path);
            let local_ends_with_slash = local.ends_with('/');

            let (actual_remote, actual_local) = match (remote_type.clone(), local_type.clone(), recursive, local_ends_with_slash) {
                (PathType::File, PathType::Dir, _, _) => {
                    let fname = Path::new(&remote).file_name().unwrap().to_string_lossy().to_string();
                    (remote.clone(), local_path.join(fname))
                }
                (PathType::File, PathType::NotFound, _, true) => {
                    let fname = Path::new(&remote).file_name().unwrap().to_string_lossy().to_string();
                    (remote.clone(), local_path.join(fname))
                }
                (PathType::File, _, _, _) => (remote.clone(), local_path.to_path_buf()),
                (PathType::Dir, PathType::File, _, _) => {
                    anyhow::bail!("Cannot overwrite local file '{}' with remote directory '{}'", local, remote);
                }
                (PathType::Dir, _, true, true) => (remote.clone(), local_path.to_path_buf()),
                (PathType::Dir, PathType::Dir, true, _) => {
                    let dirname = Path::new(&remote).file_name().unwrap().to_string_lossy().to_string();
                    (remote.clone(), local_path.join(dirname))
                }
                (PathType::Dir, PathType::NotFound, true, _) => (remote.clone(), local_path.to_path_buf()),
                (PathType::Dir, _, false, _) => {
                    anyhow::bail!("'{}' is a remote directory. Use -r to transfer directories.", remote);
                }
                (PathType::NotFound, _, _, _) => {
                    anyhow::bail!("Remote path '{}' not found", remote);
                }
            };

            if remote_type == PathType::File {
                let stat = ssh_sftp.stat(Path::new(&actual_remote))?;
                let total = stat.size.unwrap_or(0);
                let fname = actual_local.file_name().unwrap().to_string_lossy().to_string();
                let cb = |written: u64, total: u64| {
                    let p = sftp::TransferProgress {
                        file_name: fname.clone(),
                        bytes_written: written,
                        total_bytes: total,
                        file_index: 1,
                        total_files: 1,
                    };
                    print_progress(&p, false);
                };
                if let Some(parent) = actual_local.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                print_progress(&sftp::TransferProgress {
                    file_name: fname.clone(), bytes_written: 0, total_bytes: total,
                    file_index: 1, total_files: 1,
                }, false);
                sftp::download_file(&ssh_sftp, &actual_remote, &actual_local, &cb)?;
                let p = sftp::TransferProgress {
                    file_name: fname, bytes_written: total, total_bytes: total,
                    file_index: 1, total_files: 1,
                };
                print_progress(&p, true);
                #[cfg(unix)]
                if let Some(perm) = stat.perm {
                    let _ = sftp::set_perm_local(&actual_local, perm);
                }
                println!("\nTransferred: 1 file, {}", sftp::format_size(total));
            } else {
                if let Some(parent) = actual_local.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                let total_count = std::sync::atomic::AtomicUsize::new(0);
                let total_count_ref = &total_count;
                let cb = |p: &sftp::TransferProgress| {
                    total_count_ref.store(p.file_index, std::sync::atomic::Ordering::Relaxed);
                    print_progress(p, false);
                };
                let errors = sftp::download_recursive(&ssh_sftp, &actual_remote, &actual_local, &cb)?;
                println!();
                let count = total_count.load(std::sync::atomic::Ordering::Relaxed);
                if errors.is_empty() {
                    println!("Transferred: {} files", count);
                } else {
                    let succeeded = count.saturating_sub(errors.len());
                    println!("Transferred: {}/{} files ({} errors)", succeeded, count, errors.len());
                    for e in &errors {
                        println!("  {}", e);
                    }
                }
            }
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
