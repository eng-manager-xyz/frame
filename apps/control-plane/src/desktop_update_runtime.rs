//! Public, same-origin Tauri updater delivery backed by the private R2 binding.
//!
//! Release automation is the only writer. The Worker derives every R2 key
//! from bounded route coordinates; neither a request nor a release pointer can
//! name an arbitrary object. Tauri still verifies the downloaded artifact
//! against the public key embedded in the desktop binary.

use serde::{Deserialize, Serialize};
use worker::{Env, Method, Request, Response, send::IntoSendFuture};

const RELEASE_PREFIX: &str = "system/desktop-updates/v1/stable";
const MAX_POINTER_BYTES: u64 = 16 * 1_024;
const MAX_ARTIFACT_BYTES: u64 = 512 * 1_024 * 1_024;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ReleasePointerV1 {
    schema_version: u16,
    version: String,
    signature: String,
    bytes: u64,
    sha256: String,
    notes: Option<String>,
    pub_date: Option<String>,
}

impl ReleasePointerV1 {
    fn validate(&self) -> bool {
        self.schema_version == 1
            && valid_version(&self.version)
            && (1..=MAX_ARTIFACT_BYTES).contains(&self.bytes)
            && self.sha256.len() == 64
            && self.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
            && (32..=2_048).contains(&self.signature.len())
            && self.signature.is_ascii()
            && !self.signature.chars().any(char::is_whitespace)
            && self.notes.as_ref().is_none_or(|notes| {
                notes.len() <= 8_192 && !notes.chars().any(|character| character == '\0')
            })
            && self.pub_date.as_ref().is_none_or(|date| {
                date.len() <= 64 && date.is_ascii() && !date.chars().any(char::is_whitespace)
            })
    }
}

pub(crate) async fn manifest_response(
    request: &Request,
    env: &Env,
    canonical_origin: &str,
    target: &str,
    arch: &str,
    current_version: &str,
) -> worker::Result<Response> {
    if request.method() != Method::Get {
        return response_error(405, "method not allowed");
    }
    let Some(bundle) = exact_bundle_query(request)? else {
        return response_error(400, "invalid update request");
    };
    if !valid_coordinates(target, arch, bundle) || !valid_version(current_version) {
        return response_error(400, "invalid update request");
    }

    let bucket = env.bucket("RECORDINGS")?;
    let pointer_key = pointer_key(target, arch, bundle);
    let Some(object) = bucket.get(&pointer_key).execute().into_send().await? else {
        return response_error(503, "update service unavailable");
    };
    if object.size() == 0 || object.size() > MAX_POINTER_BYTES {
        return response_error(503, "update service unavailable");
    }
    let Some(body) = object.body() else {
        return response_error(503, "update service unavailable");
    };
    let text = body.text().into_send().await?;
    let Ok(pointer) = serde_json::from_str::<ReleasePointerV1>(&text) else {
        return response_error(503, "update service unavailable");
    };
    if !pointer.validate() {
        return response_error(503, "update service unavailable");
    }

    let artifact_key = artifact_key(target, arch, bundle, &pointer.version);
    let Some(artifact) = bucket.head(&artifact_key).into_send().await? else {
        return response_error(503, "update service unavailable");
    };
    if artifact.size() != pointer.bytes {
        return response_error(503, "update service unavailable");
    }

    let url = format!(
        "{canonical_origin}/api/v1/desktop/updates/artifacts/{target}/{arch}/{bundle}/{}",
        pointer.version
    );
    let body = serde_json::json!({
        "version": pointer.version,
        "notes": pointer.notes,
        "pub_date": pointer.pub_date,
        "url": url,
        "signature": pointer.signature,
    });
    let mut response = Response::from_json(&body)?;
    let headers = response.headers_mut();
    headers.set("cache-control", "public, max-age=60, no-transform")?;
    headers.set("content-type", "application/json; charset=utf-8")?;
    headers.set("x-content-type-options", "nosniff")?;
    Ok(response)
}

