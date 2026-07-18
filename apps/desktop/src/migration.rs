use serde::{Deserialize, Serialize};

pub const SETTINGS_FORMAT_VERSION: u16 = 1;
pub const PROJECT_FORMAT_VERSION: u16 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MigrationDisposition {
    Compatible,
    Migratable,
    NeedsReview,
    Unsupported,
    Invalid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingCode {
    NewerVersion,
    LegacyVersion,
    UnknownField,
    InvalidValue,
    IntegrityFailure,
    MissingAsset,
    UnsupportedEffect,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MigrationFinding {
    pub code: FindingCode,
    /// A schema field/effect identifier, never a path or user-provided value.
    pub subject: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordingMode {
    Instant,
    Studio,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacySettings {
    pub format_version: u16,
    pub recording_mode: String,
    pub frame_rate: u32,
    pub microphone_enabled: bool,
    pub system_audio_enabled: bool,
    pub camera_enabled: bool,
    /// Keys are retained for reporting; values are intentionally not loaded into
    /// this security boundary where they could be logged or silently discarded.
    pub unknown_fields: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct NormalizedSettings {
    pub format_version: u16,
    pub recording_mode: RecordingMode,
    pub frame_rate: u32,
    pub microphone_enabled: bool,
    pub system_audio_enabled: bool,
    pub camera_enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettingsMigrationReport {
    pub source_version: u16,
    pub target_version: u16,
    pub disposition: MigrationDisposition,
    pub source_must_remain_read_only: bool,
    pub findings: Vec<MigrationFinding>,
    pub proposed: Option<NormalizedSettings>,
}

#[must_use]
pub fn inspect_legacy_settings(source: &LegacySettings) -> SettingsMigrationReport {
    let mut findings = Vec::new();
    if source.format_version > SETTINGS_FORMAT_VERSION {
        findings.push(finding(FindingCode::NewerVersion, "format_version"));
        return SettingsMigrationReport {
            source_version: source.format_version,
            target_version: SETTINGS_FORMAT_VERSION,
            disposition: MigrationDisposition::Unsupported,
            source_must_remain_read_only: true,
            findings,
            proposed: None,
        };
    }
    if source.format_version < SETTINGS_FORMAT_VERSION {
        findings.push(finding(FindingCode::LegacyVersion, "format_version"));
    }

    let recording_mode = match source.recording_mode.as_str() {
        "instant" => Some(RecordingMode::Instant),
        "studio" => Some(RecordingMode::Studio),
        _ => {
            findings.push(finding(FindingCode::InvalidValue, "recording_mode"));
            None
        }
    };
    if !matches!(source.frame_rate, 24 | 25 | 30 | 50 | 60) {
        findings.push(finding(FindingCode::InvalidValue, "frame_rate"));
    }
    for field in &source.unknown_fields {
        if safe_schema_identifier(field) {
            findings.push(MigrationFinding {
                code: FindingCode::UnknownField,
                subject: field.clone(),
            });
        } else {
            findings.push(finding(FindingCode::UnknownField, "<invalid-field-name>"));
        }
    }

    let invalid = recording_mode.is_none()
        || findings
            .iter()
            .any(|finding| finding.code == FindingCode::InvalidValue);
    let needs_review = !source.unknown_fields.is_empty();
    let proposed = recording_mode
        .filter(|_| !invalid && !needs_review)
        .map(|recording_mode| NormalizedSettings {
            format_version: SETTINGS_FORMAT_VERSION,
            recording_mode,
            frame_rate: source.frame_rate,
            microphone_enabled: source.microphone_enabled,
            system_audio_enabled: source.system_audio_enabled,
            camera_enabled: source.camera_enabled,
        });
    let disposition = if invalid {
        MigrationDisposition::Invalid
    } else if needs_review {
        MigrationDisposition::NeedsReview
    } else if source.format_version < SETTINGS_FORMAT_VERSION {
        MigrationDisposition::Migratable
    } else {
        MigrationDisposition::Compatible
    };
    SettingsMigrationReport {
        source_version: source.format_version,
        target_version: SETTINGS_FORMAT_VERSION,
        disposition,
        source_must_remain_read_only: true,
        findings,
        proposed,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyProjectHeader {
    pub format_version: u16,
    pub asset_count: u32,
    pub edit_count: u32,
    pub integrity_valid: bool,
    pub effects: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectMigrationPlan {
    pub target_version: u16,
    pub expected_assets: u32,
    pub expected_edits: u32,
    pub preserve_original: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectMigrationReport {
    pub source_version: u16,
    pub target_version: u16,
    pub disposition: MigrationDisposition,
    pub source_must_remain_read_only: bool,
    pub findings: Vec<MigrationFinding>,
    pub unsupported_effects: Vec<String>,
    pub proposed: Option<ProjectMigrationPlan>,
}

const SUPPORTED_EFFECTS: &[&str] = &[
    "trim",
    "split",
    "delete",
    "speed",
    "audio_gain",
    "camera_layout",
    "cursor",
    "background",
];

#[must_use]
pub fn inspect_legacy_project(source: &LegacyProjectHeader) -> ProjectMigrationReport {
    let mut findings = Vec::new();
    if source.format_version > PROJECT_FORMAT_VERSION {
        findings.push(finding(FindingCode::NewerVersion, "format_version"));
        return project_report(
            source,
            MigrationDisposition::Unsupported,
            findings,
            Vec::new(),
            None,
        );
    }
    if !source.integrity_valid {
        findings.push(finding(FindingCode::IntegrityFailure, "project_integrity"));
        return project_report(
            source,
            MigrationDisposition::Invalid,
            findings,
            Vec::new(),
            None,
        );
    }
    if source.asset_count == 0 {
        findings.push(finding(FindingCode::MissingAsset, "assets"));
        return project_report(
            source,
            MigrationDisposition::Invalid,
            findings,
            Vec::new(),
            None,
        );
    }
    if source.format_version < PROJECT_FORMAT_VERSION {
        findings.push(finding(FindingCode::LegacyVersion, "format_version"));
    }

    let mut unsupported = Vec::new();
    for effect in &source.effects {
        if !SUPPORTED_EFFECTS.contains(&effect.as_str()) {
            let safe_effect = if safe_schema_identifier(effect) {
                effect.clone()
            } else {
                "<invalid-effect-name>".into()
            };
            findings.push(MigrationFinding {
                code: FindingCode::UnsupportedEffect,
                subject: safe_effect.clone(),
            });
            unsupported.push(safe_effect);
        }
    }
    unsupported.sort();
    unsupported.dedup();

    if !unsupported.is_empty() {
        return project_report(
            source,
            MigrationDisposition::NeedsReview,
            findings,
            unsupported,
            None,
        );
    }

    let disposition = if source.format_version < PROJECT_FORMAT_VERSION {
        MigrationDisposition::Migratable
    } else {
        MigrationDisposition::Compatible
    };
    project_report(
        source,
        disposition,
        findings,
        unsupported,
        Some(ProjectMigrationPlan {
            target_version: PROJECT_FORMAT_VERSION,
            expected_assets: source.asset_count,
            expected_edits: source.edit_count,
            preserve_original: true,
        }),
    )
}

fn project_report(
    source: &LegacyProjectHeader,
    disposition: MigrationDisposition,
    findings: Vec<MigrationFinding>,
    unsupported_effects: Vec<String>,
    proposed: Option<ProjectMigrationPlan>,
) -> ProjectMigrationReport {
    ProjectMigrationReport {
        source_version: source.format_version,
        target_version: PROJECT_FORMAT_VERSION,
        disposition,
        source_must_remain_read_only: true,
        findings,
        unsupported_effects,
        proposed,
    }
}

fn finding(code: FindingCode, subject: &str) -> MigrationFinding {
    MigrationFinding {
        code,
        subject: subject.into(),
    }
}

fn safe_schema_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "_-".contains(character))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsupported_settings_are_reported_without_mutating_input() {
        let source = LegacySettings {
            format_version: 99,
            recording_mode: "future".into(),
            frame_rate: 120,
            microphone_enabled: true,
            system_audio_enabled: true,
            camera_enabled: true,
            unknown_fields: vec!["future_secret_shape".into()],
        };
        let before = source.clone();
        let report = inspect_legacy_settings(&source);
        assert_eq!(source, before);
        assert_eq!(report.disposition, MigrationDisposition::Unsupported);
        assert!(report.proposed.is_none());
        assert!(report.source_must_remain_read_only);
    }

    #[test]
    fn unknown_settings_require_review_instead_of_silent_drop() {
        let source = LegacySettings {
            format_version: SETTINGS_FORMAT_VERSION,
            recording_mode: "instant".into(),
            frame_rate: 30,
            microphone_enabled: true,
            system_audio_enabled: false,
            camera_enabled: false,
            unknown_fields: vec!["future_option".into()],
        };
        let report = inspect_legacy_settings(&source);
        assert_eq!(report.disposition, MigrationDisposition::NeedsReview);
        assert!(report.proposed.is_none());
    }

    #[test]
    fn known_legacy_settings_produce_an_explicit_plan() {
        let source = LegacySettings {
            format_version: 0,
            recording_mode: "studio".into(),
            frame_rate: 60,
            microphone_enabled: true,
            system_audio_enabled: true,
            camera_enabled: false,
            unknown_fields: Vec::new(),
        };
        let report = inspect_legacy_settings(&source);
        assert_eq!(report.disposition, MigrationDisposition::Migratable);
        assert_eq!(
            report.proposed.expect("plan").recording_mode,
            RecordingMode::Studio
        );
    }

    #[test]
    fn unsupported_project_effects_never_modify_or_plan_overwrite() {
        let source = LegacyProjectHeader {
            format_version: PROJECT_FORMAT_VERSION,
            asset_count: 3,
            edit_count: 4,
            integrity_valid: true,
            effects: vec!["trim".into(), "future_ai_reframe".into()],
        };
        let before = source.clone();
        let report = inspect_legacy_project(&source);
        assert_eq!(source, before);
        assert_eq!(report.disposition, MigrationDisposition::NeedsReview);
        assert_eq!(report.unsupported_effects, vec!["future_ai_reframe"]);
        assert!(report.proposed.is_none());
        assert!(report.source_must_remain_read_only);
    }

    #[test]
    fn supported_project_plan_always_preserves_original() {
        let source = LegacyProjectHeader {
            format_version: 0,
            asset_count: 2,
            edit_count: 5,
            integrity_valid: true,
            effects: vec!["trim".into(), "camera_layout".into()],
        };
        let report = inspect_legacy_project(&source);
        assert_eq!(report.disposition, MigrationDisposition::Migratable);
        let plan = report.proposed.expect("plan");
        assert!(plan.preserve_original);
        assert_eq!(plan.expected_assets, 2);
    }
}
