//! Source-pinned contract for Cap's retained upload/storage RPCs, actions,
//! and stale-edit reconciliation workflow.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::legacy_mobile_iso_from_millis;

pub const LEGACY_UPLOAD_STORAGE_CAP_COMMIT: &str = "6ba69561ac86b8efdb17616d6727f9638015546b";
pub const LEGACY_GET_UPLOAD_PROGRESS_OPERATION_ID: &str = "cap-v1-e7c4af25d620a9b3";
pub const LEGACY_VIDEO_GET_DOWNLOAD_INFO_RPC_OPERATION_ID: &str = "cap-v1-43270700eca33966";
pub const LEGACY_VIDEO_UPLOAD_PROGRESS_UPDATE_OPERATION_ID: &str = "cap-v1-4245d3bd72f59e22";
pub const LEGACY_CREATE_VIDEO_UPLOAD_URL_OPERATION_ID: &str = "cap-v1-dd270efc913f9af9";
pub const LEGACY_DELETE_VIDEO_RESULT_OPERATION_ID: &str = "cap-v1-6ed7083eeb37e3f8";
pub const LEGACY_DOWNLOAD_VIDEO_OPERATION_ID: &str = "cap-v1-09c995d62aea0fe7";
pub const LEGACY_GET_VIDEO_DOWNLOAD_INFO_OPERATION_ID: &str = "cap-v1-cbc5472b81366dcb";
pub const LEGACY_RECONCILE_STALE_EDIT_UPLOAD_OPERATION_ID: &str = "cap-v1-d89571c3e0f65def";
pub const LEGACY_SHARE_CAP_OPERATION_ID: &str = "cap-v1-55d41a7742153f1b";
pub const LEGACY_UPLOAD_STORAGE_ACTION_SCHEMA_V1: &str =
    "frame.web-upload-storage-action-request.v1";
pub const LEGACY_UPLOAD_STORAGE_FIXTURE_SCHEMA_V1: &str = "frame.legacy-upload-storage.v1";
pub const LEGACY_UPLOAD_STORAGE_MAX_BODY_BYTES: usize = 8 * 1024 * 1024;
pub const LEGACY_UPLOAD_STORAGE_CAPABILITY_TTL_SECONDS: u32 = 3_600;
pub const LEGACY_STALE_EDIT_THUMBNAIL_MS: i64 = 5 * 60 * 1_000;
pub const LEGACY_STALE_EDIT_PROCESSING_PROGRESS_MS: i64 = 10 * 60 * 1_000;
pub const LEGACY_STALE_EDIT_PROCESSING_START_MS: i64 = 15 * 60 * 1_000;
pub const LEGACY_UPLOAD_STORAGE_NO_PROTECTED_GATES: &[&str] = &[];
pub const LEGACY_GET_UPLOAD_PROGRESS_SOURCE_MANIFEST_SHA256: &str =
    "cf23aeb83b2a69f2533bb5f2bd6ea014e7b59cdc880134ef29eaa6c29e3736f5";
pub const LEGACY_VIDEO_GET_DOWNLOAD_INFO_RPC_SOURCE_MANIFEST_SHA256: &str =
    "db5d1f077a062da7e631624473a96176de6fd1e5825b24124ef55037d59c2c52";
pub const LEGACY_VIDEO_UPLOAD_PROGRESS_UPDATE_SOURCE_MANIFEST_SHA256: &str =
    "61b186fa5111e86cd88c78e7535571e1e2d7576f981c202cee7087a0aed4eb53";
pub const LEGACY_CREATE_VIDEO_UPLOAD_URL_SOURCE_MANIFEST_SHA256: &str =
    "9a664fd3fdd7b607bc3abebcd8f17ce24e9d0b59a4c668fd5e2a4e2a33530155";
pub const LEGACY_DELETE_VIDEO_RESULT_SOURCE_MANIFEST_SHA256: &str =
    "6f37d3f442c9819c803f0b9f17079d3d049f05697f592229c34b735911a7f2bc";
