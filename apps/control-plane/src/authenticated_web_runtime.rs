//! Tenant-scoped D1 read model for server-rendered authenticated web routes.

use serde::{Deserialize, Serialize};
use wasm_bindgen::JsValue;
use worker::{D1Database, Error, Result, send::IntoSendFuture};

pub const WEB_WORKSPACE_SCHEMA_V1: &str = "frame.web-workspace.v1";
const PAGE_SIZE: i64 = 20;
const MAX_COLLECTION_ROWS: usize = 50;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WebLoadFailure {
    Invalid,
    Unavailable,
}

pub type WebLoadOutcome<T> = std::result::Result<T, WebLoadFailure>;

#[derive(Debug, Clone, Copy)]
pub struct WebLoadAuthority<'a> {
    pub tenant_id: &'a str,
    pub user_id: &'a str,
    pub selection_revision: u64,
    pub selection_context: &'a str,
    pub membership_role: &'a str,
    pub membership_revision: u64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WebLoadQuery {
    pub q: Option<String>,
    pub filter: Option<String>,
    pub page: Option<u16>,
    pub resource_id: Option<String>,
}

impl WebLoadQuery {
    pub fn validate(&self, surface: &str) -> WebLoadOutcome<()> {
        if !valid_surface(surface)
            || self.q.as_ref().is_some_and(|value| {
                value.trim() != value
                    || value.is_empty()
                    || value.len() > 120
                    || value.chars().any(char::is_control)
            })
            || !matches!(
                self.filter.as_deref().unwrap_or("all"),
                "all" | "ready" | "processing" | "failed"
            )
            || !(1..=1_000).contains(&self.page.unwrap_or(1))
            || self
                .resource_id
                .as_ref()
                .is_some_and(|value| !valid_id(value))
            || matches!(surface, "space" | "folder") != self.resource_id.is_some()
        {
            return Err(WebLoadFailure::Invalid);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WebWorkspaceV1 {
    pub schema_version: String,
    pub organization_name: String,
    pub member_label: String,
    pub role: String,
    pub revision: u64,
    pub selection_revision: u64,
    pub selection_context: String,
    pub selection_required: bool,
    pub organizations: Vec<WebOrganizationChoiceV1>,
    pub recordings: Vec<WebRecordingV1>,
    pub spaces: Vec<WebResourceV1>,
    pub folders: Vec<WebResourceV1>,
    pub import: Option<WebImportProgressV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WebOrganizationChoiceV1 {
    pub id: String,
    pub name: String,
    pub active: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WebRecordingV1 {
    pub id: String,
    pub title: String,
    pub state: String,
    pub duration_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WebResourceV1 {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WebImportProgressV1 {
    pub completed: u16,
    pub total: u16,
}

#[derive(Debug, Deserialize)]
struct WorkspaceRow {
    organization_name: String,
    member_label: Option<String>,
    role: String,
    revision: i64,
}

#[derive(Debug, Deserialize)]
struct RecordingRow {
    id: String,
    title: String,
    state: String,
    duration_ms: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct ResourceRow {
    id: String,
    name: String,
}

#[derive(Debug, Deserialize)]
struct OrganizationChoiceRow {
    id: String,
    name: String,
    active: i64,
}

#[derive(Debug, Deserialize)]
struct ImportRow {
    completed: i64,
    total: i64,
}

pub async fn load(
    database: &D1Database,
    authority: WebLoadAuthority<'_>,
    surface: &str,
    query: &WebLoadQuery,
) -> Result<WebLoadOutcome<WebWorkspaceV1>> {
    let WebLoadAuthority {
        tenant_id,
        user_id,
        selection_revision,
        selection_context,
        membership_role,
        membership_revision,
    } = authority;
    if query.validate(surface).is_err() {
        return Ok(Err(WebLoadFailure::Invalid));
    }
    let Some(workspace) = database
        .prepare(
            "SELECT o.name AS organization_name,u.display_name AS member_label,m.role,o.revision \
             FROM users u \
             JOIN organization_members m ON m.user_id=u.id \
               AND m.organization_id=u.active_organization_id AND m.state='active' \
               AND m.role IN ('owner','admin','member') \
             JOIN organizations o ON o.id=m.organization_id AND o.status='active' \
             WHERE u.id=?2 AND u.status='active' AND u.deleted_at_ms IS NULL \
               AND u.active_organization_id=?1 AND u.organization_preference_revision=?3 \
               AND m.role=?4 AND m.revision=?5 LIMIT 1",
        )
        .bind(&[
            JsValue::from_str(tenant_id),
            JsValue::from_str(user_id),
            JsValue::from_f64(selection_revision as f64),
            JsValue::from_str(membership_role),
            JsValue::from_f64(membership_revision as f64),
        ])?
        .first::<WorkspaceRow>(None)
        .await?
    else {
        return Ok(Err(WebLoadFailure::Unavailable));
    };
    if !role_permits_surface(&workspace.role, surface)
        || !resource_is_visible(
            database,
            tenant_id,
            user_id,
            &workspace.role,
            surface,
            query.resource_id.as_deref(),
        )
        .await?
    {
        return Ok(Err(WebLoadFailure::Unavailable));
    }

    let offset = i64::from(query.page.unwrap_or(1).saturating_sub(1)) * PAGE_SIZE;
    let search = query.q.as_deref().unwrap_or("");
    let filter = query.filter.as_deref().unwrap_or("all");
    let recording_result = database
        .prepare(
            "SELECT id,title,state,duration_ms FROM videos \
             WHERE organization_id=?1 AND deleted_at_ms IS NULL AND state<>'deleted' \
               AND (?2='' OR instr(lower(title),lower(?2))>0) \
               AND (?3='all' OR (?3='ready' AND state='ready') \
                 OR (?3='processing' AND state IN ('pending','uploading','processing')) \
                 OR (?3='failed' AND state='failed')) \
             ORDER BY created_at_ms DESC,id LIMIT 20 OFFSET ?4",
        )
        .bind(&[
            JsValue::from_str(tenant_id),
            JsValue::from_str(search),
            JsValue::from_str(filter),
            JsValue::from_f64(offset as f64),
        ])?
        .all()
        .into_send()
        .await?;
    if !recording_result.success() {
        return Err(Error::RustError(
            "authenticated recording load failed".into(),
        ));
    }
    let recording_rows = recording_result.results::<RecordingRow>()?;
    if recording_rows.len() > PAGE_SIZE as usize {
        return Err(Error::RustError(
            "authenticated recording load exceeded its bound".into(),
        ));
    }

    let spaces = visible_spaces(database, tenant_id, user_id, &workspace.role).await?;
    let folders = visible_folders(database, tenant_id, user_id, &workspace.role).await?;
    let organizations = organization_choices(database, user_id, Some(tenant_id)).await?;
    if organizations.is_empty() || organizations.iter().filter(|choice| choice.active).count() != 1
    {
        return Ok(Err(WebLoadFailure::Unavailable));
    }
    let import = database
        .prepare(
            "SELECT SUM(CASE WHEN state='complete' THEN 1 ELSE 0 END) AS completed, \
                    COUNT(*) AS total FROM imported_videos WHERE organization_id=?1",
        )
        .bind(&[JsValue::from_str(tenant_id)])?
        .first::<ImportRow>(None)
        .await?
        .and_then(decode_import);

    let revision = checked_u64(workspace.revision)?;
    let recordings = recording_rows
        .into_iter()
        .map(decode_recording)
        .collect::<Result<Vec<_>>>()?;
    Ok(Ok(WebWorkspaceV1 {
        schema_version: WEB_WORKSPACE_SCHEMA_V1.into(),
        organization_name: workspace.organization_name,
        member_label: workspace
            .member_label
            .unwrap_or_else(|| "Workspace member".into()),
        role: workspace.role,
        revision,
        selection_revision,
        selection_context: selection_context.to_owned(),
        selection_required: false,
        organizations,
        recordings,
        spaces,
        folders,
        import,
    }))
}

/// Return only organizations for which the authenticated user has a current
/// product membership. The active marker is presentation data; mutation
/// authority is reasserted atomically by the action repository.
pub async fn organization_choices(
    database: &D1Database,
    user_id: &str,
    active_organization_id: Option<&str>,
) -> Result<Vec<WebOrganizationChoiceV1>> {
    let result = database
        .prepare(
            "SELECT o.id,o.name,CASE WHEN o.id=?2 THEN 1 ELSE 0 END AS active \
             FROM organization_members m \
             JOIN organizations o ON o.id=m.organization_id AND o.status='active' \
             JOIN users u ON u.id=m.user_id AND u.status='active' AND u.deleted_at_ms IS NULL \
             WHERE m.user_id=?1 AND m.state='active' AND m.role IN ('owner','admin','member') \
             ORDER BY active DESC,o.created_at_ms,o.id LIMIT 50",
        )
        .bind(&[
            JsValue::from_str(user_id),
            active_organization_id.map_or(JsValue::NULL, JsValue::from_str),
        ])?
        .all()
        .into_send()
        .await?;
    if !result.success() {
        return Err(Error::RustError(
            "authenticated organization choice load failed".into(),
        ));
    }
    let rows = result.results::<OrganizationChoiceRow>()?;
    if rows.len() > MAX_COLLECTION_ROWS {
        return Err(Error::RustError(
            "authenticated organization choices exceeded their bound".into(),
        ));
    }
    rows.into_iter()
        .map(|row| {
            if !uuid::Uuid::parse_str(&row.id)
                .is_ok_and(|id| !id.is_nil() && id.as_hyphenated().to_string() == row.id)
                || row.name.is_empty()
                || row.name.len() > 160
                || row.name.chars().any(char::is_control)
                || !matches!(row.active, 0 | 1)
            {
                return Err(Error::RustError(
                    "authenticated organization choice is corrupt".into(),
                ));
            }
            Ok(WebOrganizationChoiceV1 {
                id: row.id,
                name: row.name,
                active: row.active == 1,
            })
        })
        .collect()
}

async fn resource_is_visible(
    database: &D1Database,
    tenant_id: &str,
    user_id: &str,
    role: &str,
    surface: &str,
    resource_id: Option<&str>,
) -> Result<bool> {
    let Some(resource_id) = resource_id else {
        return Ok(true);
    };
    let privileged = matches!(role, "owner" | "admin");
    let sql = match surface {
        "space" => {
            "SELECT 1 AS visible FROM spaces s WHERE s.id=?1 AND s.organization_id=?2 \
             AND s.deleted_at_ms IS NULL AND (?4=1 OR s.is_public=1 OR s.created_by_user_id=?3 \
               OR EXISTS (SELECT 1 FROM space_members sm WHERE sm.space_id=s.id \
                 AND sm.user_id=?3 AND sm.state='active')) LIMIT 1"
        }
        "folder" => {
            "SELECT 1 AS visible FROM folders f JOIN spaces s ON s.id=f.space_id \
             AND s.organization_id=f.organization_id AND s.deleted_at_ms IS NULL \
             WHERE f.id=?1 AND f.organization_id=?2 AND f.deleted_at_ms IS NULL \
               AND (?4=1 OR f.is_public=1 OR s.is_public=1 OR f.created_by_user_id=?3 \
                 OR EXISTS (SELECT 1 FROM space_members sm WHERE sm.space_id=s.id \
                   AND sm.user_id=?3 AND sm.state='active')) LIMIT 1"
        }
        _ => return Ok(false),
    };
    Ok(database
        .prepare(sql)
        .bind(&[
            JsValue::from_str(resource_id),
            JsValue::from_str(tenant_id),
            JsValue::from_str(user_id),
            JsValue::from_f64(if privileged { 1.0 } else { 0.0 }),
        ])?
        .first::<VisibleRow>(None)
        .await?
        .is_some_and(|row| row.visible == 1))
}

#[derive(Debug, Deserialize)]
struct VisibleRow {
    visible: i64,
}

async fn visible_spaces(
    database: &D1Database,
    tenant_id: &str,
    user_id: &str,
    role: &str,
) -> Result<Vec<WebResourceV1>> {
    let privileged = matches!(role, "owner" | "admin");
    let result = database
        .prepare(
            "SELECT s.id,s.name FROM spaces s WHERE s.organization_id=?1 \
             AND s.deleted_at_ms IS NULL AND (?3=1 OR s.is_public=1 OR s.created_by_user_id=?2 \
               OR EXISTS (SELECT 1 FROM space_members sm WHERE sm.space_id=s.id \
                 AND sm.user_id=?2 AND sm.state='active')) \
             ORDER BY s.is_primary DESC,s.created_at_ms,s.id LIMIT 50",
        )
        .bind(&[
            JsValue::from_str(tenant_id),
            JsValue::from_str(user_id),
            JsValue::from_f64(if privileged { 1.0 } else { 0.0 }),
        ])?
        .all()
        .into_send()
        .await?;
    decode_resources(result, "spaces")
}

async fn visible_folders(
    database: &D1Database,
    tenant_id: &str,
    user_id: &str,
    role: &str,
) -> Result<Vec<WebResourceV1>> {
    let privileged = matches!(role, "owner" | "admin");
    let result = database
        .prepare(
            "SELECT f.id,COALESCE(f.legacy_name,f.name) AS name FROM folders f JOIN spaces s ON s.id=f.space_id \
             AND s.organization_id=f.organization_id AND s.deleted_at_ms IS NULL \
             WHERE f.organization_id=?1 AND f.deleted_at_ms IS NULL \
               AND (?3=1 OR f.is_public=1 OR s.is_public=1 OR f.created_by_user_id=?2 \
                 OR EXISTS (SELECT 1 FROM space_members sm WHERE sm.space_id=s.id \
                   AND sm.user_id=?2 AND sm.state='active')) \
             ORDER BY f.depth,f.created_at_ms,f.id LIMIT 50",
        )
        .bind(&[
            JsValue::from_str(tenant_id),
            JsValue::from_str(user_id),
            JsValue::from_f64(if privileged { 1.0 } else { 0.0 }),
        ])?
        .all()
        .into_send()
        .await?;
    decode_resources(result, "folders")
}

fn decode_resources(result: worker::D1Result, label: &str) -> Result<Vec<WebResourceV1>> {
    if !result.success() {
        return Err(Error::RustError(format!(
            "authenticated {label} load failed"
        )));
    }
    let rows = result.results::<ResourceRow>()?;
    if rows.len() > MAX_COLLECTION_ROWS {
        return Err(Error::RustError(format!(
            "authenticated {label} load exceeded its bound"
        )));
    }
    Ok(rows
        .into_iter()
        .map(|row| WebResourceV1 {
            id: row.id,
            name: row.name,
        })
        .collect())
}

fn decode_recording(row: RecordingRow) -> Result<WebRecordingV1> {
    let state = match row.state.as_str() {
        "ready" => "ready",
        "pending" | "uploading" | "processing" => "processing",
        "failed" => "failed",
        _ => {
            return Err(Error::RustError(
                "authenticated recording state is corrupt".into(),
            ));
        }
    };
    Ok(WebRecordingV1 {
        id: row.id,
        title: row.title,
        state: state.into(),
        duration_ms: row.duration_ms.map(checked_u64).transpose()?,
    })
}

fn decode_import(row: ImportRow) -> Option<WebImportProgressV1> {
    if row.total <= 0 {
        return None;
    }
    Some(WebImportProgressV1 {
        completed: u16::try_from(row.completed.clamp(0, i64::from(u16::MAX))).ok()?,
        total: u16::try_from(row.total.clamp(1, i64::from(u16::MAX))).ok()?,
    })
}

fn checked_u64(value: i64) -> Result<u64> {
    u64::try_from(value).map_err(|_| Error::RustError("authenticated web value is corrupt".into()))
}

fn role_permits_surface(role: &str, surface: &str) -> bool {
    match surface {
        "dashboard" | "library" | "spaces" | "space" | "folders" | "folder" | "onboarding"
        | "settings" | "account_settings" => {
            matches!(role, "owner" | "admin" | "member")
        }
        "imports"
        | "organization_settings"
        | "member_settings"
        | "storage_settings"
        | "developer"
        | "analytics"
        | "admin" => matches!(role, "owner" | "admin"),
        "billing" => role == "owner",
        _ => false,
    }
}

fn valid_surface(value: &str) -> bool {
    matches!(
        value,
        "dashboard"
            | "library"
            | "spaces"
            | "space"
            | "folders"
            | "folder"
            | "onboarding"
            | "imports"
            | "settings"
            | "account_settings"
            | "organization_settings"
            | "member_settings"
            | "storage_settings"
            | "developer"
            | "billing"
            | "analytics"
            | "admin"
    )
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_and_role_matrix_are_closed() {
        let valid = WebLoadQuery {
            q: Some("quarterly update".into()),
            filter: Some("ready".into()),
            page: Some(2),
            resource_id: None,
        };
        assert_eq!(valid.validate("library"), Ok(()));
        assert!(role_permits_surface("member", "library"));
        assert!(!role_permits_surface("member", "admin"));
        assert!(!role_permits_surface("viewer", "library"));
        assert!(role_permits_surface("owner", "billing"));
        assert!(!role_permits_surface("admin", "billing"));
        assert!(
            WebLoadQuery {
                resource_id: Some("space-1".into()),
                ..valid.clone()
            }
            .validate("space")
            .is_ok()
        );
        assert!(valid.validate("unknown").is_err());
    }
}
