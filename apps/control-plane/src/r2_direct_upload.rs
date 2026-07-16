//! Cloudflare R2 S3 SigV4 capabilities for browser-direct immutable PUTs.
//!
//! The signed capability binds the exact private staging key, method, content
//! type, declared SHA-256 metadata, expiry, and `If-None-Match: *`. The latter is essential:
//! an otherwise reusable presigned URL must never overwrite bytes that a
//! successful finalize has made authoritative.

use std::fmt;

use serde::Serialize;
use sha2::{Digest, Sha256};

const ALGORITHM: &str = "AWS4-HMAC-SHA256";
const REGION: &str = "auto";
const SERVICE: &str = "s3";
const TERMINATOR: &str = "aws4_request";
pub const MIN_DIRECT_UPLOAD_TTL_SECONDS: u32 = 30;
pub const MAX_DIRECT_UPLOAD_TTL_SECONDS: u32 = 900;
pub const MAX_DIRECT_UPLOAD_BYTES: u64 = 100 * 1_024 * 1_024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirectUploadSigningError {
    InvalidConfiguration,
    InvalidRequest,
    TimestampOutOfRange,
}

#[derive(Clone, PartialEq, Eq)]
pub struct R2SigningCredentials {
    access_key_id: String,
    secret_access_key: String,
}

impl R2SigningCredentials {
    pub fn parse(
        access_key_id: impl Into<String>,
        secret_access_key: impl Into<String>,
    ) -> Result<Self, DirectUploadSigningError> {
        let access_key_id = access_key_id.into();
        let secret_access_key = secret_access_key.into();
        if !(16..=128).contains(&access_key_id.len())
            || !(32..=256).contains(&secret_access_key.len())
            || !access_key_id.bytes().all(|byte| byte.is_ascii_graphic())
            || !secret_access_key
                .bytes()
                .all(|byte| byte.is_ascii_graphic())
        {
            return Err(DirectUploadSigningError::InvalidConfiguration);
        }
        Ok(Self {
            access_key_id,
            secret_access_key,
        })
    }
}

impl fmt::Debug for R2SigningCredentials {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("R2SigningCredentials")
            .field("access_key_id", &"[redacted]")
            .field("secret_access_key", &"[redacted]")
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq, Serialize)]
pub struct DirectPutCapabilityV1 {
    pub url: String,
    pub method: &'static str,
    pub required_headers: Vec<(String, String)>,
    pub expires_at_ms: u64,
}

impl fmt::Debug for DirectPutCapabilityV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DirectPutCapabilityV1")
            .field("url", &"[redacted-presigned-url]")
            .field("method", &self.method)
            .field(
                "required_header_names",
                &self
                    .required_headers
                    .iter()
                    .map(|(name, _)| name.as_str())
                    .collect::<Vec<_>>(),
            )
            .field("expires_at_ms", &self.expires_at_ms)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct R2DirectPutSigner {
    account_id: String,
    bucket_name: String,
    credentials: R2SigningCredentials,
}

impl R2DirectPutSigner {
    pub fn new(
        account_id: impl Into<String>,
        bucket_name: impl Into<String>,
        credentials: R2SigningCredentials,
    ) -> Result<Self, DirectUploadSigningError> {
        let account_id = account_id.into();
        let bucket_name = bucket_name.into();
        if account_id.len() != 32
            || !account_id
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            || !(3..=63).contains(&bucket_name.len())
            || !bucket_name
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
            || bucket_name.starts_with('-')
            || bucket_name.ends_with('-')
        {
            return Err(DirectUploadSigningError::InvalidConfiguration);
        }
        Ok(Self {
            account_id,
            bucket_name,
            credentials,
        })
    }

