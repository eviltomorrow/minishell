use minishell_server::config::load_config;
use std::io::Write;

#[test]
fn test_load_config_valid() {
    let dir = std::env::temp_dir().join("minishell_server_test");
    std::fs::create_dir_all(&dir).unwrap();
    let config_path = dir.join("config.toml");

    let mut f = std::fs::File::create(&config_path).unwrap();
    writeln!(f, r#"
[server]
bind = "127.0.0.1"
port = 2222

[host_key]
path = "{}/host_key"

[auth]
[[auth.users]]
username = "test"
password = "test123"

[log]
level = "info"
"#, dir.display()).unwrap();

    let config = load_config(config_path.to_str().unwrap()).unwrap();
    assert_eq!(config.server.bind, "127.0.0.1");
    assert_eq!(config.server.port, 2222);
    assert_eq!(config.auth.users.len(), 1);
    assert_eq!(config.auth.users[0].username, "test");

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn test_load_config_defaults() {
    let dir = std::env::temp_dir().join("minishell_server_test_defaults");
    std::fs::create_dir_all(&dir).unwrap();
    let config_path = dir.join("config.toml");

    let mut f = std::fs::File::create(&config_path).unwrap();
    writeln!(f, r#"
[auth]
[[auth.users]]
username = "admin"
password = "secret"
"#).unwrap();

    let config = load_config(config_path.to_str().unwrap()).unwrap();
    assert_eq!(config.server.bind, "0.0.0.0");
    assert_eq!(config.server.port, 2222);
    assert_eq!(config.server.max_connections, 50);
    assert_eq!(config.server.session_timeout, 3600);
    assert_eq!(config.log.level, "info");

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn test_load_config_missing_file() {
    let result = load_config("/tmp/nonexistent_config.toml");
    assert!(result.is_err());
}

#[test]
fn test_load_config_invalid_toml() {
    let dir = std::env::temp_dir().join("minishell_server_test_invalid");
    std::fs::create_dir_all(&dir).unwrap();
    let config_path = dir.join("config.toml");

    let mut f = std::fs::File::create(&config_path).unwrap();
    writeln!(f, "this is not valid toml {{{{").unwrap();

    let result = load_config(config_path.to_str().unwrap());
    assert!(result.is_err());

    std::fs::remove_dir_all(&dir).ok();
}
