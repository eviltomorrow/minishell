use thiserror::Error;

#[derive(Debug, Error)]
pub enum MinishellError {
    #[error("SSH connection failed: {0}")]
    Ssh(String),
    
    #[error("SFTP error: {0}")]
    Sftp(String),
    
    #[error("Database error: {0}")]
    Store(String),
    
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    
    #[error("File not found: {0}")]
    NotFound(String),
    
    #[error("Permission denied: {0}")]
    PermissionDenied(String),
    
    #[error("Transfer failed: {0}")]
    TransferFailed(String),
    
    #[error("Connection timeout")]
    Timeout,
}

pub type Result<T> = std::result::Result<T, MinishellError>;
