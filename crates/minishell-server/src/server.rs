use russh::server::{self, Auth, Session, Msg, ChannelOpenHandle};
use russh::{Channel, ChannelId, Pty};
use crate::config::ServerConfig;
use crate::shell::PtySession;
use crate::sftp::SftpHandler;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::mpsc;

pub struct MinishellServer {
    config: Arc<ServerConfig>,
}

impl MinishellServer {
    pub fn new(config: Arc<ServerConfig>) -> Self {
        Self { config }
    }
}

impl server::Server for MinishellServer {
    type Handler = ClientHandler;

    fn new_client(&mut self, peer_addr: Option<std::net::SocketAddr>) -> Self::Handler {
        tracing::info!("New connection from {:?}", peer_addr);
        ClientHandler {
            config: self.config.clone(),
            pty_session: None,
            session_channel: None,
            channel_id: None,
            authenticated: false,
            username: String::new(),
            cols: 80,
            rows: 24,
            term: "xterm-256color".to_string(),
        }
    }
}

pub struct ClientHandler {
    config: Arc<ServerConfig>,
    pty_session: Option<PtySession>,
    session_channel: Option<Channel<Msg>>,
    channel_id: Option<ChannelId>,
    authenticated: bool,
    username: String,
    cols: u16,
    rows: u16,
    term: String,
}

impl server::Handler for ClientHandler {
    type Error = anyhow::Error;

    async fn auth_password(&mut self, user: &str, password: &str) -> Result<Auth, Self::Error> {
        if let Some(user_config) = self.config.find_user(user) {
            if let Some(ref expected_password) = user_config.password {
                use subtle::ConstantTimeEq;
                let expected_bytes = expected_password.as_bytes();
                let provided_bytes = password.as_bytes();
                if expected_bytes.len() == provided_bytes.len()
                    && expected_bytes.ct_eq(provided_bytes).into()
                {
                    self.authenticated = true;
                    self.username = user.to_string();
                    tracing::info!("User '{}' authenticated via password", user);
                    return Ok(Auth::Accept);
                }
            }
        }
        tracing::warn!("Failed password attempt for user '{}'", user);
        Ok(Auth::reject())
    }

    async fn auth_publickey_offered(&mut self, user: &str, public_key: &russh::keys::PublicKey) -> Result<Auth, Self::Error> {
        if self.check_authorized_key(user, public_key) {
            return Ok(Auth::Accept);
        }
        Ok(Auth::reject())
    }

    async fn auth_publickey(&mut self, user: &str, public_key: &russh::keys::PublicKey) -> Result<Auth, Self::Error> {
        if self.check_authorized_key(user, public_key) {
            self.authenticated = true;
            self.username = user.to_string();
            tracing::info!("User '{}' authenticated via public key", user);
            return Ok(Auth::Accept);
        }
        Ok(Auth::reject())
    }

    async fn channel_open_session(&mut self, channel: Channel<Msg>, reply: ChannelOpenHandle, _session: &mut Session) -> Result<(), Self::Error> {
        // Only support one session channel per connection
        if self.channel_id.is_some() {
            tracing::warn!("Rejecting second session channel");
            let _ = reply.reject(russh::ChannelOpenFailure::AdministrativelyProhibited).await;
            return Ok(());
        }
        let _ = reply.accept().await;
        let id = channel.id();
        self.session_channel = Some(channel);
        self.channel_id = Some(id);
        Ok(())
    }

    async fn pty_request(
        &mut self,
        channel: ChannelId,
        term: &str,
        col_width: u32,
        row_height: u32,
        _pix_width: u32,
        _pix_height: u32,
        _modes: &[(Pty, u32)],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.term = term.to_string();
        self.cols = col_width as u16;
        self.rows = row_height as u16;
        let _ = session.channel_success(channel);
        Ok(())
    }