    pub fn sign_put(
        &self,
        key: &str,
        content_type: &str,
        checksum_sha256_hex: &str,
        expected_bytes: u64,
        now_ms: u64,
        ttl_seconds: u32,
    ) -> Result<DirectPutCapabilityV1, DirectUploadSigningError> {
        if !(MIN_DIRECT_UPLOAD_TTL_SECONDS..=MAX_DIRECT_UPLOAD_TTL_SECONDS).contains(&ttl_seconds)
            || !valid_private_staging_key(key)
            || !valid_content_type(content_type)
            || !key.ends_with(&format!(
                ".{}",
                content_type_extension(content_type).unwrap_or("")
            ))
            || !(1..=MAX_DIRECT_UPLOAD_BYTES).contains(&expected_bytes)
        {
            return Err(DirectUploadSigningError::InvalidRequest);
        }
        let checksum =
            decode_sha256(checksum_sha256_hex).ok_or(DirectUploadSigningError::InvalidRequest)?;
        let checksum_base64 = base64(&checksum);
        let (date, timestamp) = aws_timestamp(now_ms)?;
        let host = format!("{}.r2.cloudflarestorage.com", self.account_id);
        let canonical_uri = format!(
            "/{}/{}",
            percent_encode_path_segment(&self.bucket_name),
            key.split('/')
                .map(percent_encode_path_segment)
                .collect::<Vec<_>>()
                .join("/")
        );
        let credential_scope = format!("{date}/{REGION}/{SERVICE}/{TERMINATOR}");
        let signed_headers = "content-length;content-type;host;if-none-match;x-amz-checksum-sha256;x-amz-meta-frame-sha256";
        let mut query = [
            ("X-Amz-Algorithm", ALGORITHM.to_owned()),
            (
                "X-Amz-Credential",
                format!("{}/{}", self.credentials.access_key_id, credential_scope),
            ),
            ("X-Amz-Date", timestamp.clone()),
            ("X-Amz-Expires", ttl_seconds.to_string()),
            ("X-Amz-Content-Sha256", "UNSIGNED-PAYLOAD".to_owned()),
            ("X-Amz-SignedHeaders", signed_headers.to_owned()),
        ];
        query.sort_by(|left, right| left.0.cmp(right.0));
        let canonical_query = query
            .iter()
            .map(|(key, value)| {
                format!(
                    "{}={}",
                    percent_encode_query(key),
                    percent_encode_query(value)
                )
            })
            .collect::<Vec<_>>()
            .join("&");
        let canonical_headers = format!(
            "content-length:{}\ncontent-type:{}\nhost:{}\nif-none-match:*\nx-amz-checksum-sha256:{}\nx-amz-meta-frame-sha256:{}\n",
            expected_bytes, content_type, host, checksum_base64, checksum_sha256_hex
        );
        let canonical_request = format!(
            "PUT\n{canonical_uri}\n{canonical_query}\n{canonical_headers}\n{signed_headers}\nUNSIGNED-PAYLOAD"
        );
        let string_to_sign = format!(
            "{ALGORITHM}\n{timestamp}\n{credential_scope}\n{}",
            hex(&Sha256::digest(canonical_request.as_bytes()))
        );
        let date_key = hmac_sha256(
            format!("AWS4{}", self.credentials.secret_access_key).as_bytes(),
            date.as_bytes(),
        );
        let region_key = hmac_sha256(&date_key, REGION.as_bytes());
        let service_key = hmac_sha256(&region_key, SERVICE.as_bytes());
        let signing_key = hmac_sha256(&service_key, TERMINATOR.as_bytes());
        let signature = hex(&hmac_sha256(&signing_key, string_to_sign.as_bytes()));
        let expires_at_ms = now_ms
            .checked_div(1_000)
            .and_then(|seconds| seconds.checked_mul(1_000))
            .ok_or(DirectUploadSigningError::TimestampOutOfRange)?
            .checked_add(u64::from(ttl_seconds) * 1_000)
            .ok_or(DirectUploadSigningError::TimestampOutOfRange)?;
        Ok(DirectPutCapabilityV1 {
            url: format!(
                "https://{host}{canonical_uri}?{canonical_query}&X-Amz-Signature={signature}"
            ),
            method: "PUT",
            required_headers: vec![
                ("content-length".into(), expected_bytes.to_string()),
                ("content-type".into(), content_type.into()),
                ("if-none-match".into(), "*".into()),
                ("x-amz-checksum-sha256".into(), checksum_base64),
                ("x-amz-meta-frame-sha256".into(), checksum_sha256_hex.into()),
            ],
            expires_at_ms,
        })
    }
}

