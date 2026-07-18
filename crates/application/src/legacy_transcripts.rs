//! Source-pinned compatibility contract for Cap transcript reads, edits, retry, and translation.
//!
//! Cap stores WebVTT beside a video's media under the selected storage prefix.
//! Reads and translations use the optional-auth public video policy, while edit
//! and retry are owner-only. Translation remains a protected provider effect;
//! the local contract freezes validation, cache keys, and durable orchestration
//! without pretending that a Groq response was produced.

use std::fmt::Write as _;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const LEGACY_TRANSCRIPTS_CAP_COMMIT: &str = "6ba69561ac86b8efdb17616d6727f9638015546b";

pub const LEGACY_RETRY_TRANSCRIPTION_OPERATION_ID: &str = "cap-v1-c8dffb9b102dd4f7";
pub const LEGACY_EDIT_TRANSCRIPT_OPERATION_ID: &str = "cap-v1-3db394ae13895b46";
pub const LEGACY_GET_TRANSCRIPT_OPERATION_ID: &str = "cap-v1-f2659b43d5ee9162";
pub const LEGACY_TRANSLATE_TRANSCRIPT_OPERATION_ID: &str = "cap-v1-6f6ece85bd786289";
pub const LEGACY_AVAILABLE_TRANSLATIONS_OPERATION_ID: &str = "cap-v1-6c82f3cbe383d92b";

pub const LEGACY_RETRY_TRANSCRIPTION_IDENTITY: &str = "/api/videos/:videoId/retry-transcription";
pub const LEGACY_EDIT_TRANSCRIPT_IDENTITY: &str =
    "action://apps/web/actions/videos/edit-transcript.ts#editTranscriptEntry";
pub const LEGACY_GET_TRANSCRIPT_IDENTITY: &str =
    "action://apps/web/actions/videos/get-transcript.ts#getTranscript";
pub const LEGACY_TRANSLATE_TRANSCRIPT_IDENTITY: &str =
    "action://apps/web/actions/videos/translate-transcript.ts#translateTranscript";
pub const LEGACY_AVAILABLE_TRANSLATIONS_IDENTITY: &str =
    "action://apps/web/actions/videos/get-available-translations.ts#getAvailableTranslations";

pub const LEGACY_TRANSCRIPT_POLICY: &str = "collaboration_notifications.v1";
pub const LEGACY_TRANSCRIPT_MAX_BODY_BYTES: usize = 256 * 1024;
pub const LEGACY_TRANSCRIPT_MAX_OBJECT_BYTES: u64 = 8 * 1024 * 1024;
pub const LEGACY_TRANSCRIPT_NO_PROTECTED_GATES: &[&str] = &[];
pub const LEGACY_TRANSCRIPT_TRANSLATION_PROTECTED_GATES: &[&str] = &["provider_execution"];
pub const LEGACY_TRANSCRIPT_ACTION_SCHEMA_V1: &str = "frame.web-transcript-action-request.v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyTranscriptSourceRoleV1 {
    Handler,
    Authentication,
    Authorization,
    PersistenceSchema,
    Database,
    VideoIdentifier,
    Storage,
    VttEditor,
    StorageDecoder,
    LanguageCatalog,
    Provider,
    RateLimit,
    EmailRestriction,
    ApiMiddlewareExclusion,
    DependencyDeclaration,
    DependencyLock,
}

