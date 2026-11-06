use std::fmt;

/// A user-facing failure. Messages are shown verbatim in the conversation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeError(String);

impl BridgeError {
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }

    pub fn message(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for BridgeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for BridgeError {}

pub type Result<T, E = BridgeError> = std::result::Result<T, E>;