/// Build the only staging-key shape that can receive a browser capability.
///
/// The tenant UUID is irreversibly scoped before it enters the URL, while the
/// upload UUID supplies the random, collision-resistant object identity.
pub fn private_staging_key(
    tenant_id: &str,
    upload_id: &str,
    content_type: &str,
) -> Result<String, DirectUploadSigningError> {
    if !valid_uuid(tenant_id) || !valid_uuid(upload_id) || !valid_content_type(content_type) {
        return Err(DirectUploadSigningError::InvalidRequest);
    }
    let tenant_scope = hex(&Sha256::digest(
        [
            b"frame.direct-upload.tenant.v1\0".as_slice(),
            tenant_id.as_bytes(),
        ]
        .concat(),
    ));
    let extension =
        content_type_extension(content_type).ok_or(DirectUploadSigningError::InvalidRequest)?;
    let key = format!("uploads/{tenant_scope}/staging/{upload_id}.{extension}");
    valid_private_staging_key(&key)
        .then_some(key)
        .ok_or(DirectUploadSigningError::InvalidRequest)
}

fn valid_uuid(value: &str) -> bool {
    value != "00000000-0000-0000-0000-000000000000"
        && value.len() == 36
        && value.bytes().enumerate().all(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                byte == b'-'
            } else {
                byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
            }
        })
}

fn valid_private_staging_key(value: &str) -> bool {
    let segments = value.split('/').collect::<Vec<_>>();
    let object = segments.get(3).and_then(|value| value.rsplit_once('.'));
    value.len() <= 1_024
        && value.starts_with("uploads/")
        && value.contains("/staging/")
        && segments.len() == 4
        && segments[0] == "uploads"
        && segments[1].len() == 64
        && segments[1]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        && segments[2] == "staging"
        && object.is_some_and(|(upload_id, extension)| {
            valid_uuid(upload_id) && matches!(extension, "mp4" | "webm" | "mov" | "mkv")
        })
        && !value.ends_with('/')
        && !value.contains("//")
        && !value.contains('\\')
        && segments.into_iter().all(|segment| {
            !segment.is_empty()
                && segment != "."
                && segment != ".."
                && segment
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        })
}

fn valid_content_type(value: &str) -> bool {
    content_type_extension(value).is_some()
}

fn content_type_extension(value: &str) -> Option<&'static str> {
    match value {
        "video/mp4" => Some("mp4"),
        "video/webm" => Some("webm"),
        "video/quicktime" => Some("mov"),
        "video/x-matroska" => Some("mkv"),
        _ => None,
    }
}

fn decode_sha256(value: &str) -> Option<[u8; 32]> {
    if value.len() != 64 {
        return None;
    }
    let mut bytes = [0_u8; 32];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        bytes[index] = (hex_nibble(pair[0])? << 4) | hex_nibble(pair[1])?;
    }
    Some(bytes)
}

fn base64(value: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut output = String::with_capacity(value.len().div_ceil(3) * 4);
    for chunk in value.chunks(3) {
        let first = chunk[0];
        let second = chunk.get(1).copied().unwrap_or(0);
        let third = chunk.get(2).copied().unwrap_or(0);
        output.push(char::from(ALPHABET[usize::from(first >> 2)]));
        output.push(char::from(
            ALPHABET[usize::from((first & 0x03) << 4 | second >> 4)],
        ));
        if chunk.len() > 1 {
            output.push(char::from(
                ALPHABET[usize::from((second & 0x0f) << 2 | third >> 6)],
            ));
        } else {
            output.push('=');
        }
        if chunk.len() > 2 {
            output.push(char::from(ALPHABET[usize::from(third & 0x3f)]));
        } else {
            output.push('=');
        }
    }
    output
}

fn hex_nibble(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        _ => None,
    }
}

fn percent_encode_path_segment(value: &str) -> String {
    percent_encode(value, true)
}

fn percent_encode_query(value: &str) -> String {
    percent_encode(value, false)
}

fn percent_encode(value: &str, path: bool) -> String {
    let mut output = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric()
            || matches!(byte, b'-' | b'_' | b'.' | b'~')
            || (path && byte == b'/')
        {
            output.push(char::from(byte));
        } else {
            output.push('%');
            output.push(char::from(b"0123456789ABCDEF"[usize::from(byte >> 4)]));
            output.push(char::from(b"0123456789ABCDEF"[usize::from(byte & 0x0f)]));
        }
    }
    output
}

fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    let mut normalized = [0_u8; 64];
    if key.len() > normalized.len() {
        normalized[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        normalized[..key.len()].copy_from_slice(key);
    }
    let mut inner_key = [0x36_u8; 64];
    let mut outer_key = [0x5c_u8; 64];
    for index in 0..64 {
        inner_key[index] ^= normalized[index];
        outer_key[index] ^= normalized[index];
    }
    let mut inner = Sha256::new();
    inner.update(inner_key);
    inner.update(message);
    let mut outer = Sha256::new();
    outer.update(outer_key);
    outer.update(inner.finalize());
    outer.finalize().into()
}

