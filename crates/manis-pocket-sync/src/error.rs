use std::fmt;

/// Error codes returned from FFI functions.
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    Ok = 0,
    Init = 1,
    Pairing = 2,
    Network = 3,
    InvalidArg = 4,
    NotRunning = 5,
    AlreadyExists = 6,
    NotFound = 7,
    Serialization = 8,
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ok => write!(f, "OK"),
            Self::Init => write!(f, "Initialization error"),
            Self::Pairing => write!(f, "Pairing error"),
            Self::Network => write!(f, "Network error"),
            Self::InvalidArg => write!(f, "Invalid argument"),
            Self::NotRunning => write!(f, "Sync engine not running"),
            Self::AlreadyExists => write!(f, "Already exists"),
            Self::NotFound => write!(f, "Not found"),
            Self::Serialization => write!(f, "Serialization error"),
        }
    }
}

pub type Result<T> = std::result::Result<T, ErrorCode>;
