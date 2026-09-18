pub mod card;
pub mod probe;
pub mod sftp;

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::os::unix::io::{AsRawFd, RawFd};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use anyhow::{Result, Context};
use minishell_core::Machine;

static RESIZE_FLAG: AtomicBool = AtomicBool::new(false);

extern "C" fn handle_sigwinch(_: i32) {
    RESIZE_FLAG.store(true, Ordering::SeqCst);
}

// ── Session liveness ────────────────────────────────────────────────────────
//
// A session whose path is silently black-holed (laptop sleep/wake, NAT or a
// router dropping the flow without FIN/RST) never surfaces POLLHUP/POLLERR on
// its own: the kernel retransmits unacknowledged data for ~15 minutes
// (tcp_retries2) and default SO_KEEPALIVE does not probe until 2 hours idle.
// The TUI then freezes with no prompt, and keystrokes vanish into a socket
// that accepts them locally but never delivers them.
//
// libssh2 cannot detect this either: `libssh2_keepalive_send` silently swallows
// EAGAIN and returns 0 whether or not anything was written (verified in
// libssh2's keepalive.c), so its return value carries no liveness signal.
//
// The kernel therefore owns the detection. `TCP_USER_TIMEOUT` bounds how long
// sent-but-unacknowledged data may sit before the socket is errored, and the
// keepalive timers bound how long an idle connection may look alive without a
// peer response. Together they turn an indefinite freeze into a `POLLERR` plus
// reconnect prompt in well under a minute.
pub const SOCK_KEEPALIVE_IDLE_SECS: u32 = 30;
pub const SOCK_KEEPALIVE_INTVL_SECS: u32 = 10;
pub const SOCK_KEEPALIVE_CNT: u32 = 3;
pub const SOCK_USER_TIMEOUT_MS: u32 = 20_000;

/// Application-level SSH keepalive interval. Keeps NAT/conntrack entries warm
/// and guarantees in-flight data for `TCP_USER_TIMEOUT` to time out against,
/// even on an otherwise idle shell.
pub const SSH_KEEPALIVE_INTERVAL: Duration = Duration::from_secs(30);

/// Upper bound on keystrokes buffered while the channel reports `WouldBlock`.
/// Generous enough that a large paste on a momentarily busy link is never
/// mistaken for a dead session, yet bounded so a black-holed socket cannot
/// grow the buffer without limit. Reaching it means the session is not
/// absorbing input at all — treat as dead.
const MAX_PENDING_INPUT: usize = 1024 * 1024;

fn setsockopt_int(fd: RawFd, level: libc::c_int, name: libc::c_int, value: libc::c_int) {
    let _ = unsafe {
        libc::setsockopt(
            fd,
            level,
            name,
            &value as *const libc::c_int as *const libc::c_void,
            std::mem::size_of::<libc::c_int>() as libc::socklen_t,
        )
    };
}

/// Arms the kernel-level dead-peer detection described above.
///
/// `TCP_USER_TIMEOUT` and the keepalive tunables are Linux/Android socket
/// options; elsewhere only `SO_KEEPALIVE` is enabled, with system defaults.
pub fn configure_liveness(tcp: &TcpStream) {
    let fd = tcp.as_raw_fd();
    setsockopt_int(fd, libc::SOL_SOCKET, libc::SO_KEEPALIVE, 1);

    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        setsockopt_int(
            fd,
            libc::IPPROTO_TCP,
            libc::TCP_USER_TIMEOUT,
            SOCK_USER_TIMEOUT_MS as libc::c_int,
        );
        setsockopt_int(
            fd,
            libc::IPPROTO_TCP,
            libc::TCP_KEEPIDLE,
            SOCK_KEEPALIVE_IDLE_SECS as libc::c_int,
        );
        setsockopt_int(
            fd,
            libc::IPPROTO_TCP,
            libc::TCP_KEEPINTVL,
            SOCK_KEEPALIVE_INTVL_SECS as libc::c_int,
        );
        setsockopt_int(
            fd,
            libc::IPPROTO_TCP,
            libc::TCP_KEEPCNT,
            SOCK_KEEPALIVE_CNT as libc::c_int,
        );
    }
}

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

/// Drains every byte currently readable from `reader`, forwarding it to `out`.
///
/// This is the load-bearing correctness seam for the SSH read loop. It encodes
/// the three ordering rules that prevent data loss with libssh2's internal
/// buffering:
///   1. Always drain — not gated on the raw-fd `poll()` reporting `POLLIN`,
///      because libssh2 may have already consumed the kernel buffer.
///   2. Only stop at `Ok(0)` (true EOF) — never at a transient EOF flag while
///      bytes are still buffered.
///   3. Return `Ok(false)` only when the reader reports `WouldBlock`, so the
///      caller can check for hangup *after* draining.
///
/// Returns `true` when the source hit EOF, `false` after a `WouldBlock`.
pub fn drain_reads(reader: &mut impl Read, out: &mut impl Write) -> std::io::Result<bool> {
    let mut buf = [0u8; 4096];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => return Ok(true),
            Ok(n) => {
                out.write_all(&buf[..n])?;
                out.flush()?;
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => return Ok(false),
            Err(e) => return Err(e),
        }
    }
}