fn hex(value: &[u8]) -> String {
    let mut output = String::with_capacity(value.len() * 2);
    for byte in value {
        output.push(char::from(b"0123456789abcdef"[usize::from(byte >> 4)]));
        output.push(char::from(b"0123456789abcdef"[usize::from(byte & 0x0f)]));
    }
    output
}

fn aws_timestamp(now_ms: u64) -> Result<(String, String), DirectUploadSigningError> {
    let seconds =
        i64::try_from(now_ms / 1_000).map_err(|_| DirectUploadSigningError::TimestampOutOfRange)?;
    let days = seconds.div_euclid(86_400);
    let day_seconds = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days)?;
    let hour = day_seconds / 3_600;
    let minute = (day_seconds % 3_600) / 60;
    let second = day_seconds % 60;
    let date = format!("{year:04}{month:02}{day:02}");
    Ok((
        date.clone(),
        format!("{date}T{hour:02}{minute:02}{second:02}Z"),
    ))
}

// Howard Hinnant's civil-from-days algorithm, with a deliberately narrow AWS
// signing range.  This avoids locale and JS Date formatting differences.
fn civil_from_days(days_since_epoch: i64) -> Result<(i64, i64, i64), DirectUploadSigningError> {
    let z = days_since_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 }.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let month_prime = (5 * doy + 2) / 153;
    let day = doy - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    if !(2000..=9999).contains(&year) {
        return Err(DirectUploadSigningError::TimestampOutOfRange);
    }
    Ok((year, month, day))
}

#[cfg(test)]
mod tests {
    use super::*;

    const TENANT: &str = "018f47a6-7b1c-7f55-8f39-8f8a86900101";
    const UPLOAD_A: &str = "018f47a6-7b1c-7f55-8f39-8f8a86900111";
    const UPLOAD_B: &str = "018f47a6-7b1c-7f55-8f39-8f8a86900112";

    fn staging(upload_id: &str, content_type: &str) -> String {
        private_staging_key(TENANT, upload_id, content_type).expect("staging key")
    }

    fn signer() -> R2DirectPutSigner {
        R2DirectPutSigner::new(
            "0123456789abcdef0123456789abcdef",
            "frame-recordings-production",
            R2SigningCredentials::parse(
                "frame-test-access-key-0001",
                "frame-test-secret-material-that-is-long-enough-0001",
            )
            .expect("credentials"),
        )
        .expect("signer")
    }

    #[test]
    fn capability_binds_method_key_type_checksum_expiry_and_no_overwrite() {
        let signer_debug = format!("{:?}", signer());
        assert!(!signer_debug.contains("frame-test-access-key"));
        assert!(!signer_debug.contains("frame-test-secret-material"));
        let capability = signer()
            .sign_put(
                &staging(UPLOAD_A, "video/webm"),
                "video/webm",
                &"ab".repeat(32),
                1_024,
                1_721_065_845_999,
                300,
            )
            .expect("capability");
        assert_eq!(capability.method, "PUT");
        assert_eq!(capability.expires_at_ms, 1_721_066_145_000);
        assert!(capability.url.starts_with("https://0123456789abcdef0123456789abcdef.r2.cloudflarestorage.com/frame-recordings-production/uploads/"));
        assert!(capability.url.contains("X-Amz-Expires=300"));
        assert!(
            capability
                .url
                .contains("X-Amz-Content-Sha256=UNSIGNED-PAYLOAD")
        );
        assert!(capability.url.contains("X-Amz-Signature="));
        assert_eq!(
            capability.required_headers,
            vec![
                ("content-length".into(), "1024".into()),
                ("content-type".into(), "video/webm".into()),
                ("if-none-match".into(), "*".into()),
                (
                    "x-amz-checksum-sha256".into(),
                    "q6urq6urq6urq6urq6urq6urq6urq6urq6urq6urq6s=".into()
                ),
                ("x-amz-meta-frame-sha256".into(), "ab".repeat(32)),
            ]
        );
        let debug = format!("{capability:?}");
        assert!(!debug.contains("X-Amz-Signature"));
        assert!(!debug.contains(&"ab".repeat(32)));
    }

