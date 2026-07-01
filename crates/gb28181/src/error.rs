//! Crate error type.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum GbError {
    #[error("timeout")]
    Timeout,
    #[error("device offline: {0}")]
    DeviceOffline(String),
    #[error("xml decode: {0}")]
    XmlDecode(String),
    #[error("sdp: {0}")]
    Sdp(String),
    #[error("auth: {0}")]
    Auth(String),
    #[error("negotiation: {0}")]
    Negotiation(String),
    #[error("sip: {0}")]
    Sip(String),
}

pub type Result<T> = std::result::Result<T, GbError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_includes_context() {
        assert_eq!(GbError::DeviceOffline("340...1".into()).to_string(), "device offline: 340...1");
        assert_eq!(GbError::Timeout.to_string(), "timeout");
    }
}