pub(crate) async fn artifact_response(
    request: &Request,
    env: &Env,
    target: &str,
    arch: &str,
    bundle: &str,
    version: &str,
) -> worker::Result<Response> {
    if !matches!(request.method(), Method::Get | Method::Head) {
        return response_error(405, "method not allowed");
    }
    if !valid_coordinates(target, arch, bundle) || !valid_version(version) {
        return response_error(404, "update artifact not found");
    }

    let bucket = env.bucket("RECORDINGS")?;
    let key = artifact_key(target, arch, bundle, version);
    let Some(head) = bucket.head(&key).into_send().await? else {
        return response_error(404, "update artifact not found");
    };
    let size = head.size();
    if size == 0 || size > MAX_ARTIFACT_BYTES {
        return response_error(503, "update service unavailable");
    }
    let mut response = if request.method() == Method::Head {
        Response::empty()?
    } else {
        let Some(object) = bucket.get(&key).execute().into_send().await? else {
            return response_error(404, "update artifact not found");
        };
        let Some(body) = object.body() else {
            return response_error(503, "update service unavailable");
        };
        Response::from_body(body.response_body()?)?
    };
    let headers = response.headers_mut();
    headers.set(
        "cache-control",
        "public, max-age=31536000, immutable, no-transform",
    )?;
    headers.set("content-type", "application/octet-stream")?;
    headers.set("content-length", &size.to_string())?;
    headers.set("x-content-type-options", "nosniff")?;
    Ok(response)
}

fn exact_bundle_query(request: &Request) -> worker::Result<Option<&str>> {
    let url = request.url()?;
    let mut pairs = url.query_pairs();
    let Some((name, value)) = pairs.next() else {
        return Ok(None);
    };
    if name != "bundle" || pairs.next().is_some() {
        return Ok(None);
    }
    Ok(match value.as_ref() {
        "app" => Some("app"),
        "nsis" => Some("nsis"),
        "msi" => Some("msi"),
        _ => None,
    })
}

fn pointer_key(target: &str, arch: &str, bundle: &str) -> String {
    format!("{RELEASE_PREFIX}/{target}/{arch}/{bundle}/latest.json")
}

fn artifact_key(target: &str, arch: &str, bundle: &str, version: &str) -> String {
    format!("{RELEASE_PREFIX}/{target}/{arch}/{bundle}/{version}/artifact")
}

fn valid_coordinates(target: &str, arch: &str, bundle: &str) -> bool {
    matches!(target, "darwin" | "windows")
        && matches!(arch, "aarch64" | "x86_64")
        && matches!(
            (target, bundle),
            ("darwin", "app") | ("windows", "nsis" | "msi")
        )
}

fn valid_version(value: &str) -> bool {
    value.len() <= 64
        && value.is_ascii()
        && semver::Version::parse(value).is_ok()
        && !value.contains(['/', '\\', '%'])
}

fn response_error(status: u16, message: &str) -> worker::Result<Response> {
    let mut response = Response::error(message, status)?;
    response.headers_mut().set("cache-control", "no-store")?;
    response
        .headers_mut()
        .set("x-content-type-options", "nosniff")?;
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn r2_keys_are_derived_only_from_bounded_coordinates() {
        assert!(valid_coordinates("darwin", "aarch64", "app"));
        assert!(valid_coordinates("windows", "x86_64", "nsis"));
        assert!(!valid_coordinates("linux", "x86_64", "app"));
        assert!(!valid_coordinates("darwin", "../escape", "app"));
        assert_eq!(
            pointer_key("darwin", "aarch64", "app"),
            "system/desktop-updates/v1/stable/darwin/aarch64/app/latest.json"
        );
        assert_eq!(
            artifact_key("windows", "x86_64", "nsis", "1.2.3"),
            "system/desktop-updates/v1/stable/windows/x86_64/nsis/1.2.3/artifact"
        );
    }

    #[test]
    fn release_pointer_is_deny_unknown_and_bounded() {
        let valid = serde_json::json!({
            "schema_version": 1,
            "version": "1.2.3",
            "signature": "A".repeat(64),
            "bytes": 1024,
            "sha256": "a".repeat(64),
            "notes": "security update",
            "pub_date": "2026-07-29T12:00:00Z"
        });
        let pointer: ReleasePointerV1 = serde_json::from_value(valid.clone()).expect("pointer");
        assert!(pointer.validate());
        let mut unknown = valid;
        unknown["artifact_key"] = serde_json::json!("../../private");
        assert!(serde_json::from_value::<ReleasePointerV1>(unknown).is_err());
    }

    #[test]
    fn versions_are_strict_semver_segments() {
        assert!(valid_version("1.2.3"));
        assert!(valid_version("2.0.0-rc.1"));
        assert!(!valid_version("../1.2.3"));
        assert!(!valid_version("v1.2.3"));
        assert!(!valid_version("1.2"));
    }
}
