//! Provider-neutral share and player policy.
//!
//! This module intentionally contains no database, object-store, media executor,
//! session-cookie, or provider URL type. Adapters may supply already-authorized
//! state, but this boundary decides what a share surface can reveal and validates
//! every browser-originated collaboration or analytics command before persistence.

use std::fmt;

use frame_client::{CaptionTrack, PlaybackDescriptor, PublicShareSummary, ShareAvailability};
use serde::{Deserialize, Serialize};

pub const EMBED_COMMAND_SCHEMA: &str = "frame.embed-command.v1";
pub const EMBED_REPLY_SCHEMA: &str = "frame.embed-reply.v1";
pub const SHARE_HTML_CACHE_CONTROL: &str = "private, no-store, max-age=0";
pub const SHARE_MEDIA_CACHE_CONTROL: &str = "private, no-store, max-age=0";
pub const MAX_RECORDING_DURATION_MS: u64 = 24 * 60 * 60 * 1_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SharePrivacy {
    Public,
    Unlisted,
    Tenant,
    Private,
    Password,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShareLifecycle {
    Ready,
    Processing,
    Failed,
    Deleted,
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShareSurface {
    TopLevel,
    Embed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Viewer<'a> {
    Anonymous,
    Authenticated {
        tenant_digest: &'a str,
        subject_digest: &'a str,
        owner: bool,
    },
    ExplicitGrant {
        share_scope: &'a str,
        subject_digest: &'a str,
        expires_at_ms: u64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasswordGrant<'a> {
    Absent,
    Verified {
        share_scope: &'a str,
        proof_digest: &'a str,
        expires_at_ms: u64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShareAccessRequest<'a> {
    pub share_scope: &'a str,
    pub tenant_digest: &'a str,
    pub privacy: SharePrivacy,
    pub lifecycle: ShareLifecycle,
    pub surface: ShareSurface,
    pub viewer: Viewer<'a>,
    pub password: PasswordGrant<'a>,
    pub embed_enabled: bool,
    pub now_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShareResolution {
    Ready { head: HeadPolicy },
    Processing { head: HeadPolicy },
    PasswordChallenge { head: HeadPolicy },
    Unavailable { head: HeadPolicy },
}

impl ShareResolution {
    #[must_use]
    pub const fn status_code(self) -> u16 {
        match self {
            Self::Ready { .. } => 200,
            Self::Processing { .. } => 202,
            Self::PasswordChallenge { .. } => 401,
            Self::Unavailable { .. } => 404,
        }
    }

    #[must_use]
    pub const fn head(self) -> HeadPolicy {
        match self {
            Self::Ready { head }
            | Self::Processing { head }
            | Self::PasswordChallenge { head }
            | Self::Unavailable { head } => head,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HeadPolicy {
    pub robots: &'static str,
    pub emit_open_graph: bool,
    pub emit_thumbnail: bool,
    pub emit_public_title: bool,
}

impl HeadPolicy {
    const PRIVATE: Self = Self {
        robots: "noindex,nofollow",
        emit_open_graph: false,
        emit_thumbnail: false,
        emit_public_title: false,
    };

    const PUBLIC: Self = Self {
        robots: "index,follow",
        emit_open_graph: true,
        emit_thumbnail: false,
        emit_public_title: true,
    };

    const UNLISTED: Self = Self {
        robots: "noindex,nofollow",
        emit_open_graph: false,
        emit_thumbnail: false,
        emit_public_title: true,
    };
}

/// Resolve a share without exposing why an unauthorized resource was denied.
/// Password challenges are the sole deliberate existence disclosure and are
/// available only for a syntactically valid, non-terminal, top-level link.
#[must_use]
pub fn resolve_share(request: ShareAccessRequest<'_>) -> ShareResolution {
    if !safe_public_id(request.share_scope) || !valid_digest(request.tenant_digest) {
        return unavailable();
    }
    if matches!(
        request.lifecycle,
        ShareLifecycle::Failed | ShareLifecycle::Deleted | ShareLifecycle::Unavailable
    ) {
        return unavailable();
    }
    if request.surface == ShareSurface::Embed && !request.embed_enabled {
        return unavailable();
    }

    let direct = directly_authorized(request);
    let entitled = match request.privacy {
        SharePrivacy::Public | SharePrivacy::Unlisted => true,
        SharePrivacy::Tenant => direct || same_tenant_member(request),
        SharePrivacy::Private => direct,
        SharePrivacy::Password => {
            if direct || password_authorized(request) {
                true
            } else if request.surface == ShareSurface::TopLevel {
                return ShareResolution::PasswordChallenge {
                    head: HeadPolicy::PRIVATE,
                };
            } else {
                return unavailable();
            }
        }
    };
    if !entitled {
        return unavailable();
    }

    // Credentials are intentionally not consumed in an iframe in v1. This
    // avoids ambient session and password capabilities crossing frame origins.
    if request.surface == ShareSurface::Embed
        && !matches!(
            request.privacy,
            SharePrivacy::Public | SharePrivacy::Unlisted
        )
    {
        return unavailable();
    }

    let head = match (request.privacy, request.surface) {
        (SharePrivacy::Public, ShareSurface::TopLevel) => HeadPolicy::PUBLIC,
        (SharePrivacy::Unlisted, ShareSurface::TopLevel) => HeadPolicy::UNLISTED,
        _ => HeadPolicy::PRIVATE,
    };
    match request.lifecycle {
        ShareLifecycle::Ready => ShareResolution::Ready { head },
        ShareLifecycle::Processing => ShareResolution::Processing {
            // Processing never emits a title, thumbnail, or OpenGraph fields.
            head: HeadPolicy::PRIVATE,
        },
        ShareLifecycle::Failed | ShareLifecycle::Deleted | ShareLifecycle::Unavailable => {
            unavailable()
        }
    }
}

fn unavailable() -> ShareResolution {
    ShareResolution::Unavailable {
        head: HeadPolicy::PRIVATE,
    }
}

fn directly_authorized(request: ShareAccessRequest<'_>) -> bool {
    match request.viewer {
        Viewer::Anonymous => false,
        Viewer::Authenticated {
            tenant_digest,
            subject_digest,
            owner,
        } => {
            owner
                && tenant_digest == request.tenant_digest
                && valid_digest(tenant_digest)
                && valid_digest(subject_digest)
        }
        Viewer::ExplicitGrant {
            share_scope,
            subject_digest,
            expires_at_ms,
        } => {
            share_scope == request.share_scope
                && valid_digest(subject_digest)
                && expires_at_ms > request.now_ms
        }
    }
}

fn same_tenant_member(request: ShareAccessRequest<'_>) -> bool {
    matches!(
        request.viewer,
        Viewer::Authenticated {
            tenant_digest,
            subject_digest,
            ..
        } if tenant_digest == request.tenant_digest
            && valid_digest(tenant_digest)
            && valid_digest(subject_digest)
    )
}

fn password_authorized(request: ShareAccessRequest<'_>) -> bool {
    matches!(
        request.password,
        PasswordGrant::Verified {
            share_scope,
            proof_digest,
            expires_at_ms,
        } if share_scope == request.share_scope
            && valid_digest(proof_digest)
            && expires_at_ms > request.now_ms
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShareRoutes {
    pub canonical: String,
    pub embed: String,
    pub media: String,
    pub transcript: String,
    pub comments: String,
    pub analytics_consent: String,
    pub analytics_events: String,
}

#[must_use]
pub fn public_routes(share_scope: &str) -> Option<ShareRoutes> {
    safe_public_id(share_scope).then(|| {
        let api = format!("/api/v1/public/shares/{share_scope}");
        ShareRoutes {
            canonical: format!("/s/{share_scope}"),
            embed: format!("/embed/{share_scope}"),
            media: format!("{api}/media"),
            transcript: format!("{api}/transcript"),
            comments: format!("{api}/comments"),
            analytics_consent: format!("{api}/analytics/consent"),
            analytics_events: format!("{api}/analytics/events"),
        }
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DerivativeExecutor {
    Managed,
    Native,
}

/// Construct the browser descriptor from a public share identity. Executor
/// provenance is intentionally ignored: routes cannot change on failover.
#[must_use]
pub fn provider_neutral_playback(
    _executor: DerivativeExecutor,
    share_scope: &str,
    content_type: &str,
    captions: Vec<CaptionTrack>,
) -> Option<PlaybackDescriptor> {
    let routes = public_routes(share_scope)?;
    approved_media_type(content_type).then_some(PlaybackDescriptor {
        path: routes.media,
        content_type: content_type.to_owned(),
        supports_range: true,
        captions,
    })
}

/// A second, web-specific scope check closes cross-share descriptor confusion
/// even when a future client contract accidentally accepts a syntactically safe
/// path belonging to another share.
#[must_use]
pub fn summary_is_scope_safe(summary: &PublicShareSummary) -> bool {
    match summary.availability {
        ShareAvailability::Unavailable => true,
        ShareAvailability::Processing => summary
            .canonical_url
            .as_deref()
            .and_then(canonical_share_scope)
            .is_some(),
        ShareAvailability::Public => {
            let Some(scope) = summary
                .canonical_url
                .as_deref()
                .and_then(canonical_share_scope)
            else {
                return false;
            };
            let Some(playback) = summary.playback.as_ref() else {
                return false;
            };
            if !playback.supports_range
                || !approved_media_type(&playback.content_type)
                || !is_scoped_suffix(&playback.path, scope, &["media", "playback"])
                || playback.captions.len() > 32
                || playback
                    .captions
                    .iter()
                    .filter(|track| track.default)
                    .count()
                    > 1
            {
                return false;
            }
            for (index, caption) in playback.captions.iter().enumerate() {
                if !is_scoped_caption_path(&caption.path, scope)
                    || !valid_language(&caption.language)
                    || caption.label.trim().is_empty()
                    || caption.label.len() > 80
                    || caption.label.chars().any(char::is_control)
                {
                    return false;
                }
                if playback.captions[..index].iter().any(|previous| {
                    previous.path == caption.path || previous.language == caption.language
                }) {
                    return false;
                }
            }
            true
        }
    }
}

#[must_use]
pub fn summary_matches_route(summary: &PublicShareSummary, route_scope: &str) -> bool {
    safe_public_id(route_scope)
        && match summary.availability {
            ShareAvailability::Unavailable => false,
            ShareAvailability::Processing | ShareAvailability::Public => summary
                .canonical_url
                .as_deref()
                .and_then(canonical_share_scope)
                .is_some_and(|scope| scope == route_scope),
        }
}

fn canonical_share_scope(canonical: &str) -> Option<&str> {
    if canonical.contains(['?', '#', '\\', '%']) {
        return None;
    }
    let (_, scope) = canonical.rsplit_once("/s/")?;
    (!scope.contains('/') && safe_public_id(scope)).then_some(scope)
}

fn is_scoped_suffix(path: &str, scope: &str, suffixes: &[&str]) -> bool {
    suffixes
        .iter()
        .any(|suffix| path == format!("/api/v1/public/shares/{scope}/{suffix}"))
}

fn is_scoped_caption_path(path: &str, scope: &str) -> bool {
    let prefix = format!("/api/v1/public/shares/{scope}/captions/");
    path.strip_prefix(&prefix).is_some_and(|suffix| {
        !suffix.is_empty()
            && suffix.len() <= 80
            && suffix
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    })
}

fn approved_media_type(content_type: &str) -> bool {
    matches!(
        content_type,
        "video/mp4" | "video/webm" | "application/vnd.apple.mpegurl"
    )
}

fn valid_language(language: &str) -> bool {
    !language.is_empty()
        && language.len() <= 35
        && language
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-'))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LegacyRoute {
    CanonicalShare { share_scope: String },
    CanonicalEmbed { share_scope: String },
    PermanentRedirect { location: String },
    Unavailable,
}

#[must_use]
pub fn resolve_legacy_route(path: &str) -> LegacyRoute {
    if path.contains(['?', '#', '\\', '%']) || !path.starts_with('/') {
        return LegacyRoute::Unavailable;
    }
    let segments = path.split('/').collect::<Vec<_>>();
    match segments.as_slice() {
        ["", "s", scope] if safe_public_id(scope) => LegacyRoute::CanonicalShare {
            share_scope: (*scope).to_owned(),
        },
        ["", "embed", scope] if safe_public_id(scope) => LegacyRoute::CanonicalEmbed {
            share_scope: (*scope).to_owned(),
        },
        ["", "share", scope] if safe_public_id(scope) => LegacyRoute::PermanentRedirect {
            location: format!("/s/{scope}"),
        },
        _ => LegacyRoute::Unavailable,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DomainState {
    Verified,
    Revoked,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomDomainBinding {
    pub host: String,
    pub share_scope: String,
    pub tenant_digest: String,
    pub brand_label: Option<String>,
    pub state: DomainState,
    pub revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostResolution {
    Canonical,
    VerifiedCustom {
        origin: String,
        brand_label: Option<String>,
        revision: u64,
    },
    Unavailable,
}

#[must_use]
pub fn resolve_share_host(
    request_host: &str,
    canonical_host: &str,
    share_scope: &str,
    bindings: &[CustomDomainBinding],
) -> HostResolution {
    if request_host == canonical_host && valid_host(request_host) {
        return HostResolution::Canonical;
    }
    if !safe_public_id(share_scope) || !valid_host(request_host) {
        return HostResolution::Unavailable;
    }
    bindings
        .iter()
        .find(|binding| {
            binding.state == DomainState::Verified
                && binding.host == request_host
                && binding.share_scope == share_scope
                && valid_digest(&binding.tenant_digest)
                && binding.revision > 0
                && binding.brand_label.as_deref().is_none_or(valid_brand_label)
        })
        .map_or(HostResolution::Unavailable, |binding| {
            HostResolution::VerifiedCustom {
                origin: format!("https://{}", binding.host),
                brand_label: binding.brand_label.clone(),
                revision: binding.revision,
            }
        })
}

fn valid_host(host: &str) -> bool {
    host == host.to_ascii_lowercase()
        && host.len() <= 253
        && host.contains('.')
        && !host.starts_with('.')
        && !host.ends_with('.')
        && !host.contains(['*', ':', '/', '\\', '@'])
        && !host.split('.').any(|label| {
            label.is_empty()
                || label.starts_with('-')
                || label.ends_with('-')
                || label.starts_with("xn--")
                || !label
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        })
}

fn valid_brand_label(label: &str) -> bool {
    !label.trim().is_empty() && label.len() <= 80 && !label.chars().any(char::is_control)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewerResponseKind {
    Html,
    Media,
    Caption,
    Transcript,
    Comments,
    Analytics,
}

#[must_use]
pub const fn viewer_cache_control(kind: ViewerResponseKind) -> &'static str {
    match kind {
        ViewerResponseKind::Html => SHARE_HTML_CACHE_CONTROL,
        ViewerResponseKind::Media
        | ViewerResponseKind::Caption
        | ViewerResponseKind::Transcript
        | ViewerResponseKind::Comments
        | ViewerResponseKind::Analytics => SHARE_MEDIA_CACHE_CONTROL,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CacheRevision<'a> {
    pub tenant_digest: &'a str,
    pub share_scope: &'a str,
    pub privacy_revision: u64,
    pub deletion_revision: u64,
    pub domain_revision: u64,
}

#[must_use]
pub fn cache_partition(revision: CacheRevision<'_>) -> Option<String> {
    (valid_digest(revision.tenant_digest)
        && safe_public_id(revision.share_scope)
        && revision.privacy_revision > 0)
        .then(|| {
            format!(
                "share:{}:{}:{}:{}:{}",
                revision.tenant_digest,
                revision.share_scope,
                revision.privacy_revision,
                revision.deletion_revision,
                revision.domain_revision
            )
        })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RangePlan {
    Full {
        content_length: u64,
    },
    Partial {
        start: u64,
        end_inclusive: u64,
        content_length: u64,
        total_length: u64,
    },
    Unsatisfiable {
        total_length: u64,
    },
}

/// Parse one RFC 9110 byte range. Multiple ranges are deliberately rejected so
/// an adapter never accidentally builds an unbounded multipart response.
#[must_use]
pub fn plan_byte_range(
    range_header: Option<&str>,
    if_range_matches: bool,
    total_length: u64,
) -> RangePlan {
    let Some(range) = range_header else {
        return RangePlan::Full {
            content_length: total_length,
        };
    };
    if !if_range_matches {
        return RangePlan::Full {
            content_length: total_length,
        };
    }
    if total_length == 0 || range.trim() != range || range.contains(',') {
        return RangePlan::Unsatisfiable { total_length };
    }
    let Some(specification) = range.strip_prefix("bytes=") else {
        return RangePlan::Unsatisfiable { total_length };
    };
    let Some((start, end)) = specification.split_once('-') else {
        return RangePlan::Unsatisfiable { total_length };
    };
    let bounds = if start.is_empty() {
        let Ok(suffix) = end.parse::<u64>() else {
            return RangePlan::Unsatisfiable { total_length };
        };
        if suffix == 0 {
            return RangePlan::Unsatisfiable { total_length };
        }
        (total_length.saturating_sub(suffix), total_length - 1)
    } else {
        let Ok(start) = start.parse::<u64>() else {
            return RangePlan::Unsatisfiable { total_length };
        };
        if start >= total_length {
            return RangePlan::Unsatisfiable { total_length };
        }
        let end = if end.is_empty() {
            total_length - 1
        } else {
            let Ok(end) = end.parse::<u64>() else {
                return RangePlan::Unsatisfiable { total_length };
            };
            end.min(total_length - 1)
        };
        if start > end {
            return RangePlan::Unsatisfiable { total_length };
        }
        (start, end)
    };
    RangePlan::Partial {
        start: bounds.0,
        end_inclusive: bounds.1,
        content_length: bounds.1 - bounds.0 + 1,
        total_length,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmbedCommandEnvelope {
    pub schema: String,
    pub share_id: String,
    pub sequence: u64,
    pub command: EmbedCommand,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    content = "payload",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum EmbedCommand {
    Play,
    Pause,
    Seek { position_ms: u64 },
    SetPlaybackRate { basis_points: u16 },
    RequestState,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmbedReplyEnvelope {
    pub schema: String,
    pub share_id: String,
    pub sequence: u64,
    pub status: EmbedReplyStatus,
    pub state: Option<EmbedPlayerState>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EmbedReplyStatus {
    Accepted,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmbedPlayerState {
    pub paused: bool,
    pub position_ms: u64,
    pub playback_rate_basis_points: u16,
}

impl EmbedReplyEnvelope {
    #[must_use]
    pub fn accepted(
        share_scope: &str,
        sequence: u64,
        state: Option<EmbedPlayerState>,
    ) -> Option<Self> {
        (safe_public_id(share_scope) && sequence > 0).then(|| Self {
            schema: EMBED_REPLY_SCHEMA.to_owned(),
            share_id: share_scope.to_owned(),
            sequence,
            status: EmbedReplyStatus::Accepted,
            state,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EmbedRejection {
    InvalidConfiguration,
    Origin,
    Source,
    Schema,
    ShareScope,
    Replay,
    Command,
}

pub struct EmbedSession {
    share_scope: String,
    allowed_origins: Vec<String>,
    last_sequence: u64,
}

impl EmbedSession {
    pub fn new(share_scope: &str, allowed_origins: Vec<String>) -> Result<Self, EmbedRejection> {
        if !safe_public_id(share_scope)
            || allowed_origins.is_empty()
            || allowed_origins.len() > 32
            || allowed_origins
                .iter()
                .any(|origin| !valid_embed_origin(origin))
        {
            return Err(EmbedRejection::InvalidConfiguration);
        }
        Ok(Self {
            share_scope: share_scope.to_owned(),
            allowed_origins,
            last_sequence: 0,
        })
    }

    pub fn accept(
        &mut self,
        origin: &str,
        source_is_parent: bool,
        envelope: EmbedCommandEnvelope,
    ) -> Result<EmbedCommand, EmbedRejection> {
        if !self.allowed_origins.iter().any(|allowed| allowed == origin) {
            return Err(EmbedRejection::Origin);
        }
        if !source_is_parent {
            return Err(EmbedRejection::Source);
        }
        if envelope.schema != EMBED_COMMAND_SCHEMA {
            return Err(EmbedRejection::Schema);
        }
        if envelope.share_id != self.share_scope {
            return Err(EmbedRejection::ShareScope);
        }
        if envelope.sequence == 0 || envelope.sequence <= self.last_sequence {
            return Err(EmbedRejection::Replay);
        }
        if !valid_embed_command(&envelope.command) {
            return Err(EmbedRejection::Command);
        }
        self.last_sequence = envelope.sequence;
        Ok(envelope.command)
    }

    #[must_use]
    pub const fn last_sequence(&self) -> u64 {
        self.last_sequence
    }
}

fn valid_embed_origin(origin: &str) -> bool {
    if origin.contains(['?', '#', '\\', '@']) || origin.ends_with('/') {
        return false;
    }
    let Some(host) = origin.strip_prefix("https://") else {
        return origin
            .strip_prefix("http://")
            .is_some_and(|host| host.starts_with("127.0.0.1:") || host.starts_with("localhost:"));
    };
    valid_host(host)
}

fn valid_embed_command(command: &EmbedCommand) -> bool {
    match command {
        EmbedCommand::Play | EmbedCommand::Pause | EmbedCommand::RequestState => true,
        EmbedCommand::Seek { position_ms } => *position_ms <= MAX_RECORDING_DURATION_MS,
        EmbedCommand::SetPlaybackRate { basis_points } => {
            matches!(
                *basis_points,
                5_000 | 7_500 | 10_000 | 12_500 | 15_000 | 20_000
            )
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModerationMode {
    Publish,
    PreModerate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommentPolicy {
    pub enabled: bool,
    pub anonymous_enabled: bool,
    pub moderation: ModerationMode,
    pub maximum_per_minute: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommentBody<'a> {
    Text(&'a str),
    Reaction(&'a str),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommentCommand<'a> {
    pub operation_id: &'a str,
    pub tenant_digest: &'a str,
    pub share_scope: &'a str,
    pub parent_share_scope: Option<&'a str>,
    pub body: CommentBody<'a>,
    pub timeline_ms: Option<u64>,
    pub payload_digest: &'a str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommentActor<'a> {
    pub tenant_digest: &'a str,
    pub share_scope: &'a str,
    pub principal_digest: &'a str,
    pub anonymous: bool,
    pub can_comment: bool,
    pub grant_expires_at_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RateSnapshot {
    pub window_started_at_ms: u64,
    pub accepted: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplaySnapshot {
    New,
    SeenSamePayload,
    SeenDifferentPayload,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommentDecision {
    Publish,
    QueueModeration,
    Duplicate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommentRejection {
    Disabled,
    Scope,
    Authorization,
    AnonymousDisabled,
    InvalidBody,
    InvalidTimeline,
    RateLimited,
    ReplayConflict,
}

pub fn validate_comment(
    policy: CommentPolicy,
    actor: CommentActor<'_>,
    command: CommentCommand<'_>,
    rate: RateSnapshot,
    replay: ReplaySnapshot,
    duration_ms: u64,
    now_ms: u64,
) -> Result<CommentDecision, CommentRejection> {
    if !policy.enabled || policy.maximum_per_minute == 0 {
        return Err(CommentRejection::Disabled);
    }
    if !safe_operation_id(command.operation_id)
        || !valid_digest(command.payload_digest)
        || !valid_digest(command.tenant_digest)
        || !safe_public_id(command.share_scope)
        || command.tenant_digest != actor.tenant_digest
        || command.share_scope != actor.share_scope
        || command
            .parent_share_scope
            .is_some_and(|scope| scope != command.share_scope)
    {
        return Err(CommentRejection::Scope);
    }
    if !actor.can_comment
        || !valid_digest(actor.tenant_digest)
        || !safe_public_id(actor.share_scope)
        || !valid_digest(actor.principal_digest)
        || actor.grant_expires_at_ms <= now_ms
    {
        return Err(CommentRejection::Authorization);
    }
    if actor.anonymous && !policy.anonymous_enabled {
        return Err(CommentRejection::AnonymousDisabled);
    }
    if !valid_comment_body(command.body) {
        return Err(CommentRejection::InvalidBody);
    }
    if command
        .timeline_ms
        .is_some_and(|position| position > duration_ms || position > MAX_RECORDING_DURATION_MS)
    {
        return Err(CommentRejection::InvalidTimeline);
    }
    if rate.window_started_at_ms > now_ms
        || (now_ms - rate.window_started_at_ms < 60_000
            && rate.accepted >= policy.maximum_per_minute)
    {
        return Err(CommentRejection::RateLimited);
    }
    match replay {
        ReplaySnapshot::SeenDifferentPayload => Err(CommentRejection::ReplayConflict),
        ReplaySnapshot::SeenSamePayload => Ok(CommentDecision::Duplicate),
        ReplaySnapshot::New => Ok(match policy.moderation {
            ModerationMode::Publish => CommentDecision::Publish,
            ModerationMode::PreModerate => CommentDecision::QueueModeration,
        }),
    }
}

fn valid_comment_body(body: CommentBody<'_>) -> bool {
    match body {
        CommentBody::Text(text) => {
            !text.trim().is_empty()
                && text.len() <= 4_000
                && text
                    .chars()
                    .all(|character| !character.is_control() || matches!(character, '\n' | '\t'))
        }
        CommentBody::Reaction(reaction) => {
            !reaction.is_empty()
                && reaction.len() <= 32
                && reaction.chars().count() <= 8
                && reaction.chars().all(|character| {
                    !character.is_ascii_alphanumeric()
                        && !character.is_whitespace()
                        && !character.is_control()
                })
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct TranscriptSegment {
    pub start_ms: u64,
    pub end_ms: u64,
    pub speaker: Option<String>,
    pub text: String,
}

#[derive(Clone, PartialEq, Eq)]
pub struct TranscriptDocument {
    pub language: String,
    pub duration_ms: u64,
    pub segments: Vec<TranscriptSegment>,
}

impl fmt::Debug for TranscriptDocument {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TranscriptDocument")
            .field("language", &self.language)
            .field("duration_ms", &self.duration_ms)
            .field("segment_count", &self.segments.len())
            .field("text", &"<redacted>")
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TranscriptRejection {
    Language,
    Duration,
    TooLarge,
    Segment,
    Ordering,
}

impl TranscriptDocument {
    pub fn validate(&self) -> Result<(), TranscriptRejection> {
        if !valid_language(&self.language) {
            return Err(TranscriptRejection::Language);
        }
        if self.duration_ms > MAX_RECORDING_DURATION_MS {
            return Err(TranscriptRejection::Duration);
        }
        if self.segments.len() > 20_000
            || self
                .segments
                .iter()
                .map(|segment| segment.text.len())
                .sum::<usize>()
                > 1_000_000
        {
            return Err(TranscriptRejection::TooLarge);
        }
        let mut previous_start = 0;
        for (index, segment) in self.segments.iter().enumerate() {
            if segment.start_ms >= segment.end_ms
                || segment.end_ms > self.duration_ms
                || segment.text.trim().is_empty()
                || segment.text.len() > 4_000
                || segment
                    .text
                    .chars()
                    .any(|character| character.is_control() && !matches!(character, '\n' | '\t'))
                || segment.speaker.as_deref().is_some_and(|speaker| {
                    speaker.trim().is_empty()
                        || speaker.len() > 80
                        || speaker.chars().any(char::is_control)
                })
            {
                return Err(TranscriptRejection::Segment);
            }
            if index > 0 && segment.start_ms < previous_start {
                return Err(TranscriptRejection::Ordering);
            }
            previous_start = segment.start_ms;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnalyticsEventKind {
    PlaybackStarted,
    PlaybackPaused,
    PlaybackCompleted,
    PlaybackError,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnalyticsPolicy<'a> {
    pub enabled: bool,
    pub consent_required: bool,
    pub tenant_digest: &'a str,
    pub policy_version: &'a str,
    pub retention_days: u16,
    pub maximum_per_minute: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnalyticsConsent<'a> {
    Unknown,
    Denied,
    Granted {
        tenant_digest: &'a str,
        share_scope: &'a str,
        policy_version: &'a str,
        expires_at_ms: u64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnalyticsEvent<'a> {
    pub operation_id: &'a str,
    pub payload_digest: &'a str,
    pub tenant_digest: &'a str,
    pub share_scope: &'a str,
    pub session_digest: &'a str,
    pub sequence: u64,
    pub kind: AnalyticsEventKind,
    pub position_ms: Option<u64>,
    pub occurred_at_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnalyticsIgnore {
    Disabled,
    NoConsent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnalyticsDecision {
    Record,
    Duplicate,
    Ignore(AnalyticsIgnore),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnalyticsRejection {
    Policy,
    Scope,
    Event,
    RateLimited,
    ReplayConflict,
}

pub fn validate_analytics(
    policy: AnalyticsPolicy<'_>,
    consent: AnalyticsConsent<'_>,
    event: AnalyticsEvent<'_>,
    rate: RateSnapshot,
    replay: ReplaySnapshot,
    duration_ms: u64,
    now_ms: u64,
) -> Result<AnalyticsDecision, AnalyticsRejection> {
    if !policy.enabled {
        return Ok(AnalyticsDecision::Ignore(AnalyticsIgnore::Disabled));
    }
    if !valid_digest(policy.tenant_digest)
        || !safe_policy_version(policy.policy_version)
        || policy.retention_days == 0
        || policy.retention_days > 90
        || policy.maximum_per_minute == 0
    {
        return Err(AnalyticsRejection::Policy);
    }
    if policy.consent_required
        && !matches!(
            consent,
            AnalyticsConsent::Granted {
                tenant_digest,
                share_scope,
                policy_version,
                expires_at_ms,
            } if tenant_digest == policy.tenant_digest
                && tenant_digest == event.tenant_digest
                && share_scope == event.share_scope
                && policy_version == policy.policy_version
                && expires_at_ms > now_ms
        )
    {
        return Ok(AnalyticsDecision::Ignore(AnalyticsIgnore::NoConsent));
    }
    if !safe_operation_id(event.operation_id)
        || !valid_digest(event.payload_digest)
        || !valid_digest(event.tenant_digest)
        || event.tenant_digest != policy.tenant_digest
        || !safe_public_id(event.share_scope)
        || !valid_digest(event.session_digest)
    {
        return Err(AnalyticsRejection::Scope);
    }
    if event.sequence == 0
        || event
            .position_ms
            .is_some_and(|position| position > duration_ms || position > MAX_RECORDING_DURATION_MS)
        || event.occurred_at_ms > now_ms.saturating_add(30_000)
        || now_ms.saturating_sub(event.occurred_at_ms) > 5 * 60_000
    {
        return Err(AnalyticsRejection::Event);
    }
    if rate.window_started_at_ms > now_ms
        || (now_ms - rate.window_started_at_ms < 60_000
            && rate.accepted >= policy.maximum_per_minute)
    {
        return Err(AnalyticsRejection::RateLimited);
    }
    match replay {
        ReplaySnapshot::SeenDifferentPayload => Err(AnalyticsRejection::ReplayConflict),
        ReplaySnapshot::SeenSamePayload => Ok(AnalyticsDecision::Duplicate),
        ReplaySnapshot::New => Ok(AnalyticsDecision::Record),
    }
}

fn safe_public_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn safe_operation_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
}

fn safe_policy_version(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'-' | b'_')
        })
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

#[cfg(test)]
mod tests {
    use frame_client::{ApiVersion, PublicShareSummary};

    use super::*;

    const TENANT_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const TENANT_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const SUBJECT: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
    const PROOF: &str = "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";
    const PAYLOAD: &str = "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";
    const SESSION: &str = "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff";

    fn access(privacy: SharePrivacy, lifecycle: ShareLifecycle) -> ShareAccessRequest<'static> {
        ShareAccessRequest {
            share_scope: "public-demo",
            tenant_digest: TENANT_A,
            privacy,
            lifecycle,
            surface: ShareSurface::TopLevel,
            viewer: Viewer::Anonymous,
            password: PasswordGrant::Absent,
            embed_enabled: true,
            now_ms: 1_000,
        }
    }

    #[test]
    fn privacy_state_matrix_collapses_denials_and_terminal_states() {
        for lifecycle in [
            ShareLifecycle::Failed,
            ShareLifecycle::Deleted,
            ShareLifecycle::Unavailable,
        ] {
            for privacy in [
                SharePrivacy::Public,
                SharePrivacy::Unlisted,
                SharePrivacy::Tenant,
                SharePrivacy::Private,
                SharePrivacy::Password,
            ] {
                assert_eq!(resolve_share(access(privacy, lifecycle)), unavailable());
            }
        }
        for privacy in [SharePrivacy::Tenant, SharePrivacy::Private] {
            assert_eq!(
                resolve_share(access(privacy, ShareLifecycle::Ready)),
                unavailable()
            );
        }
        let public = resolve_share(access(SharePrivacy::Public, ShareLifecycle::Ready));
        assert!(matches!(public, ShareResolution::Ready { .. }));
        assert_eq!(public.head(), HeadPolicy::PUBLIC);
        let processing = resolve_share(access(SharePrivacy::Public, ShareLifecycle::Processing));
        assert_eq!(processing.status_code(), 202);
        assert_eq!(processing.head(), HeadPolicy::PRIVATE);
    }

    #[test]
    fn tenant_owner_explicit_and_password_grants_are_exactly_scoped() {
        let mut request = access(SharePrivacy::Tenant, ShareLifecycle::Ready);
        request.viewer = Viewer::Authenticated {
            tenant_digest: TENANT_A,
            subject_digest: SUBJECT,
            owner: false,
        };
        assert!(matches!(
            resolve_share(request),
            ShareResolution::Ready { .. }
        ));
        request.viewer = Viewer::Authenticated {
            tenant_digest: TENANT_B,
            subject_digest: SUBJECT,
            owner: true,
        };
        assert_eq!(resolve_share(request), unavailable());

        request.privacy = SharePrivacy::Private;
        request.viewer = Viewer::ExplicitGrant {
            share_scope: "public-demo",
            subject_digest: SUBJECT,
            expires_at_ms: 2_000,
        };
        assert!(matches!(
            resolve_share(request),
            ShareResolution::Ready { .. }
        ));
        request.viewer = Viewer::ExplicitGrant {
            share_scope: "another-share",
            subject_digest: SUBJECT,
            expires_at_ms: 2_000,
        };
        assert_eq!(resolve_share(request), unavailable());

        request.privacy = SharePrivacy::Password;
        request.viewer = Viewer::Anonymous;
        assert!(matches!(
            resolve_share(request),
            ShareResolution::PasswordChallenge { .. }
        ));
        request.password = PasswordGrant::Verified {
            share_scope: "public-demo",
            proof_digest: PROOF,
            expires_at_ms: 2_000,
        };
        assert!(matches!(
            resolve_share(request),
            ShareResolution::Ready { .. }
        ));
    }

    #[test]
    fn embeds_are_public_unlisted_only_and_always_noindex() {
        for privacy in [SharePrivacy::Public, SharePrivacy::Unlisted] {
            let mut request = access(privacy, ShareLifecycle::Ready);
            request.surface = ShareSurface::Embed;
            let resolution = resolve_share(request);
            assert!(matches!(resolution, ShareResolution::Ready { .. }));
            assert_eq!(resolution.head(), HeadPolicy::PRIVATE);
        }
        let mut private = access(SharePrivacy::Private, ShareLifecycle::Ready);
        private.surface = ShareSurface::Embed;
        private.viewer = Viewer::ExplicitGrant {
            share_scope: "public-demo",
            subject_digest: SUBJECT,
            expires_at_ms: 2_000,
        };
        assert_eq!(resolve_share(private), unavailable());
        private.privacy = SharePrivacy::Public;
        private.embed_enabled = false;
        assert_eq!(resolve_share(private), unavailable());
    }

    #[test]
    fn managed_and_native_descriptors_are_identical_and_provider_free() {
        let managed = provider_neutral_playback(
            DerivativeExecutor::Managed,
            "public-demo",
            "video/mp4",
            Vec::new(),
        )
        .expect("managed descriptor");
        let native = provider_neutral_playback(
            DerivativeExecutor::Native,
            "public-demo",
            "video/mp4",
            Vec::new(),
        )
        .expect("native descriptor");
        assert_eq!(managed, native);
        assert_eq!(managed.path, "/api/v1/public/shares/public-demo/media");
        assert!(!format!("{managed:?}").contains("object"));
        assert!(!format!("{native:?}").contains("cloudflare"));
    }

    #[test]
    fn summary_scope_check_rejects_cross_share_paths_and_weak_range() {
        let mut summary = PublicShareSummary {
            api_version: ApiVersion::current(),
            availability: ShareAvailability::Public,
            title: Some("Public demo".into()),
            description: None,
            canonical_url: Some("https://frame.engmanager.xyz/s/public-demo".into()),
            duration_ms: Some(10_000),
            playback: provider_neutral_playback(
                DerivativeExecutor::Native,
                "public-demo",
                "video/mp4",
                vec![CaptionTrack {
                    path: "/api/v1/public/shares/public-demo/captions/en".into(),
                    language: "en".into(),
                    label: "English".into(),
                    default: true,
                }],
            ),
            processing_status: None,
        };
        assert!(summary_is_scope_safe(&summary));
        assert!(summary_matches_route(&summary, "public-demo"));
        assert!(!summary_matches_route(&summary, "another-share"));
        summary.playback.as_mut().expect("playback").path =
            "/api/v1/public/shares/another-share/media".into();
        assert!(!summary_is_scope_safe(&summary));
        summary.playback.as_mut().expect("playback").path =
            "/api/v1/public/shares/public-demo/media".into();
        summary.playback.as_mut().expect("playback").supports_range = false;
        assert!(!summary_is_scope_safe(&summary));
    }

    #[test]
    fn legacy_routes_never_preserve_query_tokens_or_ambiguous_ids() {
        assert_eq!(
            resolve_legacy_route("/share/public-demo"),
            LegacyRoute::PermanentRedirect {
                location: "/s/public-demo".into()
            }
        );
        assert!(matches!(
            resolve_legacy_route("/s/public-demo"),
            LegacyRoute::CanonicalShare { .. }
        ));
        for path in [
            "/share/public-demo?token=secret",
            "/share/%2e%2e",
            "/share/a/b",
            "//share/public-demo",
        ] {
            assert_eq!(resolve_legacy_route(path), LegacyRoute::Unavailable);
        }
    }

    #[test]
    fn custom_domains_require_exact_live_bindings_and_partition_caches() {
        let binding = CustomDomainBinding {
            host: "watch.example.com".into(),
            share_scope: "public-demo".into(),
            tenant_digest: TENANT_A.into(),
            brand_label: Some("Example recordings".into()),
            state: DomainState::Verified,
            revision: 7,
        };
        assert!(matches!(
            resolve_share_host(
                "watch.example.com",
                "frame.engmanager.xyz",
                "public-demo",
                std::slice::from_ref(&binding)
            ),
            HostResolution::VerifiedCustom { revision: 7, .. }
        ));
        assert_eq!(
            resolve_share_host(
                "sub.watch.example.com",
                "frame.engmanager.xyz",
                "public-demo",
                std::slice::from_ref(&binding)
            ),
            HostResolution::Unavailable
        );
        let before = cache_partition(CacheRevision {
            tenant_digest: TENANT_A,
            share_scope: "public-demo",
            privacy_revision: 1,
            deletion_revision: 0,
            domain_revision: 7,
        });
        let after = cache_partition(CacheRevision {
            tenant_digest: TENANT_A,
            share_scope: "public-demo",
            privacy_revision: 2,
            deletion_revision: 1,
            domain_revision: 8,
        });
        assert_ne!(before, after);
        for kind in [
            ViewerResponseKind::Html,
            ViewerResponseKind::Media,
            ViewerResponseKind::Caption,
            ViewerResponseKind::Transcript,
            ViewerResponseKind::Comments,
            ViewerResponseKind::Analytics,
        ] {
            assert!(viewer_cache_control(kind).contains("no-store"));
        }
    }

    #[test]
    fn range_contract_handles_seek_suffix_if_range_and_416() {
        assert_eq!(
            plan_byte_range(Some("bytes=10-19"), true, 100),
            RangePlan::Partial {
                start: 10,
                end_inclusive: 19,
                content_length: 10,
                total_length: 100,
            }
        );
        assert_eq!(
            plan_byte_range(Some("bytes=-10"), true, 100),
            RangePlan::Partial {
                start: 90,
                end_inclusive: 99,
                content_length: 10,
                total_length: 100,
            }
        );
        assert_eq!(
            plan_byte_range(Some("bytes=10-19"), false, 100),
            RangePlan::Full {
                content_length: 100
            }
        );
        for invalid in ["bytes=100-", "bytes=20-10", "bytes=0-1,5-6", "items=0-1"] {
            assert_eq!(
                plan_byte_range(Some(invalid), true, 100),
                RangePlan::Unsatisfiable { total_length: 100 }
            );
        }
    }

    #[test]
    fn embed_messages_are_exact_origin_parent_scoped_and_replay_safe() {
        let mut session = EmbedSession::new("public-demo", vec!["https://engmanager.xyz".into()])
            .expect("embed session");
        let message = |sequence| EmbedCommandEnvelope {
            schema: EMBED_COMMAND_SCHEMA.into(),
            share_id: "public-demo".into(),
            sequence,
            command: EmbedCommand::Seek { position_ms: 500 },
        };
        assert_eq!(
            session.accept("https://evil.example", true, message(1)),
            Err(EmbedRejection::Origin)
        );
        assert_eq!(
            session.accept("https://engmanager.xyz", false, message(1)),
            Err(EmbedRejection::Source)
        );
        assert!(
            session
                .accept("https://engmanager.xyz", true, message(1))
                .is_ok()
        );
        assert_eq!(
            session.accept("https://engmanager.xyz", true, message(1)),
            Err(EmbedRejection::Replay)
        );
        assert_eq!(session.last_sequence(), 1);
        let reply = EmbedReplyEnvelope::accepted(
            "public-demo",
            1,
            Some(EmbedPlayerState {
                paused: false,
                position_ms: 500,
                playback_rate_basis_points: 10_000,
            }),
        )
        .expect("reply");
        let reply = serde_json::to_value(reply).expect("serialize reply");
        assert_eq!(reply["schema"], EMBED_REPLY_SCHEMA);
        assert_eq!(reply["share_id"], "public-demo");
        assert_eq!(reply["status"], "accepted");
        assert!(serde_json::from_str::<EmbedCommandEnvelope>(
            r#"{"schema":"frame.embed-command.v1","share_id":"public-demo","sequence":2,"command":{"type":"play","token":"secret"}}"#
        )
        .is_err());
    }

    fn comment_actor() -> CommentActor<'static> {
        CommentActor {
            tenant_digest: TENANT_A,
            share_scope: "public-demo",
            principal_digest: SUBJECT,
            anonymous: true,
            can_comment: true,
            grant_expires_at_ms: 2_000,
        }
    }

    fn comment_command() -> CommentCommand<'static> {
        CommentCommand {
            operation_id: "comment:01",
            tenant_digest: TENANT_A,
            share_scope: "public-demo",
            parent_share_scope: None,
            body: CommentBody::Text("Useful walkthrough"),
            timeline_ms: Some(500),
            payload_digest: PAYLOAD,
        }
    }

    #[test]
    fn comments_enforce_scope_moderation_rate_and_replay() {
        let policy = CommentPolicy {
            enabled: true,
            anonymous_enabled: true,
            moderation: ModerationMode::PreModerate,
            maximum_per_minute: 3,
        };
        assert_eq!(
            validate_comment(
                policy,
                comment_actor(),
                comment_command(),
                RateSnapshot {
                    window_started_at_ms: 0,
                    accepted: 0,
                },
                ReplaySnapshot::New,
                1_000,
                1_000,
            ),
            Ok(CommentDecision::QueueModeration)
        );
        let mut cross_tenant = comment_command();
        cross_tenant.tenant_digest = TENANT_B;
        assert_eq!(
            validate_comment(
                policy,
                comment_actor(),
                cross_tenant,
                RateSnapshot {
                    window_started_at_ms: 0,
                    accepted: 0,
                },
                ReplaySnapshot::New,
                1_000,
                1_000,
            ),
            Err(CommentRejection::Scope)
        );
        assert_eq!(
            validate_comment(
                policy,
                comment_actor(),
                comment_command(),
                RateSnapshot {
                    window_started_at_ms: 999,
                    accepted: 3,
                },
                ReplaySnapshot::New,
                1_000,
                1_000,
            ),
            Err(CommentRejection::RateLimited)
        );
        assert_eq!(
            validate_comment(
                policy,
                comment_actor(),
                comment_command(),
                RateSnapshot {
                    window_started_at_ms: 0,
                    accepted: 0,
                },
                ReplaySnapshot::SeenSamePayload,
                1_000,
                1_000,
            ),
            Ok(CommentDecision::Duplicate)
        );
    }

    #[test]
    fn transcript_is_bounded_ordered_and_debug_redacted() {
        let document = TranscriptDocument {
            language: "en-US".into(),
            duration_ms: 2_000,
            segments: vec![
                TranscriptSegment {
                    start_ms: 0,
                    end_ms: 500,
                    speaker: Some("Speaker 1".into()),
                    text: "Confidential spoken phrase".into(),
                },
                TranscriptSegment {
                    start_ms: 500,
                    end_ms: 1_000,
                    speaker: None,
                    text: "Second caption".into(),
                },
            ],
        };
        document.validate().expect("valid transcript");
        assert!(!format!("{document:?}").contains("Confidential"));
        let mut unordered = document.clone();
        unordered.segments[1].start_ms = 0;
        unordered.segments[0].start_ms = 100;
        assert_eq!(unordered.validate(), Err(TranscriptRejection::Ordering));
    }

    fn analytics_event() -> AnalyticsEvent<'static> {
        AnalyticsEvent {
            operation_id: "analytics:01",
            payload_digest: PAYLOAD,
            tenant_digest: TENANT_A,
            share_scope: "public-demo",
            session_digest: SESSION,
            sequence: 1,
            kind: AnalyticsEventKind::PlaybackStarted,
            position_ms: Some(0),
            occurred_at_ms: 1_000,
        }
    }

    #[test]
    fn analytics_is_consent_first_tenant_bound_and_idempotent() {
        let policy = AnalyticsPolicy {
            enabled: true,
            consent_required: true,
            tenant_digest: TENANT_A,
            policy_version: "analytics.v1",
            retention_days: 30,
            maximum_per_minute: 30,
        };
        let rate = RateSnapshot {
            window_started_at_ms: 0,
            accepted: 0,
        };
        assert_eq!(
            validate_analytics(
                policy,
                AnalyticsConsent::Unknown,
                analytics_event(),
                rate,
                ReplaySnapshot::New,
                2_000,
                1_000,
            ),
            Ok(AnalyticsDecision::Ignore(AnalyticsIgnore::NoConsent))
        );
        let consent = AnalyticsConsent::Granted {
            tenant_digest: TENANT_A,
            share_scope: "public-demo",
            policy_version: "analytics.v1",
            expires_at_ms: 2_000,
        };
        assert_eq!(
            validate_analytics(
                policy,
                consent,
                analytics_event(),
                rate,
                ReplaySnapshot::New,
                2_000,
                1_000,
            ),
            Ok(AnalyticsDecision::Record)
        );
        assert_eq!(
            validate_analytics(
                policy,
                consent,
                analytics_event(),
                rate,
                ReplaySnapshot::SeenSamePayload,
                2_000,
                1_000,
            ),
            Ok(AnalyticsDecision::Duplicate)
        );
        let mut cross_tenant = analytics_event();
        cross_tenant.tenant_digest = TENANT_B;
        assert_eq!(
            validate_analytics(
                policy,
                consent,
                cross_tenant,
                rate,
                ReplaySnapshot::New,
                2_000,
                1_000,
            ),
            Ok(AnalyticsDecision::Ignore(AnalyticsIgnore::NoConsent))
        );
    }
}
