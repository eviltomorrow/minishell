pub mod card;

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::os::unix::io::{AsRawFd, RawFd};
use std::time::{Duration, Instant};
use anyhow::{Result, Context};
use minishell_core::Machine;

#[derive(Clone)]
pub struct ConnectConfig {
    pub username: String,
    pub password: String,
    pub private_key_path: String,
    pub host: String,
    pub port: i32,
    pub timeout: Duration,
    pub device: String,
}

enum SessionEnd {
    Normal,
    Disconnected,
}

fn run_session_loop(channel: &mut ssh2::Channel, session_fd: RawFd) -> SessionEnd {
    let stdin_fd = libc::STDIN_FILENO;

    let mut stdin_buf = [0u8; 4096];
    let mut ssh_buf = [0u8; 4096];
    let mut stdout = std::io::stdout();
    let mut stdin = std::io::stdin();
    let connect_start = Instant::now();
    let max_duration = Duration::from_secs(3600);

    loop {
        if connect_start.elapsed() > max_duration {
            return SessionEnd::Normal;
        }

        let mut pollfds = [
            libc::pollfd { fd: stdin_fd, events: libc::POLLIN, revents: 0 },
            libc::pollfd { fd: session_fd, events: libc::POLLIN, revents: 0 },
        ];

        let ret = unsafe { libc::poll(pollfds.as_mut_ptr(), pollfds.len() as libc::nfds_t, 100) };
        if ret < 0 {
            let err = std::io::Error::last_os_error();
            if err.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return SessionEnd::Disconnected;
        }

        let remote_events = pollfds[1].revents;
        if remote_events & (libc::POLLHUP | libc::POLLERR) != 0 {
            return SessionEnd::Disconnected;
        }
        if remote_events & libc::POLLIN != 0 {
            match channel.read(&mut ssh_buf) {
                Ok(0) => return SessionEnd::Normal,
                Ok(n) => {
                    let _ = stdout.write_all(&ssh_buf[..n]);
                    let _ = stdout.flush();
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(_) => return SessionEnd::Disconnected,
            }
            if channel.eof() {
                return SessionEnd::Normal;
            }
        }

        let stdin_events = pollfds[0].revents;
        if stdin_events & (libc::POLLHUP | libc::POLLERR) != 0 {
            return SessionEnd::Normal;
        }
        if stdin_events & libc::POLLIN != 0 {
            match stdin.read(&mut stdin_buf) {
                Ok(0) => return SessionEnd::Normal,
                Ok(n) => {
                    let _ = channel.write_all(&stdin_buf[..n]);
                    let _ = channel.flush();
                }
                Err(_) => return SessionEnd::Normal,
            }
        }
    }
}

pub fn connect(config: &ConnectConfig) -> Result<()> {
    let term = std::env::var("TERM").unwrap_or_else(|_| "xterm-256color".to_string());

    let _ = crossterm::terminal::enable_raw_mode();
    let _ = crossterm::execute!(std::io::stdout(), crossterm::cursor::Show);

    // Initial connection — fail fast, no retry
    let outcome = try_session(config, &term);
    let mut outcome = match outcome {
        Ok(o) => o,
        Err(e) => {
            let _ = crossterm::terminal::disable_raw_mode();
            return Err(e);
        }
    };

    // Reconnection loop — only retry if session was established and then lost
    let max_retries = 3;
    let mut retry = 0u32;

    loop {
        match outcome {
            SessionEnd::Normal => break,
            SessionEnd::Disconnected => {
                retry += 1;
                if retry > max_retries {
                    let _ = crossterm::terminal::disable_raw_mode();
                    anyhow::bail!(
                        "Connection lost. All {} reconnection attempts failed.",
                        max_retries
                    );
                }
                let msg = format!(
                    "\r\n\x1b[2mConnection lost. Press any key to reconnect... (attempt {}/{})\x1b[0m\r\n",
                    retry, max_retries
                );
                let _ = std::io::stdout().write_all(msg.as_bytes());
                let _ = std::io::stdout().flush();
                let _ = crossterm::event::read();
                outcome = match try_session(config, &term) {
                    Ok(o) => o,
                    Err(e) => {
                        let _ = crossterm::terminal::disable_raw_mode();
                        return Err(e);
                    }
                };
            }
        }
    }

    let _ = crossterm::terminal::disable_raw_mode();
    Ok(())
}

pub fn create_session(config: &ConnectConfig) -> Result<ssh2::Session> {
    let addr = format!("{}:{}", config.host, config.port);
    let parsed_addr: std::net::SocketAddr = match addr.parse() {
        Ok(addr) => addr,
        Err(_) => addr
            .to_socket_addrs()
            .context("Failed to resolve hostname")?
            .next()
            .ok_or_else(|| anyhow::anyhow!("No addresses found for hostname"))?,
    };

    let tcp = TcpStream::connect_timeout(&parsed_addr, config.timeout)
        .with_context(|| format!("Failed to connect to {}:{}", config.host, config.port))?;

    let mut session = ssh2::Session::new().context("Failed to create SSH session")?;
    session.set_tcp_stream(tcp);
    session.handshake().context("SSH handshake failed")?;

    if !config.private_key_path.is_empty() {
        let key_path = std::path::Path::new(&config.private_key_path);
        session
            .userauth_pubkey_file(&config.username, None, key_path, None)
            .context("Public key auth failed")?;
    } else if !config.password.is_empty() {
        session
            .userauth_password(&config.username, &config.password)
            .context("Password auth failed")?;
    } else {
        session
            .userauth_agent(&config.username)
            .context("Agent auth failed")?;
    }

    if !session.authenticated() {
        anyhow::bail!("Authentication failed");
    }

    Ok(session)
}

fn try_session(
    config: &ConnectConfig,
    term: &str,
) -> Result<SessionEnd> {
    let session = create_session(config)?;
    let session_fd = session.as_raw_fd();

    let mut channel = session.channel_session().context("Failed to open channel")?;

    let (cols, rows) = crossterm::terminal::size().unwrap_or((80, 24));
    channel.request_pty(term, None, Some((cols as u32, rows as u32, 0, 0)))?;
    channel.shell().context("Failed to start shell")?;

    if config.device == "Linux" {
        let _ = channel.write_all(
            format!("export PS1=\"[{}] $PS1\"\n", config.host).as_bytes(),
        );
        let _ = channel.flush();
    }

    session.set_blocking(false);

    Ok(run_session_loop(&mut channel, session_fd))
}

pub fn login_to_machine(machine: &Machine) -> Result<Duration> {
    let host = machine.effective_host();
    let auth_method = if !machine.private_key_path.is_empty() && machine.private_key_path != "-" {
        machine.private_key_path.split('/').last().unwrap_or("key")
    } else if !machine.password.is_empty() && machine.password != "-" {
        "password"
    } else {
        "none"
    };

    let max_width = card::terminal_width();
    let (card_top, card_width) = card::connect_card_top(&machine.ip, host, machine.port, &machine.username, auth_method, max_width);
    println!("{}", card_top);
    println!("{}\n", card::connect_card_status_line("Connecting...", card_width));
    let _ = std::io::stdout().flush();

    let config = ConnectConfig {
        username: machine.username.clone(),
        password: if machine.password == "-" { String::new() } else { machine.password.clone() },
        private_key_path: if machine.private_key_path == "-" { String::new() } else { machine.private_key_path.clone() },
        host: host.to_string(),
        port: machine.port,
        timeout: Duration::from_secs(10),
        device: machine.device.clone(),
    };

    let start = Instant::now();
    let result = connect(&config);
    let duration = start.elapsed();

    print!("\x1b[A\x1b[A\r\x1b[K");
    match &result {
        Ok(()) => println!("{}", card::connect_success_line(duration, card_width)),
        Err(e) => println!("{}", card::connect_fail_line(&e.to_string(), card_width)),
    }

    println!("{}", card::disconnect_card(host, duration, result.err().map(|e| e.to_string()).as_deref(), max_width));
    let _ = std::io::stdout().flush();

    Ok(duration)
}
