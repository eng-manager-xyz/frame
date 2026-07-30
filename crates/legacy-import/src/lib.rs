#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};

pub const LEGACY_PROJECT_CATALOG_VERSION: u16 = 1;
pub const MAX_LEGACY_PROJECT_CATALOG_ENTRIES: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LegacyProjectCatalogAvailability {
    Unavailable,
    Ready,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LegacySettingsInspection {
    NotFound,
    Read,
    Invalid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LegacyProjectStatus {
    Importable,
    Imported,
    NeedsReview,
    Unsupported,
    Invalid,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyProjectSummary {
    pub project_token: String,
    pub ordinal: u16,
    pub status: LegacyProjectStatus,
    pub source_asset_count: u16,
    pub supported_effect_count: u16,
    pub unsupported_effect_count: u16,
}

impl std::fmt::Debug for LegacyProjectSummary {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LegacyProjectSummary")
            .field("project_token", &"<redacted>")
            .field("ordinal", &self.ordinal)
            .field("status", &self.status)
            .field("source_asset_count", &self.source_asset_count)
            .field("supported_effect_count", &self.supported_effect_count)
            .field("unsupported_effect_count", &self.unsupported_effect_count)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyProjectCatalog {
    pub schema_version: u16,
    pub generation: u64,
    pub availability: LegacyProjectCatalogAvailability,
    pub settings_inspection: LegacySettingsInspection,
    pub projects: Vec<LegacyProjectSummary>,
}

impl Default for LegacyProjectCatalog {
    fn default() -> Self {
        Self::unavailable()
    }
}

impl LegacyProjectCatalog {
    #[must_use]
    pub const fn unavailable() -> Self {
        Self {
            schema_version: LEGACY_PROJECT_CATALOG_VERSION,
            generation: 0,
            availability: LegacyProjectCatalogAvailability::Unavailable,
            settings_inspection: LegacySettingsInspection::NotFound,
            projects: Vec::new(),
        }
    }

    pub fn validate(&self) -> Result<(), LegacyImportError> {
        if self.schema_version != LEGACY_PROJECT_CATALOG_VERSION
            || self.projects.len() > MAX_LEGACY_PROJECT_CATALOG_ENTRIES
            || (self.availability == LegacyProjectCatalogAvailability::Unavailable
                && (self.generation != 0 || !self.projects.is_empty()))
            || (self.availability == LegacyProjectCatalogAvailability::Ready
                && self.generation == 0)
        {
            return Err(LegacyImportError::InvalidCatalog);
        }
        let mut tokens = std::collections::BTreeSet::new();
        for (index, project) in self.projects.iter().enumerate() {
            if project.ordinal != u16::try_from(index + 1).map_err(|_| LegacyImportError::Bound)?
                || !valid_token(&project.project_token)
                || !tokens.insert(project.project_token.as_str())
                || (project.status == LegacyProjectStatus::Importable
                    && project.source_asset_count == 0)
            {
                return Err(LegacyImportError::InvalidCatalog);
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyImportReceipt {
    pub imported_assets: u16,
    pub project_revision: u64,
}

#[derive(Debug, thiserror::Error, Clone, Copy, PartialEq, Eq)]
pub enum LegacyImportError {
    #[error("legacy project migration is unavailable")]
    Unavailable,
    #[error("legacy project catalog is stale")]
    StaleCatalog,
    #[error("legacy project requires review")]
    NeedsReview,
    #[error("legacy project is unsupported")]
    Unsupported,
    #[error("legacy project is invalid")]
    InvalidProject,
    #[error("legacy project changed during import")]
    SourceChanged,
    #[error("legacy project migration exceeded a bounded policy")]
    Bound,
    #[error("legacy project catalog is invalid")]
    InvalidCatalog,
    #[error("legacy project storage failed safely")]
    Storage,
    #[error("secure randomness is unavailable")]
    RandomUnavailable,
}

fn valid_token(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 96
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ready_catalog_requires_a_generation_and_redacts_tokens() {
        let mut catalog = LegacyProjectCatalog {
            schema_version: LEGACY_PROJECT_CATALOG_VERSION,
            generation: 0,
            availability: LegacyProjectCatalogAvailability::Ready,
            settings_inspection: LegacySettingsInspection::Read,
            projects: Vec::new(),
        };
        assert_eq!(catalog.validate(), Err(LegacyImportError::InvalidCatalog));
        catalog.generation = 1;
        catalog.projects.push(LegacyProjectSummary {
            project_token: "legacy-project:0011223344556677".into(),
            ordinal: 1,
            status: LegacyProjectStatus::Importable,
            source_asset_count: 1,
            supported_effect_count: 0,
            unsupported_effect_count: 0,
        });
        assert!(catalog.validate().is_ok());
        assert!(!format!("{:?}", catalog.projects[0]).contains("0011223344556677"));
    }
}

#[cfg(feature = "cap-media")]
mod service;
#[cfg(feature = "cap-media")]
pub use service::LegacyProjectMigrationService;
