use std::{fmt, net::IpAddr};

use url::{Host, Url};

use crate::{ClientError, ClientErrorCode};

#[derive(Clone, PartialEq, Eq, Hash)]
pub struct FrameOrigin {
    serialized: String,
    url: Url,
}

impl FrameOrigin {
    /// Parse an HTTPS production origin. Credentials, paths, query strings,
    /// fragments, non-default ports, IP literals, Unicode/punycode hosts, and
    /// ambiguous URL spellings are rejected.
    pub fn parse_https(value: &str) -> Result<Self, ClientError> {
        Self::parse(value, false)
    }

    /// Explicit local/test constructor. HTTP is accepted only for loopback
    /// hosts; this cannot silently weaken the production constructor.
    pub fn for_local_testing(value: &str) -> Result<Self, ClientError> {
        Self::parse(value, true)
    }

    fn parse(value: &str, local: bool) -> Result<Self, ClientError> {
        if value.is_empty()
            || value.trim() != value
            || !value.is_ascii()
            || value.contains(['\\', '@', '?', '#', '%'])
            || value.starts_with("//")
            || !(value.starts_with("https://") || local && value.starts_with("http://"))
            || !local && has_explicit_port(value)
        {
            return Err(ClientError::new(ClientErrorCode::InvalidOrigin));
        }

        let mut url =
            Url::parse(value).map_err(|_| ClientError::new(ClientErrorCode::InvalidOrigin))?;
        if url.cannot_be_a_base()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || !matches!(url.path(), "" | "/")
        {
            return Err(ClientError::new(ClientErrorCode::InvalidOrigin));
        }

        let loopback = is_loopback(&url);
        if url.scheme() != "https" && !(local && url.scheme() == "http" && loopback) {
            return Err(ClientError::new(ClientErrorCode::InvalidOrigin));
        }
        if !local && (url.port().is_some() || !is_unambiguous_dns_host(&url)) {
            return Err(ClientError::new(ClientErrorCode::InvalidOrigin));
        }
        if local && url.scheme() == "http" && !loopback {
            return Err(ClientError::new(ClientErrorCode::InvalidOrigin));
        }

        url.set_path("");
        let serialized = url.as_str().trim_end_matches('/').to_owned();
        Ok(Self { serialized, url })
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.serialized
    }

    pub fn api_v1_url(&self, resource: &str) -> Result<String, ClientError> {
        if resource.is_empty()
            || !resource.is_ascii()
            || resource.starts_with('/')
            || resource.contains(['\\', '?', '#', '%'])
            || resource
                .split('/')
                .any(|segment| segment.is_empty() || matches!(segment, "." | ".."))
        {
            return Err(ClientError::new(ClientErrorCode::InvalidPath));
        }
        Ok(format!("{}/api/v1/{resource}", self.serialized))
    }

    #[must_use]
    pub fn is_same_origin_url(&self, candidate: &str) -> bool {
        Url::parse(candidate).is_ok_and(|candidate| {
            candidate.username().is_empty()
                && candidate.password().is_none()
                && candidate.origin() == self.url.origin()
        })
    }
}

impl fmt::Debug for FrameOrigin {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("FrameOrigin(<validated-origin>)")
    }
}

impl fmt::Display for FrameOrigin {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<frame-origin>")
    }
}

fn is_unambiguous_dns_host(url: &Url) -> bool {
    match url.host() {
        Some(Host::Domain(host)) => {
            host.contains('.')
                && host
                    .split('.')
                    .all(|label| !label.is_empty() && !label.starts_with("xn--"))
        }
        Some(Host::Ipv4(_) | Host::Ipv6(_)) | None => false,
    }
}

fn is_loopback(url: &Url) -> bool {
    match url.host() {
        Some(Host::Domain(host)) => {
            host.eq_ignore_ascii_case("localhost") || host.ends_with(".localhost")
        }
        Some(Host::Ipv4(address)) => IpAddr::V4(address).is_loopback(),
        Some(Host::Ipv6(address)) => IpAddr::V6(address).is_loopback(),
        None => false,
    }
}

