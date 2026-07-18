use russh::server::{self, Auth, Session, Msg, ChannelOpenHandle};
use russh::{Channel, ChannelId, Pty};
use crate::config::ServerConfig;
use crate::shell::PtySession;
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
            pty_rx: None,
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
    pty_rx: Option<mpsc::Receiver<Vec<u8>>>,
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
        let _ = reply.accept().await;
        self.channel_id = Some(channel.id());
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

                // Spawn background task to read PTY output
                let (tx, rx) = mpsc::channel::<Vec<u8>>(256);
                self.pty_rx = Some(rx);

                tokio::task::spawn_blocking(move || {
                    let mut buf = [0u8; 4096];
                    loop {
                        let mut pollfds = [libc::pollfd {
                            fd: master_fd,
                            events: libc::POLLIN,
                            revents: 0,
                        }];
                        let ret = unsafe { libc::poll(pollfds.as_mut_ptr(), 1, 100) };
                        if ret < 0 {
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
                                break;
                            }
                            if tx.blocking_send(buf[..n as usize].to_vec()).is_err() {
                                break;
                            }
                        }
                        // Check if PTY fd is closed (POLLHUP)
                        if pollfds[0].revents & (libc::POLLHUP | libc::POLLERR) != 0 {
                            break;
                        }
                    }
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
        let _ = session.channel_failure(channel);
        Ok(())
    }

    async fn subsystem_request(&mut self, channel: ChannelId, name: &str, session: &mut Session) -> Result<(), Self::Error> {
        if name == "sftp" && self.authenticated {
            let _ = session.channel_success(channel);
        } else {
            let _ = session.channel_failure(channel);
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

    async fn data(&mut self, channel: ChannelId, data: &[u8], session: &mut Session) -> Result<(), Self::Error> {
        // Write user input to PTY
        if let Some(ref pty) = self.pty_session {
            unsafe {
                libc::write(pty.master_fd, data.as_ptr() as *const libc::c_void, data.len());
            }
        }

        // Drain PTY output and send back to client
        if let Some(ref mut rx) = self.pty_rx {
            while let Ok(output) = rx.try_recv() {
                let _ = session.data(channel, output);
            }
        }

        Ok(())
    }

    async fn channel_close(&mut self, _channel: ChannelId, _session: &mut Session) -> Result<(), Self::Error> {
        self.pty_session = None;
        self.pty_rx = None;
        Ok(())
    }

    async fn channel_eof(&mut self, _channel: ChannelId, _session: &mut Session) -> Result<(), Self::Error> {
        self.pty_session = None;
        self.pty_rx = None;
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
                        if let Ok(authorized_key) = russh::keys::parse_public_key_base64(line) {
                            if authorized_key == *public_key {
                                return true;
                            }
                        }
                    }
                }
            }
        }
        false
    }
}