impl LegacyTranscriptSourceRoleV1 {
    #[must_use]
    pub const fn stable_code(self) -> &'static str {
        match self {
            Self::Handler => "handler",
            Self::Authentication => "authentication",
            Self::Authorization => "authorization",
            Self::PersistenceSchema => "persistence_schema",
            Self::Database => "database",
            Self::VideoIdentifier => "video_identifier",
            Self::Storage => "storage",
            Self::VttEditor => "vtt_editor",
            Self::StorageDecoder => "storage_decoder",
            Self::LanguageCatalog => "language_catalog",
            Self::Provider => "provider",
            Self::RateLimit => "rate_limit",
            Self::EmailRestriction => "email_restriction",
            Self::ApiMiddlewareExclusion => "api_middleware_exclusion",
            Self::DependencyDeclaration => "dependency_declaration",
            Self::DependencyLock => "dependency_lock",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyTranscriptSourcePinV1 {
    pub path: &'static str,
    pub symbol: &'static str,
    pub sha256: &'static str,
    pub role: LegacyTranscriptSourceRoleV1,
}

const SESSION: LegacyTranscriptSourcePinV1 = LegacyTranscriptSourcePinV1 {
    path: "packages/database/auth/session.ts",
    symbol: "getCurrentUser",
    sha256: "d526dc9d7a6a1a7cb6a8695c24ab88b843ce09b4444f8e4ade24b7a06cbbc1ee",
    role: LegacyTranscriptSourceRoleV1::Authentication,
};
const SCHEMA: LegacyTranscriptSourcePinV1 = LegacyTranscriptSourcePinV1 {
    path: "packages/database/schema.ts",
    symbol: "videos+organizations+spaces+memberships",
    sha256: "7fce297f9076be78a9ac6280d9d060bf6e836a62e0f82b5390fa0e42dc7bb9e9",
    role: LegacyTranscriptSourceRoleV1::PersistenceSchema,
};
const DATABASE: LegacyTranscriptSourcePinV1 = LegacyTranscriptSourcePinV1 {
    path: "packages/database/index.ts",
    symbol: "db",
    sha256: "161c1d1fd2a561fd2846aeceb148f24b58afc58bdaa95175240e48dbe61d9bbb",
    role: LegacyTranscriptSourceRoleV1::Database,
};
const VIDEO: LegacyTranscriptSourcePinV1 = LegacyTranscriptSourcePinV1 {
    path: "packages/web-domain/src/Video.ts",
    symbol: "Video.VideoId+verifyPasswordCandidates",
    sha256: "adc3db0eded2670b1ed89969e7bc85993e04021acb303082b3d015f0afb1c9a7",
    role: LegacyTranscriptSourceRoleV1::VideoIdentifier,
};
const STORAGE: LegacyTranscriptSourcePinV1 = LegacyTranscriptSourcePinV1 {
    path: "packages/web-backend/src/Storage/index.ts",
    symbol: "Storage.getAccessForVideo+translation objects",
    sha256: "3ea22f76907104e26df8f48bdcac87a5dc2d3d60497dfc409110eb0fa8446b4c",
    role: LegacyTranscriptSourceRoleV1::Storage,
};
const STORAGE_DECODER: LegacyTranscriptSourcePinV1 = LegacyTranscriptSourcePinV1 {
    path: "apps/web/lib/video-storage.ts",
    symbol: "decodeStorageVideo",
    sha256: "bc40152ad191ba397a1900016ca04f2b47c6fe07063c5abe82455a9d98d13714",
    role: LegacyTranscriptSourceRoleV1::StorageDecoder,
};
const OPTIONAL_AUTH: LegacyTranscriptSourcePinV1 = LegacyTranscriptSourcePinV1 {
    path: "packages/web-backend/src/Auth.ts",
    symbol: "provideOptionalAuth",
    sha256: "aea054db2b84a8c4bd6684fefe8d0e971a094a9faa9653105b0c33ab52ab824d",
    role: LegacyTranscriptSourceRoleV1::Authentication,
};
const VIDEO_POLICY: LegacyTranscriptSourcePinV1 = LegacyTranscriptSourcePinV1 {
    path: "packages/web-backend/src/Videos/VideosPolicy.ts",
    symbol: "VideosPolicy.canView+buildCanView",
    sha256: "39e4b55f59e0758450d76401706cb2d258c8fe850fef91f395662df9146f7540",
    role: LegacyTranscriptSourceRoleV1::Authorization,
};
const EFFECTIVE_RULES: LegacyTranscriptSourcePinV1 = LegacyTranscriptSourcePinV1 {
    path: "packages/web-backend/src/Videos/EffectiveVideoRules.ts",
    symbol: "collectPasswordHashes",
    sha256: "e9b26784e4a1ed5782f9a5cfab52231de629b2f0a3d1b5f40d577b3c798cd015",
    role: LegacyTranscriptSourceRoleV1::Authorization,
};
const EMAIL_RESTRICTION: LegacyTranscriptSourcePinV1 = LegacyTranscriptSourcePinV1 {
    path: "packages/utils/src/helpers.ts",
    symbol: "isEmailAllowedByRestriction",
    sha256: "58e55441727cd5ddf82bd0755e4b9923074d8fa68804d14875892d21cd077621",
    role: LegacyTranscriptSourceRoleV1::EmailRestriction,
};
const PROXY: LegacyTranscriptSourcePinV1 = LegacyTranscriptSourcePinV1 {
    path: "apps/web/proxy.ts",
    symbol: "API matcher exclusion",
    sha256: "7da98445a31f6b48d01b56877c47aaa79ba3af93dff8c015ad06a6e94fb42fcb",
    role: LegacyTranscriptSourceRoleV1::ApiMiddlewareExclusion,
};
const PACKAGE: LegacyTranscriptSourcePinV1 = LegacyTranscriptSourcePinV1 {
    path: "apps/web/package.json",
    symbol: "next+effect+groq+storage dependencies",
    sha256: "c1358cd1880ac5dc9d659760c2788cedd5c4f61fec2cb0dd1b60cbc9bb8af920",
    role: LegacyTranscriptSourceRoleV1::DependencyDeclaration,
};
const LOCK: LegacyTranscriptSourcePinV1 = LegacyTranscriptSourcePinV1 {
    path: "pnpm-lock.yaml",
    symbol: "dependency lock",
    sha256: "fc0fe122ae5fbea4dcaa7e510bd6275635c14071c4e031996431a54fb7e25e3a",
    role: LegacyTranscriptSourceRoleV1::DependencyLock,
};

pub const LEGACY_RETRY_TRANSCRIPTION_SOURCES: &[LegacyTranscriptSourcePinV1] = &[
    LegacyTranscriptSourcePinV1 {
        path: "apps/web/app/api/videos/[videoId]/retry-transcription/route.ts",
        symbol: "POST",
        sha256: "a443e83d8bbf243e1661cfaf9795c1d901e650c7f773493aad885e9c90ab54e3",
        role: LegacyTranscriptSourceRoleV1::Handler,
    },
    SESSION,
    SCHEMA,
    DATABASE,
    VIDEO,
    PROXY,
    PACKAGE,
    LOCK,
];

pub const LEGACY_EDIT_TRANSCRIPT_SOURCES: &[LegacyTranscriptSourcePinV1] = &[
    LegacyTranscriptSourcePinV1 {
        path: "apps/web/actions/videos/edit-transcript.ts",
        symbol: "editTranscriptEntry",
        sha256: "f9e1ae7841e79c58c98fd5087f375f8d5d6c0c0978d6eff40e477ed4c6266da9",
        role: LegacyTranscriptSourceRoleV1::Handler,
    },
    SESSION,
    SCHEMA,
    DATABASE,
    VIDEO,
    LegacyTranscriptSourcePinV1 {
        path: "apps/web/lib/transcript-vtt.ts",
        symbol: "normalizeTranscriptCueText+updateVttEntryText",
        sha256: "972027f6c6a72e9e9ccd820ba86c409f008d02a3f60349cd3a50ff7f4ed3be88",
        role: LegacyTranscriptSourceRoleV1::VttEditor,
    },
    STORAGE,
    STORAGE_DECODER,
    PROXY,
    PACKAGE,
    LOCK,
];

pub const LEGACY_GET_TRANSCRIPT_SOURCES: &[LegacyTranscriptSourcePinV1] = &[
    LegacyTranscriptSourcePinV1 {
        path: "apps/web/actions/videos/get-transcript.ts",
        symbol: "getTranscript",
        sha256: "7edf7bbf932c1ca32053e8047a71ac988fc49cede88e2aa4793762d6b0302adb",
        role: LegacyTranscriptSourceRoleV1::Handler,
    },
    SESSION,
    SCHEMA,
    DATABASE,
    VIDEO,
    OPTIONAL_AUTH,
    VIDEO_POLICY,
    EFFECTIVE_RULES,
    EMAIL_RESTRICTION,
    STORAGE,
    STORAGE_DECODER,
    PROXY,
    PACKAGE,
    LOCK,
];

pub const LEGACY_TRANSLATE_TRANSCRIPT_SOURCES: &[LegacyTranscriptSourcePinV1] = &[
    LegacyTranscriptSourcePinV1 {
        path: "apps/web/actions/videos/translate-transcript.ts",
        symbol: "translateTranscript",
        sha256: "8a58b5af8366fb212b64bbb68233477938675248b9f19b4a359a5feabb1b73c1",
        role: LegacyTranscriptSourceRoleV1::Handler,
    },
    SCHEMA,
    DATABASE,
    VIDEO,
    LegacyTranscriptSourcePinV1 {
        path: "packages/web-domain/src/Language.ts",
        symbol: "SUPPORTED_LANGUAGES+LanguageCode",
        sha256: "663875a0fade53bc441b9c6ee8eda1ecb706235b6ab16c71b4899c4724095dfb",
        role: LegacyTranscriptSourceRoleV1::LanguageCatalog,
    },
    LegacyTranscriptSourcePinV1 {
        path: "apps/web/actions/videos/translation-languages.ts",
        symbol: "SUPPORTED_LANGUAGES re-export",
        sha256: "7e01211fe74456afd82d196416ead4008e27024c57e0d1dbd12fc0a0334b3e2d",
        role: LegacyTranscriptSourceRoleV1::LanguageCatalog,
    },
    OPTIONAL_AUTH,
    VIDEO_POLICY,
    EFFECTIVE_RULES,
    EMAIL_RESTRICTION,
    STORAGE,
    LegacyTranscriptSourcePinV1 {
        path: "packages/web-backend/src/Storage/index.ts",
        symbol: "Storage translation objects",
        sha256: "3ea22f76907104e26df8f48bdcac87a5dc2d3d60497dfc409110eb0fa8446b4c",
        role: LegacyTranscriptSourceRoleV1::Storage,
    },
    STORAGE_DECODER,
    LegacyTranscriptSourcePinV1 {
        path: "apps/web/lib/groq-client.ts",
        symbol: "getGroqClient+GROQ_MODEL",
        sha256: "0a7e75e482958d8392df564f5cb5d0029dc3b4361883d7b5206aa19fc5ba2702",
        role: LegacyTranscriptSourceRoleV1::Provider,
    },
    LegacyTranscriptSourcePinV1 {
        path: "apps/web/lib/groq-client.ts",
        symbol: "getGroqClient",
        sha256: "0a7e75e482958d8392df564f5cb5d0029dc3b4361883d7b5206aa19fc5ba2702",
        role: LegacyTranscriptSourceRoleV1::Provider,
    },
    LegacyTranscriptSourcePinV1 {
        path: "apps/web/lib/rate-limit.ts",
        symbol: "isRateLimited+RATE_LIMIT_IDS.TRANSLATE_TRANSCRIPT",
        sha256: "fc94007b1bcbfb1ebc09ca95aa0661e50816089c713b102f885c2448e42579e9",
        role: LegacyTranscriptSourceRoleV1::RateLimit,
    },
    PROXY,
    PACKAGE,
    LOCK,
];

pub const LEGACY_AVAILABLE_TRANSLATIONS_SOURCES: &[LegacyTranscriptSourcePinV1] = &[
    LegacyTranscriptSourcePinV1 {
        path: "apps/web/actions/videos/get-available-translations.ts",
        symbol: "getAvailableTranslations",
        sha256: "4aa195266716146cc5c87dbc39a4d30f27ed32747b1474e990e231bd9f81a921",
        role: LegacyTranscriptSourceRoleV1::Handler,
    },
    SESSION,
    SCHEMA,
    DATABASE,
    VIDEO,
    LegacyTranscriptSourcePinV1 {
        path: "packages/web-domain/src/Language.ts",
        symbol: "SUPPORTED_LANGUAGES+LanguageCode",
        sha256: "663875a0fade53bc441b9c6ee8eda1ecb706235b6ab16c71b4899c4724095dfb",
        role: LegacyTranscriptSourceRoleV1::LanguageCatalog,
    },
    LegacyTranscriptSourcePinV1 {
        path: "apps/web/actions/videos/translation-languages.ts",
        symbol: "SUPPORTED_LANGUAGES re-export",
        sha256: "7e01211fe74456afd82d196416ead4008e27024c57e0d1dbd12fc0a0334b3e2d",
        role: LegacyTranscriptSourceRoleV1::LanguageCatalog,
    },
    OPTIONAL_AUTH,
    VIDEO_POLICY,
    EFFECTIVE_RULES,
    EMAIL_RESTRICTION,
    LegacyTranscriptSourcePinV1 {
        path: "packages/web-domain/src/Policy.ts",
        symbol: "Policy.withPublicPolicy",
        sha256: "0621949aa1f994836d0d168b39dc3aada3ad0478052b712de564b105c94ebe5c",
        role: LegacyTranscriptSourceRoleV1::Authorization,
    },
    STORAGE,
    STORAGE_DECODER,
    LegacyTranscriptSourcePinV1 {
        path: "apps/web/lib/server.ts",
        symbol: "runPromise+runPromiseExit",
        sha256: "f24b68bbb31c99ddfa7983a468aa80d293da56d1652e8a0a0a28506e5a9cd63e",
        role: LegacyTranscriptSourceRoleV1::Database,
    },
    PROXY,
    PACKAGE,
    LOCK,
];

pub const LEGACY_RETRY_TRANSCRIPTION_SOURCE_MANIFEST_SHA256: &str =
    "57293fe5ddca7d4d661d2ae05ece50722976909d6af70426689f1b3102ebd5e9";
pub const LEGACY_EDIT_TRANSCRIPT_SOURCE_MANIFEST_SHA256: &str =
    "d4c854287c834e13c237d86c0568140f580f1dd86de0b9262fa7505bf5455ce7";
pub const LEGACY_GET_TRANSCRIPT_SOURCE_MANIFEST_SHA256: &str =
    "388836a7cccbd3d52e0a500afadeda9ba8d1d2eaaded1ddb63e750ad00ab0374";
pub const LEGACY_TRANSLATE_TRANSCRIPT_SOURCE_MANIFEST_SHA256: &str =
    "8da125a543bd855db93d39326c57b5391bda2a4fb081699cd732a3a6bb2e875f";
pub const LEGACY_AVAILABLE_TRANSLATIONS_SOURCE_MANIFEST_SHA256: &str =
    "e035c20f41879ae831e66d2ed9ea3af1365d979a794ec8065d9862605fef8c35";

#[must_use]
pub fn legacy_transcript_source_manifest(sources: &[LegacyTranscriptSourcePinV1]) -> String {
    let mut digest = Sha256::new();
    digest.update(b"frame-cap-transcript-source-manifest-v1\0");
    for source in sources {
        digest.update(source.path.as_bytes());
        digest.update([0]);
        digest.update(source.symbol.as_bytes());
        digest.update([0]);
        digest.update(source.sha256.as_bytes());
        digest.update([0]);
        digest.update(source.role.stable_code().as_bytes());
        digest.update(b"\n");
    }
    let mut encoded = String::with_capacity(64);
    for byte in digest.finalize() {
        write!(&mut encoded, "{byte:02x}").expect("write digest");
    }
    encoded
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyTranscriptSurfaceV1 {
    Retry,
    Edit,
    Get,
    Translate,
    AvailableTranslations,
}

impl LegacyTranscriptSurfaceV1 {
    #[must_use]
    pub fn parse(operation_id: &str) -> Option<Self> {
        match operation_id {
            LEGACY_RETRY_TRANSCRIPTION_OPERATION_ID => Some(Self::Retry),
            LEGACY_EDIT_TRANSCRIPT_OPERATION_ID => Some(Self::Edit),
            LEGACY_GET_TRANSCRIPT_OPERATION_ID => Some(Self::Get),
            LEGACY_TRANSLATE_TRANSCRIPT_OPERATION_ID => Some(Self::Translate),
            LEGACY_AVAILABLE_TRANSLATIONS_OPERATION_ID => Some(Self::AvailableTranslations),
            _ => None,
        }
    }

    #[must_use]
    pub const fn operation_id(self) -> &'static str {
        match self {
            Self::Retry => LEGACY_RETRY_TRANSCRIPTION_OPERATION_ID,
            Self::Edit => LEGACY_EDIT_TRANSCRIPT_OPERATION_ID,
            Self::Get => LEGACY_GET_TRANSCRIPT_OPERATION_ID,
            Self::Translate => LEGACY_TRANSLATE_TRANSCRIPT_OPERATION_ID,
            Self::AvailableTranslations => LEGACY_AVAILABLE_TRANSLATIONS_OPERATION_ID,
        }
    }

    #[must_use]
    pub const fn identity(self) -> &'static str {
        match self {
            Self::Retry => LEGACY_RETRY_TRANSCRIPTION_IDENTITY,
            Self::Edit => LEGACY_EDIT_TRANSCRIPT_IDENTITY,
            Self::Get => LEGACY_GET_TRANSCRIPT_IDENTITY,
            Self::Translate => LEGACY_TRANSLATE_TRANSCRIPT_IDENTITY,
            Self::AvailableTranslations => LEGACY_AVAILABLE_TRANSLATIONS_IDENTITY,
        }
    }

    #[must_use]
    pub const fn auth(self) -> &'static str {
        match self {
            Self::Retry | Self::Edit => "session_owner",
            Self::Get | Self::Translate | Self::AvailableTranslations => {
                "optional_session_public_video_policy"
            }
        }
    }

