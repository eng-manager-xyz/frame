use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ClientErrorCode {
    InvalidOrigin,
    InvalidPath,
    InvalidIdentifier,
    IncompatibleVersion,
    InvalidContract,
    PrivacyViolation,
    DeadlineExceeded,
    TransportUnavailable,
    ResponseTooLarge,
    UnsupportedContentType,
    RedirectRejected,
    MalformedResponse,
    ApiRejected,
}

impl ClientErrorCode {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidOrigin => "invalid_origin",
            Self::InvalidPath => "invalid_path",
            Self::InvalidIdentifier => "invalid_identifier",
            Self::IncompatibleVersion => "incompatible_version",
            Self::InvalidContract => "invalid_contract",
            Self::PrivacyViolation => "privacy_violation",
            Self::DeadlineExceeded => "deadline_exceeded",
            Self::TransportUnavailable => "transport_unavailable",
            Self::ResponseTooLarge => "response_too_large",
            Self::UnsupportedContentType => "unsupported_content_type",
            Self::RedirectRejected => "redirect_rejected",
            Self::MalformedResponse => "malformed_response",
            Self::ApiRejected => "api_rejected",
        }
    }

    #[must_use]
    pub const fn retryable(self) -> bool {
        matches!(self, Self::DeadlineExceeded | Self::TransportUnavailable)
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct ClientError {
    code: ClientErrorCode,
}

impl ClientError {
    pub(crate) const fn new(code: ClientErrorCode) -> Self {
        Self { code }
    }

    /// Construct the redacted retryable error used by consumer-side circuit
    /// breakers and other transport admission controls.
    #[must_use]
    pub const fn transport_unavailable() -> Self {
        Self::new(ClientErrorCode::TransportUnavailable)
    }

    #[must_use]
    pub const fn code(&self) -> ClientErrorCode {
        self.code
    }

    #[must_use]
    pub const fn retryable(&self) -> bool {
        self.code.retryable()
    }
}

impl fmt::Debug for ClientError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ClientError")
            .field("code", &self.code.as_str())
            .field("retryable", &self.retryable())
            .finish()
    }
}

impl fmt::Display for ClientError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.code {
            ClientErrorCode::InvalidOrigin => "the Frame origin is invalid",
            ClientErrorCode::InvalidPath => "the Frame API path is invalid",
            ClientErrorCode::InvalidIdentifier => "the public identifier is invalid",
            ClientErrorCode::IncompatibleVersion => "the Frame API major version is incompatible",
            ClientErrorCode::InvalidContract => "the Frame response violates the public contract",
            ClientErrorCode::PrivacyViolation => "the Frame response violates public-data policy",
            ClientErrorCode::DeadlineExceeded => "the Frame request deadline was exceeded",
            ClientErrorCode::TransportUnavailable => "the Frame service is temporarily unavailable",
            ClientErrorCode::ResponseTooLarge => "the Frame response exceeded the configured limit",
            ClientErrorCode::UnsupportedContentType => {
                "the Frame response content type is unsupported"
            }
            ClientErrorCode::RedirectRejected => {
                "the Frame response attempted an unapproved redirect"
            }
            ClientErrorCode::MalformedResponse => "the Frame response could not be decoded",
            ClientErrorCode::ApiRejected => "the Frame API rejected the request",
        })
    }
}

impl std::error::Error for ClientError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_and_display_never_include_supplied_secrets() {
        let secret = "frame-secret-do-not-log";
        let error = ClientError::new(ClientErrorCode::TransportUnavailable);
        assert!(!format!("{error:?}").contains(secret));
        assert!(!error.to_string().contains(secret));
        assert_eq!(error.code().as_str(), "transport_unavailable");
    }
}