/// Writes as much of `pending` into `channel` as the channel will take,
/// retaining everything else for the next call.
///
/// `Write::write_all` cannot be used here: the session is non-blocking, so when
/// libssh2's send buffer fills, `write_all` returns `WouldBlock` after having
/// already written an unknown prefix — which is how `let _ = write_all(..)`
/// silently dropped keystrokes. Tracking the count ourselves keeps every byte.
///
/// A cleared `pending` means fully flushed; `WouldBlock` returns early with the
/// remainder preserved.
///
/// No `Channel::flush` call accompanies this: in ssh2-rs it maps to
/// `libssh2_channel_flush_ex`, which discards *received* data (banner/prompt),
/// not outgoing data. Nothing in this module calls it.
pub fn flush_pending(channel: &mut impl Write, pending: &mut Vec<u8>) -> std::io::Result<()> {
    let mut written = 0usize;
    while written < pending.len() {
        match channel.write(&pending[written..]) {
            Ok(0) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::WriteZero,
                    "SSH channel closed while writing",
                ))
            }
            Ok(n) => written += n,
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
            Err(e) => return Err(e),
        }
    }
    if written > 0 {
        pending.drain(..written);
    }
    Ok(())
}

fn run_session_loop(channel: &mut ssh2::Channel, session: &ssh2::Session, session_fd: RawFd) -> SessionEnd {
    let stdin_fd = libc::STDIN_FILENO;

    let mut stdin_buf = [0u8; 4096];
    let mut stdout = std::io::stdout();
    let mut stdin = std::io::stdin();
    let mut last_keepalive = Instant::now();
    // Keystrokes the channel could not accept yet (WouldBlock). Held across
    // loop iterations so a busy send buffer never drops input.
    let mut pending: Vec<u8> = Vec::new();

    loop {
        // libssh2's keepalive has no reply watchdog (see the liveness notes at
        // the top of this file), so this is only a NAT/keepalive nudge. It does
        // report a real transport error though — the old `let _ = ...` also
        // swallowed that.
        if last_keepalive.elapsed() >= SSH_KEEPALIVE_INTERVAL {
            match session.keepalive_send() {
                Ok(_) => last_keepalive = Instant::now(),
                Err(_) => return SessionEnd::Disconnected,
            }
        }

        // Handle terminal resize
        if RESIZE_FLAG.swap(false, Ordering::SeqCst) {
            if let Ok((cols, rows)) = crossterm::terminal::size() {
                let _ = channel.request_pty_size(cols as u32, rows as u32, None, None);
            }
        }

        let mut pollfds = [
            libc::pollfd { fd: stdin_fd, events: libc::POLLIN, revents: 0 },
            libc::pollfd { fd: session_fd, events: libc::POLLIN, revents: 0 },
        ];

        let ret = unsafe { libc::poll(pollfds.as_mut_ptr(), pollfds.len() as libc::nfds_t, 100) };
        if ret < 0 {
            let err = std::io::Error::last_os_error();
            if err.kind() == std::io::ErrorKind::Interrupted {
                // poll was interrupted (e.g. by SIGWINCH) — re-enter loop to handle resize
                continue;
            }
            return SessionEnd::Disconnected;
        }

        // Always drain SSH data — not gated on POLLIN.
        // libssh2 may have data buffered internally even when poll()
        // on the raw socket fd returns no POLLIN (the kernel buffer
        // was already consumed by libssh2's transport layer).
        match drain_reads(channel, &mut stdout) {
            Ok(true) => return SessionEnd::Normal,
            Ok(false) => {}
            Err(_) => return SessionEnd::Disconnected,
        }

        // Check disconnection AFTER draining any buffered data
        if pollfds[1].revents & (libc::POLLHUP | libc::POLLERR) != 0 {
            return SessionEnd::Disconnected;
        }

        let stdin_events = pollfds[0].revents;
        if stdin_events & (libc::POLLHUP | libc::POLLERR) != 0 {
            return SessionEnd::Normal;
        }
        if stdin_events & libc::POLLIN != 0 {
            match stdin.read(&mut stdin_buf) {
                Ok(0) => return SessionEnd::Normal,
                Ok(n) => {
                    if pending.len() + n > MAX_PENDING_INPUT {
                        // The channel is not absorbing input at all; a live
                        // session would have drained these bytes long ago.
                        return SessionEnd::Disconnected;
                    }
                    pending.extend_from_slice(&stdin_buf[..n]);
                }
                Err(_) => return SessionEnd::Normal,
            }
        }

        // Flush every iteration, not only after stdin: bytes buffered on a
        // previous pass still need to reach the channel once it can take them.
        if flush_pending(channel, &mut pending).is_err() {
            return SessionEnd::Disconnected;
        }
    }
}