    async fn shell_request(&mut self, channel: ChannelId, session: &mut Session) -> Result<(), Self::Error> {
        if !self.authenticated {
            let _ = session.channel_failure(channel);
            return Ok(());
        }

        match PtySession::spawn(&self.username, &self.term, self.cols, self.rows) {
            Ok(pty) => {
                let master_fd = pty.master_fd;
                self.pty_session = Some(pty);

                // Get a Handle for sending data from async task
                let handle = session.handle();

                // Channel: blocking PTY reader → async sender
                let (tx, mut rx) = mpsc::channel::<Vec<u8>>(1024);

                // Task 1: Blocking PTY reader (runs on blocking thread pool)
                tokio::task::spawn_blocking(move || {
                    tracing::debug!("PTY reader started for fd={}", master_fd);
                    let mut buf = [0u8; 4096];
                    loop {
                        let mut pollfds = [libc::pollfd {
                            fd: master_fd,
                            events: libc::POLLIN,
                            revents: 0,
                        }];
                        let ret = unsafe { libc::poll(pollfds.as_mut_ptr(), 1, 100) };
                        if ret < 0 {
                            tracing::debug!("PTY poll error");
                            break;
                        }
                        if ret > 0 && pollfds[0].revents & libc::POLLIN != 0 {
                            let n = unsafe {
                                libc::read(
                                    master_fd,
                                    buf.as_mut_ptr() as *mut libc::c_void,
                                    buf.len(),
                                )
                            };
                            if n <= 0 {
                                tracing::debug!("PTY read EOF");
                                break;
                            }
                            tracing::debug!("PTY read {} bytes", n);
                            if tx.blocking_send(buf[..n as usize].to_vec()).is_err() {
                                tracing::debug!("PTY channel closed");
                                break;
                            }
                        }
                        if pollfds[0].revents & (libc::POLLHUP | libc::POLLERR) != 0 {
                            tracing::debug!("PTY HUP/ERR");
                            break;
                        }
                    }
                });

                // Task 2: Async sender (runs on tokio runtime)
                tokio::spawn(async move {
                    tracing::debug!("PTY async sender started for channel {:?}", channel);
                    while let Some(data) = rx.recv().await {
                        tracing::debug!("Sending {} bytes to SSH channel", data.len());
                        if handle.data(channel, data).await.is_err() {
                            break;
                        }
                    }
                    tracing::debug!("Shell exited, closing channel {:?}", channel);
                    let _ = handle.eof(channel).await;
                    let _ = handle.close(channel).await;
                });

                let _ = session.channel_success(channel);
            }
            Err(e) => {
                tracing::error!("Failed to spawn shell: {}", e);
                let _ = session.channel_failure(channel);
            }
        }
        Ok(())
    }

    async fn exec_request(&mut self, channel: ChannelId, _data: &[u8], session: &mut Session) -> Result<(), Self::Error> {
        tracing::debug!("exec_request rejected (not supported)");
        let _ = session.channel_failure(channel);
        Ok(())
    }

    async fn subsystem_request(&mut self, channel_id: ChannelId, name: &str, session: &mut Session) -> Result<(), Self::Error> {
        if name == "sftp" && self.authenticated {
            if let Some(channel) = self.session_channel.take() {
                let _ = session.channel_success(channel_id);
                let stream = channel.into_stream();
                let handler = SftpHandler::new(PathBuf::from("/"));
                tokio::spawn(async move {
                    russh_sftp::server::run(stream, handler).await;
                    tracing::debug!("SFTP session ended");
                });
            }
        } else {
            let _ = session.channel_failure(channel_id);
        }
        Ok(())
    }

    async fn window_change_request(
        &mut self,
        channel: ChannelId,
        col_width: u32,
        row_height: u32,
        _pix_width: u32,
        _pix_height: u32,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.cols = col_width as u16;
        self.rows = row_height as u16;
        if let Some(ref pty) = self.pty_session {
            pty.resize(self.cols, self.rows);
        }
        let _ = session.channel_success(channel);
        Ok(())
    }

    async fn data(&mut self, _channel: ChannelId, data: &[u8], _session: &mut Session) -> Result<(), Self::Error> {
        if let Some(ref pty) = self.pty_session {
            let fd = pty.master_fd;
            let data = data.to_vec();
            tokio::task::spawn_blocking(move || {
                let mut written = 0;
                while written < data.len() {
                    let n = unsafe {
                        libc::write(fd, data[written..].as_ptr() as *const libc::c_void, data.len() - written)
                    };
                    if n < 0 {
                        let err = std::io::Error::last_os_error();
                        if err.kind() == std::io::ErrorKind::Interrupted {
                            continue;
                        }
                        break;
                    }
                    written += n as usize;
                }
            }).await.ok();
        }
        Ok(())
    }

    async fn channel_close(&mut self, channel: ChannelId, _session: &mut Session) -> Result<(), Self::Error> {
        if Some(channel) == self.channel_id {
            self.pty_session = None;
            self.channel_id = None;
        }
        Ok(())
    }

    async fn channel_eof(&mut self, channel: ChannelId, _session: &mut Session) -> Result<(), Self::Error> {
        if Some(channel) == self.channel_id {
            self.pty_session = None;
            self.channel_id = None;
        }
        Ok(())
    }
}

impl ClientHandler {
    fn check_authorized_key(&self, user: &str, public_key: &russh::keys::PublicKey) -> bool {
        if let Some(user_config) = self.config.find_user(user) {
            if let Some(ref keys_path) = user_config.authorized_keys {
                let keys_path = crate::config::expand_tilde(keys_path);
                if let Ok(keys_content) = std::fs::read_to_string(&keys_path) {
                    for line in keys_content.lines() {
                        let line = line.trim();
                        if line.is_empty() || line.starts_with('#') { continue; }
                        // authorized_keys format: "ssh-ed25519 AAAAC3NzaC... comment"
                        // parse_public_key_base64 expects only the base64 part
                        let parts: Vec<&str> = line.split_whitespace().collect();
                        let key_str = if parts.len() >= 2 { parts[1] } else { parts[0] };
                        if let Ok(authorized_key) = russh::keys::parse_public_key_base64(key_str) {
                            if authorized_key == *public_key {
                                return true;
                            }
                        }
                    }
                    tracing::warn!("Public key for user '{}' not found in authorized_keys", user);
                } else {
                    tracing::warn!("Could not read authorized_keys file '{}'", keys_path.display());
                }
            }
        }
        false
    }
}
