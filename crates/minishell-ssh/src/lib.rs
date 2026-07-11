pub mod card;

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::os::unix::io::AsRawFd;
use std::time::{Duration, Instant};
use anyhow::{Result, Context};
use minishell_core::Machine;

pub struct ConnectConfig {
    pub username: String,
    pub password: String,
    pub private_key_path: String,
    pub host: String,
    pub port: i32,
    pub timeout: Duration,
}

pub fn connect(config: &ConnectConfig) -> Result<()> {
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
        .with_context(|| format!("Failed to connect to {}", addr))?;

    // Save the raw fd before moving tcp into session
    let session_fd = tcp.as_raw_fd();

    let mut session = ssh2::Session::new().context("Failed to create SSH session")?;
    session.set_tcp_stream(tcp);
    session.handshake().context("SSH handshake failed")?;

    if !config.private_key_path.is_empty() {
        let key_path = std::path::Path::new(&config.private_key_path);
        session.userauth_pubkey_file(&config.username, None, key_path, None)
            .context("Public key auth failed")?;
    } else if !config.password.is_empty() {
        session.userauth_password(&config.username, &config.password)
            .context("Password auth failed")?;
    } else {
        session.userauth_agent(&config.username)
            .context("Agent auth failed")?;
    }

    if !session.authenticated() {
        anyhow::bail!("Authentication failed");
    }

    let mut channel = session.channel_session().context("Failed to open channel")?;

    let (cols, rows) = crossterm::terminal::size().unwrap_or((80, 24));
    let term = std::env::var("TERM").unwrap_or_else(|_| "xterm-256color".to_string());
    channel.request_pty(&term, None, Some((cols as u32, rows as u32, 0, 0)))?;
    channel.shell().context("Failed to start shell")?;

    let _ = crossterm::terminal::enable_raw_mode();

    // Set session to non-blocking so channel.read() returns WouldBlock
    session.set_blocking(false);

    let stdin_fd = libc::STDIN_FILENO;

    let mut stdin_buf = [0u8; 4096];
    let mut ssh_buf = [0u8; 4096];
    let mut stdout = std::io::stdout();
    let mut stdin = std::io::stdin();
    let connect_start = Instant::now();
    let max_duration = Duration::from_secs(3600); // 1 hour max session

    loop {
        // Safety timeout
        if connect_start.elapsed() > max_duration {
            break;
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
            break;
        }

        // Check if remote closed connection (POLLHUP) or has data (POLLIN)
        let remote_events = pollfds[1].revents;
        if remote_events & (libc::POLLHUP | libc::POLLERR) != 0 {
            break;
        }
        if remote_events & libc::POLLIN != 0 {
            match channel.read(&mut ssh_buf) {
                Ok(0) => break, // EOF
                Ok(n) => {
                    let _ = stdout.write_all(&ssh_buf[..n]);
                    let _ = stdout.flush();
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(_) => break,
            }
            // Also check if channel received EOF
            if channel.eof() {
                break;
            }
        }

        // Check if local stdin closed or has data
        let stdin_events = pollfds[0].revents;
        if stdin_events & (libc::POLLHUP | libc::POLLERR) != 0 {
            break;
        }
        if stdin_events & libc::POLLIN != 0 {
            match stdin.read(&mut stdin_buf) {
                Ok(0) => break,
                Ok(n) => {
                    let _ = channel.write_all(&stdin_buf[..n]);
                    let _ = channel.flush();
                }
                Err(_) => break,
            }
        }
    }

    let _ = crossterm::terminal::disable_raw_mode();
    Ok(())
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

    print!("\x1b]0;{}\x07", host);
    let _ = std::io::stdout().flush();

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

    print!("\x1b]0;minishell\x07");
    let _ = std::io::stdout().flush();

    Ok(duration)
}
