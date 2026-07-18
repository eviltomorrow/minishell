use serde::Deserialize;
use std::path::PathBuf;
use russh::keys::PrivateKey;

#[derive(Debug, Deserialize)]
pub struct ServerConfig {
    #[serde(default = "default_server")]
    pub server: ServerSection,

    #[serde(default = "default_host_key")]
    pub host_key: HostKeySection,

    pub auth: AuthSection,

    #[serde(default = "default_log")]
    pub log: LogSection,
}

#[derive(Debug, Deserialize)]
pub struct ServerSection {
    #[serde(default = "default_bind")]
    pub bind: String,
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default = "default_max_connections")]
    pub max_connections: usize,
    #[serde(default = "default_session_timeout")]
    pub session_timeout: u64,
}

#[derive(Debug, Deserialize)]
pub struct HostKeySection {
    #[serde(default = "default_host_key_path")]
    pub path: String,
}

#[derive(Debug, Deserialize)]
pub struct AuthSection {
    pub users: Vec<UserConfig>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct UserConfig {
    pub username: String,
    #[serde(default)]
    pub password: Option<String>,
    #[serde(default)]
    pub authorized_keys: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct LogSection {
    #[serde(default = "default_log_level")]
    pub level: String,
}

fn default_server() -> ServerSection {
    ServerSection {
        bind: default_bind(),
        port: default_port(),
        max_connections: default_max_connections(),
        session_timeout: default_session_timeout(),
    }
}

fn default_host_key() -> HostKeySection {
    HostKeySection { path: default_host_key_path() }
}

fn default_log() -> LogSection {
    LogSection { level: default_log_level() }
}

fn default_bind() -> String { "0.0.0.0".to_string() }
fn default_port() -> u16 { 2222 }
fn default_max_connections() -> usize { 50 }
fn default_session_timeout() -> u64 { 3600 }
fn default_host_key_path() -> String {
    dirs::home_dir()
        .map(|h| h.join(".config/minishell-server/host_key"))
        .unwrap_or_else(|| PathBuf::from("/etc/minishell-server/host_key"))
        .to_string_lossy()
        .to_string()
}
fn default_log_level() -> String { "info".to_string() }

pub fn default_config_path() -> String {
    if let Some(home) = dirs::home_dir() {
        let p = home.join(".config/minishell-server/config.toml");
        if p.exists() {
            return p.to_string_lossy().to_string();
        }
    }
    "/etc/minishell-server/config.toml".to_string()
}

pub fn load_config(path: &str) -> anyhow::Result<ServerConfig> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("Failed to read config '{}': {}", path, e))?;
    let config: ServerConfig = toml::from_str(&content)
        .map_err(|e| anyhow::anyhow!("Failed to parse config '{}': {}", path, e))?;
    Ok(config)
}

impl ServerConfig {
    pub fn find_user(&self, username: &str) -> Option<&UserConfig> {
        self.auth.users.iter().find(|u| u.username == username)
    }

    pub fn expanded_host_key_path(&self) -> PathBuf {
        expand_tilde(&self.host_key.path)
    }

    pub fn expanded_authorized_keys_path(&self, user: &UserConfig) -> Option<PathBuf> {
        user.authorized_keys.as_ref().map(|p| expand_tilde(p))
    }
}

pub fn expand_tilde(path: &str) -> PathBuf {
    if let Some(stripped) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(stripped);
        }
    }
    PathBuf::from(path)
}

pub fn load_or_generate_host_key(path: &std::path::Path) -> anyhow::Result<PrivateKey> {
    if path.exists() {
        let content = std::fs::read_to_string(path)?;
        let key_pair = russh::keys::decode_secret_key(&content, None)?;
        tracing::info!("Loaded host key from {}", path.display());
        return Ok(key_pair);
    }

    // Generate new Ed25519 key
    let mut rng = russh::keys::key::safe_rng();
    let key_pair = PrivateKey::random(&mut rng, russh::keys::Algorithm::Ed25519)
        .map_err(|e| anyhow::anyhow!("Failed to generate Ed25519 key: {}", e))?;

    // Save to file
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut buf = Vec::new();
    russh::keys::encode_pkcs8_pem(&key_pair, &mut buf)?;
    let key_str = String::from_utf8_lossy(&buf);
    std::fs::write(path, key_str.as_ref())?;

    // Set restrictive permissions (owner only)
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }

    tracing::info!("Generated new Ed25519 host key at {}", path.display());
    Ok(key_pair)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_valid_config() {
        let toml = r#"
[server]
bind = "127.0.0.1"
port = 2222

[host_key]
path = "/tmp/test_key"

[auth]
[[auth.users]]
username = "admin"
password = "secret"

[log]
level = "debug"
"#;
        let config: ServerConfig = toml::from_str(toml).unwrap();
        assert_eq!(config.server.bind, "127.0.0.1");
        assert_eq!(config.server.port, 2222);
        assert_eq!(config.auth.users.len(), 1);
        assert_eq!(config.auth.users[0].username, "admin");
        assert_eq!(config.auth.users[0].password, Some("secret".to_string()));
    }

    #[test]
    fn test_defaults() {
        let toml = r#"
[auth]
[[auth.users]]
username = "admin"
password = "secret"
"#;
        let config: ServerConfig = toml::from_str(toml).unwrap();
        assert_eq!(config.server.bind, "0.0.0.0");
        assert_eq!(config.server.port, 2222);
        assert_eq!(config.server.max_connections, 50);
        assert_eq!(config.server.session_timeout, 3600);
        assert_eq!(config.log.level, "info");
    }

    #[test]
    fn test_find_user() {
        let toml = r#"
[auth]
[[auth.users]]
username = "admin"
password = "secret"
[[auth.users]]
username = "deploy"
authorized_keys = "~/.ssh/authorized_keys"
"#;
        let config: ServerConfig = toml::from_str(toml).unwrap();
        assert!(config.find_user("admin").is_some());
        assert!(config.find_user("deploy").is_some());
        assert!(config.find_user("unknown").is_none());
    }

    #[test]
    fn test_expand_tilde() {
        let result = expand_tilde("~/test");
        assert!(result.to_string_lossy().contains("test"));
        assert!(!result.to_string_lossy().starts_with("~"));
    }
}