fn has_explicit_port(value: &str) -> bool {
    let Some((_, remainder)) = value.split_once("://") else {
        return false;
    };
    let authority = remainder.split('/').next().unwrap_or_default();
    if authority.starts_with('[') {
        authority.contains("]:")
    } else {
        authority.contains(':')
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_canonical_https_and_builds_api_urls() {
        let origin =
            FrameOrigin::parse_https("https://frame.engmanager.xyz").expect("canonical origin");
        assert_eq!(
            origin.api_v1_url("health").expect("health URL"),
            "https://frame.engmanager.xyz/api/v1/health"
        );
        assert!(origin.is_same_origin_url("https://frame.engmanager.xyz/s/public"));
        assert!(!origin.is_same_origin_url("https://attacker.example/s/public"));
    }

    #[test]
    fn rejects_ambiguous_and_secret_bearing_origins() {
        for value in [
            "//frame.engmanager.xyz",
            "http://frame.engmanager.xyz",
            "https://user:secret@frame.engmanager.xyz",
            "https://frame.engmanager.xyz/path",
            "https://frame.engmanager.xyz?token=secret",
            "https://frame.engmanager.xyz/#fragment",
            "https://frame.engmanager.xyz\\@attacker.example",
            "https://frame.engmanager.xyz:8443",
            "https://frame.engmanager.xyz:443",
            "HTTPS://frame.engmanager.xyz",
            "https://127.0.0.1",
            "https://främe.engmanager.xyz",
            "https://xn--frme-0ra.engmanager.xyz",
        ] {
            let error = FrameOrigin::parse_https(value).expect_err("origin must fail closed");
            assert_eq!(
                error.code(),
                ClientErrorCode::InvalidOrigin,
                "accepted {value}"
            );
            assert!(!format!("{error:?}").contains("secret"));
            assert!(!error.to_string().contains("secret"));
        }
    }

    #[test]
    fn loopback_http_requires_explicit_constructor() {
        assert!(FrameOrigin::parse_https("http://127.0.0.1:3000").is_err());
        assert!(FrameOrigin::for_local_testing("http://127.0.0.1:3000").is_ok());
        assert!(FrameOrigin::for_local_testing("http://localhost:3000").is_ok());
        assert!(FrameOrigin::for_local_testing("http://example.com:3000").is_err());
    }

    #[test]
    fn api_paths_reject_delimiters_and_traversal() {
        let origin =
            FrameOrigin::parse_https("https://frame.engmanager.xyz").expect("canonical origin");
        for path in [
            "",
            "/health",
            "../health",
            "shares//one",
            "shares/%2e%2e/admin",
            "health?token=x",
            "health#x",
        ] {
            assert!(origin.api_v1_url(path).is_err(), "accepted {path}");
        }
    }

    #[test]
    fn debug_and_display_are_redacted() {
        let origin =
            FrameOrigin::parse_https("https://frame.engmanager.xyz").expect("canonical origin");
        assert_eq!(format!("{origin:?}"), "FrameOrigin(<validated-origin>)");
        assert_eq!(origin.to_string(), "<frame-origin>");
    }

    #[test]
    fn deterministic_untrusted_input_corpus_never_weakens_origin_invariants() {
        // A dependency-free property corpus keeps this test available on the
        // core-only and wasm-compatible feature boundary. The seed is fixed so
        // a failure is reproducible, while the generated bytes exercise URL
        // delimiters, controls, non-UTF-8 replacement, and arbitrary casing.
        let mut state = 0x4f2d_79b9_6a31_c5e7_u64;
        for _ in 0..4_096 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let length = usize::try_from(state & 63).expect("bounded length");
            let mut bytes = Vec::with_capacity(length);
            for index in 0..length {
                let shift = u32::try_from((index % 8) * 8).expect("bounded shift");
                bytes.push((state.rotate_left(shift) & 0xff) as u8);
            }
            let candidate = String::from_utf8_lossy(&bytes);
            if let Ok(origin) = FrameOrigin::parse_https(&candidate) {
                assert!(origin.as_str().starts_with("https://"));
                assert!(origin.as_str().is_ascii());
                assert!(!origin.as_str().contains(['@', '?', '#', '%', '\\']));
                assert!(!origin.as_str().trim_start_matches("https://").contains(':'));
            }
        }
    }
}