    #[must_use]
    pub const fn idempotency(self) -> &'static str {
        match self {
            Self::Get | Self::AvailableTranslations => "forbidden",
            Self::Retry | Self::Edit | Self::Translate => "required_frame_carrier",
        }
    }

    #[must_use]
    pub const fn sources(self) -> &'static [LegacyTranscriptSourcePinV1] {
        match self {
            Self::Retry => LEGACY_RETRY_TRANSCRIPTION_SOURCES,
            Self::Edit => LEGACY_EDIT_TRANSCRIPT_SOURCES,
            Self::Get => LEGACY_GET_TRANSCRIPT_SOURCES,
            Self::Translate => LEGACY_TRANSLATE_TRANSCRIPT_SOURCES,
            Self::AvailableTranslations => LEGACY_AVAILABLE_TRANSLATIONS_SOURCES,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyAvailableTranslationV1 {
    pub code: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyAvailableTranslationsResultV1 {
    pub success: bool,
    pub has_original: bool,
    pub translations: Vec<LegacyAvailableTranslationV1>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl LegacyAvailableTranslationsResultV1 {
    #[must_use]
    pub fn failure(message: impl Into<String>) -> Self {
        Self {
            success: false,
            has_original: false,
            translations: Vec::new(),
            message: Some(message.into()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyTranscriptResultV1 {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub translated_vtt: Option<String>,
    pub message: String,
}

impl LegacyTranscriptResultV1 {
    #[must_use]
    pub fn failure(message: impl Into<String>) -> Self {
        Self {
            success: false,
            content: None,
            translated_vtt: None,
            message: message.into(),
        }
    }
}

/// Exact Rust port of `updateVttEntryText`, including CRLF normalization and
/// replacement of every cue whose numeric identifier matches `entry_id`.
#[must_use]
pub fn legacy_update_vtt_entry_text(
    vtt_content: &str,
    entry_id: u64,
    new_text: &str,
) -> (String, bool) {
    let normalized = new_text.split_whitespace().collect::<Vec<_>>().join(" ");
    let lines = vtt_content
        .split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line))
        .collect::<Vec<_>>();
    let mut output = Vec::with_capacity(lines.len());
    let mut index = 0;
    let mut updated = false;
    while index < lines.len() {
        let line = lines[index];
        let trimmed = line.trim();
        if trimmed.is_empty() || !trimmed.bytes().all(|byte| byte.is_ascii_digit()) {
            output.push(line.to_owned());
            index += 1;
            continue;
        }
        let cue_id = trimmed.parse::<u64>().ok();
        let cue_start = index;
        let mut cue_end = cue_start + 1;
        while cue_end < lines.len() && !lines[cue_end].trim().is_empty() {
            cue_end += 1;
        }
        if cue_id != Some(entry_id) {
            output.extend(lines[cue_start..cue_end].iter().map(ToString::to_string));
        } else {
            let timing = lines[cue_start..cue_end]
                .iter()
                .position(|line| line.contains("-->"));
            if let Some(timing) = timing {
                output.extend(
                    lines[cue_start..=cue_start + timing]
                        .iter()
                        .map(ToString::to_string),
                );
                output.push(normalized.clone());
                updated = true;
            } else {
                output.extend(lines[cue_start..cue_end].iter().map(ToString::to_string));
            }
        }
        if cue_end < lines.len() {
            output.push(lines[cue_end].to_owned());
        }
        index = cue_end + 1;
    }
    (output.join("\n"), updated)
}

#[must_use]
pub fn legacy_transcript_object_key(prefix: &str, target_language: Option<&str>) -> Option<String> {
    if prefix.len() < 3
        || prefix.len() > 512
        || !prefix.ends_with('/')
        || prefix.starts_with('/')
        || prefix.contains('\\')
        || prefix.split('/').any(|segment| segment == "..")
        || prefix.chars().any(char::is_control)
    {
        return None;
    }
    match target_language {
        None => Some(format!("{prefix}transcription.vtt")),
        Some(language) if legacy_transcript_language_name(language).is_some() => {
            Some(format!("{prefix}transcription.{language}.vtt"))
        }
        Some(_) => None,
    }
}

#[must_use]
pub fn legacy_transcript_language_name(code: &str) -> Option<&'static str> {
    match code {
        "en" => Some("English"),
        "es" => Some("Spanish"),
        "fr" => Some("French"),
        "de" => Some("German"),
        "pt" => Some("Portuguese"),
        "it" => Some("Italian"),
        "nl" => Some("Dutch"),
        "pl" => Some("Polish"),
        "ro" => Some("Romanian"),
        "sk" => Some("Slovak"),
        "ru" => Some("Russian"),
        "tr" => Some("Turkish"),
        "ja" => Some("Japanese"),
        "ko" => Some("Korean"),
        "zh" => Some("Chinese (Simplified)"),
        "ar" => Some("Arabic"),
        "hi" => Some("Hindi"),
        "bn" => Some("Bengali"),
        "ta" => Some("Tamil"),
        "te" => Some("Telugu"),
        "mr" => Some("Marathi"),
        "gu" => Some("Gujarati"),
        "pa" => Some("Punjabi"),
        "ur" => Some("Urdu"),
        "fa" => Some("Persian"),
        "he" => Some("Hebrew"),
        _ => None,
    }
}

/// Preserve Cap's R2 listing order and permissive key scan while admitting
/// only the closed two-letter language catalog.
#[must_use]
pub fn legacy_available_translations_from_keys<I, S>(
    keys: I,
) -> (bool, Vec<LegacyAvailableTranslationV1>)
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut has_original = false;
    let mut translations = Vec::new();
    for key in keys {
        let key = key.as_ref();
        if key.ends_with("/transcription.vtt") {
            has_original = true;
            continue;
        }
        let Some(without_extension) = key.strip_suffix(".vtt") else {
            continue;
        };
        let Some((_, code)) = without_extension.rsplit_once("transcription.") else {
            continue;
        };
        if code.len() != 2 || !code.bytes().all(|byte| byte.is_ascii_lowercase()) {
            continue;
        }
        if let Some(name) = legacy_transcript_language_name(code) {
            translations.push(LegacyAvailableTranslationV1 {
                code: code.to_owned(),
                name: name.to_owned(),
            });
        }
    }
    (has_original, translations)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vtt_editor_preserves_source_control_flow() {
        let source = "WEBVTT\r\n\r\n1\r\n00:00:01.000 --> 00:00:02.000\r\nold\r\nline\r\n\r\n2\r\nno timing\r\n";
        let (updated, changed) = legacy_update_vtt_entry_text(source, 1, "  new\n  words ");
        assert!(changed);
        assert_eq!(
            updated,
            "WEBVTT\n\n1\n00:00:01.000 --> 00:00:02.000\nnew words\n\n2\nno timing\n"
        );
        let (unchanged, changed) = legacy_update_vtt_entry_text(source, 9, "ignored");
        assert!(!changed);
        assert_eq!(unchanged, source.replace("\r\n", "\n"));
    }

    #[test]
    fn storage_keys_and_language_catalog_are_closed() {
        assert_eq!(
            legacy_transcript_object_key("owner/video/", None).as_deref(),
            Some("owner/video/transcription.vtt")
        );
        assert_eq!(
            legacy_transcript_object_key("owner/video/", Some("zh")).as_deref(),
            Some("owner/video/transcription.zh.vtt")
        );
        assert!(legacy_transcript_object_key("../video/", None).is_none());
        assert!(legacy_transcript_object_key("owner/video/", Some("xx")).is_none());
        assert_eq!(legacy_transcript_language_name("pa"), Some("Punjabi"));
        let (has_original, translations) = legacy_available_translations_from_keys([
            "owner/video/transcription.es.vtt",
            "owner/video/transcription.vtt",
            "owner/video/transcription.xx.vtt",
            "other-transcription.fr.vtt",
            "owner/video/transcription.EN.vtt",
        ]);
        assert!(has_original);
        assert_eq!(
            translations,
            vec![
                LegacyAvailableTranslationV1 {
                    code: "es".into(),
                    name: "Spanish".into(),
                },
                LegacyAvailableTranslationV1 {
                    code: "fr".into(),
                    name: "French".into(),
                },
            ]
        );
    }

    #[test]
    fn source_manifests_and_security_corrections_are_frozen() {
        assert_eq!(
            legacy_transcript_source_manifest(LEGACY_RETRY_TRANSCRIPTION_SOURCES),
            LEGACY_RETRY_TRANSCRIPTION_SOURCE_MANIFEST_SHA256
        );
        assert_eq!(
            legacy_transcript_source_manifest(LEGACY_EDIT_TRANSCRIPT_SOURCES),
            LEGACY_EDIT_TRANSCRIPT_SOURCE_MANIFEST_SHA256
        );
        assert_eq!(
            legacy_transcript_source_manifest(LEGACY_GET_TRANSCRIPT_SOURCES),
            LEGACY_GET_TRANSCRIPT_SOURCE_MANIFEST_SHA256
        );
        assert_eq!(
            legacy_transcript_source_manifest(LEGACY_TRANSLATE_TRANSCRIPT_SOURCES),
            LEGACY_TRANSLATE_TRANSCRIPT_SOURCE_MANIFEST_SHA256
        );
        assert_eq!(
            legacy_transcript_source_manifest(LEGACY_AVAILABLE_TRANSLATIONS_SOURCES),
            LEGACY_AVAILABLE_TRANSLATIONS_SOURCE_MANIFEST_SHA256
        );
        assert_eq!(
            LegacyTranscriptSurfaceV1::Get.auth(),
            "optional_session_public_video_policy"
        );
        assert_eq!(LegacyTranscriptSurfaceV1::Get.idempotency(), "forbidden");
        assert_eq!(
            LegacyTranscriptSurfaceV1::AvailableTranslations.idempotency(),
            "forbidden"
        );
        assert_eq!(
            LEGACY_TRANSCRIPT_TRANSLATION_PROTECTED_GATES,
            &["provider_execution"]
        );
    }
}