pub const LEGACY_DOWNLOAD_VIDEO_SOURCE_MANIFEST_SHA256: &str =
    "a76cee98c2af8e2e6694ff7194fed93c63ba8d6caea6ecd0a4112d6cc8b07133";
pub const LEGACY_GET_VIDEO_DOWNLOAD_INFO_SOURCE_MANIFEST_SHA256: &str =
    "4c6591dd4f6e300256f9ebc8ef333e72c579ecf2b033c95fe1ba8d4ce8741bdd";
pub const LEGACY_RECONCILE_STALE_EDIT_UPLOAD_SOURCE_MANIFEST_SHA256: &str =
    "d54e3fb23770a4d8317ea49f665c9e72bb1faebe51ef9fa9036a5de69494ab6a";
pub const LEGACY_SHARE_CAP_SOURCE_MANIFEST_SHA256: &str =
    "7e163b7de3f3b1e1a51525ca46411c8925fe1898c368a5fa91f5886eaf30a341";

pub const LEGACY_UPLOAD_STORAGE_OPERATION_IDS: &[&str] = &[
    LEGACY_GET_UPLOAD_PROGRESS_OPERATION_ID,
    LEGACY_VIDEO_GET_DOWNLOAD_INFO_RPC_OPERATION_ID,
    LEGACY_VIDEO_UPLOAD_PROGRESS_UPDATE_OPERATION_ID,
    LEGACY_CREATE_VIDEO_UPLOAD_URL_OPERATION_ID,
    LEGACY_DELETE_VIDEO_RESULT_OPERATION_ID,
    LEGACY_DOWNLOAD_VIDEO_OPERATION_ID,
    LEGACY_GET_VIDEO_DOWNLOAD_INFO_OPERATION_ID,
    LEGACY_RECONCILE_STALE_EDIT_UPLOAD_OPERATION_ID,
    LEGACY_SHARE_CAP_OPERATION_ID,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyUploadStorageActionV1 {
    CreateVideoAndGetUploadUrl,
    DeleteVideoResultFile,
    DownloadVideo,
    GetVideoDownloadInfo,
    ReconcileStaleEditUpload,
    ShareCap,
}

impl LegacyUploadStorageActionV1 {
    #[must_use]
    pub fn parse(operation_id: &str) -> Option<Self> {
        match operation_id {
            LEGACY_CREATE_VIDEO_UPLOAD_URL_OPERATION_ID => Some(Self::CreateVideoAndGetUploadUrl),
            LEGACY_DELETE_VIDEO_RESULT_OPERATION_ID => Some(Self::DeleteVideoResultFile),
            LEGACY_DOWNLOAD_VIDEO_OPERATION_ID => Some(Self::DownloadVideo),
            LEGACY_GET_VIDEO_DOWNLOAD_INFO_OPERATION_ID => Some(Self::GetVideoDownloadInfo),
            LEGACY_RECONCILE_STALE_EDIT_UPLOAD_OPERATION_ID => Some(Self::ReconcileStaleEditUpload),
            LEGACY_SHARE_CAP_OPERATION_ID => Some(Self::ShareCap),
            _ => None,
        }
    }

    #[must_use]
    pub const fn mutation(self) -> bool {
        matches!(
            self,
            Self::CreateVideoAndGetUploadUrl
                | Self::DeleteVideoResultFile
                | Self::ReconcileStaleEditUpload
                | Self::ShareCap
        )
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LegacyCreateVideoUploadInputV1 {
    pub video_id: Option<String>,
    pub duration: Option<f64>,
    pub resolution: Option<String>,
    pub video_codec: Option<String>,
    pub audio_codec: Option<String>,
    #[serde(default)]
    pub is_screenshot: bool,
    #[serde(default)]
    pub is_upload: bool,
    pub folder_id: Option<String>,
    pub org_id: String,
    pub screenshot_content_type: Option<String>,
    #[serde(default)]
    pub supports_upload_progress: bool,
}

impl LegacyCreateVideoUploadInputV1 {
    #[must_use]
    pub fn valid(&self) -> bool {
        self.video_id.as_deref().is_none_or(valid_cap_id)
            && valid_cap_id(&self.org_id)
            && self.folder_id.as_deref().is_none_or(valid_cap_id)
            && self.duration.is_none_or(f64::is_finite)
            && self
                .resolution
                .as_deref()
                .is_none_or(|v| valid_text(v, 255))
            && self
                .video_codec
                .as_deref()
                .is_none_or(|v| valid_text(v, 255))
            && self
                .audio_codec
                .as_deref()
                .is_none_or(|v| valid_text(v, 255))
            && self
                .screenshot_content_type
                .as_deref()
                .is_none_or(|v| valid_text(v, 127))
    }

    #[must_use]
    pub fn object_suffix(&self) -> &'static str {
        if self.is_screenshot {
            if self
                .screenshot_content_type
                .as_deref()
                .is_some_and(|v| v.eq_ignore_ascii_case("image/png"))
            {
                "screenshot/screen-capture.png"
            } else {
                "screenshot/screen-capture.jpg"
            }
        } else {
            "result.mp4"
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LegacyUploadProgressUpdateInputV1 {
    pub video_id: String,
    pub uploaded: u64,
    pub total: u64,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyUploadProgressV1 {
    pub uploaded: u64,
    pub total: u64,
    pub started_at: String,
    pub updated_at: String,
    pub phase: String,
    pub processing_progress: u64,
    pub processing_message: LegacyEffectOptionV1<String>,
    pub processing_error: LegacyEffectOptionV1<String>,
    pub has_raw_fallback: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "_tag")]
pub enum LegacyEffectOptionV1<T> {
    None,
    Some { value: T },
}

impl<T> From<Option<T>> for LegacyEffectOptionV1<T> {
    fn from(value: Option<T>) -> Self {
        value.map_or(Self::None, |value| Self::Some { value })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyUploadTargetV1 {
    #[serde(rename = "type")]
    pub target_type: String,
    pub url: String,
    pub headers: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyCreateVideoUploadResultV1 {
    pub id: String,
    pub presigned_post_data: Option<serde_json::Value>,
    pub upload_target: LegacyUploadTargetV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyDownloadSuccessV1 {
    pub success: bool,
    pub download_url: String,
    pub filename: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum LegacyDownloadInfoResultV1 {
    Success(LegacyDownloadSuccessV1),
    Failure { success: bool, error: String },
}

#[must_use]
pub fn legacy_upload_content_type(key: &str) -> &'static str {
    let key = key.to_ascii_lowercase();
    if key.ends_with(".aac") {
        "audio/aac"
    } else if key.ends_with(".webm") {
        "audio/webm"
    } else if key.ends_with(".mp4") {
        "video/mp4"
    } else if key.ends_with(".jpg") || key.ends_with(".jpeg") {
        "image/jpeg"
    } else if key.ends_with(".png") {
        "image/png"
    } else if key.ends_with(".mp3") {
        "audio/mpeg"
    } else if key.ends_with(".m3u8") {
        "application/x-mpegURL"
    } else {
        "video/mp2t"
    }
}

#[must_use]
pub fn legacy_create_video_title(
    now_ms: i64,
    is_screenshot: bool,
    is_upload: bool,
) -> Option<String> {
    let iso = legacy_mobile_iso_from_millis(now_ms)?;
    let year = &iso[0..4];
    let month_number = iso[5..7].parse::<usize>().ok()?;
    let day = iso[8..10].parse::<u8>().ok()?;
    let month = [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ]
    .get(month_number.checked_sub(1)?)?;
    let kind = if is_screenshot {
        "Screenshot"
    } else if is_upload {
        "Upload"
    } else {
        "Recording"
    };
    Some(format!("Cap {kind} - {day} {month} {year}"))
}

#[must_use]
pub fn legacy_should_clear_edit_upload(
    phase: &str,
    processing_progress: f64,
    updated_at_ms: i64,
    now_ms: i64,
) -> bool {
    if phase == "error" {
        return true;
    }
    let age_ms = now_ms.saturating_sub(updated_at_ms);
    match phase {
        "complete" | "generating_thumbnail" => age_ms > LEGACY_STALE_EDIT_THUMBNAIL_MS,
        "processing" if processing_progress == 0.0 => {
            age_ms > LEGACY_STALE_EDIT_PROCESSING_START_MS
        }
        "processing" => age_ms > LEGACY_STALE_EDIT_PROCESSING_PROGRESS_MS,
        _ => false,
    }
}

/// Parse the canonical millisecond ISO representation emitted by Effect's
/// Date schema without relying on a JavaScript host during native tests.
#[must_use]
pub fn legacy_upload_storage_iso_millis(value: &str) -> Option<i64> {
    let bytes = value.as_bytes();
    if bytes.len() != 24
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
        || bytes[19] != b'.'
        || bytes[23] != b'Z'
    {
        return None;
    }
    let number = |start: usize, end: usize| -> Option<i64> {
        bytes
            .get(start..end)?
            .iter()
            .try_fold(0_i64, |value, byte| {
                byte.is_ascii_digit()
                    .then_some(value * 10 + i64::from(byte - b'0'))
            })
    };
    let year = number(0, 4)?;
    let month = number(5, 7)?;
    let day = number(8, 10)?;
    let hour = number(11, 13)?;
    let minute = number(14, 16)?;
    let second = number(17, 19)?;
    let millis = number(20, 23)?;
    if year == 0 || !(1..=12).contains(&month) || hour > 23 || minute > 59 || second > 59 {
        return None;
    }
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days_in_month = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    if day == 0 || day > days_in_month[usize::try_from(month - 1).ok()?] {
        return None;
    }
    let adjusted_year = year - i64::from(month <= 2);
    let era = adjusted_year.div_euclid(400);
    let year_of_era = adjusted_year - era * 400;
    let adjusted_month = month + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * adjusted_month + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    let days = era * 146_097 + day_of_era - 719_468;
    let result = days
        .checked_mul(86_400_000)?
        .checked_add(hour * 3_600_000 + minute * 60_000 + second * 1_000 + millis)?;
    (0..=9_007_199_254_740_991)
        .contains(&result)
        .then_some(result)
}

#[must_use]
pub fn valid_cap_id(value: &str) -> bool {
    value.len() == 15
        && value
            .bytes()
            .all(|byte| b"0123456789abcdefghjkmnpqrstvwxyz".contains(&byte))
}

fn valid_text(value: &str, maximum: usize) -> bool {
    value.len() <= maximum && !value.chars().any(char::is_control)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suffix_title_and_staleness_match_cap() {
        let input = LegacyCreateVideoUploadInputV1 {
            video_id: None,
            duration: None,
            resolution: None,
            video_codec: None,
            audio_codec: None,
            is_screenshot: true,
            is_upload: false,
            folder_id: None,
            org_id: "0123456789abcde".into(),
            screenshot_content_type: Some("IMAGE/PNG".into()),
            supports_upload_progress: false,
        };
        assert!(input.valid());
        assert_eq!(input.object_suffix(), "screenshot/screen-capture.png");
        assert_eq!(
            legacy_upload_content_type(input.object_suffix()),
            "image/png"
        );
        assert_eq!(
            legacy_create_video_title(1_735_787_045_006, false, true).as_deref(),
            Some("Cap Upload - 2 January 2025")
        );
        let now = 1_000_000;
        assert!(!legacy_should_clear_edit_upload(
            "processing",
            0.0,
            now - LEGACY_STALE_EDIT_PROCESSING_START_MS,
            now
        ));
        assert!(legacy_should_clear_edit_upload(
            "processing",
            0.0,
            now - LEGACY_STALE_EDIT_PROCESSING_START_MS - 1,
            now
        ));
        assert!(legacy_should_clear_edit_upload("error", 1.0, now, now));
        assert!(!legacy_should_clear_edit_upload("uploading", 0.0, 0, now));
        assert_eq!(
            legacy_upload_storage_iso_millis("2025-01-02T03:04:05.006Z"),
            Some(1_735_787_045_006)
        );
    }
}