pub fn connect(config: &ConnectConfig) -> Result<()> {
    let term = std::env::var("TERM").unwrap_or_else(|_| "xterm-256color".to_string());

    // Install SIGWINCH handler for terminal resize notifications
    unsafe {
        libc::signal(libc::SIGWINCH, handle_sigwinch as *const () as libc::sighandler_t);
    }

    // Phase 1: Establish SSH session (raw mode OFF — Ctrl+C generates SIGINT)
    let (mut channel, session, session_fd) = prepare_session(config, &term)?;

    // Phase 2: Enable raw mode for PTY interaction
    let _ = crossterm::terminal::enable_raw_mode();
    let _ = crossterm::execute!(std::io::stdout(), crossterm::cursor::Show);

    let mut outcome = run_session_loop(&mut channel, &session, session_fd);

    // Reconnection loop — only retry if session was established and then lost
    let max_retries = 3;
    let mut retry = 0u32;
    let mut last_error: Option<String> = None;

    loop {
        match outcome {
            SessionEnd::Normal => break,
            SessionEnd::Disconnected => {
                retry += 1;
                if retry > max_retries {
                    let _ = crossterm::terminal::disable_raw_mode();
                    match last_error {
                        Some(e) => anyhow::bail!(
                            "Connection lost. {} reconnection attempts failed (last error: {}).",
                            max_retries,
                            e
                        ),
                        None => anyhow::bail!(
                            "Connection lost. All {} reconnection attempts failed.",
                            max_retries
                        ),
                    }
                }
                let msg = format!(
                    "\r\n\x1b[2mConnection lost. Press any key to reconnect... (attempt {}/{})\x1b[0m\r\n",
                    retry, max_retries
                );
                let _ = std::io::stdout().write_all(msg.as_bytes());
                let _ = std::io::stdout().flush();

                match crossterm::event::read() {
                    Ok(crossterm::event::Event::Key(key))
                        if key.code == crossterm::event::KeyCode::Char('c')
                            && key.modifiers == crossterm::event::KeyModifiers::CONTROL =>
                    {
                        let _ = crossterm::terminal::disable_raw_mode();
                        anyhow::bail!("Connection cancelled.");
                    }
                    _ => {}
                }

                outcome = match prepare_session(config, &term) {
                    Ok((mut ch, sess, fd)) => {
                        last_error = None;
                        run_session_loop(&mut ch, &sess, fd)
                    }
                    Err(e) => {
                        // Do NOT abort here. Right after a laptop wakes, the
                        // route is often not up yet, so the first reconnect
                        // attempt can fail even though a retry a moment later
                        // succeeds. Report it and fall through to the next
                        // prompt instead of dropping the user back to the shell.
                        let msg = format!("\r\n\x1b[31mReconnect failed: {e}\x1b[0m\r\n");
                        let _ = std::io::stdout().write_all(msg.as_bytes());
                        let _ = std::io::stdout().flush();
                        last_error = Some(e.to_string());
                        SessionEnd::Disconnected
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
    let addrs: Vec<std::net::SocketAddr> = match addr.parse() {
        Ok(addr) => vec![addr],
        Err(_) => addr
            .to_socket_addrs()
            .context("Failed to resolve hostname")?
            .collect(),
    };

    if addrs.is_empty() {
        anyhow::bail!("No addresses resolved for {}", config.host);
    }

    let mut last_err = anyhow::anyhow!("Failed to connect");
    for parsed_addr in &addrs {
        match TcpStream::connect_timeout(parsed_addr, config.timeout) {
            Ok(tcp) => {
                configure_liveness(&tcp);
                let mut session = ssh2::Session::new().context("Failed to create SSH session")?;
                session.set_tcp_stream(tcp);
                session.set_timeout(config.timeout.as_millis() as u32);
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

                return Ok(session);
            }
            Err(e) => {
                last_err = anyhow::anyhow!("Failed to connect to {}: {}", parsed_addr, e);
                continue;
            }
        }
    }

    Err(last_err)
}

fn prepare_session(
    config: &ConnectConfig,
    term: &str,
) -> Result<(ssh2::Channel, ssh2::Session, RawFd)> {
    let session = create_session(config)?;
    let session_fd = session.as_raw_fd();

    let mut channel = session.channel_session().context("Failed to open channel")?;

    let (cols, rows) = crossterm::terminal::size().unwrap_or((80, 24));
    channel.request_pty(term, None, Some((cols as u32, rows as u32, 0, 0)))?;
    channel.shell().context("Failed to start shell")?;

    if config.device == "Linux" {
        // The session is still blocking here, so write_all cannot return
        // WouldBlock; no flush is needed (and Channel::flush would discard
        // received data — see flush_pending).
        let _ = channel.write_all(
            format!("export PS1=\"[{}] $PS1\"\n", config.host).as_bytes(),
        );
    }

    session.set_blocking(false);
    session.set_keepalive(true, SSH_KEEPALIVE_INTERVAL.as_secs() as u32);

    Ok((channel, session, session_fd))
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

    println!("{}", card::disconnect_card(host, duration, None, max_width));
    let _ = std::io::stdout().flush();

    Ok(duration)
}
