//! One error type for the whole library.

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// A capability handle was rejected. Carries why.
    Handle(HandleError),
    /// A request did not satisfy its provider definition.
    Constraint(String),
    /// Policy said no.
    Denied(String),
    /// Policy wants a human first.
    StepUpRequired(String),
    /// The named thing does not exist (task, provider, ref).
    NotFound(String),
    /// A definition or manifest could not be parsed.
    Parse(String),
    /// The secret store failed.
    Store(String),
    /// An OS-level operation failed.
    Os(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandleError {
    /// No such handle. Also what a forged handle looks like.
    Unknown,
    /// Past its expiry.
    Expired,
    /// Issued to a different session.
    WrongSession,
    /// Issued for a different tool-call epoch.
    WrongEpoch,
    /// Sequence number replayed or went backwards.
    StaleSequence,
    /// Already redeemed the maximum number of times.
    Exhausted,
}

impl fmt::Display for HandleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            HandleError::Unknown => "unknown handle",
            HandleError::Expired => "handle expired",
            HandleError::WrongSession => "handle belongs to another session",
            HandleError::WrongEpoch => "handle belongs to another tool-call",
            HandleError::StaleSequence => "sequence replayed or out of order",
            HandleError::Exhausted => "handle already used",
        };
        f.write_str(s)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Handle(e) => write!(f, "{}", e),
            Error::Constraint(m) => write!(f, "constraint violation: {}", m),
            Error::Denied(m) => write!(f, "denied: {}", m),
            Error::StepUpRequired(m) => write!(f, "approval required: {}", m),
            Error::NotFound(m) => write!(f, "not found: {}", m),
            Error::Parse(m) => write!(f, "parse error: {}", m),
            Error::Store(m) => write!(f, "store error: {}", m),
            Error::Os(m) => write!(f, "os error: {}", m),
        }
    }
}

impl std::error::Error for Error {}

impl From<HandleError> for Error {
    fn from(e: HandleError) -> Self {
        Error::Handle(e)
    }
}

pub type Result<T> = std::result::Result<T, Error>;