    #[test]
    fn every_security_boundary_changes_the_signature() {
        let baseline = signer()
            .sign_put(
                &staging(UPLOAD_A, "video/webm"),
                "video/webm",
                &"01".repeat(32),
                1_024,
                1_721_065_845_000,
                300,
            )
            .expect("baseline");
        let changes = [
            signer().sign_put(
                &staging(UPLOAD_B, "video/webm"),
                "video/webm",
                &"01".repeat(32),
                1_024,
                1_721_065_845_000,
                300,
            ),
            signer().sign_put(
                &staging(UPLOAD_A, "video/mp4"),
                "video/mp4",
                &"01".repeat(32),
                1_024,
                1_721_065_845_000,
                300,
            ),
            signer().sign_put(
                &staging(UPLOAD_A, "video/webm"),
                "video/webm",
                &"02".repeat(32),
                1_024,
                1_721_065_845_000,
                300,
            ),
            signer().sign_put(
                &staging(UPLOAD_A, "video/webm"),
                "video/webm",
                &"01".repeat(32),
                1_025,
                1_721_065_845_000,
                300,
            ),
            signer().sign_put(
                &staging(UPLOAD_A, "video/webm"),
                "video/webm",
                &"01".repeat(32),
                1_024,
                1_721_065_846_000,
                300,
            ),
        ];
        for changed in changes {
            assert_ne!(changed.expect("changed").url, baseline.url);
        }
    }

    #[test]
    fn signer_rejects_canonical_paths_unsafe_types_checksums_and_ttls() {
        for key in [
            "tenants/t/videos/v/source.webm",
            "uploads/t/staging/../escape",
            "uploads/t/staging/a/b",
            "/uploads/t/staging/a",
        ] {
            assert_eq!(
                signer().sign_put(
                    key,
                    "video/webm",
                    &"01".repeat(32),
                    1_024,
                    1_721_065_845_000,
                    300
                ),
                Err(DirectUploadSigningError::InvalidRequest)
            );
        }
        assert!(
            signer()
                .sign_put(
                    &staging(UPLOAD_A, "video/webm"),
                    "video/webm;evil=1",
                    &"01".repeat(32),
                    1_024,
                    1_721_065_845_000,
                    300
                )
                .is_err()
        );
        assert!(
            signer()
                .sign_put(
                    &staging(UPLOAD_A, "video/webm"),
                    "video/webm",
                    &"A1".repeat(32),
                    1_024,
                    1_721_065_845_000,
                    300
                )
                .is_err()
        );
        assert!(
            signer()
                .sign_put(
                    &staging(UPLOAD_A, "video/webm"),
                    "video/webm",
                    &"01".repeat(32),
                    1_024,
                    1_721_065_845_000,
                    901
                )
                .is_err()
        );
        for bytes in [0, MAX_DIRECT_UPLOAD_BYTES + 1] {
            assert!(
                signer()
                    .sign_put(
                        &staging(UPLOAD_A, "video/webm"),
                        "video/webm",
                        &"01".repeat(32),
                        bytes,
                        1_721_065_845_000,
                        300
                    )
                    .is_err()
            );
        }
    }

    #[test]
    fn private_staging_keys_are_randomized_tenant_opaque_and_type_bound() {
        let tenant = TENANT;
        let first_upload = UPLOAD_A;
        let second_upload = UPLOAD_B;
        let first = private_staging_key(tenant, first_upload, "video/webm").expect("key");
        let second = private_staging_key(tenant, second_upload, "video/webm").expect("key");
        assert_ne!(first, second);
        assert!(!first.contains(tenant));
        assert!(first.ends_with(&format!("/{first_upload}.webm")));
        assert_eq!(first.split('/').nth(1).expect("tenant scope").len(), 64);
        assert!(private_staging_key(tenant, first_upload, "text/plain").is_err());
        assert!(private_staging_key(&tenant.to_uppercase(), first_upload, "video/webm").is_err());
    }

    #[test]
    fn timestamp_and_hmac_primitives_have_known_answers() {
        assert_eq!(
            aws_timestamp(0),
            Err(DirectUploadSigningError::TimestampOutOfRange)
        );
        assert_eq!(
            aws_timestamp(1_721_065_845_000).expect("timestamp"),
            ("20240715".into(), "20240715T175045Z".into())
        );
        assert_eq!(
            hex(&hmac_sha256(
                b"key",
                b"The quick brown fox jumps over the lazy dog"
            )),
            "f7bc83f430538424b13298e6aa6fb143ef4d59a14946175997479dbc2d1a3cd8"
        );
    }
}
