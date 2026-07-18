#!/usr/bin/env python3
"""Generate and validate the Cap-to-Frame API/workflow parity inventory.

The ignored `.tmp/cap` checkout is an optional parity oracle in normal CI and
is required by `--generate`/`--require-reference`. The committed report remains
fully checkable without network access or a vendored upstream checkout.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import subprocess
import sys
from collections import Counter
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[2]
CAP = ROOT / ".tmp" / "cap"
REPORT = ROOT / "fixtures" / "api-parity" / "v1" / "route-workflow-report.json"
OPERATION_CONTRACT_CATALOG = (
    ROOT / "fixtures" / "api-parity" / "v1" / "operation-contract-catalog.json"
)
CHANGELOG_FEED = ROOT / "fixtures" / "api-parity" / "v1" / "changelog-feed.json"
THEME_ACTION_FIXTURE = ROOT / "fixtures" / "api-parity" / "v1" / "theme-action.json"
FOLDER_ASSIGNMENT_FIXTURE = (
    ROOT / "fixtures" / "api-parity" / "v1" / "folder-assignment-actions.json"
)
LIBRARY_PLACEMENT_FIXTURE = (
    ROOT / "fixtures" / "api-parity" / "v1" / "library-placement-actions.json"
)
NOTIFICATION_ACTIONS_FIXTURE = (
    ROOT / "fixtures" / "api-parity" / "v1" / "notification-actions.json"
)
NOTIFICATION_READ_FIXTURE = (
    ROOT / "fixtures" / "api-parity" / "v1" / "notification-read.json"
)
ORG_CUSTOM_DOMAIN_FIXTURE = (
    ROOT / "fixtures" / "api-parity" / "v1" / "org-custom-domain.json"
)
DEVELOPER_ACTIONS_FIXTURE = (
    ROOT / "fixtures" / "api-parity" / "v1" / "developer-actions.json"
)
MEMBERSHIP_ACTIONS_FIXTURE = (
    ROOT / "fixtures" / "api-parity" / "v1" / "membership-actions.json"
)
FOLDER_CRUD_FIXTURE = (
    ROOT / "fixtures" / "api-parity" / "v1" / "folder-crud.json"
)
USER_ACCOUNT_FIXTURE = (
    ROOT / "fixtures" / "api-parity" / "v1" / "user-account.json"
)
MESSENGER_RETIREMENT_FIXTURE = (
    ROOT / "fixtures" / "api-parity" / "v1" / "messenger-retirement.json"
)
LIBRARY_ID_READS_FIXTURE = (
    ROOT / "fixtures" / "api-parity" / "v1" / "library-id-reads.json"
)
LIBRARY_DETAIL_READS_FIXTURE = (
    ROOT / "fixtures" / "api-parity" / "v1" / "library-detail-reads.json"
)
SPACE_AUTHORIZATION_FIXTURE = (
    ROOT / "fixtures" / "api-parity" / "v1" / "space-authorization.json"
)
INVITE_LIFECYCLE_FIXTURE = (
    ROOT / "fixtures" / "api-parity" / "v1" / "invite-lifecycle.json"
)
MOBILE_SESSION_FIXTURE = (
    ROOT / "fixtures" / "api-parity" / "v1" / "mobile-session.json"
)
MOBILE_BOOTSTRAP_CAPS_FIXTURE = (
    ROOT / "fixtures" / "api-parity" / "v1" / "mobile-bootstrap-caps.json"
)
MOBILE_UPLOADS_FIXTURE = (
    ROOT / "fixtures" / "api-parity" / "v1" / "mobile-uploads.json"
)
EXTENSION_AUTH_FIXTURE = (
    ROOT / "fixtures" / "api-parity" / "v1" / "extension-auth.json"
)
EXTENSION_INSTANT_FIXTURE = (
    ROOT / "fixtures" / "api-parity" / "v1" / "extension-instant-recordings.json"
)
DESKTOP_SESSION_FIXTURE = (
    ROOT / "fixtures" / "api-parity" / "v1" / "desktop-session.json"
)
DESKTOP_COMPATIBILITY_FIXTURE = (
    ROOT / "fixtures" / "api-parity" / "v1" / "desktop-compatibility.json"
)
VIDEO_DOMAIN_INFO_FIXTURE = (
    ROOT / "fixtures" / "api-parity" / "v1" / "video-domain-info.json"
)
TRANSCRIPTS_FIXTURE = (
    ROOT / "fixtures" / "api-parity" / "v1" / "transcripts.json"
)
VIDEO_LIFECYCLE_FIXTURE = (
    ROOT / "fixtures" / "api-parity" / "v1" / "video-lifecycle.json"
)
CORE_STORAGE_FIXTURE = (
    ROOT / "fixtures" / "api-parity" / "v1" / "core-storage.json"
)
UPLOAD_STORAGE_FIXTURE = (
    ROOT / "fixtures" / "api-parity" / "v1" / "upload-storage.json"
)
ANALYTICS_FIXTURE = ROOT / "fixtures" / "api-parity" / "v1" / "analytics.json"
ORGANIZATION_LIBRARY_FIXTURE = (
    ROOT / "fixtures" / "api-parity" / "v1" / "organization-library.json"
)
PROTECTED_MEDIA_FIXTURE = (
    ROOT / "fixtures" / "api-parity" / "v1" / "protected-media-contracts.json"
)
PROTECTED_INTEGRATIONS_FIXTURE = (
    ROOT / "fixtures" / "api-parity" / "v1" / "protected-integrations.json"
)
PROTECTED_BILLING_AUTH_FIXTURE = (
    ROOT / "fixtures" / "api-parity" / "v1" / "protected-billing-auth.json"
)
DEVELOPER_API_FIXTURE = (
    ROOT / "fixtures" / "api-parity" / "v1" / "developer-api.json"
)
DECLARATION_ONLY_DISPOSITIONS_FIXTURE = (
    ROOT
    / "fixtures"
    / "api-parity"
    / "v1"
    / "declaration-only-dispositions.json"
)
DOC = ROOT / "docs" / "generated" / "api-workflow-parity-v1.md"
REGISTRY = ROOT / "crates" / "application" / "src" / "legacy_compatibility.rs"
APPLICATION_LIB = ROOT / "crates" / "application" / "src" / "lib.rs"
CONTROL_RUNTIME = ROOT / "apps" / "control-plane" / "src" / "legacy_compatibility_runtime.rs"
CONTROL_LIB = ROOT / "apps" / "control-plane" / "src" / "lib.rs"
CONTROL_BROWSER_RUNTIME = ROOT / "apps" / "control-plane" / "src" / "browser_web_runtime.rs"
CONTROL_WEB_ACTION_RUNTIME = (
    ROOT / "apps" / "control-plane" / "src" / "legacy_web_action_runtime.rs"
)
CONTROL_ROUTING = ROOT / "apps" / "control-plane" / "src" / "routing.rs"
APPLICATION_THEME = ROOT / "crates" / "application" / "src" / "legacy_theme.rs"
APPLICATION_FOLDER_ASSIGNMENT = (
    ROOT / "crates" / "application" / "src" / "legacy_folder_assignment.rs"
)
APPLICATION_NOTIFICATION_ACTIONS = (
    ROOT / "crates" / "application" / "src" / "legacy_notification_actions.rs"
)
APPLICATION_NOTIFICATION_READ = (
    ROOT / "crates" / "application" / "src" / "legacy_notification_read.rs"
)
APPLICATION_ORG_CUSTOM_DOMAIN = (
    ROOT / "crates" / "application" / "src" / "legacy_org_custom_domain.rs"
)
APPLICATION_DEVELOPER_ACTIONS = (
    ROOT / "crates" / "application" / "src" / "legacy_developer_actions.rs"
)
APPLICATION_MEMBERSHIP_ACTIONS = (
    ROOT / "crates" / "application" / "src" / "legacy_membership_actions.rs"
)
APPLICATION_FOLDER_CRUD = (
    ROOT / "crates" / "application" / "src" / "legacy_folder_crud.rs"
)
CONTROL_FOLDER_RUNTIME = (
    ROOT / "apps" / "control-plane" / "src" / "legacy_folder_assignment_runtime.rs"
)
CONTROL_FOLDER_WEB_RUNTIME = (
    ROOT / "apps" / "control-plane" / "src" / "legacy_folder_web_runtime.rs"
)
FOLDER_ASSIGNMENT_MIGRATION = (
    ROOT
    / "apps"
    / "control-plane"
    / "migrations"
    / "0036_legacy_folder_assignment_expand.sql"
)
FOLDER_ASSIGNMENT_QUERY_ROOT = (
    ROOT / "apps" / "control-plane" / "queries" / "legacy_folder_assignment"
)
FOLDER_ASSIGNMENT_CONFORMANCE = (
    ROOT / "scripts" / "ci" / "legacy-folder-assignment-sqlite-conformance.py"
)
CONTROL_NOTIFICATION_ACTIONS_RUNTIME = (
    ROOT / "apps" / "control-plane" / "src" / "legacy_notification_actions_runtime.rs"
)
CONTROL_NOTIFICATION_WEB_RUNTIME = (
    ROOT / "apps" / "control-plane" / "src" / "legacy_notification_web_runtime.rs"
)
NOTIFICATION_ACTIONS_MIGRATION = (
    ROOT
    / "apps"
    / "control-plane"
    / "migrations"
    / "0038_legacy_notification_actions_expand.sql"
)
NOTIFICATION_ACTIONS_QUERY_ROOT = (
    ROOT / "apps" / "control-plane" / "queries" / "legacy_notification_actions"
)
NOTIFICATION_ACTIONS_CONFORMANCE = (
    ROOT / "scripts" / "ci" / "legacy-notification-actions-sqlite-conformance.py"
)
CONTROL_DEVELOPER_ACTIONS_RUNTIME = (
    ROOT / "apps" / "control-plane" / "src" / "legacy_developer_actions_runtime.rs"
)
CONTROL_DEVELOPER_WEB_RUNTIME = (
    ROOT / "apps" / "control-plane" / "src" / "legacy_developer_web_runtime.rs"
)
DEVELOPER_ACTIONS_MIGRATION = (
    ROOT
    / "apps"
    / "control-plane"
    / "migrations"
    / "0039_legacy_developer_actions_expand.sql"
)
DEVELOPER_ACTIONS_QUERY_ROOT = (
    ROOT / "apps" / "control-plane" / "queries" / "legacy_developer_actions"
)
DEVELOPER_ACTIONS_CONFORMANCE = (
    ROOT / "scripts" / "ci" / "legacy-developer-actions-sqlite-conformance.py"
)
CONTROL_MEMBERSHIP_ACTIONS_RUNTIME = (
    ROOT / "apps" / "control-plane" / "src" / "legacy_membership_actions_runtime.rs"
)
CONTROL_MEMBERSHIP_WEB_RUNTIME = (
    ROOT / "apps" / "control-plane" / "src" / "legacy_membership_web_runtime.rs"
)
MEMBERSHIP_ACTIONS_MIGRATION = (
    ROOT
    / "apps"
    / "control-plane"
    / "migrations"
    / "0040_legacy_membership_actions_expand.sql"
)
MEMBERSHIP_ACTIONS_QUERY_ROOT = (
    ROOT / "apps" / "control-plane" / "queries" / "legacy_membership_actions"
)
MEMBERSHIP_ACTIONS_CONFORMANCE = (
    ROOT / "scripts" / "ci" / "legacy-membership-actions-sqlite-conformance.py"
)
CONTROL_FOLDER_CRUD_RUNTIME = (
    ROOT / "apps" / "control-plane" / "src" / "legacy_folder_crud_runtime.rs"
)
CONTROL_FOLDER_CRUD_WEB_RUNTIME = (
    ROOT / "apps" / "control-plane" / "src" / "legacy_folder_crud_web_runtime.rs"
)
FOLDER_CRUD_MIGRATION = (
    ROOT / "apps" / "control-plane" / "migrations" / "0041_legacy_folder_crud_expand.sql"
)
FOLDER_CRUD_QUERY_ROOT = ROOT / "apps" / "control-plane" / "queries" / "legacy_folder_crud"
FOLDER_CRUD_CONFORMANCE = (
    ROOT / "scripts" / "ci" / "legacy-folder-crud-sqlite-conformance.py"
)
APPLICATION_USER_ACCOUNT = (
    ROOT / "crates" / "application" / "src" / "legacy_user_account.rs"
)
CONTROL_USER_ACCOUNT_RUNTIME = (
    ROOT / "apps" / "control-plane" / "src" / "legacy_user_account_runtime.rs"
)
CONTROL_USER_ACCOUNT_WEB_RUNTIME = (
    ROOT / "apps" / "control-plane" / "src" / "legacy_user_account_web_runtime.rs"
)
USER_ACCOUNT_MIGRATION = (
    ROOT / "apps" / "control-plane" / "migrations" / "0042_legacy_user_account_expand.sql"
)
USER_ACCOUNT_QUERY_ROOT = (
    ROOT / "apps" / "control-plane" / "queries" / "legacy_user_account"
)
USER_ACCOUNT_CONFORMANCE = (
    ROOT / "scripts" / "ci" / "legacy-user-account-sqlite-conformance.py"
)
COLLABORATION_FIXTURE = (
    ROOT / "fixtures" / "api-parity" / "v1" / "collaboration-actions.json"
)
APPLICATION_COLLABORATION = (
    ROOT / "crates" / "application" / "src" / "legacy_collaboration.rs"
)
CONTROL_COLLABORATION_RUNTIME = (
    ROOT / "apps" / "control-plane" / "src" / "legacy_collaboration_runtime.rs"
)
CONTROL_COLLABORATION_WEB_RUNTIME = (
    ROOT / "apps" / "control-plane" / "src" / "legacy_collaboration_web_runtime.rs"
)
COLLABORATION_MIGRATION = (
    ROOT / "apps" / "control-plane" / "migrations" / "0043_legacy_collaboration_expand.sql"
)
COLLABORATION_QUERY_ROOT = (
    ROOT / "apps" / "control-plane" / "queries" / "legacy_collaboration"
)
COLLABORATION_CONFORMANCE = (
    ROOT / "scripts" / "ci" / "legacy-collaboration-sqlite-conformance.py"
)
VIDEO_PROPERTIES_FIXTURE = (
    ROOT / "fixtures" / "api-parity" / "v1" / "video-properties.json"
)
APPLICATION_VIDEO_PROPERTIES = (
    ROOT / "crates" / "application" / "src" / "legacy_video_properties.rs"
)
CONTROL_VIDEO_PROPERTIES_RUNTIME = (
    ROOT / "apps" / "control-plane" / "src" / "legacy_video_properties_runtime.rs"
)
CONTROL_VIDEO_PROPERTIES_WEB_RUNTIME = (
    ROOT
    / "apps"
    / "control-plane"
    / "src"
    / "legacy_video_properties_web_runtime.rs"
)
VIDEO_PROPERTIES_MIGRATION = (
    ROOT
    / "apps"
    / "control-plane"
    / "migrations"
    / "0044_legacy_video_properties_expand.sql"
)
VIDEO_PROPERTIES_QUERY_ROOT = (
    ROOT / "apps" / "control-plane" / "queries" / "legacy_video_properties"
)
VIDEO_PROPERTIES_CONFORMANCE = (
    ROOT / "scripts" / "ci" / "legacy-video-properties-sqlite-conformance.py"
)
APPLICATION_LIBRARY_ID_READS = (
    ROOT / "crates" / "application" / "src" / "legacy_library_id_reads.rs"
)
CONTROL_LIBRARY_ID_READ_RUNTIME = (
    ROOT / "apps" / "control-plane" / "src" / "legacy_library_id_read_runtime.rs"
)
CONTROL_LIBRARY_ID_READ_WEB_RUNTIME = (
    ROOT / "apps" / "control-plane" / "src" / "legacy_library_id_read_web_runtime.rs"
)
LIBRARY_ID_READ_MIGRATION = (
    ROOT
    / "apps"
    / "control-plane"
    / "migrations"
    / "0045_legacy_library_id_reads_expand.sql"
)
LIBRARY_ID_READ_QUERY_ROOT = (
    ROOT / "apps" / "control-plane" / "queries" / "legacy_library_id_reads"
)
LIBRARY_ID_READ_CONFORMANCE = (
    ROOT / "scripts" / "ci" / "legacy-library-id-reads-sqlite-conformance.py"
)
APPLICATION_LIBRARY_DETAIL_READS = (
    ROOT / "crates" / "application" / "src" / "legacy_library_detail_reads.rs"
)
CONTROL_LIBRARY_DETAIL_READ_RUNTIME = (
    ROOT / "apps" / "control-plane" / "src" / "legacy_library_detail_read_runtime.rs"
)
CONTROL_LIBRARY_DETAIL_READ_WEB_RUNTIME = (
    ROOT
    / "apps"
    / "control-plane"
    / "src"
    / "legacy_library_detail_read_web_runtime.rs"
)
LIBRARY_DETAIL_READ_MIGRATION = (
    ROOT
    / "apps"
    / "control-plane"
    / "migrations"
    / "0046_legacy_library_detail_reads_expand.sql"
)
LIBRARY_DETAIL_READ_QUERY_ROOT = (
    ROOT / "apps" / "control-plane" / "queries" / "legacy_library_detail_reads"
)
LIBRARY_DETAIL_READ_CONFORMANCE = (
    ROOT / "scripts" / "ci" / "legacy-library-detail-reads-sqlite-conformance.py"
)
APPLICATION_SPACE_AUTHORIZATION = (
    ROOT / "crates" / "application" / "src" / "legacy_space_authorization.rs"
)
CONTROL_SPACE_AUTHORIZATION_RUNTIME = (
    ROOT / "apps" / "control-plane" / "src" / "legacy_space_authorization_runtime.rs"
)
CONTROL_SPACE_AUTHORIZATION_WEB_RUNTIME = (
    ROOT
    / "apps"
    / "control-plane"
    / "src"
    / "legacy_space_authorization_web_runtime.rs"
)
SPACE_AUTHORIZATION_QUERY_ROOT = (
    ROOT / "apps" / "control-plane" / "queries" / "legacy_space_authorization"
)
SPACE_AUTHORIZATION_CONFORMANCE = (
    ROOT / "scripts" / "ci" / "legacy-space-authorization-sqlite-conformance.py"
)
APPLICATION_INVITE_LIFECYCLE = (
    ROOT / "crates" / "application" / "src" / "legacy_invite_lifecycle.rs"
)
CONTROL_INVITE_LIFECYCLE_RUNTIME = (
    ROOT / "apps" / "control-plane" / "src" / "legacy_invite_lifecycle_runtime.rs"
)
CONTROL_INVITE_LIFECYCLE_WEB_RUNTIME = (
    ROOT / "apps" / "control-plane" / "src" / "legacy_invite_lifecycle_web_runtime.rs"
)
INVITE_LIFECYCLE_MIGRATION = (
    ROOT / "apps" / "control-plane" / "migrations" / "0047_legacy_invite_lifecycle_expand.sql"
)
INVITE_LIFECYCLE_QUERY_ROOT = (
    ROOT / "apps" / "control-plane" / "queries" / "legacy_invite_lifecycle"
)
INVITE_LIFECYCLE_CONFORMANCE = (
    ROOT / "scripts" / "ci" / "legacy-invite-lifecycle-sqlite-conformance.py"
)
APPLICATION_MOBILE_SESSION = (
    ROOT / "crates" / "application" / "src" / "legacy_mobile_session.rs"
)
CONTROL_MOBILE_SESSION_RUNTIME = (
    ROOT / "apps" / "control-plane" / "src" / "legacy_mobile_session_runtime.rs"
)
CONTROL_MOBILE_SESSION_WEB_RUNTIME = (
    ROOT / "apps" / "control-plane" / "src" / "legacy_mobile_session_web_runtime.rs"
)
MOBILE_SESSION_MIGRATION = (
    ROOT / "apps" / "control-plane" / "migrations" / "0049_legacy_mobile_session_expand.sql"
)
MOBILE_SESSION_QUERY_ROOT = (
    ROOT / "apps" / "control-plane" / "queries" / "legacy_mobile_session"
)
MOBILE_SESSION_CONFORMANCE = (
    ROOT / "scripts" / "ci" / "legacy-mobile-session-sqlite-conformance.py"
)
APPLICATION_MOBILE_BOOTSTRAP_CAPS = (
    ROOT / "crates" / "application" / "src" / "legacy_mobile_bootstrap_caps.rs"
)
CONTROL_MOBILE_BOOTSTRAP_CAPS_RUNTIME = (
    ROOT
    / "apps"
    / "control-plane"
    / "src"
    / "legacy_mobile_bootstrap_caps_runtime.rs"
)
CONTROL_MOBILE_BOOTSTRAP_CAPS_WEB_RUNTIME = (
    ROOT
    / "apps"
    / "control-plane"
    / "src"
    / "legacy_mobile_bootstrap_caps_web_runtime.rs"
)
MOBILE_BOOTSTRAP_CAPS_MIGRATION = (
    ROOT
    / "apps"
    / "control-plane"
    / "migrations"
    / "0052_legacy_mobile_bootstrap_caps_expand.sql"
)
MOBILE_BOOTSTRAP_CAPS_QUERY_ROOT = (
    ROOT / "apps" / "control-plane" / "queries" / "legacy_mobile_bootstrap_caps"
)
MOBILE_BOOTSTRAP_CAPS_CONFORMANCE = (
    ROOT / "scripts" / "ci" / "legacy-mobile-bootstrap-caps-sqlite-conformance.py"
)
APPLICATION_MOBILE_UPLOADS = (
    ROOT / "crates" / "application" / "src" / "legacy_mobile_uploads.rs"
)
CONTROL_MOBILE_UPLOADS_RUNTIME = (
    ROOT / "apps" / "control-plane" / "src" / "legacy_mobile_uploads_runtime.rs"
)
CONTROL_MOBILE_UPLOADS_WEB_RUNTIME = (
    ROOT / "apps" / "control-plane" / "src" / "legacy_mobile_uploads_web_runtime.rs"
)
MOBILE_UPLOADS_MIGRATION = (
    ROOT
    / "apps"
    / "control-plane"
    / "migrations"
    / "0056_legacy_mobile_uploads_expand.sql"
)
MOBILE_UPLOADS_QUERY_ROOT = (
    ROOT / "apps" / "control-plane" / "queries" / "legacy_mobile_uploads"
)
MOBILE_UPLOADS_CONFORMANCE = (
    ROOT / "scripts" / "ci" / "legacy-mobile-uploads-sqlite-conformance.py"
)
APPLICATION_TRANSCRIPTS = (
    ROOT / "crates" / "application" / "src" / "legacy_transcripts.rs"
)
CONTROL_TRANSCRIPTS_RUNTIME = (
    ROOT / "apps" / "control-plane" / "src" / "legacy_transcripts_runtime.rs"
)
CONTROL_TRANSCRIPTS_WEB_RUNTIME = (
    ROOT / "apps" / "control-plane" / "src" / "legacy_transcripts_web_runtime.rs"
)
TRANSCRIPTS_MIGRATION = (
    ROOT
    / "apps"
    / "control-plane"
    / "migrations"
    / "0055_legacy_transcripts_expand.sql"
)
TRANSCRIPTS_QUERY_ROOT = (
    ROOT / "apps" / "control-plane" / "queries" / "legacy_transcripts"
)
TRANSCRIPTS_CONFORMANCE = (
    ROOT / "scripts" / "ci" / "legacy-transcripts-sqlite-conformance.py"
)
APPLICATION_DEVELOPER_API = (
    ROOT / "crates" / "application" / "src" / "legacy_developer_api.rs"
)
CONTROL_DEVELOPER_API_RUNTIME = (
    ROOT / "apps" / "control-plane" / "src" / "legacy_developer_api_runtime.rs"
)
CONTROL_DEVELOPER_API_WEB_RUNTIME = (
    ROOT / "apps" / "control-plane" / "src" / "legacy_developer_api_web_runtime.rs"
)
DEVELOPER_API_MIGRATION = (
    ROOT
    / "apps"
    / "control-plane"
    / "migrations"
    / "0054_legacy_developer_api_expand.sql"
)
DEVELOPER_API_QUERY_ROOT = (
    ROOT / "apps" / "control-plane" / "queries" / "legacy_developer_api"
)
DEVELOPER_API_CONFORMANCE = (
    ROOT / "scripts" / "ci" / "legacy-developer-api-sqlite-conformance.py"
)
APPLICATION_EXTENSION_AUTH = (
    ROOT / "crates" / "application" / "src" / "legacy_extension_auth.rs"
)
CONTROL_EXTENSION_AUTH_RUNTIME = (
    ROOT / "apps" / "control-plane" / "src" / "legacy_extension_auth_runtime.rs"
)
CONTROL_EXTENSION_AUTH_WEB_RUNTIME = (
    ROOT / "apps" / "control-plane" / "src" / "legacy_extension_auth_web_runtime.rs"
)
EXTENSION_AUTH_MIGRATION = (
    ROOT / "apps" / "control-plane" / "migrations" / "0048_legacy_extension_auth_expand.sql"
)
EXTENSION_AUTH_QUERY_ROOT = (
    ROOT / "apps" / "control-plane" / "queries" / "legacy_extension_auth"
)
EXTENSION_AUTH_CONFORMANCE = (
    ROOT / "scripts" / "ci" / "legacy-extension-auth-sqlite-conformance.py"
)
APPLICATION_EXTENSION_INSTANT = (
    ROOT / "crates" / "application" / "src" / "legacy_extension_instant_recordings.rs"
)
CONTROL_EXTENSION_INSTANT_RUNTIME = (
    ROOT
    / "apps"
    / "control-plane"
    / "src"
    / "legacy_extension_instant_recordings_runtime.rs"
)
CONTROL_EXTENSION_INSTANT_WEB_RUNTIME = (
    ROOT
    / "apps"
    / "control-plane"
    / "src"
    / "legacy_extension_instant_recordings_web_runtime.rs"
)
EXTENSION_INSTANT_MIGRATION = (
    ROOT
    / "apps"
    / "control-plane"
    / "migrations"
    / "0051_legacy_extension_instant_recordings_expand.sql"
)
EXTENSION_INSTANT_QUERY_ROOT = (
    ROOT / "apps" / "control-plane" / "queries" / "legacy_extension_instant"
)
EXTENSION_INSTANT_CONFORMANCE = (
    ROOT
    / "scripts"
    / "ci"
    / "legacy-extension-instant-recordings-sqlite-conformance.py"
)
R2_DIRECT_UPLOAD_RUNTIME = (
    ROOT / "apps" / "control-plane" / "src" / "r2_direct_upload.rs"
)
APPLICATION_DESKTOP_SESSION = (
    ROOT / "crates" / "application" / "src" / "legacy_desktop_session.rs"
)
CONTROL_DESKTOP_SESSION_RUNTIME = (
    ROOT / "apps" / "control-plane" / "src" / "legacy_desktop_session_runtime.rs"
)
CONTROL_DESKTOP_SESSION_WEB_RUNTIME = (
    ROOT / "apps" / "control-plane" / "src" / "legacy_desktop_session_web_runtime.rs"
)
DESKTOP_SESSION_QUERY_ROOT = (
    ROOT / "apps" / "control-plane" / "queries" / "legacy_desktop_session"
)
DESKTOP_SESSION_CONFORMANCE = (
    ROOT / "scripts" / "ci" / "legacy-desktop-session-sqlite-conformance.py"
)
APPLICATION_DESKTOP_COMPATIBILITY = (
    ROOT / "crates" / "application" / "src" / "legacy_desktop_compatibility.rs"
)
CONTROL_DESKTOP_COMPATIBILITY_RUNTIME = (
    ROOT / "apps" / "control-plane" / "src" / "legacy_desktop_compatibility_runtime.rs"
)
CONTROL_DESKTOP_COMPATIBILITY_WEB_RUNTIME = (
    ROOT / "apps" / "control-plane" / "src" / "legacy_desktop_compatibility_web_runtime.rs"
)
DESKTOP_COMPATIBILITY_MIGRATION = (
    ROOT
    / "apps"
    / "control-plane"
    / "migrations"
    / "0050_legacy_desktop_compatibility_expand.sql"
)
DESKTOP_COMPATIBILITY_QUERY_ROOT = (
    ROOT / "apps" / "control-plane" / "queries" / "legacy_desktop_compatibility"
)
DESKTOP_COMPATIBILITY_CONFORMANCE = (
    ROOT / "scripts" / "ci" / "legacy-desktop-compatibility-sqlite-conformance.py"
)
APPLICATION_VIDEO_DOMAIN_INFO = (
    ROOT / "crates" / "application" / "src" / "legacy_video_domain_info.rs"
)
CONTROL_VIDEO_DOMAIN_INFO_RUNTIME = (
    ROOT / "apps" / "control-plane" / "src" / "legacy_video_domain_info_runtime.rs"
)
CONTROL_VIDEO_DOMAIN_INFO_WEB_RUNTIME = (
    ROOT
    / "apps"
    / "control-plane"
    / "src"
    / "legacy_video_domain_info_web_runtime.rs"
)
VIDEO_DOMAIN_INFO_QUERY_ROOT = (
    ROOT / "apps" / "control-plane" / "queries" / "legacy_video_domain_info"
)
VIDEO_DOMAIN_INFO_CONFORMANCE = (
    ROOT / "scripts" / "ci" / "legacy-video-domain-info-sqlite-conformance.py"
)
APPLICATION_VIDEO_LIFECYCLE = (
    ROOT / "crates" / "application" / "src" / "legacy_video_lifecycle.rs"
)
CONTROL_VIDEO_LIFECYCLE_RUNTIME = (
    ROOT / "apps" / "control-plane" / "src" / "legacy_video_lifecycle_runtime.rs"
)
CONTROL_VIDEO_LIFECYCLE_WEB_RUNTIME = (
    ROOT / "apps" / "control-plane" / "src" / "legacy_video_lifecycle_web_runtime.rs"
)
VIDEO_LIFECYCLE_MIGRATION = (
    ROOT
    / "apps"
    / "control-plane"
    / "migrations"
    / "0060_legacy_video_lifecycle_expand.sql"
)
VIDEO_LIFECYCLE_QUERY_ROOT = (
    ROOT / "apps" / "control-plane" / "queries" / "legacy_video_lifecycle"
)
VIDEO_LIFECYCLE_CONFORMANCE = (
    ROOT / "scripts" / "ci" / "legacy-video-lifecycle-sqlite-conformance.py"
)
APPLICATION_CORE_STORAGE = (
    ROOT / "crates" / "application" / "src" / "legacy_core_storage.rs"
)
CONTROL_CORE_STORAGE_RUNTIME = (
    ROOT / "apps" / "control-plane" / "src" / "legacy_core_storage_runtime.rs"
)
CONTROL_CORE_STORAGE_WEB_RUNTIME = (
    ROOT / "apps" / "control-plane" / "src" / "legacy_core_storage_web_runtime.rs"
)
CORE_STORAGE_MIGRATION = (
    ROOT
    / "apps"
    / "control-plane"
    / "migrations"
    / "0053_legacy_core_storage_expand.sql"
)
CORE_STORAGE_QUERY_ROOT = (
    ROOT / "apps" / "control-plane" / "queries" / "legacy_core_storage"
)
CORE_STORAGE_CONFORMANCE = (
    ROOT / "scripts" / "ci" / "legacy-core-storage-sqlite-conformance.py"
)
APPLICATION_UPLOAD_STORAGE = (
    ROOT / "crates" / "application" / "src" / "legacy_upload_storage.rs"
)
CONTROL_UPLOAD_STORAGE_RUNTIME = (
    ROOT / "apps" / "control-plane" / "src" / "legacy_upload_storage_runtime.rs"
)
CONTROL_UPLOAD_STORAGE_WEB_RUNTIME = (
    ROOT / "apps" / "control-plane" / "src" / "legacy_upload_storage_web_runtime.rs"
)
UPLOAD_STORAGE_MIGRATION = (
    ROOT
    / "apps"
    / "control-plane"
    / "migrations"
    / "0057_legacy_upload_storage_expand.sql"
)
UPLOAD_STORAGE_QUERY_ROOT = (
    ROOT / "apps" / "control-plane" / "queries" / "legacy_upload_storage"
)
UPLOAD_STORAGE_CONFORMANCE = (
    ROOT / "scripts" / "ci" / "legacy-upload-storage-sqlite-conformance.py"
)
APPLICATION_ANALYTICS = (
    ROOT / "crates" / "application" / "src" / "legacy_analytics.rs"
)
CONTROL_ANALYTICS_RUNTIME = (
    ROOT / "apps" / "control-plane" / "src" / "legacy_analytics_runtime.rs"
)
CONTROL_ANALYTICS_WEB_RUNTIME = (
    ROOT / "apps" / "control-plane" / "src" / "legacy_analytics_web_runtime.rs"
)
ANALYTICS_MIGRATION = (
    ROOT
    / "apps"
    / "control-plane"
    / "migrations"
    / "0059_legacy_analytics_expand.sql"
)
ANALYTICS_QUERY_ROOT = (
    ROOT / "apps" / "control-plane" / "queries" / "legacy_analytics"
)
ANALYTICS_CONFORMANCE = (
    ROOT / "scripts" / "ci" / "legacy-analytics-sqlite-conformance.py"
)
APPLICATION_ORGANIZATION_LIBRARY = (
    ROOT / "crates" / "application" / "src" / "legacy_organization_library.rs"
)
CONTROL_ORGANIZATION_LIBRARY_RUNTIME = (
    ROOT / "apps" / "control-plane" / "src" / "legacy_organization_library_runtime.rs"
)
CONTROL_ORGANIZATION_LIBRARY_WEB_RUNTIME = (
    ROOT
    / "apps"
    / "control-plane"
    / "src"
    / "legacy_organization_library_web_runtime.rs"
)
ORGANIZATION_LIBRARY_MIGRATION = (
    ROOT
    / "apps"
    / "control-plane"
    / "migrations"
    / "0058_legacy_organization_library_expand.sql"
)
ORGANIZATION_LIBRARY_QUERY_ROOT = (
    ROOT / "apps" / "control-plane" / "queries" / "legacy_organization_library"
)
ORGANIZATION_LIBRARY_CONFORMANCE = (
    ROOT / "scripts" / "ci" / "legacy-organization-library-sqlite-conformance.py"
)
APPLICATION_PROTECTED_MEDIA = (
    ROOT / "crates" / "application" / "src" / "legacy_protected_media.rs"
)
CONTROL_PROTECTED_MEDIA_RUNTIME = (
    ROOT / "apps" / "control-plane" / "src" / "legacy_protected_media_runtime.rs"
)
CONTROL_PROTECTED_MEDIA_WEB_RUNTIME = (
    ROOT / "apps" / "control-plane" / "src" / "legacy_protected_media_web_runtime.rs"
)
PROTECTED_MEDIA_MIGRATION = (
    ROOT
    / "apps"
    / "control-plane"
    / "migrations"
    / "0061_legacy_protected_media_expand.sql"
)
PROTECTED_MEDIA_QUERY_ROOT = (
    ROOT / "apps" / "control-plane" / "queries" / "legacy_protected_media"
)
PROTECTED_MEDIA_CONFORMANCE = (
    ROOT / "scripts" / "ci" / "legacy-protected-media-sqlite-conformance.py"
)
APPLICATION_PROTECTED_INTEGRATIONS = (
    ROOT / "crates" / "application" / "src" / "legacy_protected_integrations.rs"
)
CONTROL_PROTECTED_INTEGRATIONS_RUNTIME = (
    ROOT
    / "apps"
    / "control-plane"
    / "src"
    / "legacy_protected_integrations_runtime.rs"
)
CONTROL_PROTECTED_INTEGRATIONS_WEB_RUNTIME = (
    ROOT
    / "apps"
    / "control-plane"
    / "src"
    / "legacy_protected_integrations_web_runtime.rs"
)
PROTECTED_INTEGRATIONS_MIGRATION = (
    ROOT
    / "apps"
    / "control-plane"
    / "migrations"
    / "0062_legacy_protected_integrations_expand.sql"
)
PROTECTED_INTEGRATIONS_QUERY_ROOT = (
    ROOT / "apps" / "control-plane" / "queries" / "legacy_protected_integrations"
)
PROTECTED_INTEGRATIONS_CONFORMANCE = (
    ROOT / "scripts" / "ci" / "legacy-protected-integrations-sqlite-conformance.py"
)
APPLICATION_PROTECTED_BILLING_AUTH = (
    ROOT / "crates" / "application" / "src" / "legacy_protected_billing_auth.rs"
)
CONTROL_PROTECTED_BILLING_AUTH_RUNTIME = (
    ROOT
    / "apps"
    / "control-plane"
    / "src"
    / "legacy_protected_billing_auth_runtime.rs"
)
CONTROL_PROTECTED_BILLING_AUTH_WEB_RUNTIME = (
    ROOT
    / "apps"
    / "control-plane"
    / "src"
    / "legacy_protected_billing_auth_web_runtime.rs"
)
PROTECTED_BILLING_AUTH_MIGRATION = (
    ROOT
    / "apps"
    / "control-plane"
    / "migrations"
    / "0063_legacy_protected_billing_auth_expand.sql"
)
PROTECTED_BILLING_AUTH_QUERY_ROOT = (
    ROOT / "apps" / "control-plane" / "queries" / "legacy_protected_billing_auth"
)
PROTECTED_BILLING_AUTH_CONFORMANCE = (
    ROOT / "scripts" / "ci" / "legacy-protected-billing-auth-sqlite-conformance.py"
)
CONTROL_NOTIFICATION_READ_RUNTIME = (
    ROOT / "apps" / "control-plane" / "src" / "legacy_notification_read_runtime.rs"
)
NOTIFICATION_READ_QUERY_ROOT = (
    ROOT / "apps" / "control-plane" / "queries" / "legacy_notification_read"
)
NOTIFICATION_READ_CONFORMANCE = (
    ROOT / "scripts" / "ci" / "legacy-notification-read-sqlite-conformance.py"
)
CONTROL_ORG_CUSTOM_DOMAIN_RUNTIME = (
    ROOT / "apps" / "control-plane" / "src" / "legacy_org_custom_domain_runtime.rs"
)
CONTROL_ORG_CUSTOM_DOMAIN_WEB_RUNTIME = (
    ROOT
    / "apps"
    / "control-plane"
    / "src"
    / "legacy_org_custom_domain_web_runtime.rs"
)
ORG_CUSTOM_DOMAIN_MIGRATION = (
    ROOT
    / "apps"
    / "control-plane"
    / "migrations"
    / "0030_legacy_org_custom_domain_projection.sql"
)
ORG_CUSTOM_DOMAIN_QUERY_ROOT = (
    ROOT / "apps" / "control-plane" / "queries" / "legacy_org_custom_domain"
)
ORG_CUSTOM_DOMAIN_CONFORMANCE = (
    ROOT / "scripts" / "ci" / "legacy-org-custom-domain-sqlite-conformance.py"
)
WEB_BROWSER_CLIENT = ROOT / "apps" / "web" / "src" / "browser_authenticated.rs"
WEB_HYDRATION = ROOT / "apps" / "web" / "src" / "hydration.rs"
ORGANIZATION_REPOSITORY = (
    ROOT / "apps" / "control-plane" / "src" / "organization_repository.rs"
)
ORGANIZATION_SELECTION_CONFORMANCE = (
    ROOT / "scripts" / "ci" / "legacy-organization-selection-sqlite-conformance.py"
)
NOTIFICATION_RUNTIME = (
    ROOT / "apps" / "control-plane" / "src" / "legacy_notification_preferences_runtime.rs"
)
NOTIFICATION_QUERY = (
    ROOT
    / "apps"
    / "control-plane"
    / "queries"
    / "legacy_notification_preferences"
    / "read_for_actor.sql"
)
NOTIFICATION_CONFORMANCE = (
    ROOT / "scripts" / "ci" / "legacy-notification-preferences-sqlite-conformance.py"
)
EXECUTION_MIGRATION = (
    ROOT / "apps" / "control-plane" / "migrations" / "0026_legacy_api_execution.sql"
)
EXECUTION_QUERY_ROOT = ROOT / "apps" / "control-plane" / "queries" / "api_workflow"
EXECUTION_CONFORMANCE = ROOT / "scripts" / "ci" / "legacy-api-execution-sqlite-conformance.py"
WORKFLOW = ROOT / ".github" / "workflows" / "api-workflow-parity.yml"
PARITY_RUNNER = ROOT / "scripts" / "ci" / "run-legacy-api-parity.py"
REFERENCE_COMMIT = "6ba69561ac86b8efdb17616d6727f9638015546b"
SCHEMA_VERSION = "frame.api-parity.v1"
OPERATION_CONTRACT_CATALOG_SCHEMA_VERSION = "frame.api-operation-contract-catalog.v1"
CONTRACT_VERSION = "frame.api.v1"
CHANGELOG_FEED_BODY_BYTES = 88_817
CHANGELOG_FEED_BODY_SHA256 = (
    "333c789a76f6f496f94e5e2a47a192fe0c9f87165971689c9c297e5eb43b7499"
)
CHANGELOG_FEED_SOURCE_MANIFEST_SHA256 = (
    "dace60a24a816766681282e4569eda38e16fd85c96a9b2ab311a59351ef58b2d"
)
HTTP_METHODS = ("GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS")
ALL_METHODS = set(HTTP_METHODS) | {"ACTION", "RPC", "WORKFLOW"}


def parity_workflow_contract() -> str:
    """Return the workflow plus its single-source aggregate runner contract."""

    return "\n".join(
        path.read_text(encoding="utf-8") for path in (WORKFLOW, PARITY_RUNNER)
    )

HMAC_WEBHOOK_PATHS = ("/api/webhooks/",)

HonoMount = tuple[str, str, tuple[str, ...]]
HONO_MOUNTS: tuple[HonoMount, ...] = (
    ("apps/web/app/api/desktop/[...route]/root.ts", "/api/desktop", ("app",)),
    (
        "apps/web/app/api/desktop/[...route]/s3Config.ts",
        "/api/desktop/s3/config",
        ("app",),
    ),
    (
        "apps/web/app/api/desktop/[...route]/session.ts",
        "/api/desktop/session",
        ("app",),
    ),
    (
        "apps/web/app/api/desktop/[...route]/storage.ts",
        "/api/desktop/storage",
        ("app", "protectedApp"),
    ),
    (
        "apps/web/app/api/desktop/[...route]/video.ts",
        "/api/desktop/video",
        ("app",),
    ),
    (
        "apps/web/app/api/developer/credits/checkout/route.ts",
        "/api/developer/credits/checkout",
        ("app",),
    ),
    (
        "apps/web/app/api/developer/sdk/v1/[...route]/upload.ts",
        "/api/developer/sdk/v1/upload/multipart",
        ("app",),
    ),
    (
        "apps/web/app/api/developer/sdk/v1/[...route]/video-create.ts",
        "/api/developer/sdk/v1/videos",
        ("app",),
    ),
    (
        "apps/web/app/api/developer/v1/[...route]/usage.ts",
        "/api/developer/v1/usage",
        ("app",),
    ),
    (
        "apps/web/app/api/developer/v1/[...route]/videos.ts",
        "/api/developer/v1/videos",
        ("app",),
    ),
    (
        "apps/web/app/api/upload/[...route]/multipart.ts",
        "/api/upload/multipart",
        ("app",),
    ),
    (
        "apps/web/app/api/upload/[...route]/recording-complete.ts",
        "/api/upload/recording-complete",
        ("app",),
    ),
    (
        "apps/web/app/api/upload/[...route]/signed.ts",
        "/api/upload/signed",
        ("app",),
    ),
    ("apps/media-server/src/app.ts", "/media-server", ("app",)),
    ("apps/media-server/src/routes/health.ts", "/media-server/health", ("health",)),
    ("apps/media-server/src/routes/audio.ts", "/media-server/audio", ("audio",)),
    ("apps/media-server/src/routes/video.ts", "/media-server/video", ("video",)),
)

EFFECT_SOURCES: tuple[tuple[str, str], ...] = (
    ("packages/web-domain/src/Mobile.ts", "/api/mobile"),
    ("packages/web-domain/src/Extension.ts", "/api/extension"),
    ("packages/web-domain/src/Loom.ts", "/api/loom"),
    ("packages/web-api-contract-effect/src/index.ts", "/api"),
)

MOBILE_DECLARATION_SOURCE = "packages/web-domain/src/Mobile.ts"
MOBILE_HANDLER_SOURCE = "apps/web/app/api/mobile/[...route]/route.ts"
EFFECT_RPC_TRANSPORT_SOURCES: tuple[tuple[str, str], ...] = (
    ("apps/web/app/api/erpc/route.ts", "Effect RPC HTTP transport"),
    ("packages/web-backend/src/Rpcs.ts", "RpcsLive+RpcAuthMiddlewareLive"),
)
EFFECT_RPC_IMPLEMENTATION_SOURCES: dict[str, tuple[tuple[str, str], ...]] = {
    "FolderCreate": (
        ("packages/web-backend/src/Folders/FoldersRpcs.ts", "FolderCreate"),
        ("packages/web-backend/src/Folders/index.ts", "Folders.create"),
        ("packages/web-backend/src/Folders/FoldersPolicy.ts", "FoldersPolicy"),
        ("packages/web-backend/src/Folders/FoldersRepo.ts", "FoldersRepo.create"),
    ),
    "FolderDelete": (
        ("packages/web-backend/src/Folders/FoldersRpcs.ts", "FolderDelete"),
        ("packages/web-backend/src/Folders/index.ts", "Folders.delete"),
        ("packages/web-backend/src/Folders/FoldersPolicy.ts", "FoldersPolicy"),
        ("packages/web-backend/src/Folders/FoldersRepo.ts", "FoldersRepo"),
    ),
    "FolderUpdate": (
        ("packages/web-backend/src/Folders/FoldersRpcs.ts", "FolderUpdate"),
        ("packages/web-backend/src/Folders/index.ts", "Folders.update"),
        ("packages/web-backend/src/Folders/FoldersPolicy.ts", "FoldersPolicy"),
        ("packages/web-backend/src/Folders/FoldersRepo.ts", "FoldersRepo.update"),
    ),
    "OrganisationSoftDelete": (
        (
            "packages/web-backend/src/Organisations/OrganisationsRpcs.ts",
            "OrganisationSoftDelete",
        ),
        (
            "packages/web-backend/src/Organisations/index.ts",
            "Organisations.softDelete",
        ),
        (
            "packages/web-backend/src/Organisations/OrganisationsPolicy.ts",
            "OrganisationsPolicy.isOwner",
        ),
        ("packages/web-backend/src/S3Buckets/index.ts", "S3Buckets"),
        ("packages/web-backend/src/Tinybird/index.ts", "Tinybird.deleteData"),
    ),
    "OrganisationUpdate": (
        (
            "packages/web-backend/src/Organisations/OrganisationsRpcs.ts",
            "OrganisationUpdate",
        ),
        ("packages/web-backend/src/Organisations/index.ts", "Organisations.update"),
        (
            "packages/web-backend/src/Organisations/OrganisationsPolicy.ts",
            "OrganisationsPolicy.isAdminOrOwner",
        ),
        ("packages/web-backend/src/ImageUploads/index.ts", "ImageUploads.applyUpdate"),
    ),
    "UserCompleteOnboardingStep": (
        ("packages/web-backend/src/Users/UsersRpcs.ts", "UserCompleteOnboardingStep"),
        (
            "packages/web-backend/src/Users/UsersOnboarding.ts",
            "UsersOnboarding",
        ),
    ),
    "UserUpdate": (
        ("packages/web-backend/src/Users/UsersRpcs.ts", "UserUpdate"),
        ("packages/web-backend/src/Users/index.ts", "Users.update"),
        ("packages/web-backend/src/ImageUploads/index.ts", "ImageUploads.applyUpdate"),
    ),
    "GetUploadProgress": (
        ("packages/web-backend/src/Videos/VideosRpcs.ts", "GetUploadProgress"),
        ("packages/web-backend/src/Videos/index.ts", "Videos.getUploadProgress"),
    ),
    "VideoDelete": (
        ("packages/web-backend/src/Videos/VideosRpcs.ts", "VideoDelete"),
        ("packages/web-backend/src/Videos/index.ts", "Videos.delete"),
    ),
    "VideoDuplicate": (
        ("packages/web-backend/src/Videos/VideosRpcs.ts", "VideoDuplicate"),
        ("packages/web-backend/src/Videos/index.ts", "Videos.duplicate"),
    ),
    "VideoGetDownloadInfo": (
        ("packages/web-backend/src/Videos/VideosRpcs.ts", "VideoGetDownloadInfo"),
        ("packages/web-backend/src/Videos/index.ts", "Videos.getDownloadInfo"),
    ),
    "VideoInstantCreate": (
        ("packages/web-backend/src/Videos/VideosRpcs.ts", "VideoInstantCreate"),
        (
            "packages/web-backend/src/Videos/index.ts",
            "Videos.createInstantRecording",
        ),
    ),
    "VideoUploadProgressUpdate": (
        (
            "packages/web-backend/src/Videos/VideosRpcs.ts",
            "VideoUploadProgressUpdate",
        ),
        ("packages/web-backend/src/Videos/index.ts", "Videos.updateUploadProgress"),
    ),
    "VideosGetAnalytics": (
        ("packages/web-backend/src/Videos/VideosRpcs.ts", "VideosGetAnalytics"),
        ("packages/web-backend/src/Videos/index.ts", "Videos.getAnalyticsBulk"),
    ),
    "VideosGetThumbnails": (
        ("packages/web-backend/src/Videos/VideosRpcs.ts", "VideosGetThumbnails"),
        ("packages/web-backend/src/Videos/index.ts", "Videos.getThumbnailURL"),
    ),
}

ORGANIZATION_ACTION_TRANSITIVE_SOURCES: dict[
    str, tuple[tuple[str, str], ...]
] = {
    "removeOrganizationInvite": (
        (
            "apps/web/actions/organization/authorization.ts",
            "requireOrganizationSettingsManager",
        ),
        ("apps/web/lib/permissions/roles.ts", "organization settings roles"),
    ),
    "updateOrganizationSettings": (
        (
            "apps/web/actions/organization/authorization.ts",
            "requireOrganizationSettingsManager",
        ),
        ("apps/web/lib/permissions/roles.ts", "organization settings roles"),
        ("apps/web/lib/playback-speed.ts", "normalizePlaybackSpeed"),
        ("packages/utils/src/constants/plans.ts", "userIsPro"),
    ),
    "updateOrganizationDetails": (
        (
            "apps/web/actions/organization/authorization.ts",
            "requireOrganizationSettingsManager",
        ),
        ("apps/web/lib/permissions/roles.ts", "organization settings roles"),
    ),
    "getSpaceAccess": (
        ("apps/web/lib/permissions/roles.ts", "space access roles"),
    ),
    "requireSpaceManager": (
        ("apps/web/lib/permissions/roles.ts", "space manager roles"),
    ),
    "addSpaceMember": (
        (
            "apps/web/actions/organization/space-authorization.ts",
            "requireSpaceManager",
        ),
        ("apps/web/lib/permissions/roles.ts", "space membership roles"),
    ),
    "addSpaceMembers": (
        (
            "apps/web/actions/organization/space-authorization.ts",
            "requireSpaceManager",
        ),
        ("apps/web/lib/permissions/roles.ts", "space membership roles"),
    ),
    "setSpaceMembers": (
        (
            "apps/web/actions/organization/space-authorization.ts",
            "requireSpaceManager",
        ),
        ("apps/web/lib/permissions/roles.ts", "space membership roles"),
    ),
}

ORGANIZATION_AUDIT_IDS = {
    "cap-v1-05776c542380771e",
    "cap-v1-a3b4c805d409bc7c",
    "cap-v1-7160c4389375c682",
    "cap-v1-9e125712cee9ce5a",
    "cap-v1-eea1796482b3af28",
    "cap-v1-a193e9e08b2c3f7d",
    "cap-v1-866dbe8fbbfd7887",
    "cap-v1-3a1228254de4338a",
    "cap-v1-91184d308c393034",
    "cap-v1-5595a9d384765e76",
    "cap-v1-14cb48febfd0fa5a",
    "cap-v1-455046db3d6ef019",
    "cap-v1-b177854e2386c877",
    "cap-v1-9fc80bdec80fb248",
}
ORGANISATION_SOFT_DELETE_ID = "cap-v1-5cd4cac9da73f975"
LICENSING_DECLARATION_ONLY_ID = "cap-v1-700b21489623a3e4"
WEB_CUSTOM_DOMAIN_DECLARATION_ONLY_ID = "cap-v1-9323d0178c5a63b5"
DECLARATION_ONLY_OWNER_DECISION_IDS = {
    LICENSING_DECLARATION_ONLY_ID,
    WEB_CUSTOM_DOMAIN_DECLARATION_ONLY_ID,
}

# Operation-level transport facts that cannot be inferred from a path or family.
# Every override is tied to the stable identity and the implementation graph below,
# so a broad family rule cannot silently rewrite released-client behavior.
OPERATION_CONTRACT_OVERRIDES: dict[str, dict[str, Any]] = {
    "cap-v1-51dc2aa9f19a48cc": {
        "idempotency": "optional",
        "max_body_bytes": 256 * 1024,
        "accepted_content_types": ["application/json"],
        "tenant_non_disclosure": False,
    },
    "cap-v1-7c47f9a2a9a24ac0": {
        "idempotency": "forbidden",
        "max_body_bytes": 256 * 1024,
        "accepted_content_types": ["application/json"],
        "tenant_non_disclosure": False,
    },
    "cap-v1-dd88ded400188c1e": {
        "idempotency": "optional",
        "max_body_bytes": 256 * 1024,
        "accepted_content_types": ["application/json"],
        "tenant_non_disclosure": False,
    },
    "cap-v1-9186738740a1ece1": {
        "idempotency": "forbidden",
        "max_body_bytes": 256 * 1024,
        "accepted_content_types": ["application/json"],
        "tenant_non_disclosure": False,
    },
    "cap-v1-d9d8d275d476c8be": {
        "max_body_bytes": 0,
        "accepted_content_types": [],
        "tenant_non_disclosure": False,
    },
    "cap-v1-b5388e4ddf2d7f17": {
        "max_body_bytes": 0,
        "accepted_content_types": [],
        "tenant_non_disclosure": True,
    },
    "cap-v1-09e9e5a5c86b98c1": {
        "max_body_bytes": 0,
        "accepted_content_types": [],
        "tenant_non_disclosure": True,
    },
    "cap-v1-ac0d7aa564f2991c": {
        "auth": "session",
        "idempotency": "forbidden",
        "max_body_bytes": 0,
        "accepted_content_types": [],
        "tenant_non_disclosure": True,
    },
    "cap-v1-0f178cf038854d4a": {
        "auth": "scheduler_secret",
        "idempotency": "forbidden",
        "max_body_bytes": 0,
        "accepted_content_types": [],
        "tenant_non_disclosure": True,
    },
    "cap-v1-5914aa6459d24ff1": {
        "auth": "developer_api_key",
        "idempotency": "optional",
        "max_body_bytes": 256 * 1024,
        "accepted_content_types": ["application/json"],
        "tenant_non_disclosure": True,
    },
    "cap-v1-5c98b9755e4643ba": {
        "auth": "developer_api_key",
        "idempotency": "optional",
        "max_body_bytes": 256 * 1024,
        "accepted_content_types": ["application/json"],
        "tenant_non_disclosure": True,
    },
    "cap-v1-0d3940728bc19e0e": {
        "auth": "developer_api_key",
        "idempotency": "optional",
        "max_body_bytes": 256 * 1024,
        "accepted_content_types": ["application/json"],
        "tenant_non_disclosure": True,
    },
    "cap-v1-b6fe5aec600a2e1a": {
        "auth": "developer_api_key",
        "idempotency": "optional",
        "max_body_bytes": 256 * 1024,
        "accepted_content_types": ["application/json"],
        "tenant_non_disclosure": True,
    },
    "cap-v1-c904ef9c11983a40": {
        "auth": "developer_api_key",
        "idempotency": "optional",
        "max_body_bytes": 256 * 1024,
        "accepted_content_types": ["application/json"],
        "tenant_non_disclosure": True,
    },
    "cap-v1-cbf22d62a64d3486": {
        "auth": "developer_api_key",
        "idempotency": "forbidden",
        "max_body_bytes": 0,
        "accepted_content_types": [],
        "tenant_non_disclosure": True,
    },
    "cap-v1-6e2296f9695261a3": {
        "auth": "developer_api_key",
        "idempotency": "forbidden",
        "max_body_bytes": 0,
        "accepted_content_types": [],
        "tenant_non_disclosure": True,
    },
    "cap-v1-1cbfe3ecac36f198": {
        "auth": "developer_api_key",
        "idempotency": "optional",
        "max_body_bytes": 0,
        "accepted_content_types": [],
        "tenant_non_disclosure": True,
    },
    "cap-v1-aed411f91e977fe5": {
        "auth": "developer_api_key",
        "idempotency": "forbidden",
        "max_body_bytes": 0,
        "accepted_content_types": [],
        "tenant_non_disclosure": True,
    },
    "cap-v1-718e84b39180c0ac": {
        "auth": "developer_api_key",
        "idempotency": "forbidden",
        "max_body_bytes": 0,
        "accepted_content_types": [],
        "tenant_non_disclosure": True,
    },
    "cap-v1-c8dffb9b102dd4f7": {
        "auth": "session",
        "idempotency": "required",
        "max_body_bytes": 0,
        "accepted_content_types": [],
        "tenant_non_disclosure": False,
    },
    "cap-v1-3db394ae13895b46": {
        "auth": "session",
        "idempotency": "required",
        "max_body_bytes": 256 * 1024,
        "accepted_content_types": ["application/json"],
        "tenant_non_disclosure": False,
    },
    "cap-v1-f2659b43d5ee9162": {
        "auth": "optional_session_or_share_capability",
        "idempotency": "forbidden",
        "max_body_bytes": 256 * 1024,
        "accepted_content_types": ["application/json"],
        "tenant_non_disclosure": True,
    },
    "cap-v1-6f6ece85bd786289": {
        "auth": "optional_session_or_share_capability",
        "idempotency": "required",
        "max_body_bytes": 256 * 1024,
        "accepted_content_types": ["application/json"],
        "tenant_non_disclosure": True,
    },
    "cap-v1-6c82f3cbe383d92b": {
        "auth": "optional_session_or_share_capability",
        "idempotency": "forbidden",
        "max_body_bytes": 256 * 1024,
        "accepted_content_types": ["application/json"],
        "tenant_non_disclosure": True,
    },
    "cap-v1-32a24fe16a4c4a4f": {
        "auth": "session_or_api_key",
        "idempotency": "forbidden",
        "max_body_bytes": 0,
        "accepted_content_types": [],
        "tenant_non_disclosure": True,
    },
    "cap-v1-951ad1523ae9dff4": {
        "auth": "session_or_api_key",
        "idempotency": "forbidden",
        "max_body_bytes": 0,
        "accepted_content_types": [],
        "tenant_non_disclosure": True,
    },
    "cap-v1-6b8a689bf00a9187": {
        "auth": "session_or_api_key",
        "idempotency": "forbidden",
        "max_body_bytes": 0,
        "accepted_content_types": [],
        "tenant_non_disclosure": True,
    },
    "cap-v1-7f0ed5caf3eaf97c": {
        "auth": "session_or_api_key",
        "idempotency": "forbidden",
        "max_body_bytes": 0,
        "accepted_content_types": [],
        "tenant_non_disclosure": True,
    },
    "cap-v1-95fe41c72ce5ca9f": {
        "auth": "session_or_api_key",
        "idempotency": "forbidden",
        "max_body_bytes": 0,
        "accepted_content_types": [],
        "tenant_non_disclosure": True,
    },
    "cap-v1-bde34617e42a8834": {
        "auth": "session_or_api_key",
        "idempotency": "forbidden",
        "max_body_bytes": 0,
        "accepted_content_types": [],
        "tenant_non_disclosure": True,
    },
    "cap-v1-ab49cf36a3f243ac": {
        "auth": "session_or_api_key",
        "idempotency": "forbidden",
        "max_body_bytes": 0,
        "accepted_content_types": [],
    },
    "cap-v1-cdfdf7db0f5cb243": {
        "auth": "session_or_api_key",
        "idempotency": "optional",
        "max_body_bytes": 1_500_000,
        "accepted_content_types": ["application/json"],
    },
    "cap-v1-a77171e54b2ba955": {
        "auth": "session_or_api_key",
        "idempotency": "optional",
        "max_body_bytes": 256 * 1024,
        "accepted_content_types": ["application/json"],
    },
    "cap-v1-7508c5a7da637a0b": {
        "auth": "session_or_api_key",
        "idempotency": "forbidden",
        "max_body_bytes": 0,
        "accepted_content_types": [],
    },
    "cap-v1-acc98d2d5e8ff345": {
        "auth": "session_or_api_key",
        "idempotency": "optional",
        "max_body_bytes": 0,
        "accepted_content_types": [],
    },
    "cap-v1-117b0cb801816693": {
        "auth": "session_or_api_key",
        "idempotency": "optional",
        "max_body_bytes": 256 * 1024,
        "accepted_content_types": ["application/json"],
    },
    "cap-v1-447e3212d20351f6": {
        "auth": "session",
        "idempotency": "forbidden",
        "max_body_bytes": 256 * 1024,
        "accepted_content_types": ["application/json"],
    },
    "cap-v1-cddad884de1190b1": {
        "auth": "session",
        "idempotency": "forbidden",
        "max_body_bytes": 256 * 1024,
        "accepted_content_types": ["application/json"],
    },
    "cap-v1-e16563e40f697519": {
        "auth": "public_or_flow_token",
        "idempotency": "forbidden",
        "max_body_bytes": 256 * 1024,
        "accepted_content_types": ["application/json"],
    },
    "cap-v1-139a189f8a00b38c": {
        "auth": "public_or_flow_token",
        "idempotency": "forbidden",
        "max_body_bytes": 256 * 1024,
        "accepted_content_types": ["application/json"],
    },
    "cap-v1-ea999fdc5829fbd1": {
        "auth": "optional_session_or_share_capability",
        "idempotency": "forbidden",
        "max_body_bytes": 0,
        "accepted_content_types": [],
    },
    "cap-v1-1eef72e518a37abd": {
        "auth": "session_or_api_key",
        "idempotency": "forbidden",
        "max_body_bytes": 0,
        "accepted_content_types": [],
    },
    "cap-v1-a3b4c805d409bc7c": {
        "auth": "session",
        "idempotency": "forbidden",
        "max_body_bytes": 0,
        "accepted_content_types": [],
    },
    "cap-v1-7773d3e70d1d5919": {
        "auth": "session",
        "idempotency": "forbidden",
        "max_body_bytes": 0,
        "accepted_content_types": [],
    },
    "cap-v1-05776c542380771e": {
        "auth": "session_or_api_key",
        "idempotency": "optional",
        "max_body_bytes": 256 * 1024,
        "accepted_content_types": ["application/json"],
    },
    "cap-v1-7160c4389375c682": {
        "auth": "session_or_api_key",
        "idempotency": "optional",
        "max_body_bytes": 256 * 1024,
        "accepted_content_types": ["application/json"],
    },
    "cap-v1-661d23fdcca80bd2": {
        "auth": "session_or_api_key",
        "idempotency": "required",
        "max_body_bytes": 256 * 1024,
        "accepted_content_types": ["application/json"],
    },
    "cap-v1-bd59425c2e7074ae": {
        "auth": "session_or_api_key",
        "idempotency": "required",
        "max_body_bytes": 256 * 1024,
        "accepted_content_types": ["application/json"],
    },
    "cap-v1-b6ec2f719de27105": {
        "auth": "session_or_api_key",
        "idempotency": "required",
        "max_body_bytes": 256 * 1024,
        "accepted_content_types": ["application/json"],
    },
    "cap-v1-9e125712cee9ce5a": {
        "idempotency": "optional",
        "max_body_bytes": 256 * 1024,
        "accepted_content_types": ["application/json"],
    },
    "cap-v1-eea1796482b3af28": {
        "idempotency": "forbidden",
        "max_body_bytes": 256 * 1024,
        "accepted_content_types": ["application/json"],
    },
    "cap-v1-a193e9e08b2c3f7d": {
        "idempotency": "optional",
        "max_body_bytes": 256 * 1024,
        "accepted_content_types": ["application/json"],
    },
    "cap-v1-0c233c1115838206": {
        "idempotency": "optional",
        "max_body_bytes": 2 * 1024 * 1024,
        "accepted_content_types": ["multipart/form-data"],
    },
    "cap-v1-3a394a2798233b0b": {
        "idempotency": "optional",
        "max_body_bytes": 2 * 1024 * 1024,
        "accepted_content_types": ["multipart/form-data"],
    },
    "cap-v1-5e7e4265d65c8365": {
        "idempotency": "optional",
        "max_body_bytes": 2 * 1024 * 1024,
        "accepted_content_types": ["multipart/form-data"],
    },
    "cap-v1-d05af581fbeb145e": {
        "idempotency": "optional",
        "max_body_bytes": 2 * 1024 * 1024,
        "accepted_content_types": ["multipart/form-data"],
    },
    "cap-v1-4f21920a947c4c84": {
        "auth": "public_or_flow_token",
        "idempotency": "forbidden",
        "max_body_bytes": 0,
        "accepted_content_types": [],
    },
    "cap-v1-b0116dd82b010477": {
        "auth": "session_or_api_key",
        "idempotency": "forbidden",
        "max_body_bytes": 8 * 1024 * 1024,
        "accepted_content_types": ["application/json"],
        "tenant_non_disclosure": True,
    },
    "cap-v1-b43b6ede64a73798": {
        "auth": "session_or_api_key",
        "idempotency": "forbidden",
        "max_body_bytes": 8 * 1024 * 1024,
        "accepted_content_types": ["application/json"],
        "tenant_non_disclosure": True,
    },
    "cap-v1-62469fe03e030052": {
        "auth": "session_or_api_key",
        "idempotency": "forbidden",
        "max_body_bytes": 8 * 1024 * 1024,
        "accepted_content_types": ["application/json"],
        "tenant_non_disclosure": True,
    },
    "cap-v1-fdc3d5d49bb5ad6d": {
        "auth": "session",
        "idempotency": "forbidden",
        "max_body_bytes": 256 * 1024,
        "accepted_content_types": ["application/json"],
    },
    "cap-v1-c7827a1de563f856": {
        "auth": "session",
        "idempotency": "forbidden",
        "max_body_bytes": 256 * 1024,
        "accepted_content_types": ["application/json"],
    },
    "cap-v1-295a3eb4ba9ffe6f": {
        "auth": "session",
        "idempotency": "forbidden",
        "max_body_bytes": 256 * 1024,
        "accepted_content_types": ["application/json"],
    },
    "cap-v1-fdf4d6473b7f6608": {
        "auth": "session",
        "idempotency": "required",
        "max_body_bytes": 256 * 1024,
        "accepted_content_types": ["application/json"],
    },
    "cap-v1-c067d69850110640": {
        "auth": "session",
        "idempotency": "required",
        "max_body_bytes": 256 * 1024,
        "accepted_content_types": ["application/json"],
    },
    "cap-v1-3d28eb7593bd4b1e": {
        "auth": "session",
        "idempotency": "required",
        "max_body_bytes": 256 * 1024,
        "accepted_content_types": ["application/json"],
    },
    "cap-v1-e0040a01322ea19e": {
        "auth": "session",
        "idempotency": "required",
        "max_body_bytes": 256 * 1024,
        "accepted_content_types": ["application/json"],
    },
    "cap-v1-859bad07650343aa": {
        "auth": "session",
        "idempotency": "required",
        "max_body_bytes": 256 * 1024,
        "accepted_content_types": ["application/json"],
    },
    "cap-v1-2cfe7fc40a6f5a78": {
        "auth": "session_or_api_key",
        "idempotency": "optional",
        "max_body_bytes": 256 * 1024,
        "accepted_content_types": ["application/json"],
    },
    "cap-v1-5fdf332d1448aedc": {
        "auth": "session_or_api_key",
        "idempotency": "optional",
        "max_body_bytes": 256 * 1024,
        "accepted_content_types": ["application/json"],
    },
    "cap-v1-b2db0e7ec51f7898": {
        "auth": "session_or_api_key",
        "idempotency": "optional",
        "max_body_bytes": 256 * 1024,
        "accepted_content_types": ["application/json"],
    },
    "cap-v1-5b36dac105856ede": {
        "auth": "session",
        "idempotency": "optional",
        "max_body_bytes": 256 * 1024,
        "accepted_content_types": ["application/json"],
    },
    "cap-v1-96c52e9330f9a131": {
        "auth": "session",
        "idempotency": "optional",
        "max_body_bytes": 256 * 1024,
        "accepted_content_types": ["application/json"],
    },
    "cap-v1-6e9f3d370f1ce239": {
        "auth": "session",
        "idempotency": "optional",
        "max_body_bytes": 256 * 1024,
        "accepted_content_types": ["application/json"],
    },
    "cap-v1-ab11637faa2de45e": {
        "auth": "session",
        "idempotency": "optional",
        "max_body_bytes": 256 * 1024,
        "accepted_content_types": ["application/json"],
    },
    "cap-v1-455e6a1b82e647d9": {
        "auth": "session",
        "idempotency": "optional",
        "max_body_bytes": 256 * 1024,
        "accepted_content_types": ["application/json"],
    },
    "cap-v1-0a2c44d7a626a1fe": {
        "auth": "anonymous",
        "idempotency": "optional",
        "max_body_bytes": 256 * 1024,
        "accepted_content_types": ["application/json"],
    },
    "cap-v1-49dba3fbc7c4a74c": {
        "auth": "session",
        "idempotency": "optional",
        "max_body_bytes": 256 * 1024,
        "accepted_content_types": ["application/json"],
    },
    "cap-v1-b1027c7caafb92e2": {
        "auth": "session",
        "idempotency": "forbidden",
        "max_body_bytes": 1024,
        "accepted_content_types": ["application/json"],
    },
    "cap-v1-cc52545598164806": {
        "auth": "session",
        "idempotency": "forbidden",
        "max_body_bytes": 1024,
        "accepted_content_types": ["application/json"],
    },
    "cap-v1-a8ace95c6ab712f6": {
        "auth": "session",
        "idempotency": "forbidden",
        "max_body_bytes": 1024,
        "accepted_content_types": ["application/json"],
    },
    "cap-v1-17a71c3e18600d06": {
        "auth": "session",
        "idempotency": "forbidden",
        "max_body_bytes": 2 * 1024,
        "accepted_content_types": ["application/json"],
    },
    "cap-v1-39e8966f308c1528": {
        "auth": "session",
        "idempotency": "forbidden",
        "max_body_bytes": 2 * 1024,
        "accepted_content_types": ["application/json"],
    },
    "cap-v1-5595a9d384765e76": {
        "auth": "session",
        "idempotency": "forbidden",
        "max_body_bytes": 1024,
        "accepted_content_types": ["application/json"],
    },
    "cap-v1-14cb48febfd0fa5a": {
        "auth": "session",
        "idempotency": "forbidden",
        "max_body_bytes": 1024,
        "accepted_content_types": ["application/json"],
    },
    "cap-v1-249fbd2f77ee7209": {
        "auth": "public_or_flow_token",
        "idempotency": "forbidden",
        "max_body_bytes": 0,
        "accepted_content_types": [],
    },
    "cap-v1-96499b6c8e845b35": {
        "auth": "public_or_flow_token",
        "idempotency": "forbidden",
        "max_body_bytes": 8 * 1024,
        "accepted_content_types": ["application/x-www-form-urlencoded"],
    },
    "cap-v1-ed715d4d23e82181": {
        "auth": "session_or_api_key",
        "idempotency": "forbidden",
        "max_body_bytes": 0,
        "accepted_content_types": [],
    },
    "cap-v1-12159b1acbaeba7a": {
        "auth": "session_or_api_key",
        "idempotency": "forbidden",
        "max_body_bytes": 0,
        "accepted_content_types": [],
    },
    "cap-v1-00422c50f4d39053": {
        "auth": "session_or_api_key",
        "idempotency": "forbidden",
        "max_body_bytes": 256 * 1024,
        "accepted_content_types": ["application/json"],
    },
    "cap-v1-82dec55d0fbea3db": {
        "auth": "session_or_api_key",
        "idempotency": "forbidden",
        "max_body_bytes": 256 * 1024,
        "accepted_content_types": ["application/json"],
    },
    "cap-v1-8fd4741d6e52465e": {
        "auth": "session_or_api_key",
        "idempotency": "forbidden",
        "max_body_bytes": 0,
        "accepted_content_types": [],
    },
    "cap-v1-10e17d0e86b49830": {
        "auth": "anonymous",
        "idempotency": "forbidden",
        "max_body_bytes": 0,
        "accepted_content_types": [],
        "tenant_non_disclosure": False,
    },
}

OPERATION_COMPLETION_OVERRIDES: dict[str, dict[str, Any]] = {
    "cap-v1-6f6ece85bd786289": {
        "decision": "retain_replace_with_provider_effect",
        "local_work": "complete",
        "protected_gates": ["provider_execution"],
        "retirement_decision": "not_proposed",
        "production_behavior": "fail_closed_unavailable",
    },
    "cap-v1-05776c542380771e": {
        "decision": "retain_replace_with_provider_effect",
        "local_work": "exact_image_signing_and_nullable_root_folder_bootstrap_projection_required",
        "protected_gates": ["provider_execution"],
        "retirement_decision": "not_proposed",
        "production_behavior": "fail_closed_unavailable",
    },
    "cap-v1-e16563e40f697519": {
        "decision": "retain_replace_with_provider_effect",
        "local_work": "complete",
        "protected_gates": ["provider_execution"],
        "retirement_decision": "not_proposed",
        "production_behavior": "fail_closed_unavailable",
    },
    "cap-v1-139a189f8a00b38c": {
        "decision": "retain_replace_with_provider_effect",
        "local_work": "complete",
        "protected_gates": ["provider_execution"],
        "retirement_decision": "not_proposed",
        "production_behavior": "fail_closed_unavailable",
    },
}

MESSENGER_RETIREMENT_IDENTITIES: dict[str, str] = {
    "cap-v1-112d9985edf52908": (
        "action://apps/web/actions/messenger.ts#adminSendMessengerMessage"
    ),
    "cap-v1-280ec8c395aafaab": (
        "action://apps/web/actions/messenger.ts#adminSetMessengerMode"
    ),
    "cap-v1-ae7b61be76feb518": (
        "action://apps/web/actions/messenger.ts#adminSyncMessengerKnowledge"
    ),
    "cap-v1-178e6b9650de9bb0": (
        "action://apps/web/actions/messenger.ts#createMessengerConversation"
    ),
    "cap-v1-34da1e92c2f12e0a": (
        "action://apps/web/actions/messenger.ts#fetchAdminConversation"
    ),
    "cap-v1-8168c530994107e5": (
        "action://apps/web/actions/messenger.ts#fetchAdminConversations"
    ),
    "cap-v1-6b8f779317d1c7ca": (
        "action://apps/web/actions/messenger.ts#fetchMessengerConversation"
    ),
    "cap-v1-fa3d76343c35a814": (
        "action://apps/web/actions/messenger.ts#fetchMessengerConversations"
    ),
    "cap-v1-5ce92672252fbc0f": (
        "action://apps/web/actions/messenger.ts#sendMessengerUserMessage"
    ),
    "cap-v1-2bdad000348e66b4": (
        "action://apps/web/app/messenger/page.tsx#startConversation"
    ),
}

SPACE_ACTION_LOCAL_SOURCE_SPECS: tuple[tuple[str, str], ...] = (
    ("apps/web/actions/organization/space-settings.ts", "space form settings"),
    (
        "apps/web/actions/organization/space-authorization.ts",
        "space management authorization",
    ),
    ("apps/web/lib/org-pro.ts", "organization owner Pro policy"),
    ("apps/web/lib/permissions/roles.ts", "space role normalization"),
    ("packages/database/crypto.ts", "space password hashing"),
    ("packages/database/helpers.ts", "space NanoID generation"),
    ("packages/database/schema.ts", "space persistence schema"),
    ("packages/utils/src/constants/plans.ts", "space Pro entitlement policy"),
)
SPACE_ACTION_PROVIDER_SOURCE_SPECS: tuple[tuple[str, str], ...] = (
    ("apps/web/actions/organization/upload-space-icon.ts", "space icon upload"),
    ("packages/web-backend/src/S3Buckets/index.ts", "space icon storage effect"),
    (
        "packages/web-backend/src/S3Buckets/S3BucketAccess.ts",
        "space icon bucket access",
    ),
    ("apps/web/lib/sanitizeFile.ts", "space icon SVG sanitization"),
)
FOLDER_RPC_LOCAL_SOURCE_SPECS: tuple[tuple[str, str], ...] = (
    ("packages/web-domain/src/PublicCollection.ts", "folder public-page contract"),
    ("packages/web-backend/src/Auth.ts", "Effect RPC auth layer"),
    ("packages/web-domain/src/Authentication.ts", "authentication contract"),
    ("packages/web-domain/src/Policy.ts", "policy error contract"),
    ("packages/web-backend/src/Spaces/index.ts", "space service"),
    ("packages/web-backend/src/Spaces/SpacesPolicy.ts", "space policy"),
    ("packages/web-backend/src/Spaces/SpacesRepo.ts", "space repository"),
    (
        "packages/web-backend/src/Organisations/OrganisationsPolicy.ts",
        "organization folder policy",
    ),
    (
        "packages/web-backend/src/Organisations/OrganisationsRepo.ts",
        "organization entitlement repository",
    ),
    ("packages/database/schema.ts", "folder persistence schema"),
    ("packages/utils/src/constants/plans.ts", "folder Pro entitlement policy"),
)

OPERATION_TRANSITIVE_SOURCE_SPECS: dict[str, tuple[tuple[str, str], ...]] = {
    "cap-v1-a3b4c805d409bc7c": (
        ("packages/database/auth/session.ts", "getCurrentUser"),
        ("packages/database/schema.ts", "organization selection persistence schema"),
        ("packages/web-domain/src/Organisation.ts", "OrganisationId"),
    ),
    "cap-v1-7773d3e70d1d5919": (
        (
            "apps/web/app/(org)/dashboard/layout.tsx",
            "authenticated dashboard theme cookie reader",
        ),
        (
            "apps/web/app/(org)/dashboard/Contexts.tsx",
            "dashboard client-side theme persistence behavior",
        ),
        ("apps/web/package.json", "next dependency declaration"),
        ("pnpm-lock.yaml", "next@16.2.1 resolution"),
    ),
    "cap-v1-f5daa7be337a2979": (
        ("packages/database/auth/session.ts", "getCurrentUser"),
        ("packages/database/auth/auth-options.ts", "authOptions session callback"),
        ("packages/database/schema.ts", "folder assignment persistence schema"),
        ("packages/database/helpers.ts", "Cap NanoID helper"),
        ("packages/web-domain/src/Folder.ts", "FolderId"),
        ("packages/web-domain/src/Space.ts", "SpaceId"),
        ("packages/web-domain/src/Video.ts", "VideoId"),
    ),
    "cap-v1-1af3645bf2ae7168": (
        ("packages/database/auth/session.ts", "getCurrentUser"),
        ("packages/database/auth/auth-options.ts", "authOptions session callback"),
        ("packages/database/schema.ts", "folder assignment persistence schema"),
        ("packages/web-domain/src/Folder.ts", "FolderId"),
        ("packages/web-domain/src/Space.ts", "SpaceId"),
        ("packages/web-domain/src/Video.ts", "VideoId"),
    ),
    "cap-v1-eaf277e644aa4b92": (
        ("packages/database/auth/session.ts", "getCurrentUser"),
        ("packages/database/auth/auth-options.ts", "authOptions session callback"),
        ("packages/database/schema.ts", "folder assignment persistence schema"),
        ("packages/web-domain/src/Folder.ts", "FolderId"),
        ("packages/web-domain/src/Space.ts", "SpaceId"),
        ("packages/web-domain/src/Video.ts", "VideoId"),
    ),
    "cap-v1-05776c542380771e": (
        ("apps/web/lib/server.ts", "mobile HttpApi server wiring"),
        ("packages/web-backend/src/Auth.ts", "HttpAuthMiddlewareLive"),
        ("packages/web-domain/src/Authentication.ts", "HttpAuthMiddleware"),
        ("packages/database/schema.ts", "mobile bootstrap persistence schema"),
        ("packages/web-backend/src/ImageUploads/index.ts", "resolveImageUrl"),
        ("packages/web-backend/src/S3Buckets/index.ts", "image bucket signing"),
        (
            "packages/web-backend/src/S3Buckets/S3BucketAccess.ts",
            "provider bucket access",
        ),
    ),
    "cap-v1-4f21920a947c4c84": (
        (
            "apps/web/app/api/mobile/[...route]/route.ts",
            "mobile handler:getAuthConfig",
        ),
        ("packages/web-domain/src/Mobile.ts", "getAuthConfig"),
    ),
    "cap-v1-d130c840f654bd72": (
        ("packages/database/auth/session.ts", "getCurrentUser"),
        ("packages/database/auth/auth-options.ts", "authOptions session callback"),
        ("packages/database/schema.ts", "users.preferences"),
        ("apps/web/proxy.ts", "API path middleware exclusion"),
        ("apps/web/next.config.mjs", "Next response runtime configuration"),
        ("apps/web/package.json", "next dependency declaration"),
        ("pnpm-lock.yaml", "next@16.2.1 resolution"),
    ),
    "cap-v1-10e17d0e86b49830": (
        ("packages/database/schema.ts", "videos+sharedVideos+organizations"),
        ("packages/web-domain/src/Video.ts", "Video.VideoId"),
        ("packages/database/index.ts", "db"),
        ("apps/web/proxy.ts", "API matcher exclusion"),
        ("apps/web/package.json", "next+drizzle dependencies"),
        ("pnpm-lock.yaml", "dependency lock"),
    ),
    "cap-v1-14dcca6d36eee6b3": (
        ("packages/database/auth/session.ts", "getCurrentUser"),
        ("packages/database/schema.ts", "notifications+users"),
        (
            "packages/web-backend/src/ImageUploads/index.ts",
            "ImageUploads.resolveImageUrl",
        ),
    ),
    "cap-v1-ed9957ac480103b9": (
        ("apps/desktop/src/utils/queries.ts", "createCustomDomainQuery"),
        (
            "apps/desktop/src/utils/web-api.ts",
            "orgCustomDomainClient+protectedHeaders",
        ),
        (
            "apps/web/app/api/desktop/[...route]/route.ts",
            "desktop mount+GET+OPTIONS",
        ),
        (
            "apps/web/app/api/utils.ts",
            "getAuth+withAuth+corsMiddleware",
        ),
        (
            "packages/database/schema.ts",
            "users+organizations+authApiKeys",
        ),
    ),
    "cap-v1-9e125712cee9ce5a": FOLDER_RPC_LOCAL_SOURCE_SPECS,
    "cap-v1-a193e9e08b2c3f7d": FOLDER_RPC_LOCAL_SOURCE_SPECS,
    "cap-v1-0c233c1115838206": SPACE_ACTION_LOCAL_SOURCE_SPECS,
    "cap-v1-3a394a2798233b0b": SPACE_ACTION_LOCAL_SOURCE_SPECS,
    "cap-v1-5e7e4265d65c8365": (
        *SPACE_ACTION_LOCAL_SOURCE_SPECS,
        (
            "apps/web/actions/organization/create-space.ts",
            "delegated createSpace action",
        ),
    ),
    "cap-v1-d05af581fbeb145e": (
        *SPACE_ACTION_LOCAL_SOURCE_SPECS,
        (
            "apps/web/actions/organization/update-space.ts",
            "delegated updateSpace action",
        ),
    ),
}

# These identities execute an external provider in the pinned Cap implementation,
# even though their declaration/entrypoint names alone do not classify them as
# protected. The source specs are intentionally minimal and operation-specific;
# classification_searchable excludes them so pinning an implementation cannot
# silently change the route family or disposition.
PROVIDER_BACKED_SOURCE_SPECS: dict[str, tuple[tuple[str, str], ...]] = {
    "cap-v1-05776c542380771e": (
        ("packages/web-backend/src/ImageUploads/index.ts", "resolveImageUrl"),
        ("packages/web-backend/src/S3Buckets/index.ts", "image bucket signing"),
        (
            "packages/web-backend/src/S3Buckets/S3BucketAccess.ts",
            "provider bucket access",
        ),
    ),
    "cap-v1-0c233c1115838206": SPACE_ACTION_PROVIDER_SOURCE_SPECS,
    "cap-v1-3a394a2798233b0b": SPACE_ACTION_PROVIDER_SOURCE_SPECS,
    "cap-v1-5e7e4265d65c8365": SPACE_ACTION_PROVIDER_SOURCE_SPECS,
    "cap-v1-d05af581fbeb145e": SPACE_ACTION_PROVIDER_SOURCE_SPECS,
    "cap-v1-4054bc310aa16e98": (
        (
            "packages/web-backend/src/Storage/GoogleDrive.ts",
            "getGoogleDriveAccessToken",
        ),
    ),
    "cap-v1-83721706f7b0e2e6": (
        (
            "packages/web-backend/src/Storage/GoogleDrive.ts",
            "getGoogleDriveAccessToken",
        ),
    ),
    "cap-v1-50d0ffd9f5f7bcb6": (
        (
            "packages/web-backend/src/Storage/GoogleDrive.ts",
            "getGoogleDriveFolderLocation+getGoogleDriveUserEmail",
        ),
    ),
    "cap-v1-6446dc02a25cef2f": (
        ("apps/web/actions/organization/domain-utils.ts", "checkDomainStatus"),
    ),
    "cap-v1-3f2885312f79698e": (
        ("packages/utils/src/lib/stripe/stripe.ts", "stripe.subscriptions.retrieve"),
    ),
    "cap-v1-55b98b0f419abf86": (
        (
            "apps/web/actions/organization/remove-domain.ts",
            "Vercel DELETE provider effect",
        ),
    ),
    "cap-v1-f0ba260c29c295f3": (
        ("packages/database/emails/config.ts", "sendEmail via Resend"),
    ),
    "cap-v1-0823c0b806bd38a5": (
        (
            "apps/web/actions/organization/domain-utils.ts",
            "addDomain+checkDomainStatus",
        ),
    ),
    "cap-v1-aa00fc906599e89c": (
        ("packages/utils/src/lib/stripe/stripe.ts", "stripe subscription preview"),
        ("apps/web/utils/organization.ts", "calculateProSeats"),
    ),
    "cap-v1-17470f7df902263e": (
        ("packages/utils/src/lib/stripe/stripe.ts", "stripe subscription update"),
        ("apps/web/utils/organization.ts", "calculateProSeats"),
    ),
    "cap-v1-6f6ece85bd786289": (
        ("apps/web/lib/groq-client.ts", "getGroqClient"),
        ("packages/web-backend/src/Storage/index.ts", "Storage translation objects"),
    ),
    "cap-v1-46bda1c18ffba076": (
        ("packages/database/auth/auth-options.ts", "NextAuth OAuth providers"),
        ("packages/database/emails/config.ts", "NextAuth email via Resend"),
    ),
    "cap-v1-82a39c991fae1050": (
        ("packages/database/auth/auth-options.ts", "NextAuth OAuth providers"),
        ("packages/database/emails/config.ts", "NextAuth email via Resend"),
    ),
    "cap-v1-e16563e40f697519": (
        ("packages/database/emails/config.ts", "mobile verification via Resend"),
    ),
    "cap-v1-30b7af7323aa2c37": (
        ("packages/database/emails/config.ts", "desktop feedback via Resend"),
    ),
    "cap-v1-dfbbc4c0b56179d1": (
        (
            "apps/web/app/api/desktop/[...route]/root.ts",
            "Discord diagnostics webhook provider effect",
        ),
    ),
    "cap-v1-10180c4650ffde88": (
        ("packages/utils/src/lib/stripe/stripe.ts", "desktop plan Stripe lookup"),
    ),
    "cap-v1-2e4ee222efc29606": (
        (
            "apps/web/app/api/desktop/[...route]/root.ts",
            "remote profile image fetch provider effect",
        ),
    ),
    "cap-v1-c8a43dc80c502b6d": (
        ("apps/web/actions/videos/get-analytics.ts", "getVideoAnalytics"),
        ("packages/web-backend/src/Tinybird/index.ts", "Tinybird.querySql"),
    ),
    "cap-v1-51dc2aa9f19a48cc": (
        ("packages/web-backend/src/Tinybird/index.ts", "Tinybird.appendEvents"),
    ),
    "cap-v1-9b093898957efebb": (
        (
            "apps/web/app/(org)/dashboard/analytics/data.ts",
            "getOrgAnalyticsData",
        ),
        ("packages/web-backend/src/Tinybird/index.ts", "Tinybird analytics queries"),
    ),
    "cap-v1-be2ea6b474aae7c9": (
        ("apps/web/actions/videos/get-analytics.ts", "getVideoAnalytics"),
        ("packages/web-backend/src/Tinybird/index.ts", "Tinybird.querySql"),
    ),
    "cap-v1-7c47f9a2a9a24ac0": (
        ("packages/web-backend/src/Tinybird/index.ts", "Tinybird.querySql"),
    ),
    "cap-v1-9186738740a1ece1": (
        ("packages/web-backend/src/Tinybird/index.ts", "Tinybird.querySql"),
    ),
    "cap-v1-0b36c9acda9bd6a2": (
        (
            "packages/web-backend/src/Storage/GoogleDrive.ts",
            "Google Drive integration access",
        ),
        ("apps/web/lib/google-drive-storage-quota.ts", "Google Drive quota"),
    ),
    "cap-v1-efc19423a62b7976": (
        ("packages/web-backend/src/Storage/index.ts", "Storage.multipart.complete"),
    ),
    "cap-v1-f9deb8104204a30d": (
        (
            "apps/web/lib/desktop-segments-finalization.ts",
            "queueDesktopSegmentsFinalization",
        ),
        (
            "apps/web/workflows/finalize-desktop-recording.ts",
            "finalizeDesktopRecordingWorkflow media provider",
        ),
    ),
    "cap-v1-8160d7c3ce8507d9": (
        ("packages/database/emails/config.ts", "download link via Resend"),
    ),
    "cap-v1-60f863b2cb19353f": (
        ("packages/utils/src/lib/dub.ts", "Dub link creation"),
        ("packages/database/emails/config.ts", "desktop upload email via Resend"),
        ("packages/web-backend/src/Storage/index.ts", "desktop video storage"),
    ),
    "cap-v1-d9b654b30f6c362a": (
        ("apps/web/lib/transcribe.ts", "transcribeVideo"),
        ("apps/web/workflows/transcribe.ts", "Deepgram transcription provider"),
    ),
    "cap-v1-8a1e6c87b4426f93": (
        (
            "apps/web/app/api/releases/tauri/[version]/[target]/[arch]/route.ts",
            "GitHub release provider effect",
        ),
    ),
}
PROVIDER_BACKED_IDS = frozenset(PROVIDER_BACKED_SOURCE_SPECS)

TS_REST_CONTRACT_SOURCES: tuple[str, ...] = (
    "packages/web-api-contract/src/index.ts",
    "packages/web-api-contract/src/desktop.ts",
)

EXTENSION_ENDPOINTS: tuple[tuple[str, str, str], ...] = (
    ("GET", "/auth/start", "startAuth"),
    ("POST", "/auth/approve", "approveAuth"),
    ("POST", "/auth/revoke", "revokeAuth"),
    ("GET", "/bootstrap", "bootstrap"),
    ("POST", "/instant-recordings", "createInstantRecording"),
    ("POST", "/instant-recordings/progress", "updateInstantRecordingProgress"),
    ("DELETE", "/instant-recordings/:videoId", "deleteInstantRecording"),
)

CURATED_WORKFLOW_ENTRYPOINTS: tuple[tuple[str, str], ...] = (
    ("apps/web/lib/video-processing.ts", "startVideoProcessingWorkflow"),
    ("apps/web/lib/desktop-segments-finalization.ts", "queueDesktopSegmentsFinalization"),
    (
        "apps/web/lib/desktop-segments-recovery.ts",
        "completeDesktopSegmentsManifestAndQueue",
    ),
    ("apps/web/lib/desktop-segments-recovery.ts", "recoverStaleDesktopSegments"),
    ("apps/web/lib/video-edit-processing.ts", "reconcileStaleEditUpload"),
    ("apps/web/lib/generate-ai.ts", "startAiGeneration"),
)

TRANSPORT_WRAPPERS = (
    "apps/web/app/api/[[...route]]/route.ts",
    "apps/web/app/api/desktop/[...route]/route.ts",
    "apps/web/app/api/developer/sdk/v1/[...route]/route.ts",
    "apps/web/app/api/developer/v1/[...route]/route.ts",
    "apps/web/app/api/mobile/[...route]/route.ts",
    "apps/web/app/api/upload/[...route]/route.ts",
)

EVIDENCE_VALUES = {
    "local_contract",
    "family_contract",
    "retirement_contract_pending_approval",
    "endpoint_adapter_pending",
    "protected_evidence_required",
    "dependency_pending",
}
PROTECTED_COMPLETION_GATES = {
    "hardware_execution",
    "human_approval",
    "provider_execution",
    "released_legacy_client_e2e",
}

FOLDER_ASSIGNMENT_RUNTIME_SOURCES: tuple[dict[str, str], ...] = (
    {
        "path": "packages/database/auth/session.ts",
        "symbol": "getCurrentUser",
        "sha256": "d526dc9d7a6a1a7cb6a8695c24ab88b843ce09b4444f8e4ade24b7a06cbbc1ee",
    },
    {
        "path": "packages/database/auth/auth-options.ts",
        "symbol": "authOptions session callback",
        "sha256": "22b8923e1cab6b5b1b318609abe664e171fb740ae39817c2c962908ca0dc8595",
    },
    {
        "path": "packages/database/schema.ts",
        "symbol": "folder assignment persistence schema",
        "sha256": "7fce297f9076be78a9ac6280d9d060bf6e836a62e0f82b5390fa0e42dc7bb9e9",
    },
    {
        "path": "packages/web-domain/src/Folder.ts",
        "symbol": "FolderId",
        "sha256": "4201376991878efc79979f77901908d542573f5b0f9e1ca6b6b246e04d881e9e",
    },
    {
        "path": "packages/web-domain/src/Space.ts",
        "symbol": "SpaceId",
        "sha256": "ad9cb2ae26767bebf00640846bce4cab6feee6a6308ac0d7b068cd6e006542c3",
    },
    {
        "path": "packages/web-domain/src/Video.ts",
        "symbol": "VideoId",
        "sha256": "adc3db0eded2670b1ed89969e7bc85993e04021acb303082b3d015f0afb1c9a7",
    },
)
FOLDER_ASSIGNMENT_ADD_RUNTIME_SOURCES: tuple[dict[str, str], ...] = (
    *FOLDER_ASSIGNMENT_RUNTIME_SOURCES,
    {
        "path": "packages/database/helpers.ts",
        "symbol": "Cap NanoID helper",
        "sha256": "e976df51a8186737a1c4696a20cd52f2c029aa630b2463d1970b8667b0dd20cd",
    },
)

LIBRARY_PLACEMENT_ORGANIZATION_RUNTIME_SOURCES: tuple[dict[str, str], ...] = (
    {
        "path": "packages/database/auth/session.ts",
        "symbol": "getCurrentUser",
        "sha256": "d526dc9d7a6a1a7cb6a8695c24ab88b843ce09b4444f8e4ade24b7a06cbbc1ee",
    },
    {
        "path": "packages/database/auth/auth-options.ts",
        "symbol": "authOptions session callback",
        "sha256": "22b8923e1cab6b5b1b318609abe664e171fb740ae39817c2c962908ca0dc8595",
    },
    {
        "path": "packages/database/schema.ts",
        "symbol": "library placement persistence schema",
        "sha256": "7fce297f9076be78a9ac6280d9d060bf6e836a62e0f82b5390fa0e42dc7bb9e9",
    },
    {
        "path": "packages/web-domain/src/Organisation.ts",
        "symbol": "OrganisationId",
        "sha256": "14d634ad8910d3921af2ea5b136b9c3d2a8ae26f74b3dcb7a82b9cf19d6a3264",
    },
    {
        "path": "packages/web-domain/src/Video.ts",
        "symbol": "VideoId",
        "sha256": "adc3db0eded2670b1ed89969e7bc85993e04021acb303082b3d015f0afb1c9a7",
    },
)
LIBRARY_PLACEMENT_ADD_ORGANIZATION_RUNTIME_SOURCES: tuple[dict[str, str], ...] = (
    *LIBRARY_PLACEMENT_ORGANIZATION_RUNTIME_SOURCES,
    {
        "path": "packages/database/helpers.ts",
        "symbol": "Cap NanoID helper",
        "sha256": "e976df51a8186737a1c4696a20cd52f2c029aa630b2463d1970b8667b0dd20cd",
    },
)
LIBRARY_PLACEMENT_SPACE_RUNTIME_SOURCES: tuple[dict[str, str], ...] = (
    {
        "path": "packages/database/auth/session.ts",
        "symbol": "getCurrentUser",
        "sha256": "d526dc9d7a6a1a7cb6a8695c24ab88b843ce09b4444f8e4ade24b7a06cbbc1ee",
    },
    {
        "path": "packages/database/auth/auth-options.ts",
        "symbol": "authOptions session callback",
        "sha256": "22b8923e1cab6b5b1b318609abe664e171fb740ae39817c2c962908ca0dc8595",
    },
    {
        "path": "packages/database/schema.ts",
        "symbol": "library placement persistence schema",
        "sha256": "7fce297f9076be78a9ac6280d9d060bf6e836a62e0f82b5390fa0e42dc7bb9e9",
    },
    {
        "path": "apps/web/actions/organization/authorization.ts",
        "symbol": "organization management authorization",
        "sha256": "6b1422de53d0915a985dc1dbf70f14494fd1c5fe49ca61fbacabd90bebb00980",
    },
    {
        "path": "apps/web/actions/organization/space-authorization.ts",
        "symbol": "space management authorization",
        "sha256": "2a656f25f7c73f2342104127d818a56fffd7d05768d787489b65e08f70a43445",
    },
    {
        "path": "apps/web/lib/permissions/roles.ts",
        "symbol": "space role normalization",
        "sha256": "97bf35a09f4ef403dd0ffaa572c40c29f5776c4e6ae73c3e1e511ca376d5a407",
    },
    {
        "path": "packages/web-domain/src/Organisation.ts",
        "symbol": "OrganisationId",
        "sha256": "14d634ad8910d3921af2ea5b136b9c3d2a8ae26f74b3dcb7a82b9cf19d6a3264",
    },
    {
        "path": "packages/web-domain/src/Space.ts",
        "symbol": "SpaceId",
        "sha256": "ad9cb2ae26767bebf00640846bce4cab6feee6a6308ac0d7b068cd6e006542c3",
    },
    {
        "path": "packages/web-domain/src/User.ts",
        "symbol": "UserId",
        "sha256": "5b3374425a4c9df1501af34c8f1f780c3f7612f093cd2ff0ed5c442e41e7cee1",
    },
    {
        "path": "packages/web-domain/src/Video.ts",
        "symbol": "VideoId",
        "sha256": "adc3db0eded2670b1ed89969e7bc85993e04021acb303082b3d015f0afb1c9a7",
    },
)
LIBRARY_PLACEMENT_ADD_SPACE_RUNTIME_SOURCES: tuple[dict[str, str], ...] = (
    *LIBRARY_PLACEMENT_SPACE_RUNTIME_SOURCES,
    {
        "path": "packages/database/helpers.ts",
        "symbol": "Cap NanoID helper",
        "sha256": "e976df51a8186737a1c4696a20cd52f2c029aa630b2463d1970b8667b0dd20cd",
    },
)
NOTIFICATION_MARK_READ_RUNTIME_SOURCES: tuple[dict[str, str], ...] = (
    {
        "path": "apps/web/app/(org)/dashboard/_components/Notifications/NotificationItem.tsx",
        "symbol": "selected notification caller",
        "sha256": "d32121e41e5eee0cf84da0e992c14497378e28361c015ffa3d8b4637d42deab1",
    },
    {
        "path": "apps/web/app/(org)/dashboard/_components/Notifications/NotificationHeader.tsx",
        "symbol": "bulk notification caller",
        "sha256": "2b748c4dbba2d943caaf67083ee9835bbbc68dbf25f124652e3c3f61570a4711",
    },
    {
        "path": "apps/web/app/(org)/dashboard/_components/Navbar/Top.tsx",
        "symbol": "notification navigation caller",
        "sha256": "beae5ea8e688fd5c0d7d239cd61788e30e99964bd7323e8c93acdebf584919dd",
    },
    {
        "path": "apps/web/app/api/notifications/route.ts",
        "symbol": "notification read projection",
        "sha256": "1c0571a385328c53ec106967a717201ed2aa04cbcfd108c419f03f8b51b3ae17",
    },
    {
        "path": "packages/database/auth/session.ts",
        "symbol": "getCurrentUser",
        "sha256": "d526dc9d7a6a1a7cb6a8695c24ab88b843ce09b4444f8e4ade24b7a06cbbc1ee",
    },
    {
        "path": "packages/database/auth/auth-options.ts",
        "symbol": "authOptions session callback",
        "sha256": "22b8923e1cab6b5b1b318609abe664e171fb740ae39817c2c962908ca0dc8595",
    },
    {
        "path": "packages/database/schema.ts",
        "symbol": "notification persistence schema",
        "sha256": "7fce297f9076be78a9ac6280d9d060bf6e836a62e0f82b5390fa0e42dc7bb9e9",
    },
    {
        "path": "packages/database/helpers.ts",
        "symbol": "Cap NanoID helper",
        "sha256": "e976df51a8186737a1c4696a20cd52f2c029aa630b2463d1970b8667b0dd20cd",
    },
    {
        "path": "packages/database/index.ts",
        "symbol": "database factory",
        "sha256": "161c1d1fd2a561fd2846aeceb148f24b58afc58bdaa95175240e48dbe61d9bbb",
    },
    {
        "path": "packages/web-domain/src/User.ts",
        "symbol": "UserId",
        "sha256": "5b3374425a4c9df1501af34c8f1f780c3f7612f093cd2ff0ed5c442e41e7cee1",
    },
    {
        "path": "apps/web/package.json",
        "symbol": "next and drizzle dependency declaration",
        "sha256": "c1358cd1880ac5dc9d659760c2788cedd5c4f61fec2cb0dd1b60cbc9bb8af920",
    },
    {
        "path": "packages/database/package.json",
        "symbol": "database dependency declaration",
        "sha256": "95629fc376bfc4df4f9f69a28a874e8bcf8496ccec276fd2168cfc9720e4a057",
    },
    {
        "path": "pnpm-lock.yaml",
        "symbol": "notification dependency resolution",
        "sha256": "fc0fe122ae5fbea4dcaa7e510bd6275635c14071c4e031996431a54fb7e25e3a",
    },
)
NOTIFICATION_UPDATE_PREFERENCES_RUNTIME_SOURCES: tuple[dict[str, str], ...] = (
    {
        "path": "apps/web/app/(org)/dashboard/settings/notifications/NotificationsSettings.tsx",
        "symbol": "notification preferences caller",
        "sha256": "94b3d9ac1e93a7a46e7c1e20d942b16fc82cfc6b5c96e1563835acb8d70f910f",
    },
    {
        "path": "apps/web/app/(org)/dashboard/dashboard-data.ts",
        "symbol": "notification preferences read projection",
        "sha256": "73115151676d808c5e5731fd717792d12a87bbfa2bd827c69d3fbf16ac42fdad",
    },
    {
        "path": "packages/database/auth/session.ts",
        "symbol": "getCurrentUser",
        "sha256": "d526dc9d7a6a1a7cb6a8695c24ab88b843ce09b4444f8e4ade24b7a06cbbc1ee",
    },
    {
        "path": "packages/database/auth/auth-options.ts",
        "symbol": "authOptions session callback",
        "sha256": "22b8923e1cab6b5b1b318609abe664e171fb740ae39817c2c962908ca0dc8595",
    },
    {
        "path": "packages/database/schema.ts",
        "symbol": "user preferences persistence schema",
        "sha256": "7fce297f9076be78a9ac6280d9d060bf6e836a62e0f82b5390fa0e42dc7bb9e9",
    },
    {
        "path": "packages/database/helpers.ts",
        "symbol": "Cap NanoID helper",
        "sha256": "e976df51a8186737a1c4696a20cd52f2c029aa630b2463d1970b8667b0dd20cd",
    },
    {
        "path": "packages/database/index.ts",
        "symbol": "database factory",
        "sha256": "161c1d1fd2a561fd2846aeceb148f24b58afc58bdaa95175240e48dbe61d9bbb",
    },
    {
        "path": "packages/web-domain/src/User.ts",
        "symbol": "UserId",
        "sha256": "5b3374425a4c9df1501af34c8f1f780c3f7612f093cd2ff0ed5c442e41e7cee1",
    },
    {
        "path": "apps/web/package.json",
        "symbol": "next and drizzle dependency declaration",
        "sha256": "c1358cd1880ac5dc9d659760c2788cedd5c4f61fec2cb0dd1b60cbc9bb8af920",
    },
    {
        "path": "packages/database/package.json",
        "symbol": "database dependency declaration",
        "sha256": "95629fc376bfc4df4f9f69a28a874e8bcf8496ccec276fd2168cfc9720e4a057",
    },
    {
        "path": "pnpm-lock.yaml",
        "symbol": "notification dependency resolution",
        "sha256": "fc0fe122ae5fbea4dcaa7e510bd6275635c14071c4e031996431a54fb7e25e3a",
    },
)

MEMBERSHIP_REMOVE_INVITE_RUNTIME_SOURCES: tuple[dict[str, str], ...] = (
    {
        "path": "apps/web/app/(org)/dashboard/settings/organization/components/MembersCard.tsx",
        "symbol": "remove organization invite caller",
        "sha256": "65e4e28028188a3ee29c25d94161419dbfdec04cb458e7aff9a450c51dbed743",
    },
    {
        "path": "packages/database/auth/session.ts",
        "symbol": "getCurrentUser",
        "sha256": "d526dc9d7a6a1a7cb6a8695c24ab88b843ce09b4444f8e4ade24b7a06cbbc1ee",
    },
    {
        "path": "packages/database/auth/auth-options.ts",
        "symbol": "authOptions session callback",
        "sha256": "22b8923e1cab6b5b1b318609abe664e171fb740ae39817c2c962908ca0dc8595",
    },
    {
        "path": "apps/web/actions/organization/authorization.ts",
        "symbol": "requireOrganizationSettingsManager",
        "sha256": "6b1422de53d0915a985dc1dbf70f14494fd1c5fe49ca61fbacabd90bebb00980",
    },
    {
        "path": "apps/web/lib/permissions/roles.ts",
        "symbol": "organization settings roles",
        "sha256": "97bf35a09f4ef403dd0ffaa572c40c29f5776c4e6ae73c3e1e511ca376d5a407",
    },
    {
        "path": "packages/database/schema.ts",
        "symbol": "membership persistence schema",
        "sha256": "7fce297f9076be78a9ac6280d9d060bf6e836a62e0f82b5390fa0e42dc7bb9e9",
    },
    {
        "path": "packages/database/index.ts",
        "symbol": "database factory",
        "sha256": "161c1d1fd2a561fd2846aeceb148f24b58afc58bdaa95175240e48dbe61d9bbb",
    },
    {
        "path": "packages/web-domain/src/Organisation.ts",
        "symbol": "OrganisationId",
        "sha256": "14d634ad8910d3921af2ea5b136b9c3d2a8ae26f74b3dcb7a82b9cf19d6a3264",
    },
    {
        "path": "packages/web-domain/src/User.ts",
        "symbol": "UserId",
        "sha256": "5b3374425a4c9df1501af34c8f1f780c3f7612f093cd2ff0ed5c442e41e7cee1",
    },
    {
        "path": "pnpm-lock.yaml",
        "symbol": "membership dependency resolution",
        "sha256": "fc0fe122ae5fbea4dcaa7e510bd6275635c14071c4e031996431a54fb7e25e3a",
    },
)

MEMBERSHIP_SPACE_RUNTIME_SOURCES: tuple[dict[str, str], ...] = (
    {
        "path": "packages/database/auth/session.ts",
        "symbol": "getCurrentUser",
        "sha256": "d526dc9d7a6a1a7cb6a8695c24ab88b843ce09b4444f8e4ade24b7a06cbbc1ee",
    },
    {
        "path": "packages/database/auth/auth-options.ts",
        "symbol": "authOptions session callback",
        "sha256": "22b8923e1cab6b5b1b318609abe664e171fb740ae39817c2c962908ca0dc8595",
    },
    {
        "path": "apps/web/actions/organization/space-authorization.ts",
        "symbol": "requireSpaceManager",
        "sha256": "2a656f25f7c73f2342104127d818a56fffd7d05768d787489b65e08f70a43445",
    },
    {
        "path": "apps/web/lib/permissions/roles.ts",
        "symbol": "space membership roles",
        "sha256": "97bf35a09f4ef403dd0ffaa572c40c29f5776c4e6ae73c3e1e511ca376d5a407",
    },
    {
        "path": "packages/database/schema.ts",
        "symbol": "membership persistence schema",
        "sha256": "7fce297f9076be78a9ac6280d9d060bf6e836a62e0f82b5390fa0e42dc7bb9e9",
    },
    {
        "path": "packages/database/helpers.ts",
        "symbol": "Cap NanoID helper",
        "sha256": "e976df51a8186737a1c4696a20cd52f2c029aa630b2463d1970b8667b0dd20cd",
    },
    {
        "path": "packages/database/index.ts",
        "symbol": "database factory",
        "sha256": "161c1d1fd2a561fd2846aeceb148f24b58afc58bdaa95175240e48dbe61d9bbb",
    },
    {
        "path": "packages/web-domain/src/Organisation.ts",
        "symbol": "OrganisationId",
        "sha256": "14d634ad8910d3921af2ea5b136b9c3d2a8ae26f74b3dcb7a82b9cf19d6a3264",
    },
    {
        "path": "packages/web-domain/src/Space.ts",
        "symbol": "SpaceId",
        "sha256": "ad9cb2ae26767bebf00640846bce4cab6feee6a6308ac0d7b068cd6e006542c3",
    },
    {
        "path": "packages/web-domain/src/User.ts",
        "symbol": "UserId",
        "sha256": "5b3374425a4c9df1501af34c8f1f780c3f7612f093cd2ff0ed5c442e41e7cee1",
    },
    {
        "path": "pnpm-lock.yaml",
        "symbol": "membership dependency resolution",
        "sha256": "fc0fe122ae5fbea4dcaa7e510bd6275635c14071c4e031996431a54fb7e25e3a",
    },
)

MEMBERSHIP_SET_SPACE_RUNTIME_SOURCES = (
    {
        "path": "apps/web/app/(org)/dashboard/spaces/[spaceId]/components/MembersIndicator.tsx",
        "symbol": "set space members caller",
        "sha256": "7981d1f2320f618efbf8de916d6a2a8828dfa832ebbb1a93b8555955209d4790",
    },
    *MEMBERSHIP_SPACE_RUNTIME_SOURCES,
)

LIBRARY_ID_READ_COMMON_RUNTIME_SOURCES: tuple[dict[str, str], ...] = (
    {
        "path": "packages/database/auth/session.ts",
        "symbol": "getCurrentUser",
        "sha256": "d526dc9d7a6a1a7cb6a8695c24ab88b843ce09b4444f8e4ade24b7a06cbbc1ee",
    },
    {
        "path": "packages/database/auth/auth-options.ts",
        "symbol": "authOptions+session callback",
        "sha256": "22b8923e1cab6b5b1b318609abe664e171fb740ae39817c2c962908ca0dc8595",
    },
    {
        "path": "packages/database/schema.ts",
        "symbol": "users+folders+sharedVideos+spaces+spaceVideos+videos",
        "sha256": "7fce297f9076be78a9ac6280d9d060bf6e836a62e0f82b5390fa0e42dc7bb9e9",
    },
    {
        "path": "packages/database/index.ts",
        "symbol": "db",
        "sha256": "161c1d1fd2a561fd2846aeceb148f24b58afc58bdaa95175240e48dbe61d9bbb",
    },
)
LIBRARY_ID_READ_VIDEO_RUNTIME_SOURCE: tuple[dict[str, str], ...] = (
    {
        "path": "packages/web-domain/src/Video.ts",
        "symbol": "VideoId",
        "sha256": "adc3db0eded2670b1ed89969e7bc85993e04021acb303082b3d015f0afb1c9a7",
    },
    {
        "path": "pnpm-lock.yaml",
        "symbol": "drizzle-orm+mysql2+next-auth resolutions",
        "sha256": "fc0fe122ae5fbea4dcaa7e510bd6275635c14071c4e031996431a54fb7e25e3a",
    },
)
LIBRARY_ID_READ_FOLDER_RUNTIME_SOURCES: tuple[dict[str, str], ...] = (
    {
        "path": "apps/web/app/(org)/dashboard/spaces/[spaceId]/folder/[folderId]/AddVideosButton.tsx",
        "symbol": "getEntityVideoIds",
        "sha256": "a526a81701c68b9d76367817164fcd0b72e7e3c930d89de03f782c6f388ff871",
    },
    *LIBRARY_ID_READ_COMMON_RUNTIME_SOURCES,
    {
        "path": "packages/web-domain/src/Folder.ts",
        "symbol": "FolderId",
        "sha256": "4201376991878efc79979f77901908d542573f5b0f9e1ca6b6b246e04d881e9e",
    },
    {
        "path": "packages/web-domain/src/Space.ts",
        "symbol": "SpaceIdOrOrganisationId",
        "sha256": "ad9cb2ae26767bebf00640846bce4cab6feee6a6308ac0d7b068cd6e006542c3",
    },
    *LIBRARY_ID_READ_VIDEO_RUNTIME_SOURCE,
)
LIBRARY_ID_READ_ORGANIZATION_RUNTIME_SOURCES: tuple[dict[str, str], ...] = (
    {
        "path": "apps/web/app/(org)/dashboard/spaces/[spaceId]/components/AddVideosToOrganizationDialog.tsx",
        "symbol": "getEntityVideoIds",
        "sha256": "c8bc5ef4dc2cc0dc8f452d2769be9c3be49e8be6204cb5d9c2b9bdd0d327efd7",
    },
    *LIBRARY_ID_READ_COMMON_RUNTIME_SOURCES,
    {
        "path": "packages/web-domain/src/Organisation.ts",
        "symbol": "OrganisationId",
        "sha256": "14d634ad8910d3921af2ea5b136b9c3d2a8ae26f74b3dcb7a82b9cf19d6a3264",
    },
    *LIBRARY_ID_READ_VIDEO_RUNTIME_SOURCE,
)
LIBRARY_ID_READ_SPACE_RUNTIME_SOURCES: tuple[dict[str, str], ...] = (
    {
        "path": "apps/web/app/(org)/dashboard/spaces/[spaceId]/components/AddVideosDialog.tsx",
        "symbol": "getEntityVideoIds",
        "sha256": "238104cd063757bb8bf785f94acf5c75ccb7b9ef14b7ef519636925b091a9201",
    },
    *LIBRARY_ID_READ_COMMON_RUNTIME_SOURCES,
    {
        "path": "packages/web-domain/src/Space.ts",
        "symbol": "SpaceIdOrOrganisationId",
        "sha256": "ad9cb2ae26767bebf00640846bce4cab6feee6a6308ac0d7b068cd6e006542c3",
    },
    *LIBRARY_ID_READ_VIDEO_RUNTIME_SOURCE,
)

LIBRARY_DETAIL_COMMON_RUNTIME_SOURCES: tuple[dict[str, str], ...] = (
    {
        "path": "packages/database/auth/session.ts",
        "symbol": "getCurrentUser",
        "sha256": "d526dc9d7a6a1a7cb6a8695c24ab88b843ce09b4444f8e4ade24b7a06cbbc1ee",
    },
    {
        "path": "packages/database/auth/auth-options.ts",
        "symbol": "authOptions+session callback",
        "sha256": "22b8923e1cab6b5b1b318609abe664e171fb740ae39817c2c962908ca0dc8595",
    },
    {
        "path": "packages/database/schema.ts",
        "symbol": "users+organizations+memberships+spaces+folders+videos+placements+comments+uploads",
        "sha256": "7fce297f9076be78a9ac6280d9d060bf6e836a62e0f82b5390fa0e42dc7bb9e9",
    },
    {
        "path": "packages/database/index.ts",
        "symbol": "db",
        "sha256": "161c1d1fd2a561fd2846aeceb148f24b58afc58bdaa95175240e48dbe61d9bbb",
    },
)
LIBRARY_DETAIL_IDENTIFIER_RUNTIME_SOURCES: tuple[dict[str, str], ...] = (
    {
        "path": "packages/web-domain/src/Video.ts",
        "symbol": "VideoId",
        "sha256": "adc3db0eded2670b1ed89969e7bc85993e04021acb303082b3d015f0afb1c9a7",
    },
    {
        "path": "packages/web-domain/src/User.ts",
        "symbol": "UserId",
        "sha256": "5b3374425a4c9df1501af34c8f1f780c3f7612f093cd2ff0ed5c442e41e7cee1",
    },
    {
        "path": "apps/web/package.json",
        "symbol": "server-action runtime dependencies",
        "sha256": "c1358cd1880ac5dc9d659760c2788cedd5c4f61fec2cb0dd1b60cbc9bb8af920",
    },
    {
        "path": "packages/database/package.json",
        "symbol": "drizzle database dependencies",
        "sha256": "95629fc376bfc4df4f9f69a28a874e8bcf8496ccec276fd2168cfc9720e4a057",
    },
    {
        "path": "pnpm-lock.yaml",
        "symbol": "drizzle-orm+mysql2+next-auth resolutions",
        "sha256": "fc0fe122ae5fbea4dcaa7e510bd6275635c14071c4e031996431a54fb7e25e3a",
    },
)
LIBRARY_DETAIL_GET_USER_VIDEOS_RUNTIME_SOURCES: tuple[dict[str, str], ...] = (
    {
        "path": "apps/web/app/(org)/dashboard/spaces/[spaceId]/components/AddVideosDialog.tsx",
        "symbol": "getVideos",
        "sha256": "238104cd063757bb8bf785f94acf5c75ccb7b9ef14b7ef519636925b091a9201",
    },
    {
        "path": "apps/web/app/(org)/dashboard/spaces/[spaceId]/components/AddVideosToOrganizationDialog.tsx",
        "symbol": "getVideos",
        "sha256": "c8bc5ef4dc2cc0dc8f452d2769be9c3be49e8be6204cb5d9c2b9bdd0d327efd7",
    },
    {
        "path": "apps/web/app/(org)/dashboard/spaces/[spaceId]/folder/[folderId]/AddVideosButton.tsx",
        "symbol": "getVideos",
        "sha256": "a526a81701c68b9d76367817164fcd0b72e7e3c930d89de03f782c6f388ff871",
    },
    {
        "path": "apps/web/app/(org)/dashboard/spaces/[spaceId]/components/AddVideosDialogBase.tsx",
        "symbol": "VideoData+user-videos query",
        "sha256": "f0af1cb1bb501582cc83c4c68b9613e7c8b21823d246b18d29edb405b0777c89",
    },
    *LIBRARY_DETAIL_COMMON_RUNTIME_SOURCES,
    {
        "path": "packages/web-domain/src/Space.ts",
        "symbol": "SpaceIdOrOrganisationId",
        "sha256": "ad9cb2ae26767bebf00640846bce4cab6feee6a6308ac0d7b068cd6e006542c3",
    },
    *LIBRARY_DETAIL_IDENTIFIER_RUNTIME_SOURCES,
)
LIBRARY_DETAIL_SEARCH_RUNTIME_SOURCES: tuple[dict[str, str], ...] = (
    {
        "path": "apps/web/app/(org)/dashboard/_components/Navbar/DashboardSearch.tsx",
        "symbol": "video search debounce+cache+projection",
        "sha256": "2b5a3a4027023c4f2dc61cee8673ab52a69aae6bb2601f900e41f57ac196c3da",
    },
    *LIBRARY_DETAIL_COMMON_RUNTIME_SOURCES,
    *LIBRARY_DETAIL_IDENTIFIER_RUNTIME_SOURCES,
)

SPACE_ACCESS_ROLE_RUNTIME_SOURCES: tuple[dict[str, str], ...] = (
    {
        "path": "apps/web/lib/permissions/roles.ts",
        "symbol": "space access roles",
        "sha256": "97bf35a09f4ef403dd0ffaa572c40c29f5776c4e6ae73c3e1e511ca376d5a407",
    },
)
SPACE_MANAGER_ROLE_RUNTIME_SOURCES: tuple[dict[str, str], ...] = (
    {
        "path": "apps/web/lib/permissions/roles.ts",
        "symbol": "space manager roles",
        "sha256": "97bf35a09f4ef403dd0ffaa572c40c29f5776c4e6ae73c3e1e511ca376d5a407",
    },
)

INVITE_LIFECYCLE_COMMON_RUNTIME_SOURCES: tuple[dict[str, str], ...] = (
    {
        "path": "apps/web/app/(org)/invite/[inviteId]/InviteAccept.tsx",
        "symbol": "InviteAccept accept+decline callers",
        "sha256": "987b73562aef6f5c5d6c8cfc4189572961fdd4ee5b992182f3b022d3d5dcb832",
    },
    {
        "path": "packages/database/auth/session.ts",
        "symbol": "getCurrentUser",
        "sha256": "d526dc9d7a6a1a7cb6a8695c24ab88b843ce09b4444f8e4ade24b7a06cbbc1ee",
    },
    {
        "path": "packages/database/auth/auth-options.ts",
        "symbol": "authOptions+session callback",
        "sha256": "22b8923e1cab6b5b1b318609abe664e171fb740ae39817c2c962908ca0dc8595",
    },
    {
        "path": "packages/database/schema.ts",
        "symbol": "users+organizations+organizationInvites+organizationMembers+spaces+spaceMembers",
        "sha256": "7fce297f9076be78a9ac6280d9d060bf6e836a62e0f82b5390fa0e42dc7bb9e9",
    },
    {
        "path": "packages/database/index.ts",
        "symbol": "db transaction",
        "sha256": "161c1d1fd2a561fd2846aeceb148f24b58afc58bdaa95175240e48dbe61d9bbb",
    },
    {
        "path": "packages/web-domain/src/Organisation.ts",
        "symbol": "OrganisationId+empty fallback",
        "sha256": "14d634ad8910d3921af2ea5b136b9c3d2a8ae26f74b3dcb7a82b9cf19d6a3264",
    },
    {
        "path": "pnpm-lock.yaml",
        "symbol": "drizzle-orm+nanoid+next-auth resolutions",
        "sha256": "fc0fe122ae5fbea4dcaa7e510bd6275635c14071c4e031996431a54fb7e25e3a",
    },
)
INVITE_ACCEPT_RUNTIME_SOURCES: tuple[dict[str, str], ...] = (
    *INVITE_LIFECYCLE_COMMON_RUNTIME_SOURCES,
    {
        "path": "apps/web/lib/permissions/roles.ts",
        "symbol": "normalizeAssignableOrganizationRole",
        "sha256": "97bf35a09f4ef403dd0ffaa572c40c29f5776c4e6ae73c3e1e511ca376d5a407",
    },
    {
        "path": "apps/web/utils/organization.ts",
        "symbol": "calculateProSeats",
        "sha256": "dc966112b9258abb6ad4888651185614e6c48c2bd5e2abf536711b2d02af0e3b",
    },
    {
        "path": "packages/database/helpers.ts",
        "symbol": "nanoId",
        "sha256": "e976df51a8186737a1c4696a20cd52f2c029aa630b2463d1970b8667b0dd20cd",
    },
)
INVITE_DECLINE_RUNTIME_SOURCES: tuple[dict[str, str], ...] = (
    *INVITE_LIFECYCLE_COMMON_RUNTIME_SOURCES,
    {
        "path": "apps/web/__tests__/unit/invite-decline.test.ts",
        "symbol": "POST decline unit contract",
        "sha256": "47a63825d40eb87a252ba74ee777584a3c7b85317aa3d3d935e93cf88947236e",
    },
)

DEVELOPER_COMMON_RUNTIME_SOURCES: tuple[dict[str, str], ...] = (
    {
        "path": "apps/web/__tests__/unit/developer-actions.test.ts",
        "symbol": "developer action source contract tests",
        "sha256": "8bdac7dc68cf8a76476333e2c2875bde863e9a6f6076c4d010a1b62a43d09552",
    },
    {
        "path": "apps/web/app/(org)/dashboard/developers/developer-data.ts",
        "symbol": "developer dashboard read projection",
        "sha256": "74e819f058c1fc88fe0ee4af1cd52428c6074f29485e1991f5d66b97297c6d07",
    },
    {
        "path": "apps/web/app/(org)/dashboard/developers/layout.tsx",
        "symbol": "developer dashboard session boundary",
        "sha256": "ea20cbedbf18cd564b74efbeb551586fd468d95530aea134a013d2c1e62ada7c",
    },
    {
        "path": "packages/database/auth/session.ts",
        "symbol": "getCurrentUser",
        "sha256": "d526dc9d7a6a1a7cb6a8695c24ab88b843ce09b4444f8e4ade24b7a06cbbc1ee",
    },
    {
        "path": "packages/database/auth/auth-options.ts",
        "symbol": "authOptions session callback",
        "sha256": "22b8923e1cab6b5b1b318609abe664e171fb740ae39817c2c962908ca0dc8595",
    },
    {
        "path": "packages/database/schema.ts",
        "symbol": "developer persistence schema",
        "sha256": "7fce297f9076be78a9ac6280d9d060bf6e836a62e0f82b5390fa0e42dc7bb9e9",
    },
    {
        "path": "packages/database/helpers.ts",
        "symbol": "Cap NanoID helpers",
        "sha256": "e976df51a8186737a1c4696a20cd52f2c029aa630b2463d1970b8667b0dd20cd",
    },
    {
        "path": "packages/database/index.ts",
        "symbol": "database factory",
        "sha256": "161c1d1fd2a561fd2846aeceb148f24b58afc58bdaa95175240e48dbe61d9bbb",
    },
    {
        "path": "apps/web/package.json",
        "symbol": "developer dependency declaration",
        "sha256": "c1358cd1880ac5dc9d659760c2788cedd5c4f61fec2cb0dd1b60cbc9bb8af920",
    },
    {
        "path": "packages/database/package.json",
        "symbol": "database dependency declaration",
        "sha256": "95629fc376bfc4df4f9f69a28a874e8bcf8496ccec276fd2168cfc9720e4a057",
    },
    {
        "path": "pnpm-lock.yaml",
        "symbol": "developer dependency resolution",
        "sha256": "fc0fe122ae5fbea4dcaa7e510bd6275635c14071c4e031996431a54fb7e25e3a",
    },
)
DEVELOPER_SECRET_RUNTIME_SOURCES: tuple[dict[str, str], ...] = (
    {
        "path": "apps/web/lib/developer-key-hash.ts",
        "symbol": "developer API key hash",
        "sha256": "ecc93fc2828647aeaa88dcb9dda0cb2fbcb8b87d4f1a326476878834c06620b1",
    },
    {
        "path": "packages/database/crypto.ts",
        "symbol": "credential encryption",
        "sha256": "d547c7ba0f984d1e625d807e4a1e64cfb400ed2fcc796cf9f6e43713805efb6f",
    },
    {
        "path": "packages/env/index.ts",
        "symbol": "environment contract",
        "sha256": "c15990c4bfb98c65518003ba9692dd8d2c173c36e78991be1f519cce89e96dc9",
    },
    {
        "path": "packages/env/server.ts",
        "symbol": "server environment contract",
        "sha256": "235c2ea66843b610aee61c82cbcafe05086d00193545bc290650d3aa15a2a0a4",
    },
    {
        "path": "packages/env/package.json",
        "symbol": "environment dependency declaration",
        "sha256": "4a12ca3b40acec2340015815c2517b0513ee1024ad0832c80fd8824a9d7948f2",
    },
)
DEVELOPER_CALLERS: dict[str, dict[str, str]] = {
    "create": {
        "path": "apps/web/app/(org)/dashboard/developers/_components/CreateAppDialog.tsx",
        "symbol": "create developer app caller",
        "sha256": "f8ccf4849312d0359af406d79bdc0436294204eb608484249164bb5bed10e2b2",
    },
    "settings": {
        "path": "apps/web/app/(org)/dashboard/developers/apps/[appId]/settings/AppSettingsClient.tsx",
        "symbol": "developer app settings caller",
        "sha256": "3c12fbdc9d52f487c5ccfb4973630bfd187741d56f93c4ac2d8f00f09097b6eb",
    },
    "add_domain": {
        "path": "apps/web/app/(org)/dashboard/developers/apps/[appId]/domains/DomainsClient.tsx",
        "symbol": "add developer domain caller",
        "sha256": "d001d24859dea17a89f5ef39d89c5bbd00ba65662c1bd1961c0f5a4ca163e61d",
    },
    "remove_domain": {
        "path": "apps/web/app/(org)/dashboard/developers/_components/DomainRow.tsx",
        "symbol": "remove developer domain caller",
        "sha256": "1aec74bbd8388993894f0bfe06763761a027f5c749997521ee998d1550d3aa90",
    },
    "keys": {
        "path": "apps/web/app/(org)/dashboard/developers/apps/[appId]/api-keys/ApiKeysClient.tsx",
        "symbol": "developer key regeneration caller",
        "sha256": "202260b8aeebd6bf449cd99359a6d3d6bab9f756cc7db655cd9a94d46218c312",
    },
    "video": {
        "path": "apps/web/app/(org)/dashboard/developers/apps/[appId]/videos/VideosClient.tsx",
        "symbol": "developer video deletion caller",
        "sha256": "2312fef40823f173c695bc157f4c1d586833f1a2aa351b98e1a98b5cc904b5de",
    },
}

DEVELOPER_CREATE_RUNTIME_SOURCES = (
    DEVELOPER_CALLERS["create"],
    *DEVELOPER_COMMON_RUNTIME_SOURCES,
    *DEVELOPER_SECRET_RUNTIME_SOURCES,
)
DEVELOPER_SETTINGS_RUNTIME_SOURCES = (
    DEVELOPER_CALLERS["settings"],
    *DEVELOPER_COMMON_RUNTIME_SOURCES,
)
DEVELOPER_ADD_DOMAIN_RUNTIME_SOURCES = (
    DEVELOPER_CALLERS["add_domain"],
    *DEVELOPER_COMMON_RUNTIME_SOURCES,
)
DEVELOPER_REMOVE_DOMAIN_RUNTIME_SOURCES = (
    DEVELOPER_CALLERS["remove_domain"],
    *DEVELOPER_COMMON_RUNTIME_SOURCES,
)
DEVELOPER_KEYS_RUNTIME_SOURCES = (
    DEVELOPER_CALLERS["keys"],
    *DEVELOPER_COMMON_RUNTIME_SOURCES,
    *DEVELOPER_SECRET_RUNTIME_SOURCES,
)
DEVELOPER_VIDEO_RUNTIME_SOURCES = (
    DEVELOPER_CALLERS["video"],
    *DEVELOPER_COMMON_RUNTIME_SOURCES,
)
DEVELOPER_AUTO_TOP_UP_RUNTIME_SOURCES = DEVELOPER_COMMON_RUNTIME_SOURCES

USER_ACCOUNT_LOCAL_RUNTIME_SOURCES: tuple[dict[str, str], ...] = (
    {
        "path": "packages/database/index.ts",
        "symbol": "db",
        "sha256": "161c1d1fd2a561fd2846aeceb148f24b58afc58bdaa95175240e48dbe61d9bbb",
    },
    {
        "path": "packages/database/auth/session.ts",
        "symbol": "getCurrentUser",
        "sha256": "d526dc9d7a6a1a7cb6a8695c24ab88b843ce09b4444f8e4ade24b7a06cbbc1ee",
    },
    {
        "path": "packages/database/schema.ts",
        "symbol": "users+organizations+organizationMembers+sessions+authApiKeys",
        "sha256": "7fce297f9076be78a9ac6280d9d060bf6e836a62e0f82b5390fa0e42dc7bb9e9",
    },
)

USER_ONBOARDING_RUNTIME_SOURCES: tuple[dict[str, str], ...] = (
    {
        "path": "packages/web-backend/src/Rpcs.ts",
        "symbol": "RpcsLive+RpcAuthMiddlewareLive",
        "sha256": "cfb2cbee41a0abef4496fa2eb42c43688310cc13590e77c1425dc7f919304f19",
    },
    {
        "path": "packages/web-backend/src/Auth.ts",
        "symbol": "getCurrentUser+makeCurrentUser",
        "sha256": "aea054db2b84a8c4bd6684fefe8d0e971a094a9faa9653105b0c33ab52ab824d",
    },
    {
        "path": "packages/web-backend/src/Database.ts",
        "symbol": "Database",
        "sha256": "24500254943ace60c5ea3a7943f40c85ab2c9a8caba36073ff54100ab9488837",
    },
    {
        "path": "packages/web-domain/src/Authentication.ts",
        "symbol": "CurrentUser+RpcAuthMiddleware",
        "sha256": "165c9f652c39d7f1cf3b43a5c66c5a4418bbe97338279ca01d00c19f2026167b",
    },
    {
        "path": "packages/web-domain/src/User.ts",
        "symbol": "UserCompleteOnboardingStep",
        "sha256": "5b3374425a4c9df1501af34c8f1f780c3f7612f093cd2ff0ed5c442e41e7cee1",
    },
    {
        "path": "packages/web-domain/src/Errors.ts",
        "symbol": "InternalError",
        "sha256": "80493b611030104b495601652e2a87589ec9e293605ab92f4016ad38c9c67260",
    },
    {
        "path": "packages/web-domain/src/Organisation.ts",
        "symbol": "OrganisationId",
        "sha256": "14d634ad8910d3921af2ea5b136b9c3d2a8ae26f74b3dcb7a82b9cf19d6a3264",
    },
    {
        "path": "packages/web-backend/src/Users/UsersRpcs.ts",
        "symbol": "UserCompleteOnboardingStep",
        "sha256": "7446ba17a317affa70bba61d9de7c4dad19df1999cfc362c57ce874063b4bc9b",
    },
    {
        "path": "packages/web-backend/src/Users/UsersOnboarding.ts",
        "symbol": "UsersOnboarding",
        "sha256": "fb64431395e35b1ecc2901a8a1541922e98700d6e5f17b8dd907fcc4dc94dc82",
    },
    {
        "path": "packages/web-domain/src/ImageUpload.ts",
        "symbol": "ImageUpdatePayload+extractFileKey",
        "sha256": "23b81310fbe78dad7ac94d0985518e1f3ad86926df282646ca38fd5bd547f47a",
    },
    {
        "path": "packages/web-backend/src/ImageUploads/index.ts",
        "symbol": "ImageUploads.applyUpdate",
        "sha256": "1dc0952ae84d76844128d0fc5cdf2eb63519c26183f932c035638ff0d6463d1c",
    },
    {
        "path": "packages/web-backend/src/S3Buckets/index.ts",
        "symbol": "S3Buckets.getBucketAccess",
        "sha256": "5fc970066be2551488eb3d9e5bcdd1a8255798da53c9b3f4e5c0048c03551b7f",
    },
    {
        "path": "packages/web-backend/src/S3Buckets/S3BucketsRepo.ts",
        "symbol": "S3BucketsRepo",
        "sha256": "efd7081204c829384bdc13d04295364d99c3c0cb6400821df06a993c55caffba",
    },
    {
        "path": "packages/web-backend/src/S3Buckets/S3BucketAccess.ts",
        "symbol": "S3BucketAccess",
        "sha256": "d14f27a6e81e9e13c4108aaceb0098875808440b9397620a83f0d17d4c27cd3b",
    },
    {
        "path": "packages/web-backend/src/S3Buckets/S3BucketClientProvider.ts",
        "symbol": "S3BucketClientProvider",
        "sha256": "d715478d0b5a9981315259e0dd9ddf03273a075a01a9ea685facd6d0ab75242a",
    },
    {
        "path": "packages/database/schema.ts",
        "symbol": "users+organizations+organizationMembers+sessions+authApiKeys",
        "sha256": "7fce297f9076be78a9ac6280d9d060bf6e836a62e0f82b5390fa0e42dc7bb9e9",
    },
    {
        "path": "packages/database/helpers.ts",
        "symbol": "nanoId",
        "sha256": "e976df51a8186737a1c4696a20cd52f2c029aa630b2463d1970b8667b0dd20cd",
    },
    {
        "path": "pnpm-lock.yaml",
        "symbol": "@effect/rpc@0.71.2+effect@3.21.4",
        "sha256": "fc0fe122ae5fbea4dcaa7e510bd6275635c14071c4e031996431a54fb7e25e3a",
    },
)

USER_UPDATE_RUNTIME_SOURCES: tuple[dict[str, str], ...] = (
    {
        "path": "packages/web-backend/src/Rpcs.ts",
        "symbol": "RpcsLive+RpcAuthMiddlewareLive",
        "sha256": "cfb2cbee41a0abef4496fa2eb42c43688310cc13590e77c1425dc7f919304f19",
    },
    {
        "path": "packages/web-backend/src/Auth.ts",
        "symbol": "getCurrentUser+makeCurrentUser",
        "sha256": "aea054db2b84a8c4bd6684fefe8d0e971a094a9faa9653105b0c33ab52ab824d",
    },
    {
        "path": "packages/web-backend/src/Database.ts",
        "symbol": "Database",
        "sha256": "24500254943ace60c5ea3a7943f40c85ab2c9a8caba36073ff54100ab9488837",
    },
    {
        "path": "packages/web-domain/src/Authentication.ts",
        "symbol": "CurrentUser+RpcAuthMiddleware",
        "sha256": "165c9f652c39d7f1cf3b43a5c66c5a4418bbe97338279ca01d00c19f2026167b",
    },
    {
        "path": "packages/web-domain/src/User.ts",
        "symbol": "UserUpdate",
        "sha256": "5b3374425a4c9df1501af34c8f1f780c3f7612f093cd2ff0ed5c442e41e7cee1",
    },
    {
        "path": "packages/web-domain/src/Errors.ts",
        "symbol": "InternalError",
        "sha256": "80493b611030104b495601652e2a87589ec9e293605ab92f4016ad38c9c67260",
    },
    {
        "path": "packages/web-domain/src/Policy.ts",
        "symbol": "PolicyDeniedError",
        "sha256": "0621949aa1f994836d0d168b39dc3aada3ad0478052b712de564b105c94ebe5c",
    },
    {
        "path": "packages/web-backend/src/Users/UsersRpcs.ts",
        "symbol": "UserUpdate",
        "sha256": "7446ba17a317affa70bba61d9de7c4dad19df1999cfc362c57ce874063b4bc9b",
    },
    {
        "path": "packages/web-backend/src/Users/index.ts",
        "symbol": "Users.update",
        "sha256": "6e992f04942fca3647e7b9673d7fff087fad59e905d08edd1b8ef39579a808a4",
    },
    {
        "path": "packages/web-domain/src/ImageUpload.ts",
        "symbol": "ImageUpdatePayload+extractFileKey",
        "sha256": "23b81310fbe78dad7ac94d0985518e1f3ad86926df282646ca38fd5bd547f47a",
    },
    {
        "path": "packages/web-backend/src/ImageUploads/index.ts",
        "symbol": "ImageUploads.applyUpdate",
        "sha256": "1dc0952ae84d76844128d0fc5cdf2eb63519c26183f932c035638ff0d6463d1c",
    },
    {
        "path": "packages/web-backend/src/S3Buckets/index.ts",
        "symbol": "S3Buckets.getBucketAccess",
        "sha256": "5fc970066be2551488eb3d9e5bcdd1a8255798da53c9b3f4e5c0048c03551b7f",
    },
    {
        "path": "packages/database/schema.ts",
        "symbol": "users+organizations+organizationMembers+sessions+authApiKeys",
        "sha256": "7fce297f9076be78a9ac6280d9d060bf6e836a62e0f82b5390fa0e42dc7bb9e9",
    },
    {
        "path": "pnpm-lock.yaml",
        "symbol": "@effect/rpc@0.71.2+effect@3.21.4",
        "sha256": "fc0fe122ae5fbea4dcaa7e510bd6275635c14071c4e031996431a54fb7e25e3a",
    },
)

# Shared source pins for the ten video-property operations. The declaration or
# action source remains the adapter's primary source; these entries close over
# every authentication, schema, crypto, cookie, and provider-read dependency
# that changes an observable result.
VIDEO_PROPERTIES_SCHEMA_SOURCE = {
    "path": "packages/database/schema.ts",
    "symbol": "videos+spaces+spaceVideos+comments+videoUploads",
    "sha256": "7fce297f9076be78a9ac6280d9d060bf6e836a62e0f82b5390fa0e42dc7bb9e9",
}
VIDEO_PROPERTIES_SESSION_SOURCE = {
    "path": "packages/database/auth/session.ts",
    "symbol": "getCurrentUser",
    "sha256": "d526dc9d7a6a1a7cb6a8695c24ab88b843ce09b4444f8e4ade24b7a06cbbc1ee",
}
VIDEO_PROPERTIES_CRYPTO_SOURCE = {
    "path": "packages/database/crypto.ts",
    "symbol": "hashPassword+verifyPassword+encrypt+decrypt",
    "sha256": "d547c7ba0f984d1e625d807e4a1e64cfb400ed2fcc796cf9f6e43713805efb6f",
}
VIDEO_PROPERTIES_PROVIDER_READ_SOURCE = {
    "path": "packages/web-backend/src/Videos/index.ts",
    "symbol": "Videos.getThumbnailURL+getAnalytics",
    "sha256": "43b523a47ed667f70f7f10dde8677740d663811c61f1af278441929184963849",
}
VIDEO_PROPERTIES_DATE_METADATA_SOURCE = {
    "path": "packages/database/types/metadata.ts",
    "symbol": "VideoMetadata.customCreatedAt",
    "sha256": "bedb7af94afc4332bbcc2e86ed195641d670d9937ef3ddae6ba62186b6dfbcee",
}
VIDEO_PROPERTIES_TITLE_METADATA_SOURCE = {
    "path": "packages/database/types/metadata.ts",
    "symbol": "VideoMetadata.titleManuallyEdited",
    "sha256": "bedb7af94afc4332bbcc2e86ed195641d670d9937ef3ddae6ba62186b6dfbcee",
}
VIDEO_PROPERTIES_EFFECTIVE_RULES_SOURCE = {
    "path": "packages/web-backend/src/Videos/EffectiveVideoRules.ts",
    "symbol": "collectPasswordHashes",
    "sha256": "e9b26784e4a1ed5782f9a5cfab52231de629b2f0a3d1b5f40d577b3c798cd015",
}
VIDEO_PROPERTIES_PASSWORD_COOKIE_SOURCE = {
    "path": "apps/web/lib/password-cookie.ts",
    "symbol": "setVerifiedPasswordCookie+MAX_VERIFIED_HASHES",
    "sha256": "3af65d04b06ca336b5e6659806380e4552d1d5514abfbc7f7d771c7cb75260e7",
}
VIDEO_PROPERTIES_PLAYBACK_SPEED_SOURCE = {
    "path": "apps/web/lib/playback-speed.ts",
    "symbol": "normalizePlaybackSpeed+PLAYBACK_SPEEDS",
    "sha256": "ac57f7543696c735d6d60def2b62482c34a472161c0558c0cd03d97c2a3b5ced",
}
VIDEO_PROPERTIES_PROVIDER_COMPLETION = {
    "decision": "serve_frame_exact_business",
    "local_work": "complete",
    "protected_gates": ["provider_execution"],
    "retirement_decision": "not_proposed",
    "production_behavior": "fail_closed_unavailable",
}
VIDEO_PROPERTIES_HUMAN_COMPLETION = {
    "decision": "serve_frame_exact_business",
    "local_work": "complete",
    "protected_gates": ["human_approval"],
    "retirement_decision": "not_proposed",
    "production_behavior": "fail_closed_unavailable",
}
VIDEO_PROPERTIES_D1_COMPLETION = {
    "decision": "serve_frame_exact_business",
    "local_work": "complete",
    "protected_gates": [],
    "retirement_decision": "not_proposed",
    "production_behavior": "serve_exact_d1",
}
VIDEO_PROPERTIES_ACTION_COMPLETION = {
    "decision": "serve_frame_exact_business",
    "local_work": "complete",
    "protected_gates": [],
    "retirement_decision": "not_proposed",
    "production_behavior": "serve_exact_action",
}
MOBILE_BOOTSTRAP_CAPS_COMPLETION = {
    "decision": "serve_frame_exact_business",
    "local_work": "complete",
    "protected_gates": [],
    "retirement_decision": "not_proposed",
    "production_behavior": "serve_exact_d1",
}

EXTENSION_AUTH_RUNTIME_SOURCES = (
    {
        "path": "packages/web-backend/src/Extension/Http.ts",
        "symbol": "startAuth+approveAuth+revokeAuth+bootstrap",
        "sha256": "8bcaeadc626ec4b0bd43ca6f6e2bba643c7386f16f033b7f5d3c103e2173c602",
    },
    {
        "path": "packages/web-backend/src/Extension/Extensions.ts",
        "symbol": "mintAuthKey+revokeAuthKey+resolveBootstrapOrganization",
        "sha256": "097542ee0ccf8de79f310ebf8da90d982b6c58af169c1d74a4721a68adc48542",
    },
    {
        "path": "packages/web-backend/src/Auth.ts",
        "symbol": "getCurrentUser+HttpAuthMiddlewareLive",
        "sha256": "aea054db2b84a8c4bd6684fefe8d0e971a094a9faa9653105b0c33ab52ab824d",
    },
    {
        "path": "packages/web-domain/src/Authentication.ts",
        "symbol": "HttpAuthMiddleware+CurrentUser",
        "sha256": "165c9f652c39d7f1cf3b43a5c66c5a4418bbe97338279ca01d00c19f2026167b",
    },
    {
        "path": "packages/database/schema.ts",
        "symbol": "extension actor/key/tenant persistence rows",
        "sha256": "7fce297f9076be78a9ac6280d9d060bf6e836a62e0f82b5390fa0e42dc7bb9e9",
    },
    {
        "path": "packages/web-backend/src/Database.ts",
        "symbol": "Database service",
        "sha256": "24500254943ace60c5ea3a7943f40c85ab2c9a8caba36073ff54100ab9488837",
    },
    {
        "path": "packages/utils/src/constants/plans.ts",
        "symbol": "userIsPro",
        "sha256": "e047a50e6f72e3fe33985fde475b25ea4d5f9701fbe15adda0c1cb3aaaa21385",
    },
    {
        "path": "packages/web-domain/src/Video.ts",
        "symbol": "FREE_PLAN_MAX_RECORDING_SECONDS",
        "sha256": "adc3db0eded2670b1ed89969e7bc85993e04021acb303082b3d015f0afb1c9a7",
    },
    {
        "path": "packages/web-domain/src/Http/Api.ts",
        "symbol": "/api/extension mount",
        "sha256": "33f37588b210fe8be6584a51cd786b1347f5f7733fb2ac6b9111002e61437a25",
    },
    {
        "path": "packages/web-backend/src/Http/Live.ts",
        "symbol": "ExtensionHttpLive layer",
        "sha256": "fa73f7797f44f11271e0e59fe14817144733ea06fb954c30e8f2f4720fa7216c",
    },
    {
        "path": "packages/env/server.ts",
        "symbol": "WEB_URL+CAP_CHROME_EXTENSION_ID+NODE_ENV",
        "sha256": "235c2ea66843b610aee61c82cbcafe05086d00193545bc290650d3aa15a2a0a4",
    },
    {
        "path": "packages/env/build.ts",
        "symbol": "NEXT_PUBLIC_IS_CAP",
        "sha256": "454bc82ebd9ca83bae656336b67287d13bc351d357c2143444d226d84f2707bd",
    },
    {
        "path": "apps/chrome-extension/src/shared/api.ts",
        "symbol": "createAuthStart+parseAuthResponse+revokeAuth+fetchBootstrap",
        "sha256": "7439a031accac54fcd727c8b643a40f1fca885fbaa15d769c8a6c1e99bf28df7",
    },
    {
        "path": "apps/chrome-extension/src/shared/types.ts",
        "symbol": "ExtensionAuth+BootstrapData",
        "sha256": "fdd5da209e33f6a28158b4a33e52e147fb03de44c8aa6cb39b6d9cc20b52ead1",
    },
    {
        "path": "pnpm-lock.yaml",
        "symbol": "Effect+Drizzle+extension dependency resolutions",
        "sha256": "fc0fe122ae5fbea4dcaa7e510bd6275635c14071c4e031996431a54fb7e25e3a",
    },
)

EXTENSION_INSTANT_RUNTIME_SOURCES = (
    {"path": "packages/web-domain/src/Video.ts", "symbol": "InstantRecordingCreateInput+InstantRecordingCreateSuccess+VideoNotFoundError", "sha256": "adc3db0eded2670b1ed89969e7bc85993e04021acb303082b3d015f0afb1c9a7"},
    {"path": "packages/web-domain/src/Storage.ts", "symbol": "UploadTarget", "sha256": "a6ce2c9b6c70c7bd1a0f0539291ef2b7ce64093e46ab26646e432e10772bc75d"},
    {"path": "packages/web-domain/src/Policy.ts", "symbol": "PolicyDeniedError+withPolicy", "sha256": "0621949aa1f994836d0d168b39dc3aada3ad0478052b712de564b105c94ebe5c"},
    {"path": "packages/web-domain/src/Authentication.ts", "symbol": "CurrentUser+HttpAuthMiddleware", "sha256": "165c9f652c39d7f1cf3b43a5c66c5a4418bbe97338279ca01d00c19f2026167b"},
    {"path": "packages/web-domain/src/Http/Api.ts", "symbol": "ApiContract /api/extension mount", "sha256": "33f37588b210fe8be6584a51cd786b1347f5f7733fb2ac6b9111002e61437a25"},
    {"path": "packages/web-backend/src/Extension/Http.ts", "symbol": "createInstantRecording+updateInstantRecordingProgress+deleteInstantRecording", "sha256": "8bcaeadc626ec4b0bd43ca6f6e2bba643c7386f16f033b7f5d3c103e2173c602"},
    {"path": "packages/web-backend/src/Videos/index.ts", "symbol": "Videos.createInstantRecording+updateUploadProgress+delete", "sha256": "43b523a47ed667f70f7f10dde8677740d663811c61f1af278441929184963849"},
    {"path": "packages/web-backend/src/Videos/VideosRepo.ts", "symbol": "VideosRepo.create+getById+delete", "sha256": "9d444fe29cb6f22e033da1e16757e3bde2f523f22f812eeba87cca05c56d63b1"},
    {"path": "packages/web-backend/src/Videos/VideosPolicy.ts", "symbol": "VideosPolicy.isOwner", "sha256": "39e4b55f59e0758450d76401706cb2d258c8fe850fef91f395662df9146f7540"},
    {"path": "packages/web-backend/src/Storage/index.ts", "symbol": "getWritableAccessForUser+createUploadTarget", "sha256": "3ea22f76907104e26df8f48bdcac87a5dc2d3d60497dfc409110eb0fa8446b4c"},
    {"path": "packages/web-backend/src/S3Buckets/S3BucketAccess.ts", "symbol": "getPresignedPostUrl+deleteObjects", "sha256": "d14f27a6e81e9e13c4108aaceb0098875808440b9397620a83f0d17d4c27cd3b"},
    {"path": "packages/web-backend/src/S3Buckets/index.ts", "symbol": "organization+user bucket selection", "sha256": "5fc970066be2551488eb3d9e5bcdd1a8255798da53c9b3f4e5c0048c03551b7f"},
    {"path": "packages/web-backend/src/Auth.ts", "symbol": "HttpAuthMiddlewareLive", "sha256": "aea054db2b84a8c4bd6684fefe8d0e971a094a9faa9653105b0c33ab52ab824d"},
    {"path": "packages/web-backend/src/Database.ts", "symbol": "Database", "sha256": "24500254943ace60c5ea3a7943f40c85ab2c9a8caba36073ff54100ab9488837"},
    {"path": "packages/web-backend/src/Http/Live.ts", "symbol": "ExtensionHttpLive", "sha256": "fa73f7797f44f11271e0e59fe14817144733ea06fb954c30e8f2f4720fa7216c"},
    {"path": "packages/database/schema.ts", "symbol": "videos+videoUploads+organizations+folders+authApiKeys", "sha256": "7fce297f9076be78a9ac6280d9d060bf6e836a62e0f82b5390fa0e42dc7bb9e9"},
    {"path": "packages/database/helpers.ts", "symbol": "nanoId", "sha256": "e976df51a8186737a1c4696a20cd52f2c029aa630b2463d1970b8667b0dd20cd"},
    {"path": "packages/env/server.ts", "symbol": "WEB_URL+CAP_VIDEOS_DEFAULT_PUBLIC+R2-compatible storage environment", "sha256": "235c2ea66843b610aee61c82cbcafe05086d00193545bc290650d3aa15a2a0a4"},
    {"path": "packages/env/build.ts", "symbol": "NEXT_PUBLIC_IS_CAP", "sha256": "454bc82ebd9ca83bae656336b67287d13bc351d357c2143444d226d84f2707bd"},
    {"path": "apps/chrome-extension/src/shared/api.ts", "symbol": "createInstantRecording+updateUploadProgress+deleteInstantRecording", "sha256": "7439a031accac54fcd727c8b643a40f1fca885fbaa15d769c8a6c1e99bf28df7"},
    {"path": "apps/chrome-extension/src/shared/types.ts", "symbol": "InstantRecordingCreation", "sha256": "fdd5da209e33f6a28158b4a33e52e147fb03de44c8aa6cb39b6d9cc20b52ead1"},
    {"path": "apps/chrome-extension/src/offscreen/recorder.ts", "symbol": "instant create+progress+failure delete lifecycle", "sha256": "03c2128a66fc6ff2adfb5116907787fde065e2c4887aa1a8242180f1a8546ce4"},
    {"path": "apps/web/utils/upload-target.ts", "symbol": "UploadTarget transport aliases", "sha256": "4677b454e1766367c56d0d8d348628b200f41cde59e064db25699a9e4e2038a3"},
    {"path": "pnpm-lock.yaml", "symbol": "Effect+Drizzle+AWS SDK+nanoid dependency resolutions", "sha256": "fc0fe122ae5fbea4dcaa7e510bd6275635c14071c4e031996431a54fb7e25e3a"},
)

MOBILE_EMAIL_REQUEST_RUNTIME_SOURCES = (
    {"path": "packages/database/schema.ts", "symbol": "verificationTokens+users", "sha256": "7fce297f9076be78a9ac6280d9d060bf6e836a62e0f82b5390fa0e42dc7bb9e9"},
    {"path": "packages/web-backend/src/Database.ts", "symbol": "Database transaction service", "sha256": "24500254943ace60c5ea3a7943f40c85ab2c9a8caba36073ff54100ab9488837"},
    {"path": "packages/database/auth/domain-utils.ts", "symbol": "isEmailAllowedForSignup", "sha256": "f2f77ad2ee6106e482ff6cee183f1d49541de2b7705a824e72a4ea1c55c3310a"},
    {"path": "packages/database/emails/otp-email.tsx", "symbol": "OTPEmail", "sha256": "66c3c658224bc8bd0f03ed2944127dbb5971bbea489efd7bbec6c3c698ba03cc"},
    {"path": "packages/env/server.ts", "symbol": "NEXTAUTH_SECRET+RESEND_API_KEY+CAP_ALLOWED_SIGNUP_DOMAINS", "sha256": "235c2ea66843b610aee61c82cbcafe05086d00193545bc290650d3aa15a2a0a4"},
    {"path": "apps/mobile/src/api/mobile.ts", "symbol": "requestEmailSession released caller", "sha256": "dc426448ea7197353880ddfb771e7ca9d17b903a539acfa6ba28cd66227c3a08"},
    {"path": "pnpm-lock.yaml", "symbol": "Effect+Resend+React email dependency resolutions", "sha256": "fc0fe122ae5fbea4dcaa7e510bd6275635c14071c4e031996431a54fb7e25e3a"},
)

MOBILE_EMAIL_VERIFY_RUNTIME_SOURCES = (
    {"path": "packages/database/schema.ts", "symbol": "verificationTokens+users+authApiKeys", "sha256": "7fce297f9076be78a9ac6280d9d060bf6e836a62e0f82b5390fa0e42dc7bb9e9"},
    {"path": "packages/web-backend/src/Database.ts", "symbol": "Database transaction service", "sha256": "24500254943ace60c5ea3a7943f40c85ab2c9a8caba36073ff54100ab9488837"},
    {"path": "packages/database/auth/domain-utils.ts", "symbol": "isEmailAllowedForSignup", "sha256": "f2f77ad2ee6106e482ff6cee183f1d49541de2b7705a824e72a4ea1c55c3310a"},
    {"path": "packages/database/auth/auth-options.ts", "symbol": "authOptions adapter binding", "sha256": "22b8923e1cab6b5b1b318609abe664e171fb740ae39817c2c962908ca0dc8595"},
    {"path": "packages/database/auth/drizzle-adapter.ts", "symbol": "getUserByEmail+createUser+updateUser+Stripe provisioning", "sha256": "c97d95b50851adf4e809ba829d4fafcb1790a56d5e55a5331f4c614f947a5c52"},
    {"path": "packages/database/helpers.ts", "symbol": "nanoId", "sha256": "e976df51a8186737a1c4696a20cd52f2c029aa630b2463d1970b8667b0dd20cd"},
    {"path": "packages/utils/src/index.ts", "symbol": "STRIPE_AVAILABLE+stripe exports", "sha256": "1a9ebe6c9dadb39206ae8cd3bea95dbd7f9ae913426e8f3f7eb7e5acf9461c83"},
    {"path": "packages/utils/src/lib/stripe/stripe.ts", "symbol": "STRIPE_AVAILABLE+stripe", "sha256": "d2bb7868a33928f06ab543b564bd7365f0b5a48fed619c9ecd66f2a36e244dfc"},
    {"path": "packages/env/server.ts", "symbol": "NEXTAUTH_SECRET+STRIPE_SECRET_KEY+CAP_ALLOWED_SIGNUP_DOMAINS", "sha256": "235c2ea66843b610aee61c82cbcafe05086d00193545bc290650d3aa15a2a0a4"},
    {"path": "apps/mobile/src/api/mobile.ts", "symbol": "verifyEmailSession released caller", "sha256": "dc426448ea7197353880ddfb771e7ca9d17b903a539acfa6ba28cd66227c3a08"},
    {"path": "pnpm-lock.yaml", "symbol": "NextAuth+Stripe+Effect dependency resolutions", "sha256": "fc0fe122ae5fbea4dcaa7e510bd6275635c14071c4e031996431a54fb7e25e3a"},
)

MOBILE_SESSION_REQUEST_RUNTIME_SOURCES = (
    {"path": "packages/database/schema.ts", "symbol": "users+authApiKeys", "sha256": "7fce297f9076be78a9ac6280d9d060bf6e836a62e0f82b5390fa0e42dc7bb9e9"},
    {"path": "packages/web-backend/src/Database.ts", "symbol": "Database transaction service", "sha256": "24500254943ace60c5ea3a7943f40c85ab2c9a8caba36073ff54100ab9488837"},
    {"path": "packages/web-backend/src/Auth.ts", "symbol": "getCurrentUser", "sha256": "aea054db2b84a8c4bd6684fefe8d0e971a094a9faa9653105b0c33ab52ab824d"},
    {"path": "packages/env/server.ts", "symbol": "WEB_URL+VERCEL_ENV+VERCEL_BRANCH_URL_HOST", "sha256": "235c2ea66843b610aee61c82cbcafe05086d00193545bc290650d3aa15a2a0a4"},
    {"path": "apps/mobile/src/api/mobile.ts", "symbol": "requestSession released caller", "sha256": "dc426448ea7197353880ddfb771e7ca9d17b903a539acfa6ba28cd66227c3a08"},
    {"path": "pnpm-lock.yaml", "symbol": "Effect HttpApi+URL dependency resolutions", "sha256": "fc0fe122ae5fbea4dcaa7e510bd6275635c14071c4e031996431a54fb7e25e3a"},
)

MOBILE_SESSION_REVOKE_RUNTIME_SOURCES = (
    {"path": "packages/database/schema.ts", "symbol": "users+authApiKeys", "sha256": "7fce297f9076be78a9ac6280d9d060bf6e836a62e0f82b5390fa0e42dc7bb9e9"},
    {"path": "packages/web-backend/src/Database.ts", "symbol": "Database transaction service", "sha256": "24500254943ace60c5ea3a7943f40c85ab2c9a8caba36073ff54100ab9488837"},
    {"path": "packages/web-backend/src/Auth.ts", "symbol": "HttpAuthMiddlewareLive", "sha256": "aea054db2b84a8c4bd6684fefe8d0e971a094a9faa9653105b0c33ab52ab824d"},
    {"path": "packages/web-domain/src/Authentication.ts", "symbol": "HttpAuthMiddleware", "sha256": "165c9f652c39d7f1cf3b43a5c66c5a4418bbe97338279ca01d00c19f2026167b"},
    {"path": "apps/mobile/src/api/mobile.ts", "symbol": "revokeSession released caller", "sha256": "dc426448ea7197353880ddfb771e7ca9d17b903a539acfa6ba28cd66227c3a08"},
    {"path": "pnpm-lock.yaml", "symbol": "Effect HttpApi middleware dependency resolutions", "sha256": "fc0fe122ae5fbea4dcaa7e510bd6275635c14071c4e031996431a54fb7e25e3a"},
)

MOBILE_BOOTSTRAP_CAPS_AUTH_BACKEND_SOURCE = {
    "path": "packages/web-backend/src/Auth.ts",
    "symbol": "HttpAuthMiddlewareLive+CurrentUser",
    "sha256": "aea054db2b84a8c4bd6684fefe8d0e971a094a9faa9653105b0c33ab52ab824d",
}
MOBILE_BOOTSTRAP_CAPS_AUTH_DOMAIN_SOURCE = {
    "path": "packages/web-domain/src/Authentication.ts",
    "symbol": "HttpAuthMiddleware+CurrentUser",
    "sha256": "165c9f652c39d7f1cf3b43a5c66c5a4418bbe97338279ca01d00c19f2026167b",
}
MOBILE_BOOTSTRAP_CAPS_SCHEMA_SOURCE = {
    "path": "packages/database/schema.ts",
    "symbol": "users+organizations+organizationMembers+folders+videos+comments+videoUploads+authApiKeys",
    "sha256": "7fce297f9076be78a9ac6280d9d060bf6e836a62e0f82b5390fa0e42dc7bb9e9",
}
MOBILE_BOOTSTRAP_CAPS_VIDEOS_SOURCE = {
    "path": "packages/web-backend/src/Videos/index.ts",
    "symbol": "delete+getDownloadInfo+getThumbnailURL+getAnalytics+getAnalyticsBulk",
    "sha256": "43b523a47ed667f70f7f10dde8677740d663811c61f1af278441929184963849",
}
MOBILE_BOOTSTRAP_CAPS_VIDEO_REPO_SOURCE = {
    "path": "packages/web-backend/src/Videos/VideosRepo.ts",
    "symbol": "getById+delete",
    "sha256": "9d444fe29cb6f22e033da1e16757e3bde2f523f22f812eeba87cca05c56d63b1",
}
MOBILE_BOOTSTRAP_CAPS_VIDEO_DOMAIN_SOURCE = {
    "path": "packages/web-domain/src/Video.ts",
    "symbol": "Video.getSource+Mp4Source+M3U8Source+SegmentsSource",
    "sha256": "adc3db0eded2670b1ed89969e7bc85993e04021acb303082b3d015f0afb1c9a7",
}
MOBILE_BOOTSTRAP_CAPS_STORAGE_SOURCE = {
    "path": "packages/web-backend/src/Storage/index.ts",
    "symbol": "getAccessForVideo+S3 list/head/sign/delete",
    "sha256": "3ea22f76907104e26df8f48bdcac87a5dc2d3d60497dfc409110eb0fa8446b4c",
}
MOBILE_BOOTSTRAP_CAPS_S3_SOURCE = {
    "path": "packages/web-backend/src/S3Buckets/S3BucketAccess.ts",
    "symbol": "DEFAULT_PRESIGNED_GET_EXPIRES_SECONDS+listObjects+headObject+deleteObjects",
    "sha256": "d14f27a6e81e9e13c4108aaceb0098875808440b9397620a83f0d17d4c27cd3b",
}
MOBILE_BOOTSTRAP_CAPS_IMAGE_UPLOADS_SOURCE = {
    "path": "packages/web-backend/src/ImageUploads/index.ts",
    "symbol": "resolveImageUrl",
    "sha256": "1dc0952ae84d76844128d0fc5cdf2eb63519c26183f932c035638ff0d6463d1c",
}
MOBILE_BOOTSTRAP_CAPS_IMAGE_DOMAIN_SOURCE = {
    "path": "packages/web-domain/src/ImageUpload.ts",
    "symbol": "extractFileKey",
    "sha256": "23b81310fbe78dad7ac94d0985518e1f3ad86926df282646ca38fd5bd547f47a",
}

def mobile_bootstrap_caps_declaration(symbol: str) -> dict[str, str]:
    return {
        "path": "packages/web-domain/src/Mobile.ts",
        "symbol": symbol,
        "sha256": "331d76900372d62389d729f8682baca1344f3583e3f41f42ad6e3ef2be7a3d5b",
    }


def mobile_bootstrap_caps_client(symbol: str) -> dict[str, str]:
    return {
        "path": "apps/mobile/src/api/mobile.ts",
        "symbol": symbol,
        "sha256": "dc426448ea7197353880ddfb771e7ca9d17b903a539acfa6ba28cd66227c3a08",
    }


MOBILE_BOOTSTRAP_RUNTIME_SOURCES = (
    mobile_bootstrap_caps_declaration("bootstrap"),
    MOBILE_BOOTSTRAP_CAPS_AUTH_BACKEND_SOURCE,
    MOBILE_BOOTSTRAP_CAPS_AUTH_DOMAIN_SOURCE,
    MOBILE_BOOTSTRAP_CAPS_SCHEMA_SOURCE,
    mobile_bootstrap_caps_client("bootstrap released caller"),
    MOBILE_BOOTSTRAP_CAPS_IMAGE_UPLOADS_SOURCE,
    MOBILE_BOOTSTRAP_CAPS_IMAGE_DOMAIN_SOURCE,
    MOBILE_BOOTSTRAP_CAPS_S3_SOURCE,
)
MOBILE_CAPS_LIST_RUNTIME_SOURCES = (
    mobile_bootstrap_caps_declaration("listCaps"),
    MOBILE_BOOTSTRAP_CAPS_AUTH_BACKEND_SOURCE,
    MOBILE_BOOTSTRAP_CAPS_AUTH_DOMAIN_SOURCE,
    MOBILE_BOOTSTRAP_CAPS_SCHEMA_SOURCE,
    mobile_bootstrap_caps_client("listCaps released caller"),
    MOBILE_BOOTSTRAP_CAPS_VIDEOS_SOURCE,
    MOBILE_BOOTSTRAP_CAPS_VIDEO_DOMAIN_SOURCE,
    MOBILE_BOOTSTRAP_CAPS_STORAGE_SOURCE,
    MOBILE_BOOTSTRAP_CAPS_S3_SOURCE,
)
MOBILE_CAP_DELETE_RUNTIME_SOURCES = (
    mobile_bootstrap_caps_declaration("deleteCap"),
    MOBILE_BOOTSTRAP_CAPS_AUTH_BACKEND_SOURCE,
    MOBILE_BOOTSTRAP_CAPS_AUTH_DOMAIN_SOURCE,
    MOBILE_BOOTSTRAP_CAPS_SCHEMA_SOURCE,
    mobile_bootstrap_caps_client("deleteCap released caller"),
    MOBILE_BOOTSTRAP_CAPS_VIDEOS_SOURCE,
    MOBILE_BOOTSTRAP_CAPS_VIDEO_REPO_SOURCE,
    MOBILE_BOOTSTRAP_CAPS_VIDEO_DOMAIN_SOURCE,
    MOBILE_BOOTSTRAP_CAPS_STORAGE_SOURCE,
    MOBILE_BOOTSTRAP_CAPS_S3_SOURCE,
)
MOBILE_CAP_GET_RUNTIME_SOURCES = (
    mobile_bootstrap_caps_declaration("getCap"),
    MOBILE_BOOTSTRAP_CAPS_AUTH_BACKEND_SOURCE,
    MOBILE_BOOTSTRAP_CAPS_AUTH_DOMAIN_SOURCE,
    MOBILE_BOOTSTRAP_CAPS_SCHEMA_SOURCE,
    mobile_bootstrap_caps_client("getCap released caller"),
    MOBILE_BOOTSTRAP_CAPS_VIDEOS_SOURCE,
    MOBILE_BOOTSTRAP_CAPS_VIDEO_DOMAIN_SOURCE,
    MOBILE_BOOTSTRAP_CAPS_STORAGE_SOURCE,
    MOBILE_BOOTSTRAP_CAPS_S3_SOURCE,
    MOBILE_BOOTSTRAP_CAPS_IMAGE_UPLOADS_SOURCE,
    MOBILE_BOOTSTRAP_CAPS_IMAGE_DOMAIN_SOURCE,
)
MOBILE_CAP_DOWNLOAD_RUNTIME_SOURCES = (
    mobile_bootstrap_caps_declaration("getDownload"),
    MOBILE_BOOTSTRAP_CAPS_AUTH_BACKEND_SOURCE,
    MOBILE_BOOTSTRAP_CAPS_AUTH_DOMAIN_SOURCE,
    MOBILE_BOOTSTRAP_CAPS_SCHEMA_SOURCE,
    mobile_bootstrap_caps_client("getDownload released caller"),
    MOBILE_BOOTSTRAP_CAPS_VIDEOS_SOURCE,
    MOBILE_BOOTSTRAP_CAPS_VIDEO_REPO_SOURCE,
    MOBILE_BOOTSTRAP_CAPS_VIDEO_DOMAIN_SOURCE,
    MOBILE_BOOTSTRAP_CAPS_STORAGE_SOURCE,
    MOBILE_BOOTSTRAP_CAPS_S3_SOURCE,
)
MOBILE_CAP_PLAYBACK_RUNTIME_SOURCES = (
    mobile_bootstrap_caps_declaration("getPlayback"),
    MOBILE_BOOTSTRAP_CAPS_AUTH_BACKEND_SOURCE,
    MOBILE_BOOTSTRAP_CAPS_AUTH_DOMAIN_SOURCE,
    MOBILE_BOOTSTRAP_CAPS_SCHEMA_SOURCE,
    mobile_bootstrap_caps_client("getPlayback released caller"),
    MOBILE_BOOTSTRAP_CAPS_VIDEOS_SOURCE,
    MOBILE_BOOTSTRAP_CAPS_VIDEO_REPO_SOURCE,
    MOBILE_BOOTSTRAP_CAPS_VIDEO_DOMAIN_SOURCE,
    MOBILE_BOOTSTRAP_CAPS_STORAGE_SOURCE,
    MOBILE_BOOTSTRAP_CAPS_S3_SOURCE,
)

MOBILE_UPLOAD_STORAGE_DOMAIN_SOURCE = {
    "path": "packages/web-domain/src/Storage.ts",
    "symbol": "UploadTarget",
    "sha256": "a6ce2c9b6c70c7bd1a0f0539291ef2b7ce64093e46ab26646e432e10772bc75d",
}
MOBILE_UPLOAD_AUTH_BACKEND_SOURCE = {
    "path": "packages/web-backend/src/Auth.ts",
    "symbol": "HttpAuthMiddlewareLive+CurrentUser",
    "sha256": "aea054db2b84a8c4bd6684fefe8d0e971a094a9faa9653105b0c33ab52ab824d",
}
MOBILE_UPLOAD_AUTH_DOMAIN_SOURCE = {
    "path": "packages/web-domain/src/Authentication.ts",
    "symbol": "HttpAuthMiddleware+CurrentUser",
    "sha256": "165c9f652c39d7f1cf3b43a5c66c5a4418bbe97338279ca01d00c19f2026167b",
}
MOBILE_UPLOAD_SCHEMA_SOURCE = {
    "path": "packages/database/schema.ts",
    "symbol": "users+organizations+organizationMembers+folders+videos+videoUploads+storageIntegrations",
    "sha256": "7fce297f9076be78a9ac6280d9d060bf6e836a62e0f82b5390fa0e42dc7bb9e9",
}
MOBILE_UPLOAD_DATABASE_SOURCE = {
    "path": "packages/web-backend/src/Database.ts",
    "symbol": "Database use+transaction",
    "sha256": "24500254943ace60c5ea3a7943f40c85ab2c9a8caba36073ff54100ab9488837",
}
MOBILE_UPLOAD_CLIENT_SOURCE = {
    "path": "apps/mobile/src/api/mobile.ts",
    "symbol": "released createUpload+updateUploadProgress+completeUpload callers and upload target transport",
    "sha256": "dc426448ea7197353880ddfb771e7ca9d17b903a539acfa6ba28cd66227c3a08",
}
MOBILE_UPLOAD_ORCHESTRATOR_SOURCE = {
    "path": "apps/mobile/src/uploads/runMobileUpload.ts",
    "symbol": "released create-stream-progress-complete orchestration",
    "sha256": "b67c264da096baa103df5beb7e26ff95f7002c85305c97f1ff327a63be6cf253",
}
MOBILE_UPLOAD_MOUNT_SOURCE = {
    "path": "packages/web-domain/src/Http/Api.ts",
    "symbol": "ApiContract /api/mobile mount",
    "sha256": "33f37588b210fe8be6584a51cd786b1347f5f7733fb2ac6b9111002e61437a25",
}
MOBILE_UPLOAD_VIDEOS_SERVICE_SOURCE = {
    "path": "packages/web-backend/src/Videos/index.ts",
    "symbol": "updateUploadProgress+getUploadProgress",
    "sha256": "43b523a47ed667f70f7f10dde8677740d663811c61f1af278441929184963849",
}
MOBILE_UPLOAD_VIDEOS_REPO_SOURCE = {
    "path": "packages/web-backend/src/Videos/VideosRepo.ts",
    "symbol": "VideosRepo.create",
    "sha256": "9d444fe29cb6f22e033da1e16757e3bde2f523f22f812eeba87cca05c56d63b1",
}
MOBILE_UPLOAD_VIDEOS_POLICY_SOURCE = {
    "path": "packages/web-backend/src/Videos/VideosPolicy.ts",
    "symbol": "VideosPolicy.isOwner",
    "sha256": "39e4b55f59e0758450d76401706cb2d258c8fe850fef91f395662df9146f7540",
}
MOBILE_UPLOAD_STORAGE_SERVICE_SOURCE = {
    "path": "packages/web-backend/src/Storage/index.ts",
    "symbol": "getWritableAccessForUser+createUploadTarget",
    "sha256": "3ea22f76907104e26df8f48bdcac87a5dc2d3d60497dfc409110eb0fa8446b4c",
}
MOBILE_UPLOAD_S3_ACCESS_SOURCE = {
    "path": "packages/web-backend/src/S3Buckets/S3BucketAccess.ts",
    "symbol": "getPresignedPutUrl",
    "sha256": "d14f27a6e81e9e13c4108aaceb0098875808440b9397620a83f0d17d4c27cd3b",
}
MOBILE_UPLOAD_S3_SELECTION_SOURCE = {
    "path": "packages/web-backend/src/S3Buckets/index.ts",
    "symbol": "organization+user writable bucket selection",
    "sha256": "5fc970066be2551488eb3d9e5bcdd1a8255798da53c9b3f4e5c0048c03551b7f",
}
MOBILE_UPLOAD_VIDEO_PROCESSING_SOURCE = {
    "path": "apps/web/lib/video-processing.ts",
    "symbol": "transitionVideoToProcessing+startVideoProcessingWorkflow",
    "sha256": "56d755ad564725c2912a48bce70e2410b991e2bb94889aba021ad4f1ecad32a0",
}
MOBILE_UPLOAD_PROCESS_WORKFLOW_SOURCE = {
    "path": "apps/web/workflows/process-video.ts",
    "symbol": "processVideoWorkflow",
    "sha256": "972696993e47609932fedb6973f75b2c26dafdca0363b07e061d57c777d4095d",
}
MOBILE_UPLOAD_ENV_SOURCE = {
    "path": "packages/env/server.ts",
    "symbol": "WEB_URL+CAP_VIDEOS_DEFAULT_PUBLIC+R2-compatible storage environment",
    "sha256": "235c2ea66843b610aee61c82cbcafe05086d00193545bc290650d3aa15a2a0a4",
}
MOBILE_UPLOAD_LOCK_SOURCE = {
    "path": "pnpm-lock.yaml",
    "symbol": "Effect+Drizzle+AWS SDK+workflow dependency resolutions",
    "sha256": "fc0fe122ae5fbea4dcaa7e510bd6275635c14071c4e031996431a54fb7e25e3a",
}
MOBILE_UPLOAD_COMMON_RUNTIME_SOURCES = (
    MOBILE_UPLOAD_AUTH_BACKEND_SOURCE,
    MOBILE_UPLOAD_AUTH_DOMAIN_SOURCE,
    MOBILE_UPLOAD_SCHEMA_SOURCE,
    MOBILE_UPLOAD_DATABASE_SOURCE,
    MOBILE_UPLOAD_CLIENT_SOURCE,
    MOBILE_UPLOAD_ORCHESTRATOR_SOURCE,
    MOBILE_UPLOAD_MOUNT_SOURCE,
    MOBILE_UPLOAD_LOCK_SOURCE,
)
MOBILE_UPLOAD_CREATE_RUNTIME_SOURCES = (
    MOBILE_UPLOAD_STORAGE_DOMAIN_SOURCE,
    *MOBILE_UPLOAD_COMMON_RUNTIME_SOURCES,
    MOBILE_UPLOAD_VIDEOS_REPO_SOURCE,
    MOBILE_UPLOAD_STORAGE_SERVICE_SOURCE,
    MOBILE_UPLOAD_S3_ACCESS_SOURCE,
    MOBILE_UPLOAD_S3_SELECTION_SOURCE,
    MOBILE_UPLOAD_ENV_SOURCE,
)
MOBILE_UPLOAD_PROGRESS_RUNTIME_SOURCES = (
    *MOBILE_UPLOAD_COMMON_RUNTIME_SOURCES,
    MOBILE_UPLOAD_VIDEOS_SERVICE_SOURCE,
    MOBILE_UPLOAD_VIDEOS_POLICY_SOURCE,
)
MOBILE_UPLOAD_COMPLETE_RUNTIME_SOURCES = (
    *MOBILE_UPLOAD_COMMON_RUNTIME_SOURCES,
    MOBILE_UPLOAD_VIDEO_PROCESSING_SOURCE,
    MOBILE_UPLOAD_PROCESS_WORKFLOW_SOURCE,
)

# Endpoint promotion is deliberately identity- and source-pinned. A family
# authority or similar route name is never enough to enter this map.
DESKTOP_MOUNT_RUNTIME_SOURCE = {
    "path": "apps/web/app/api/desktop/[...route]/route.ts",
    "symbol": "desktop basePath mount+methods+CORS",
    "sha256": "34854ff6fc0839838165990bea1c9ebee86770b1648ec832bbbb786720c9db41",
}
DESKTOP_AUTH_RUNTIME_SOURCE = {
    "path": "apps/web/app/api/utils.ts",
    "symbol": "getAuth+withAuth+corsMiddleware",
    "sha256": "241e5259f690ece17b0c50f78a9dc30c3e783082287040fef0f47e56a937bb30",
}
DESKTOP_SESSION_RUNTIME_SOURCE = {
    "path": "packages/database/auth/session.ts",
    "symbol": "getCurrentUser",
    "sha256": "d526dc9d7a6a1a7cb6a8695c24ab88b843ce09b4444f8e4ade24b7a06cbbc1ee",
}
DESKTOP_SCHEMA_RUNTIME_SOURCE = {
    "path": "packages/database/schema.ts",
    "symbol": "users+organizations+organizationMembers+storageIntegrations+videos+videoUploads+importedVideos",
    "sha256": "7fce297f9076be78a9ac6280d9d060bf6e836a62e0f82b5390fa0e42dc7bb9e9",
}
DESKTOP_WEB_TRANSPORT_RUNTIME_SOURCE = {
    "path": "apps/desktop/src/utils/web-api.ts",
    "symbol": "apiClient+protectedHeaders+server routing",
    "sha256": "d3655b985a21a54d97b9974b17536aebab490929850baffaa5186d7a5632b45a",
}
DESKTOP_RUST_TRANSPORT_RUNTIME_SOURCE = {
    "path": "apps/desktop/src-tauri/src/web_api.rs",
    "symbol": "authed_api_request+Authorization bearer",
    "sha256": "33abf0a3ffbe2912d2fcf251eb4892713ed85b48ccf66822d916b43ce935316c",
}
DESKTOP_ORGANIZATION_PROJECTION_RUNTIME_SOURCE = {
    "path": "apps/web/app/api/desktop/[...route]/organization-branding.ts",
    "symbol": "organization branding normalization+logo validation+projection",
    "sha256": "6383ce7f6d8bc2600c19947e693ab8b7f74ab36baf220a7967f3f9127009f002",
}
DESKTOP_ORGANIZATION_ROLES_RUNTIME_SOURCE = {
    "path": "apps/web/lib/permissions/roles.ts",
    "symbol": "normalizeOrganizationRole+getEffectiveOrganizationRole",
    "sha256": "97bf35a09f4ef403dd0ffaa572c40c29f5776c4e6ae73c3e1e511ca376d5a407",
}

DESKTOP_ORGANIZATIONS_RUNTIME_SOURCES = (
    DESKTOP_ORGANIZATION_PROJECTION_RUNTIME_SOURCE,
    DESKTOP_ORGANIZATION_ROLES_RUNTIME_SOURCE,
    {
        "path": "packages/web-api-contract/src/desktop.ts",
        "symbol": "GET /desktop/organizations",
        "sha256": "e55824d1b9ba74501841905c0bc4e70179247f6cd00e6249849970898af7adb9",
    },
    DESKTOP_MOUNT_RUNTIME_SOURCE,
    DESKTOP_AUTH_RUNTIME_SOURCE,
    DESKTOP_SESSION_RUNTIME_SOURCE,
    DESKTOP_SCHEMA_RUNTIME_SOURCE,
    {
        "path": "apps/desktop/src-tauri/src/api.rs",
        "symbol": "fetch_organizations+Organization",
        "sha256": "d029c4cc7eba0be97f03bba8da2f3ab02277ce65161de69aca4ad77b3474a48e",
    },
    DESKTOP_RUST_TRANSPORT_RUNTIME_SOURCE,
)
DESKTOP_BRANDING_RUNTIME_SOURCES = (
    DESKTOP_ORGANIZATION_PROJECTION_RUNTIME_SOURCE,
    DESKTOP_ORGANIZATION_ROLES_RUNTIME_SOURCE,
    {
        "path": "packages/web-backend/src/ImageUploads/index.ts",
        "symbol": "ImageUploads.applyUpdate+resolveImageUrl",
        "sha256": "1dc0952ae84d76844128d0fc5cdf2eb63519c26183f932c035638ff0d6463d1c",
    },
    {
        "path": "packages/web-domain/src/ImageUpload.ts",
        "symbol": "ImageUpdatePayload+extractFileKey",
        "sha256": "23b81310fbe78dad7ac94d0985518e1f3ad86926df282646ca38fd5bd547f47a",
    },
    {
        "path": "packages/web-api-contract/src/desktop.ts",
        "symbol": "PATCH /desktop/organizations/:organizationId/branding",
        "sha256": "e55824d1b9ba74501841905c0bc4e70179247f6cd00e6249849970898af7adb9",
    },
    DESKTOP_MOUNT_RUNTIME_SOURCE,
    DESKTOP_AUTH_RUNTIME_SOURCE,
    DESKTOP_SESSION_RUNTIME_SOURCE,
    DESKTOP_SCHEMA_RUNTIME_SOURCE,
    {
        "path": "apps/desktop/src/utils/organization-branding.ts",
        "symbol": "updateOrganizationBranding+encodeFileAsBase64",
        "sha256": "659410130f8ff54a4fa7adb3e1d60e8515fc78b44b118bfa74f07537445ab07e",
    },
    DESKTOP_WEB_TRANSPORT_RUNTIME_SOURCE,
)
DESKTOP_STORAGE_RUNTIME_SOURCES = (
    {
        "path": "packages/web-api-contract/src/desktop.ts",
        "symbol": "POST /desktop/storage/set-active",
        "sha256": "e55824d1b9ba74501841905c0bc4e70179247f6cd00e6249849970898af7adb9",
    },
    DESKTOP_MOUNT_RUNTIME_SOURCE,
    DESKTOP_AUTH_RUNTIME_SOURCE,
    DESKTOP_SESSION_RUNTIME_SOURCE,
    DESKTOP_SCHEMA_RUNTIME_SOURCE,
    {
        "path": "apps/desktop/src/routes/(window-chrome)/settings/integrations/google-drive-config.tsx",
        "symbol": "setActive storage mutation",
        "sha256": "84b7e589f65ff6a121ad1fca963134526c75d21aea5f978c8cd50ce23b935c33",
    },
    DESKTOP_WEB_TRANSPORT_RUNTIME_SOURCE,
)
DESKTOP_PROFILE_RUNTIME_SOURCES = (
    {
        "path": "packages/web-api-contract-effect/src/index.ts",
        "symbol": "getUserProfile",
        "sha256": "9c2185ebf12be4c9d231d42938c975ea6ad596a0031ed8a0aca2bb1cbec3c7a0",
    },
    {
        "path": "packages/web-api-contract/src/desktop.ts",
        "symbol": "GET /desktop/user/profile",
        "sha256": "e55824d1b9ba74501841905c0bc4e70179247f6cd00e6249849970898af7adb9",
    },
    DESKTOP_MOUNT_RUNTIME_SOURCE,
    DESKTOP_AUTH_RUNTIME_SOURCE,
    DESKTOP_SESSION_RUNTIME_SOURCE,
    DESKTOP_SCHEMA_RUNTIME_SOURCE,
    {
        "path": "apps/desktop/src/routes/(window-chrome)/settings.tsx",
        "symbol": "getUserProfile query",
        "sha256": "4ee20069fdd0ef077e5e89e5c7bcb8f353b30302c7ab40725c44dc28db7880ae",
    },
    DESKTOP_WEB_TRANSPORT_RUNTIME_SOURCE,
)
DESKTOP_VIDEO_DELETE_RUNTIME_SOURCES = (
    {
        "path": "packages/web-backend/src/Storage/index.ts",
        "symbol": "Storage.getAccessForVideo",
        "sha256": "3ea22f76907104e26df8f48bdcac87a5dc2d3d60497dfc409110eb0fa8446b4c",
    },
    {
        "path": "packages/web-api-contract/src/desktop.ts",
        "symbol": "DELETE /desktop/video/delete",
        "sha256": "e55824d1b9ba74501841905c0bc4e70179247f6cd00e6249849970898af7adb9",
    },
    DESKTOP_MOUNT_RUNTIME_SOURCE,
    DESKTOP_AUTH_RUNTIME_SOURCE,
    DESKTOP_SESSION_RUNTIME_SOURCE,
    DESKTOP_SCHEMA_RUNTIME_SOURCE,
    {
        "path": "apps/desktop/src-tauri/src/recording.rs",
        "symbol": "delete_remote_instant_video",
        "sha256": "15e3dd2b7278e829d9f4dd0b0e5b49bc6b6cc5e21a46a1a36466ad3da319f313",
    },
    DESKTOP_RUST_TRANSPORT_RUNTIME_SOURCE,
)
DESKTOP_VIDEO_PROGRESS_RUNTIME_SOURCES = (
    DESKTOP_MOUNT_RUNTIME_SOURCE,
    DESKTOP_AUTH_RUNTIME_SOURCE,
    DESKTOP_SESSION_RUNTIME_SOURCE,
    DESKTOP_SCHEMA_RUNTIME_SOURCE,
    {
        "path": "apps/desktop/src-tauri/src/api.rs",
        "symbol": "desktop_video_progress",
        "sha256": "d029c4cc7eba0be97f03bba8da2f3ab02277ce65161de69aca4ad77b3474a48e",
    },
    DESKTOP_RUST_TRANSPORT_RUNTIME_SOURCE,
)


def transcript_runtime_sources(operation_id: str) -> tuple[dict[str, str], ...]:
    """Return the checked transitive source closure after the discovered handler."""
    fixture = json.loads(TRANSCRIPTS_FIXTURE.read_text(encoding="utf-8"))
    operation = next(
        item for item in fixture["operations"] if item.get("id") == operation_id
    )
    return tuple(operation["sources"][1:])


TRANSCRIPT_RETRY_RUNTIME_SOURCES = transcript_runtime_sources(
    "cap-v1-c8dffb9b102dd4f7"
)
TRANSCRIPT_EDIT_RUNTIME_SOURCES = transcript_runtime_sources(
    "cap-v1-3db394ae13895b46"
)
TRANSCRIPT_GET_RUNTIME_SOURCES = transcript_runtime_sources(
    "cap-v1-f2659b43d5ee9162"
)
TRANSCRIPT_TRANSLATE_RUNTIME_SOURCES = transcript_runtime_sources(
    "cap-v1-6f6ece85bd786289"
)
TRANSCRIPT_AVAILABLE_RUNTIME_SOURCES = transcript_runtime_sources(
    "cap-v1-6c82f3cbe383d92b"
)

DEVELOPER_SDK_MOUNT_SOURCE = {
    "path": "apps/web/app/api/developer/sdk/v1/[...route]/route.ts",
    "symbol": "developer SDK basePath+routes+methods",
    "sha256": "d7bff3e0512f37b7991b6728d573be787b3b70dde1fccc7fab445ce263942da4",
}
DEVELOPER_REST_MOUNT_SOURCE = {
    "path": "apps/web/app/api/developer/v1/[...route]/route.ts",
    "symbol": "developer REST basePath+routes+methods",
    "sha256": "f1ab0e78c1c9ec590b51e5f635cdae7a5fac08c01666c8a0bcf51fe42d100681",
}
DEVELOPER_API_UTILS_SOURCE = {
    "path": "apps/web/app/api/utils.ts",
    "symbol": "developerRateLimiter+developerSdkCors+withDeveloperPublicAuth+withDeveloperSecretAuth",
    "sha256": "241e5259f690ece17b0c50f78a9dc30c3e783082287040fef0f47e56a937bb30",
}
DEVELOPER_KEY_HASH_SOURCE = {
    "path": "apps/web/lib/developer-key-hash.ts",
    "symbol": "hashKey",
    "sha256": "ecc93fc2828647aeaa88dcb9dda0cb2fbcb8b87d4f1a326476878834c06620b1",
}
DEVELOPER_SCHEMA_SOURCE = {
    "path": "packages/database/schema.ts",
    "symbol": "developerApps+developerApiKeys+developerVideos+developerCreditAccounts+developerCreditTransactions+developerDailyStorageSnapshots",
    "sha256": "7fce297f9076be78a9ac6280d9d060bf6e836a62e0f82b5390fa0e42dc7bb9e9",
}
DEVELOPER_DATABASE_SOURCE = {
    "path": "packages/database/index.ts",
    "symbol": "db",
    "sha256": "161c1d1fd2a561fd2846aeceb148f24b58afc58bdaa95175240e48dbe61d9bbb",
}
DEVELOPER_IDENTIFIERS_SOURCE = {
    "path": "packages/database/helpers.ts",
    "symbol": "nanoId",
    "sha256": "e976df51a8186737a1c4696a20cd52f2c029aa630b2463d1970b8667b0dd20cd",
}
DEVELOPER_SDK_CLIENT_SOURCE = {
    "path": "packages/sdk-recorder/src/upload/multipart-client.ts",
    "symbol": "MultipartClient",
    "sha256": "dfca27f63a9ac358a4001d87d5bd7952b2a884e174773aea70addd6753af3458",
}
DEVELOPER_API_DOCS_SOURCE = {
    "path": "apps/web/content/docs/api/rest-api.mdx",
    "symbol": "REST API+SDK API wire documentation",
    "sha256": "2098d91af1ee4ac099ed944a03733bfdcd9974ccf4b7fb8463fa049f8aea6d11",
}
DEVELOPER_MULTIPART_RUNTIME_SOURCES = (
    DEVELOPER_SDK_MOUNT_SOURCE,
    DEVELOPER_API_UTILS_SOURCE,
    DEVELOPER_KEY_HASH_SOURCE,
    DEVELOPER_SCHEMA_SOURCE,
    DEVELOPER_DATABASE_SOURCE,
    DEVELOPER_IDENTIFIERS_SOURCE,
    {
        "path": "packages/web-backend/src/S3Buckets/index.ts",
        "symbol": "S3Buckets.getBucketAccess",
        "sha256": "5fc970066be2551488eb3d9e5bcdd1a8255798da53c9b3f4e5c0048c03551b7f",
    },
    {
        "path": "packages/web-backend/src/S3Buckets/S3BucketAccess.ts",
        "symbol": "multipart.create+getPresignedUploadPartUrl+complete+abort",
        "sha256": "d14f27a6e81e9e13c4108aaceb0098875808440b9397620a83f0d17d4c27cd3b",
    },
    DEVELOPER_SDK_CLIENT_SOURCE,
    DEVELOPER_API_DOCS_SOURCE,
)
DEVELOPER_VIDEO_CREATE_RUNTIME_SOURCES = (
    DEVELOPER_SDK_MOUNT_SOURCE,
    DEVELOPER_API_UTILS_SOURCE,
    DEVELOPER_KEY_HASH_SOURCE,
    DEVELOPER_SCHEMA_SOURCE,
    DEVELOPER_DATABASE_SOURCE,
    DEVELOPER_IDENTIFIERS_SOURCE,
    DEVELOPER_SDK_CLIENT_SOURCE,
    DEVELOPER_API_DOCS_SOURCE,
    {
        "path": "packages/env/index.ts",
        "symbol": "buildEnv.NEXT_PUBLIC_WEB_URL",
        "sha256": "c15990c4bfb98c65518003ba9692dd8d2c173c36e78991be1f519cce89e96dc9",
    },
)
DEVELOPER_USAGE_RUNTIME_SOURCES = (
    DEVELOPER_REST_MOUNT_SOURCE,
    DEVELOPER_API_UTILS_SOURCE,
    DEVELOPER_KEY_HASH_SOURCE,
    DEVELOPER_SCHEMA_SOURCE,
    DEVELOPER_DATABASE_SOURCE,
    DEVELOPER_API_DOCS_SOURCE,
)
DEVELOPER_VIDEOS_RUNTIME_SOURCES = DEVELOPER_USAGE_RUNTIME_SOURCES
DEVELOPER_STORAGE_CRON_RUNTIME_SOURCES = (
    DEVELOPER_SCHEMA_SOURCE,
    DEVELOPER_DATABASE_SOURCE,
    DEVELOPER_IDENTIFIERS_SOURCE,
    {
        "path": "apps/web/__tests__/unit/developer-cron-storage.test.ts",
        "symbol": "developer-storage cron job",
        "sha256": "618f6ba76fcbe104e9429514c5b7d6f6d7aaeb5e7377b9bacb6aba3789962d55",
    },
)


VIDEO_LIFECYCLE_RPCS_SOURCE = {
    "path": "packages/web-backend/src/Rpcs.ts",
    "symbol": "RpcsLive+RpcAuthMiddlewareLive",
    "sha256": "cfb2cbee41a0abef4496fa2eb42c43688310cc13590e77c1425dc7f919304f19",
}
VIDEO_LIFECYCLE_VIDEOS_SOURCE = {
    "path": "packages/web-backend/src/Videos/index.ts",
    "symbol": "Videos.delete",
    "sha256": "43b523a47ed667f70f7f10dde8677740d663811c61f1af278441929184963849",
}
VIDEO_LIFECYCLE_OG_SOURCE = {
    "path": "apps/web/actions/videos/get-og-image.tsx",
    "symbol": "generateVideoOgImage+getData",
    "sha256": "5ac48c887030592bfeb439f8a900306e6249aad47a170992eda7b27914eed958",
}


def video_lifecycle_local_adapter(
    operation_id: str,
    source_path: str,
    source_sha256: str,
    authority: str,
    auth: str,
    policy: str,
    production_behavior: str,
    extra_sources: tuple[dict[str, str], ...] = (),
) -> dict[str, Any]:
    return {
        "id": operation_id,
        "source_path": source_path,
        "source_sha256": source_sha256,
        "extra_sources": extra_sources,
        "local_status": "rust_exact_video_lifecycle_d1_r2_local_contract",
        "rust_authority": authority,
        "auth": auth,
        "policy": policy,
        "completion": {
            "decision": "serve_frame_exact_business",
            "local_work": "complete",
            "protected_gates": [],
            "retirement_decision": "not_proposed",
            "production_behavior": production_behavior,
        },
    }


VIDEO_LIFECYCLE_LOCAL_ENDPOINT_ADAPTERS = {
    ("route", "GET", "/api/erpc"): video_lifecycle_local_adapter(
        "cap-v1-6cee4f8c7f91f9fd",
        "apps/web/app/api/erpc/route.ts",
        "01a2dee0518e44fe6137513f117100e6a626b904e4ee4608fc0be6d69e210783",
        "exact Effect-RPC GET carrier with stable redacted protocol defects",
        "anonymous",
        "service_misc.v1",
        "serve_exact_d1",
        (VIDEO_LIFECYCLE_RPCS_SOURCE,),
    ),
    ("route", "POST", "/api/erpc"): video_lifecycle_local_adapter(
        "cap-v1-5d669b34ea762549",
        "apps/web/app/api/erpc/route.ts",
        "01a2dee0518e44fe6137513f117100e6a626b904e4ee4608fc0be6d69e210783",
        "single-read Effect-RPC dispatcher with per-tag D1/R2 authorities and typed Exit encoding",
        "session",
        "service_misc.v1",
        "serve_exact_d1_r2",
        (VIDEO_LIFECYCLE_RPCS_SOURCE,),
    ),
    ("route", "DELETE", "/api/video/delete"): video_lifecycle_local_adapter(
        "cap-v1-ac0d7aa564f2991c",
        "apps/web/app/api/video/delete/route.ts",
        "8bf8a91a8f5ec657e199fec9df28638759ae78d6e9021977dff0e608b008a5cc",
        "owner-scoped D1 tombstone and immutable resumable R2 prefix cleanup",
        "session",
        "video_media.v1",
        "serve_exact_d1_r2",
        (VIDEO_LIFECYCLE_VIDEOS_SOURCE,),
    ),
    ("route", "GET", "/api/video/og"): video_lifecycle_local_adapter(
        "cap-v1-7d6fa824a5356ace",
        "apps/web/app/api/video/og/route.tsx",
        "675da4bf50b4952801275b179d1b2cc833a3a6cb9d717bd468f4f67026c20758",
        "anonymous D1 public-bit projection plus bounded R2 screenshot discovery and 1200x630 PNG renderer",
        "anonymous",
        "video_media.v1",
        "serve_exact_d1_r2",
        (VIDEO_LIFECYCLE_OG_SOURCE,),
    ),
    ("rpc", "RPC", "/api/erpc#OrganisationUpdate"): video_lifecycle_local_adapter(
        "cap-v1-e32af2138aa62c8d",
        "packages/web-backend/src/Organisations/OrganisationsRpcs.ts",
        "b87253931de5e7401fa25e392dbdd111417a207d88aeba52b493e058807243d5",
        "admin/owner D1 authority plus R2 put, atomic icon pointer swap, and resumable managed-key cleanup",
        "session",
        "organization_library.v1",
        "serve_exact_d1_r2",
    ),
    ("rpc", "RPC", "/api/erpc#VideoDelete"): video_lifecycle_local_adapter(
        "cap-v1-1e909cc023a9c4a7",
        "packages/web-backend/src/Videos/VideosRpcs.ts",
        "6edf9add90a28c542fb53c9a7bfa858bc89290e2a0fbeec827210bd5af189623",
        "owner-scoped D1 tombstone and request-bound resumable R2 prefix cleanup",
        "session",
        "video_media.v1",
        "serve_exact_d1_r2",
    ),
    ("rpc", "RPC", "/api/erpc#VideoDuplicate"): video_lifecycle_local_adapter(
        "cap-v1-e6a882aeeffaa4f6",
        "packages/web-backend/src/Videos/VideosRpcs.ts",
        "6edf9add90a28c542fb53c9a7bfa858bc89290e2a0fbeec827210bd5af189623",
        "owner-scoped D1 metadata clone plus durable destination binding and per-object R2 copy receipts",
        "session",
        "video_media.v1",
        "serve_exact_d1_r2",
    ),
    ("rpc", "RPC", "/api/erpc#VideoInstantCreate"): video_lifecycle_local_adapter(
        "cap-v1-7b4e8210491e549d",
        "packages/web-backend/src/Videos/VideosRpcs.ts",
        "6edf9add90a28c542fb53c9a7bfa858bc89290e2a0fbeec827210bd5af189623",
        "active-organization D1 recording creation, exact R2 PUT capability, and atomic Effect request replay receipt",
        "session",
        "video_media.v1",
        "serve_exact_d1_r2",
    ),
}


def storage_local_adapter(
    operation_id: str,
    source_path: str,
    source_sha256: str,
    local_status: str,
    authority: str,
    auth: str,
    policy: str,
    production_behavior: str,
    protected_gates: tuple[str, ...] = (),
    extra_sources: tuple[dict[str, str], ...] = (),
) -> dict[str, Any]:
    return {
        "id": operation_id,
        "source_path": source_path,
        "source_sha256": source_sha256,
        "local_status": local_status,
        "rust_authority": authority,
        "auth": auth,
        "policy": policy,
        "extra_sources": extra_sources,
        "completion": {
            "decision": (
                "retain_replace_with_provider_effect"
                if protected_gates
                else "serve_frame_exact_business"
            ),
            "local_work": "complete",
            "protected_gates": list(protected_gates),
            "retirement_decision": "not_proposed",
            "production_behavior": production_behavior,
        },
    }


CORE_STORAGE_LOCAL_ENDPOINT_ADAPTERS = {
    ("route", "GET", "/api/download"): storage_local_adapter(
        "cap-v1-d9d8d275d476c8be",
        "apps/web/app/api/download/route.ts",
        "56b53c55eead824ad59651ac1ec27cc893c60b6440ff72af288033fcdcab380b",
        "rust_exact_download_platform_redirect_local_contract",
        "bounded platform detection and source-exact temporary redirect",
        "optional_session_or_share_capability",
        "upload_storage.v1",
        "serve_exact_redirect",
    ),
    ("route", "GET", "/api/playlist"): storage_local_adapter(
        "cap-v1-ebb74af6ba0b5848",
        "apps/web/app/api/playlist/route.ts",
        "f9e19e262054235aef305ed4e1052729035ee212fcdad5126b517a84e30c9d29",
        "rust_exact_playlist_get_d1_r2_local_contract",
        "optional viewer/password authority and exact R2 playlist projection",
        "optional_session_or_share_capability",
        "share_playback.v1",
        "serve_exact_r2",
    ),
    ("route", "HEAD", "/api/playlist"): storage_local_adapter(
        "cap-v1-428610929ae5bb26",
        "apps/web/app/api/playlist/route.ts",
        "f9e19e262054235aef305ed4e1052729035ee212fcdad5126b517a84e30c9d29",
        "rust_exact_playlist_head_d1_r2_local_contract",
        "GET-equivalent playlist authority and headers with an empty body",
        "optional_session_or_share_capability",
        "share_playback.v1",
        "serve_exact_r2",
    ),
    ("route", "GET", "/api/storage/object"): storage_local_adapter(
        "cap-v1-b5388e4ddf2d7f17",
        "apps/web/app/api/storage/object/route.ts",
        "85b77b12ca6553b592428bec9e3c9bba1a966d7c5b5f2f440edc861e7d45d775",
        "rust_exact_storage_object_get_d1_r2_local_contract",
        "session/token-bound D1 video authority and exact R2 range proxy",
        "session",
        "upload_storage.v1",
        "serve_exact_r2",
    ),
    ("route", "HEAD", "/api/storage/object"): storage_local_adapter(
        "cap-v1-09e9e5a5c86b98c1",
        "apps/web/app/api/storage/object/route.ts",
        "85b77b12ca6553b592428bec9e3c9bba1a966d7c5b5f2f440edc861e7d45d775",
        "rust_exact_storage_object_head_d1_r2_local_contract",
        "GET-equivalent object authority and headers with an empty body",
        "session",
        "upload_storage.v1",
        "serve_exact_r2",
    ),
    ("route", "POST", "/api/upload/multipart/abort"): storage_local_adapter(
        "cap-v1-f191ed86271608e3",
        "apps/web/app/api/upload/[...route]/multipart.ts",
        "97644564903178d153d1232d9767086e5c77c8cb4cb96fedc483755260e29934",
        "rust_exact_multipart_abort_d1_r2_local_contract",
        "owner-scoped immutable multipart receipt plus resumable R2 abort",
        "session",
        "upload_storage.v1",
        "serve_exact_d1",
    ),
    ("route", "POST", "/api/upload/multipart/complete"): storage_local_adapter(
        "cap-v1-efc19423a62b7976",
        "apps/web/app/api/upload/[...route]/multipart.ts",
        "97644564903178d153d1232d9767086e5c77c8cb4cb96fedc483755260e29934",
        "rust_exact_multipart_complete_d1_orchestration_provider_pending",
        "ordered multipart evidence and immutable D1 completion intent without fabricated provider success",
        "session",
        "upload_storage.v1",
        "fail_closed_unavailable",
        ("provider_execution",),
    ),
    ("route", "POST", "/api/upload/multipart/initiate"): storage_local_adapter(
        "cap-v1-f47512c6177fa691",
        "apps/web/app/api/upload/[...route]/multipart.ts",
        "97644564903178d153d1232d9767086e5c77c8cb4cb96fedc483755260e29934",
        "rust_exact_multipart_initiate_d1_r2_local_contract",
        "tenant-owned D1 multipart binding and exact R2 create capability",
        "session",
        "upload_storage.v1",
        "serve_exact_d1",
    ),
    ("route", "POST", "/api/upload/multipart/presign-part"): storage_local_adapter(
        "cap-v1-7b584d9338e8bf31",
        "apps/web/app/api/upload/[...route]/multipart.ts",
        "97644564903178d153d1232d9767086e5c77c8cb4cb96fedc483755260e29934",
        "rust_exact_multipart_presign_part_d1_r2_local_contract",
        "bound multipart session and exact R2 part capability",
        "session",
        "upload_storage.v1",
        "serve_exact_d1",
    ),
    ("route", "POST", "/api/upload/recording-complete"): storage_local_adapter(
        "cap-v1-f9deb8104204a30d",
        "apps/web/app/api/upload/[...route]/recording-complete.ts",
        "cf62e89c55960c9ff016233c4c3e463ce46111680c2a18f317743f393bd2a46a",
        "rust_exact_recording_complete_d1_orchestration_provider_pending",
        "exact recording evidence and immutable D1 finalization intent without fabricated workflow success",
        "session",
        "upload_storage.v1",
        "fail_closed_unavailable",
        ("provider_execution",),
    ),
    ("route", "POST", "/api/upload/signed"): storage_local_adapter(
        "cap-v1-7f87205cb7d39ee6",
        "apps/web/app/api/upload/[...route]/signed.ts",
        "c1b02757cc95ae84e8beb2249e00e3495061ddf8b6d9db1e7e10712e7fbb11c3",
        "rust_exact_signed_upload_d1_r2_local_contract",
        "tenant-scoped key normalization and exact R2 PUT capability",
        "session",
        "upload_storage.v1",
        "serve_exact_d1",
    ),
    ("route", "POST", "/api/upload/signed/batch"): storage_local_adapter(
        "cap-v1-c64cec46e4b828da",
        "apps/web/app/api/upload/[...route]/signed.ts",
        "c1b02757cc95ae84e8beb2249e00e3495061ddf8b6d9db1e7e10712e7fbb11c3",
        "rust_exact_signed_upload_batch_d1_r2_local_contract",
        "bounded batch key normalization and per-object exact R2 PUT capabilities",
        "session",
        "upload_storage.v1",
        "serve_exact_d1",
    ),
}


UPLOAD_STORAGE_LOCAL_ENDPOINT_ADAPTERS = {
    ("rpc", "RPC", "/api/erpc#GetUploadProgress"): storage_local_adapter(
        "cap-v1-e7c4af25d620a9b3",
        "apps/web/app/api/erpc/route.ts",
        "01a2dee0518e44fe6137513f117100e6a626b904e4ee4608fc0be6d69e210783",
        "rust_exact_upload_progress_d1_local_contract",
        "optional-viewer D1 upload progress projection with password and tenant fences",
        "optional_session_or_share_capability",
        "upload_storage.v1",
        "serve_exact_d1",
    ),
    ("rpc", "RPC", "/api/erpc#VideoGetDownloadInfo"): storage_local_adapter(
        "cap-v1-43270700eca33966",
        "apps/web/app/api/erpc/route.ts",
        "01a2dee0518e44fe6137513f117100e6a626b904e4ee4608fc0be6d69e210783",
        "rust_exact_video_download_info_rpc_d1_r2_local_contract",
        "optional-viewer D1 authority and exact-key R2 download capability",
        "optional_session_or_share_capability",
        "upload_storage.v1",
        "serve_exact_d1_r2",
    ),
    ("rpc", "RPC", "/api/erpc#VideoUploadProgressUpdate"): storage_local_adapter(
        "cap-v1-4245d3bd72f59e22",
        "apps/web/app/api/erpc/route.ts",
        "01a2dee0518e44fe6137513f117100e6a626b904e4ee4608fc0be6d69e210783",
        "rust_exact_upload_progress_update_d1_local_contract",
        "owner-only monotonic D1 progress update and Effect request replay receipt",
        "session",
        "upload_storage.v1",
        "serve_exact_d1",
    ),
    ("server_action", "ACTION", "action://apps/web/actions/video/upload.ts#createVideoAndGetUploadUrl"): storage_local_adapter(
        "cap-v1-dd270efc913f9af9",
        "apps/web/actions/video/upload.ts",
        "924e12c8a8afc92250c45cb6432ed1098155107b7223f742af3b293dd48e57ce",
        "rust_exact_create_video_upload_d1_r2_local_contract",
        "organization/folder D1 authority plus immutable video receipt and exact R2 PUT capability",
        "session",
        "upload_storage.v1",
        "serve_exact_d1_r2",
    ),
    ("server_action", "ACTION", "action://apps/web/actions/video/upload.ts#deleteVideoResultFile"): storage_local_adapter(
        "cap-v1-6ed7083eeb37e3f8",
        "apps/web/actions/video/upload.ts",
        "924e12c8a8afc92250c45cb6432ed1098155107b7223f742af3b293dd48e57ce",
        "rust_exact_delete_video_result_d1_r2_local_contract",
        "owner-only resumable exact result-object deletion with immutable replay",
        "session",
        "upload_storage.v1",
        "serve_exact_d1_r2",
    ),
    ("server_action", "ACTION", "action://apps/web/actions/videos/download.ts#downloadVideo"): storage_local_adapter(
        "cap-v1-09c995d62aea0fe7",
        "apps/web/actions/videos/download.ts",
        "76b72a7c65d79fab2798a9b7d2486bad0689d4aad48ba97e5941735b4b344d54",
        "rust_exact_download_video_d1_r2_local_contract",
        "owner-only D1 authority and exact result-object R2 GET capability",
        "session",
        "upload_storage.v1",
        "serve_exact_d1_r2",
    ),
    ("server_action", "ACTION", "action://apps/web/actions/videos/download.ts#getVideoDownloadInfo"): storage_local_adapter(
        "cap-v1-cbc5472b81366dcb",
        "apps/web/actions/videos/download.ts",
        "76b72a7c65d79fab2798a9b7d2486bad0689d4aad48ba97e5941735b4b344d54",
        "rust_exact_video_download_info_action_d1_r2_local_contract",
        "session share authority plus progress fences and exact original/current R2 capability",
        "session",
        "upload_storage.v1",
        "serve_exact_d1_r2",
    ),
    ("server_action", "ACTION", "action://apps/web/actions/caps/share.ts#shareCap"): storage_local_adapter(
        "cap-v1-55d41a7742153f1b",
        "apps/web/actions/caps/share.ts",
        "2069fc697bbf9e5e69ed49572b68b7b40c864ea7c2a67630aff5d905b4f54809",
        "rust_exact_share_cap_d1_local_contract",
        "owner-only atomic D1 replacement of organization, space, and public share state",
        "session",
        "share_playback.v1",
        "serve_exact_d1",
    ),
    ("workflow", "WORKFLOW", "workflow://apps/web/lib/video-edit-processing.ts#reconcileStaleEditUpload"): storage_local_adapter(
        "cap-v1-d89571c3e0f65def",
        "apps/web/lib/video-edit-processing.ts",
        "f6589c669096552a798a298b03e48cf55fffc4392e1fbfdef0d29dd9ad762025",
        "rust_exact_reconcile_stale_edit_d1_local_contract",
        "owner-bound snapshot comparison and idempotent D1 stale-edit reconciliation",
        "session",
        "upload_storage.v1",
        "serve_exact_d1",
    ),
}


ANALYTICS_TINYBIRD_SOURCE = {
    "path": "packages/web-backend/src/Tinybird/index.ts",
    "symbol": "Tinybird.appendEvents+querySql+response normalization",
    "sha256": "d92c0740b6f04e2c455742e24799f1923661984447e7b2e1a3d9a3cc5c723f41",
}
ANALYTICS_VIDEO_POLICY_SOURCE = {
    "path": "packages/web-backend/src/Videos/VideosPolicy.ts",
    "symbol": "buildCanView+optional-auth public/password policy",
    "sha256": "39e4b55f59e0758450d76401706cb2d258c8fe850fef91f395662df9146f7540",
}
ANALYTICS_VIDEO_REPO_SOURCE = {
    "path": "packages/web-backend/src/Videos/VideosRepo.ts",
    "symbol": "VideosRepo.getById",
    "sha256": "9d444fe29cb6f22e033da1e16757e3bde2f523f22f812eeba87cca05c56d63b1",
}
ANALYTICS_DATABASE_SCHEMA_SOURCE = {
    "path": "packages/database/schema.ts",
    "symbol": "users+videos+notifications+videoUploads",
    "sha256": "7fce297f9076be78a9ac6280d9d060bf6e836a62e0f82b5390fa0e42dc7bb9e9",
}


ANALYTICS_LOCAL_ENDPOINT_ADAPTERS = {
    ("route", "GET", "/api/analytics"): storage_local_adapter(
        "cap-v1-c8a43dc80c502b6d",
        "apps/web/actions/videos/get-analytics.ts",
        "3663ca3504f6450ac3ad05054bd7c178e04f00b2f73a40c2e9d72aef8ae2a415",
        "rust_exact_analytics_video_count_d1_provider_intent_local_contract",
        "optional viewer/password D1 authority plus immutable Tinybird query intent without fabricated count",
        "optional_session_or_share_capability",
        "analytics_consent.v1",
        "fail_closed_unavailable",
        ("provider_execution",),
        (
            {
                "path": "apps/web/app/api/analytics/route.ts",
                "symbol": "GET+parseRangeParam+canView non-disclosure",
                "sha256": "6b50a1bc796d2fc9405e90fa7a20b04067bb5e400796a9e7edef2522c5b633f9",
            },
            ANALYTICS_TINYBIRD_SOURCE,
            ANALYTICS_VIDEO_POLICY_SOURCE,
            ANALYTICS_VIDEO_REPO_SOURCE,
        ),
    ),
    ("route", "POST", "/api/analytics/track"): storage_local_adapter(
        "cap-v1-51dc2aa9f19a48cc",
        "apps/web/app/api/analytics/track/route.ts",
        "92979446f291dcdfe09d6ac660252ac8769b6823ca3a89e43f6fe4ef02ff0680",
        "rust_exact_analytics_track_d1_provider_outbox_local_contract",
        "source-normalized page-hit, email, and notification D1 outboxes without fabricated provider success",
        "optional_session_or_share_capability",
        "analytics_consent.v1",
        "fail_closed_unavailable",
        ("provider_execution",),
        (
            ANALYTICS_TINYBIRD_SOURCE,
            {
                "path": "apps/web/lib/Notification.ts",
                "symbol": "createAnonymousViewNotification+sendFirstViewEmail",
                "sha256": "5678bf28d261c9be6854a5a67827d090998bbfefa7cc4fb5cdf18a739fb4993e",
            },
            {
                "path": "apps/web/lib/anonymous-names.ts",
                "symbol": "getAnonymousName+getSessionHash",
                "sha256": "3f8a61df017b155809d86e8442baee45014d3e7a82130ad3a02f3acd1e261209",
            },
            {
                "path": "apps/web/app/s/[videoId]/Share.tsx",
                "symbol": "ensureAnalyticsSessionId+trackVideoView",
                "sha256": "79586b9e1b39c6fd91d471d11e56b722139bcc9bf33f6d95708cca1028e85baa",
            },
            ANALYTICS_DATABASE_SCHEMA_SOURCE,
        ),
    ),
    ("route", "GET", "/api/dashboard/analytics"): storage_local_adapter(
        "cap-v1-9b093898957efebb",
        "apps/web/app/(org)/dashboard/analytics/data.ts",
        "a197d55c8553045383e897d3613cd562ee1998896ed3811deae72a5f99f826bf",
        "rust_exact_dashboard_analytics_d1_provider_intent_local_contract",
        "active-organization D1 series and immutable provider query intent with no partial counts",
        "session",
        "analytics_consent.v1",
        "fail_closed_unavailable",
        ("provider_execution",),
        (
            {
                "path": "apps/web/app/api/dashboard/analytics/route.ts",
                "symbol": "GET+active organization+range fallback",
                "sha256": "80ab94d8b16eb01377d2bdad94fb71f3d628be562c321472bcf519efdf4e2c02",
            },
            {
                "path": "apps/web/app/(org)/dashboard/analytics/types.ts",
                "symbol": "AnalyticsRange+OrgAnalyticsResponse",
                "sha256": "9ee52909c63b75a069049d0bdc70081a7fb54513684ba0a488b68198d2284ded",
            },
            {
                "path": "apps/web/app/(org)/dashboard/analytics/components/AnalyticsDashboard.tsx",
                "symbol": "dashboard analytics caller/query serialization",
                "sha256": "74567e59e0a78e37c3fd2ab0e0d08ffcf1fa1aed7551a2865fb048e1592da6d0",
            },
            ANALYTICS_TINYBIRD_SOURCE,
        ),
    ),
    ("route", "GET", "/api/video/analytics"): storage_local_adapter(
        "cap-v1-be2ea6b474aae7c9",
        "apps/web/actions/videos/get-analytics.ts",
        "3663ca3504f6450ac3ad05054bd7c178e04f00b2f73a40c2e9d72aef8ae2a415",
        "rust_exact_video_analytics_http_d1_provider_intent_local_contract",
        "optional viewer/password D1 authority plus immutable Tinybird query intent without fabricated count",
        "optional_session_or_share_capability",
        "analytics_consent.v1",
        "fail_closed_unavailable",
        ("provider_execution",),
        (
            {
                "path": "packages/web-api-contract-effect/src/index.ts",
                "symbol": "getAnalytics",
                "sha256": "9c2185ebf12be4c9d231d42938c975ea6ad596a0031ed8a0aca2bb1cbec3c7a0",
            },
            {
                "path": "packages/web-api-contract/src/index.ts",
                "symbol": "GET /video/analytics",
                "sha256": "98bb2529e27eba0ed1569d286a1f5d4069cbbf23cf9e1dde62fdc1f6a9737e3e",
            },
            ANALYTICS_TINYBIRD_SOURCE,
            ANALYTICS_VIDEO_POLICY_SOURCE,
        ),
    ),
    ("rpc", "RPC", "/api/erpc#VideosGetAnalytics"): storage_local_adapter(
        "cap-v1-7c47f9a2a9a24ac0",
        "apps/web/app/api/erpc/route.ts",
        "01a2dee0518e44fe6137513f117100e6a626b904e4ee4608fc0be6d69e210783",
        "rust_exact_video_analytics_rpc_d1_provider_intent_local_contract",
        "per-item optional-viewer D1 authority and grouped immutable query intent without cross-item disclosure",
        "optional_session_or_share_capability",
        "analytics_consent.v1",
        "fail_closed_unavailable",
        ("provider_execution",),
        (
            {
                "path": "packages/web-backend/src/Rpcs.ts",
                "symbol": "RpcsLive+RpcAuthMiddlewareLive",
                "sha256": "cfb2cbee41a0abef4496fa2eb42c43688310cc13590e77c1425dc7f919304f19",
            },
            {
                "path": "packages/web-backend/src/Videos/VideosRpcs.ts",
                "symbol": "VideosGetAnalytics",
                "sha256": "6edf9add90a28c542fb53c9a7bfa858bc89290e2a0fbeec827210bd5af189623",
            },
            {
                "path": "packages/web-backend/src/Videos/index.ts",
                "symbol": "Videos.getAnalyticsBulk",
                "sha256": "43b523a47ed667f70f7f10dde8677740d663811c61f1af278441929184963849",
            },
            {
                "path": "packages/web-domain/src/Video.ts",
                "symbol": "VideosGetAnalytics schema",
                "sha256": "adc3db0eded2670b1ed89969e7bc85993e04021acb303082b3d015f0afb1c9a7",
            },
            {
                "path": "apps/web/lib/Requests/AnalyticsRequest.ts",
                "symbol": "AnalyticsRequest.DataLoaderResolver",
                "sha256": "4523b0bdd68cba229cea9cf6a8475603aaa2e9cdcb0c97e725b30c14d55cb38c",
            },
            ANALYTICS_TINYBIRD_SOURCE,
            ANALYTICS_VIDEO_POLICY_SOURCE,
        ),
    ),
    ("server_action", "ACTION", "action://apps/web/actions/analytics/track-user-signed-up.ts#checkAndMarkUserSignedUpTracked"): storage_local_adapter(
        "cap-v1-dd88ded400188c1e",
        "apps/web/actions/analytics/track-user-signed-up.ts",
        "f73188fc0f3e91f34c37c82a4a18fc7fd11cf3a17554af6f3a5db82512f6d0de",
        "rust_exact_analytics_signup_cas_d1_local_contract",
        "optional-session eligibility projection and atomic D1 JSON preference compare-and-swap",
        "optional_session_or_share_capability",
        "analytics_consent.v1",
        "serve_exact_d1",
        (),
        (
            {
                "path": "apps/web/app/Layout/PosthogIdentify.tsx",
                "symbol": "PosthogIdentify caller",
                "sha256": "7577e3cb7ff8bced61cd7e3f0ea4948b36bee2aaa75b6e01571b3f4f6dc68e95",
            },
            ANALYTICS_DATABASE_SCHEMA_SOURCE,
        ),
    ),
    ("server_action", "ACTION", "action://apps/web/actions/videos/get-analytics.ts#getVideoAnalytics"): storage_local_adapter(
        "cap-v1-9186738740a1ece1",
        "apps/web/actions/videos/get-analytics.ts",
        "3663ca3504f6450ac3ad05054bd7c178e04f00b2f73a40c2e9d72aef8ae2a415",
        "rust_exact_video_analytics_action_d1_provider_intent_local_contract",
        "optional viewer/password D1 authority plus immutable Tinybird query intent without fabricated count",
        "optional_session_or_share_capability",
        "analytics_consent.v1",
        "fail_closed_unavailable",
        ("provider_execution",),
        (
            ANALYTICS_TINYBIRD_SOURCE,
            {
                "path": "apps/web/app/s/[videoId]/_components/tabs/Activity/Analytics.tsx",
                "symbol": "getVideoAnalytics caller",
                "sha256": "d2eaef0c0d094ba4f3aac5f8fbd27d213f4f170217e0db12947e7dc947757c65",
            },
            {
                "path": "apps/web/app/s/[videoId]/page.tsx",
                "symbol": "viewsPromise caller",
                "sha256": "b9c6e5d777ed424edd14c8840c02cf66bab3f8f33060efdef739de59e7e4d673",
            },
        ),
    ),
}


ORGANIZATION_LIBRARY_EXPECTED_IDS = {
    "cap-v1-2cbdd906b6b7e371",
    "cap-v1-61e089033a34d239",
    "cap-v1-79eeb0016e42f711",
    "cap-v1-120aa129daa79b1e",
    "cap-v1-9227b0da852f2745",
    "cap-v1-575866e31832347a",
    "cap-v1-3a1228254de4338a",
    "cap-v1-1bed8d446a1553b1",
    "cap-v1-b5f1312195f03a0e",
    "cap-v1-7e1553af9e9427af",
    "cap-v1-ff1b0a4f37fb9130",
    "cap-v1-ce276ebd911b73f8",
    "cap-v1-531e69b5e2915e10",
    "cap-v1-408f009a56471811",
    "cap-v1-dd736ee15a42f26b",
    "cap-v1-0d56f082dce4f861",
    "cap-v1-989b3a5027a3f5c0",
    "cap-v1-91184d308c393034",
    "cap-v1-1ffe1392bb59f2ca",
    "cap-v1-67377a620262de2c",
    "cap-v1-404c6ea8306ad5a7",
}


def organization_library_local_adapters() -> dict[tuple[str, str, str], dict[str, Any]]:
    fixture = json.loads(ORGANIZATION_LIBRARY_FIXTURE.read_text(encoding="utf-8"))
    operations = fixture.get("operations", [])
    if (
        fixture.get("schema_version") != "frame.legacy-organization-library.v1"
        or fixture.get("reference_commit") != REFERENCE_COMMIT
        or {item.get("id") for item in operations if isinstance(item, dict)}
        != ORGANIZATION_LIBRARY_EXPECTED_IDS
    ):
        raise RuntimeError("organization-library fixture identity drifted")
    adapters: dict[tuple[str, str, str], dict[str, Any]] = {}
    for item in operations:
        operation_id = item["id"]
        anonymous = operation_id == "cap-v1-61e089033a34d239"
        identity = (item["kind"], item["method"], item["legacy_identity"])
        adapters[identity] = storage_local_adapter(
            operation_id,
            item["source_path"],
            item["source_sha256"],
            "rust_exact_organization_library_d1_r2_action_local_contract",
            f"{item['authority']}; {item['effect']}",
            "anonymous" if anonymous else "session",
            "share_playback.v1" if anonymous else "organization_library.v1",
            "serve_exact_d1_r2_action",
        )
    if len(adapters) != len(ORGANIZATION_LIBRARY_EXPECTED_IDS):
        raise RuntimeError("organization-library fixture has duplicate identities")
    return adapters


ORGANIZATION_LIBRARY_LOCAL_ENDPOINT_ADAPTERS = organization_library_local_adapters()
for _adapter in ORGANIZATION_LIBRARY_LOCAL_ENDPOINT_ADAPTERS.values():
    _anonymous = _adapter["id"] == "cap-v1-61e089033a34d239"
    OPERATION_CONTRACT_OVERRIDES[_adapter["id"]] = {
        "auth": "anonymous" if _anonymous else "session",
        "idempotency": "forbidden" if _anonymous else "required",
        "max_body_bytes": 4 * 1024 * 1024,
        "accepted_content_types": ["application/json"],
        "tenant_non_disclosure": True,
    }


def protected_media_local_adapters() -> dict[tuple[str, str, str], dict[str, Any]]:
    fixture = json.loads(PROTECTED_MEDIA_FIXTURE.read_text(encoding="utf-8"))
    operations = fixture.get("operations", [])
    if (
        fixture.get("schema_version")
        != "frame.api-parity.legacy-protected-media.v1"
        or fixture.get("reference", {}).get("commit") != REFERENCE_COMMIT
        or fixture.get("summary")
        != {
            "hardware_and_provider": 25,
            "hardware_only": 16,
            "local_terminal_behavior": "fail_closed_unavailable",
            "operation_count": 41,
        }
        or len(operations) != 41
        or len({item.get("id") for item in operations if isinstance(item, dict)})
        != 41
    ):
        raise RuntimeError("protected-media fixture identity drifted")

    adapters: dict[tuple[str, str, str], dict[str, Any]] = {}
    for item in operations:
        sources = item.get("source_manifest", [])
        gates = tuple(item.get("protected_gates", []))
        if (
            not sources
            or "hardware_execution" not in gates
            or any(gate not in PROTECTED_COMPLETION_GATES for gate in gates)
            or item.get("production_behavior") != "fail_closed_unavailable"
        ):
            raise RuntimeError(f"protected-media contract drifted: {item.get('id')}")
        primary = sources[0]
        identity = (item["kind"], item["method"], item["legacy_path"])
        adapters[identity] = storage_local_adapter(
            item["id"],
            primary["path"],
            primary["sha256"],
            "rust_exact_protected_media_execution_staging_local_contract",
            "source-pinned authority, atomic immutable execution staging, and independently verified hardware/provider evidence",
            item["auth"],
            item["rate_limit_bucket"],
            item["production_behavior"],
            gates,
            tuple(sources[1:]),
        )
        OPERATION_CONTRACT_OVERRIDES[item["id"]] = {
            "auth": item["auth"],
            "idempotency": item["idempotency"],
            "max_body_bytes": item["max_body_bytes"],
            "accepted_content_types": item["accepted_content_types"],
            "tenant_non_disclosure": item["tenant_non_disclosure"],
        }
    if len(adapters) != 41:
        raise RuntimeError("protected-media fixture has duplicate identities")
    return adapters


PROTECTED_MEDIA_LOCAL_ENDPOINT_ADAPTERS = protected_media_local_adapters()


def protected_integration_policy(path: str) -> str:
    if (
        path.startswith("/api/desktop/s3/")
        or path.startswith("/api/desktop/storage/")
        or "send-download-link" in path
    ):
        return "upload_storage.v1"
    if "loom" in path.lower():
        return "imports_integrations.v1"
    if (
        "organization" in path.lower()
        or "organisation" in path.lower()
        or "/api/mobile/user/active-organization" == path
        or "createSpace" in path
        or "updateSpace" in path
    ):
        return "organization_library.v1"
    if (
        path.startswith("/api/webhooks/media-server/")
        or path == "/api/desktop/video/create"
        or "getVideoStatus" in path
    ):
        return "video_media.v1"
    if path.startswith("/api/releases/tauri/"):
        return "service_misc.v1"
    return "client_compatibility.v1"


def protected_integration_profile_sources() -> dict[str, dict[str, str]]:
    application = APPLICATION_PROTECTED_INTEGRATIONS.read_text(encoding="utf-8")
    pattern = re.compile(
        r'profile!\(\s*"(?P<id>cap-v1-[0-9a-f]{16})"(?P<body>.*?)'
        r'\n\s*"(?P<path>[^"]+)",\s*\n\s*"(?P<symbol>[^"]+)",\s*'
        r'\n\s*"(?P<sha256>[0-9a-f]{64})"\s*\n\s*\)',
        re.DOTALL,
    )
    sources = {
        match.group("id"): {
            "path": match.group("path"),
            "symbol": match.group("symbol"),
            "sha256": match.group("sha256"),
        }
        for match in pattern.finditer(application)
    }
    if len(sources) != 45:
        raise RuntimeError("protected-integration application profile inventory drifted")
    return sources


def protected_integration_local_adapters() -> dict[tuple[str, str, str], dict[str, Any]]:
    fixture = json.loads(PROTECTED_INTEGRATIONS_FIXTURE.read_text(encoding="utf-8"))
    operations = fixture.get("operations", [])
    sources = protected_integration_profile_sources()
    operation_ids = {
        item.get("id") for item in operations if isinstance(item, dict)
    }
    if (
        fixture.get("schema_version") != "frame.legacy-protected-integrations.v1"
        or fixture.get("reference", {}).get("commit") != REFERENCE_COMMIT
        or fixture.get("operation_count") != 45
        or fixture.get("protected_gates") != ["provider_execution"]
        or len(operations) != 45
        or len(operation_ids) != 45
        or operation_ids != set(sources)
    ):
        raise RuntimeError("protected-integration fixture identity drifted")

    adapters: dict[tuple[str, str, str], dict[str, Any]] = {}
    for item in operations:
        source = sources[item["id"]]
        identity = (item["carrier"], item["method"], item["path"])
        adapters[identity] = storage_local_adapter(
            item["id"],
            source["path"],
            source["sha256"],
            "rust_exact_protected_integration_provider_staging_local_contract",
            f"{item['authority']} D1 authority plus vault-bound digest-only {item['provider']} intent and immutable sealed-terminal evidence",
            item["auth"],
            protected_integration_policy(item["path"]),
            "fail_closed_unavailable",
            ("provider_execution",),
        )
        # Protected integrations preserve several source-public, API-key,
        # signed-state, and parent-receipt branches that the report's broad
        # path heuristics cannot infer. Keep regeneration fixture-driven.
        OPERATION_CONTRACT_OVERRIDES[item["id"]] = {
            "auth": item["auth"],
            "idempotency": item["idempotency"],
            "tenant_non_disclosure": item["authority"] != "public",
        }
    if len(adapters) != 45:
        raise RuntimeError("protected-integration fixture has duplicate identities")
    return adapters


PROTECTED_INTEGRATION_LOCAL_ENDPOINT_ADAPTERS = protected_integration_local_adapters()


def protected_billing_auth_local_adapters() -> dict[tuple[str, str, str], dict[str, Any]]:
    fixture = json.loads(PROTECTED_BILLING_AUTH_FIXTURE.read_text(encoding="utf-8"))
    operations = fixture.get("operations", [])
    summary = fixture.get("summary", {})
    if (
        fixture.get("schema_version") != 1
        or fixture.get("reference", {}).get("commit") != REFERENCE_COMMIT
        or summary
        != {
            "operation_count": 17,
            "human_and_provider": 14,
            "provider_only": 2,
            "local_exact": 1,
            "local_terminal_behavior": (
                "sixteen_fail_closed_plus_credentialed_cors_preflight"
            ),
        }
        or len(operations) != 17
        or len({item.get("id") for item in operations if isinstance(item, dict)})
        != 17
    ):
        raise RuntimeError("protected billing/auth fixture identity drifted")

    adapters: dict[tuple[str, str, str], dict[str, Any]] = {}
    for item in operations:
        sources = item.get("source_manifest", [])
        gates = tuple(item.get("protected_gates", []))
        if (
            not sources
            or gates
            not in {
                (),
                ("provider_execution",),
                ("human_approval", "provider_execution"),
            }
        ):
            raise RuntimeError(f"protected billing/auth contract drifted: {item.get('id')}")
        primary = sources[0]
        identity = (item["kind"], item["method"], item["path"])
        adapters[identity] = storage_local_adapter(
            item["id"],
            primary["path"],
            primary["sha256"],
            "rust_exact_protected_billing_auth_staging_local_contract",
            (
                f"{item['authority']} authority plus redacted {item['provider']} intent "
                "and independently admitted human/provider evidence"
                if gates
                else f"{item['authority']} authority plus exact {item['provider']} behavior"
            ),
            item["auth"],
            item["rate_limit_bucket"],
            "fail_closed_unavailable" if gates else "serve_exact_static",
            gates,
            tuple(sources[1:]),
        )
        OPERATION_CONTRACT_OVERRIDES[item["id"]] = {
            "auth": item["auth"],
            "idempotency": item["idempotency"],
            "max_body_bytes": item["max_body_bytes"],
            "accepted_content_types": item["accepted_content_types"],
            "tenant_non_disclosure": item["authority"] not in {"public_flow"},
        }
    if len(adapters) != 17:
        raise RuntimeError("protected billing/auth fixture has duplicate identities")
    return adapters


PROTECTED_BILLING_AUTH_LOCAL_ENDPOINT_ADAPTERS = protected_billing_auth_local_adapters()


def developer_api_local_adapter(
    operation_id: str,
    source_path: str,
    source_sha256: str,
    extra_sources: tuple[dict[str, str], ...],
    authority: str,
    auth: str,
    policy: str,
) -> dict[str, Any]:
    return {
        "id": operation_id,
        "source_path": source_path,
        "source_sha256": source_sha256,
        "extra_sources": extra_sources,
        "local_status": "rust_exact_developer_api_d1_r2_local_contract",
        "rust_authority": authority,
        "auth": auth,
        "policy": policy,
        "completion": {
            "decision": "serve_frame_exact_business",
            "local_work": "complete",
            "protected_gates": [],
            "retirement_decision": "not_proposed",
            "production_behavior": "serve_exact_d1",
        },
    }


DEVELOPER_API_LOCAL_ENDPOINT_ADAPTERS = {
    ("route", "GET", "/api/cron/developer-storage"): developer_api_local_adapter(
        "cap-v1-0f178cf038854d4a",
        "apps/web/app/api/cron/developer-storage/route.ts",
        "362c91fcda48e52ff3287a7ac4a53ffd32f59613169ceb021a2d5d8907293fe8",
        DEVELOPER_STORAGE_CRON_RUNTIME_SOURCES,
        "constant-time scheduler secret plus atomic D1 daily snapshot and credit ledger",
        "scheduler_secret",
        "upload_storage.v1",
    ),
    ("route", "POST", "/api/developer/sdk/v1/upload/multipart/abort"): developer_api_local_adapter(
        "cap-v1-5914aa6459d24ff1",
        "apps/web/app/api/developer/sdk/v1/[...route]/upload.ts",
        "0beb3b5366236ba86540500fa854c5a95f29a7a624abbdb838f3b3715cdd83e0",
        DEVELOPER_MULTIPART_RUNTIME_SOURCES,
        "public developer key and production Origin plus resumable D1 outbox and R2 multipart abort",
        "developer_api_key",
        "upload_storage.v1",
    ),
    ("route", "POST", "/api/developer/sdk/v1/upload/multipart/complete"): developer_api_local_adapter(
        "cap-v1-5c98b9755e4643ba",
        "apps/web/app/api/developer/sdk/v1/[...route]/upload.ts",
        "0beb3b5366236ba86540500fa854c5a95f29a7a624abbdb838f3b3715cdd83e0",
        DEVELOPER_MULTIPART_RUNTIME_SOURCES,
        "public developer key and production Origin plus atomic credit ledger and resumable R2 completion",
        "developer_api_key",
        "upload_storage.v1",
    ),
    ("route", "POST", "/api/developer/sdk/v1/upload/multipart/initiate"): developer_api_local_adapter(
        "cap-v1-0d3940728bc19e0e",
        "apps/web/app/api/developer/sdk/v1/[...route]/upload.ts",
        "0beb3b5366236ba86540500fa854c5a95f29a7a624abbdb838f3b3715cdd83e0",
        DEVELOPER_MULTIPART_RUNTIME_SOURCES,
        "public developer key and production Origin plus D1 provider outbox and R2 multipart creation",
        "developer_api_key",
        "upload_storage.v1",
    ),
    ("route", "POST", "/api/developer/sdk/v1/upload/multipart/presign-part"): developer_api_local_adapter(
        "cap-v1-b6fe5aec600a2e1a",
        "apps/web/app/api/developer/sdk/v1/[...route]/upload.ts",
        "0beb3b5366236ba86540500fa854c5a95f29a7a624abbdb838f3b3715cdd83e0",
        DEVELOPER_MULTIPART_RUNTIME_SOURCES,
        "public developer key and production Origin plus D1 capability receipt and header-free R2 SigV4 URL",
        "developer_api_key",
        "upload_storage.v1",
    ),
    ("route", "POST", "/api/developer/sdk/v1/videos/create"): developer_api_local_adapter(
        "cap-v1-c904ef9c11983a40",
        "apps/web/app/api/developer/sdk/v1/[...route]/video-create.ts",
        "0b79fae22402cc26b5f13b6b185ec74f832f5bf587dc079c2b76b09bfe16405d",
        DEVELOPER_VIDEO_CREATE_RUNTIME_SOURCES,
        "public developer key and production Origin plus balance fence and atomic D1 video receipt",
        "developer_api_key",
        "developer_api.v1",
    ),
    ("route", "GET", "/api/developer/v1/usage"): developer_api_local_adapter(
        "cap-v1-cbf22d62a64d3486",
        "apps/web/app/api/developer/v1/[...route]/usage.ts",
        "e5e9962180456949598932306c50f664a873575e758e9d2fbf8a55c3d277a828",
        DEVELOPER_USAGE_RUNTIME_SOURCES,
        "secret developer key plus app-scoped D1 balance and live-video aggregates",
        "developer_api_key",
        "developer_api.v1",
    ),
    ("route", "GET", "/api/developer/v1/videos"): developer_api_local_adapter(
        "cap-v1-6e2296f9695261a3",
        "apps/web/app/api/developer/v1/[...route]/videos.ts",
        "d89f31167fd69f1955dfa6ec52c0449aabdccb08ebe4e8662a2075b0405514f9",
        DEVELOPER_VIDEOS_RUNTIME_SOURCES,
        "secret developer key plus app-scoped D1 filter pagination and source ordering",
        "developer_api_key",
        "developer_api.v1",
    ),
    ("route", "DELETE", "/api/developer/v1/videos/:id"): developer_api_local_adapter(
        "cap-v1-1cbfe3ecac36f198",
        "apps/web/app/api/developer/v1/[...route]/videos.ts",
        "d89f31167fd69f1955dfa6ec52c0449aabdccb08ebe4e8662a2075b0405514f9",
        DEVELOPER_VIDEOS_RUNTIME_SOURCES,
        "secret developer key plus app-scoped atomic D1 tombstone and immutable replay receipt",
        "developer_api_key",
        "developer_api.v1",
    ),
    ("route", "GET", "/api/developer/v1/videos/:id"): developer_api_local_adapter(
        "cap-v1-aed411f91e977fe5",
        "apps/web/app/api/developer/v1/[...route]/videos.ts",
        "d89f31167fd69f1955dfa6ec52c0449aabdccb08ebe4e8662a2075b0405514f9",
        DEVELOPER_VIDEOS_RUNTIME_SOURCES,
        "secret developer key plus app-scoped non-disclosing D1 video projection",
        "developer_api_key",
        "developer_api.v1",
    ),
    ("route", "GET", "/api/developer/v1/videos/:id/status"): developer_api_local_adapter(
        "cap-v1-718e84b39180c0ac",
        "apps/web/app/api/developer/v1/[...route]/videos.ts",
        "d89f31167fd69f1955dfa6ec52c0449aabdccb08ebe4e8662a2075b0405514f9",
        DEVELOPER_VIDEOS_RUNTIME_SOURCES,
        "secret developer key plus app-scoped D1 readiness and transcription projection",
        "developer_api_key",
        "developer_api.v1",
    ),
}

LOCAL_ENDPOINT_ADAPTERS: dict[tuple[str, str, str], dict[str, Any]] = {
    **DEVELOPER_API_LOCAL_ENDPOINT_ADAPTERS,
    **VIDEO_LIFECYCLE_LOCAL_ENDPOINT_ADAPTERS,
    **CORE_STORAGE_LOCAL_ENDPOINT_ADAPTERS,
    **UPLOAD_STORAGE_LOCAL_ENDPOINT_ADAPTERS,
    **ANALYTICS_LOCAL_ENDPOINT_ADAPTERS,
    **ORGANIZATION_LIBRARY_LOCAL_ENDPOINT_ADAPTERS,
    **PROTECTED_MEDIA_LOCAL_ENDPOINT_ADAPTERS,
    **PROTECTED_INTEGRATION_LOCAL_ENDPOINT_ADAPTERS,
    **PROTECTED_BILLING_AUTH_LOCAL_ENDPOINT_ADAPTERS,
    ("route", "POST", "/api/mobile/uploads"): {
        "id": "cap-v1-b0116dd82b010477",
        "source_path": "apps/web/app/api/mobile/[...route]/route.ts",
        "source_sha256": "02df2ce92dc6e8ae11748b6e082c1304596ba9e4c370b35069867754218f5f79",
        "extra_sources": MOBILE_UPLOAD_CREATE_RUNTIME_SOURCES,
        "local_status": "rust_exact_mobile_upload_create_d1_r2_local_contract",
        "rust_authority": "frame-application exact released mobile create contract + control-plane atomic D1 alias/native upload and exact R2 PUT capability",
        "auth": "session_or_api_key",
        "policy": "upload_storage.v1",
        "completion": {
            "decision": "serve_frame_exact_business",
            "local_work": "complete",
            "protected_gates": [],
            "retirement_decision": "not_proposed",
            "production_behavior": "serve_exact_d1_r2",
        },
    },
    ("route", "POST", "/api/mobile/uploads/:id/complete"): {
        "id": "cap-v1-b43b6ede64a73798",
        "source_path": "apps/web/app/api/mobile/[...route]/route.ts",
        "source_sha256": "02df2ce92dc6e8ae11748b6e082c1304596ba9e4c370b35069867754218f5f79",
        "extra_sources": MOBILE_UPLOAD_COMPLETE_RUNTIME_SOURCES,
        "local_status": "rust_exact_mobile_upload_complete_d1_r2_provider_intent_local_contract",
        "rust_authority": "frame-application exact released mobile completion contract + control-plane exact R2 evidence and immutable D1 provider intent",
        "auth": "session_or_api_key",
        "policy": "upload_storage.v1",
        "completion": {
            "decision": "retain_replace_with_provider_effect",
            "local_work": "complete",
            "protected_gates": ["provider_execution"],
            "retirement_decision": "not_proposed",
            "production_behavior": "fail_closed_unavailable",
        },
    },
    ("route", "POST", "/api/mobile/uploads/:id/progress"): {
        "id": "cap-v1-62469fe03e030052",
        "source_path": "apps/web/app/api/mobile/[...route]/route.ts",
        "source_sha256": "02df2ce92dc6e8ae11748b6e082c1304596ba9e4c370b35069867754218f5f79",
        "extra_sources": MOBILE_UPLOAD_PROGRESS_RUNTIME_SOURCES,
        "local_status": "rust_exact_mobile_upload_progress_d1_local_contract",
        "rust_authority": "frame-application exact safe-integer progress normalization + control-plane owner-scoped atomic D1 native/legacy projection",
        "auth": "session_or_api_key",
        "policy": "upload_storage.v1",
        "completion": {
            "decision": "serve_frame_exact_business",
            "local_work": "complete",
            "protected_gates": [],
            "retirement_decision": "not_proposed",
            "production_behavior": "serve_exact_d1",
        },
    },
    ("route", "POST", "/api/videos/:videoId/retry-transcription"): {
        "id": "cap-v1-c8dffb9b102dd4f7",
        "source_path": "apps/web/app/api/videos/[videoId]/retry-transcription/route.ts",
        "source_sha256": "a443e83d8bbf243e1661cfaf9795c1d901e650c7f773493aad885e9c90ab54e3",
        "extra_sources": TRANSCRIPT_RETRY_RUNTIME_SOURCES,
        "local_status": "rust_exact_transcription_retry_d1_local_contract",
        "rust_authority": "frame-application exact retry receipt + control-plane owner-scoped D1 status reset",
        "auth": "session",
        "policy": "collaboration_notifications.v1",
        "completion": {
            "decision": "serve_frame_exact_business",
            "local_work": "complete",
            "protected_gates": [],
            "retirement_decision": "not_proposed",
            "production_behavior": "serve_exact_d1",
        },
    },
    (
        "server_action",
        "ACTION",
        "action://apps/web/actions/videos/edit-transcript.ts#editTranscriptEntry",
    ): {
        "id": "cap-v1-3db394ae13895b46",
        "source_path": "apps/web/actions/videos/edit-transcript.ts",
        "source_sha256": "f9e1ae7841e79c58c98fd5087f375f8d5d6c0c0978d6eff40e477ed4c6266da9",
        "extra_sources": TRANSCRIPT_EDIT_RUNTIME_SOURCES,
        "local_status": "rust_exact_transcript_edit_d1_r2_local_contract",
        "rust_authority": "frame-application exact WebVTT edit + control-plane owner-scoped D1 replay and private R2 ingress",
        "auth": "session",
        "policy": "collaboration_notifications.v1",
        "completion": {
            "decision": "serve_frame_exact_business",
            "local_work": "complete",
            "protected_gates": [],
            "retirement_decision": "not_proposed",
            "production_behavior": "serve_exact_d1_r2",
        },
    },
    (
        "server_action",
        "ACTION",
        "action://apps/web/actions/videos/get-transcript.ts#getTranscript",
    ): {
        "id": "cap-v1-f2659b43d5ee9162",
        "source_path": "apps/web/actions/videos/get-transcript.ts",
        "source_sha256": "7edf7bbf932c1ca32053e8047a71ac988fc49cede88e2aa4793762d6b0302adb",
        "extra_sources": TRANSCRIPT_GET_RUNTIME_SOURCES,
        "local_status": "rust_exact_transcript_read_d1_r2_local_contract",
        "rust_authority": "frame-application exact transcript result + control-plane optional-auth public policy and private R2 ingress",
        "auth": "optional_session_or_share_capability",
        "policy": "collaboration_notifications.v1",
        "completion": {
            "decision": "serve_frame_exact_business",
            "local_work": "complete",
            "protected_gates": [],
            "retirement_decision": "not_proposed",
            "production_behavior": "serve_exact_d1_r2",
        },
    },
    (
        "server_action",
        "ACTION",
        "action://apps/web/actions/videos/translate-transcript.ts#translateTranscript",
    ): {
        "id": "cap-v1-6f6ece85bd786289",
        "source_path": "apps/web/actions/videos/translate-transcript.ts",
        "source_sha256": "8a58b5af8366fb212b64bbb68233477938675248b9f19b4a359a5feabb1b73c1",
        "extra_sources": TRANSCRIPT_TRANSLATE_RUNTIME_SOURCES,
        "local_status": "rust_exact_transcript_translation_d1_r2_provider_outbox_local_contract",
        "rust_authority": "frame-application exact language/cache contract + control-plane optional-auth public policy, D1 outbox, and private R2 ingress",
        "auth": "optional_session_or_share_capability",
        "policy": "collaboration_notifications.v1",
        "completion": {
            "decision": "retain_replace_with_provider_effect",
            "local_work": "complete",
            "protected_gates": ["provider_execution"],
            "retirement_decision": "not_proposed",
            "production_behavior": "fail_closed_unavailable",
        },
    },
    (
        "server_action",
        "ACTION",
        "action://apps/web/actions/videos/get-available-translations.ts#getAvailableTranslations",
    ): {
        "id": "cap-v1-6c82f3cbe383d92b",
        "source_path": "apps/web/actions/videos/get-available-translations.ts",
        "source_sha256": "4aa195266716146cc5c87dbc39a4d30f27ed32747b1474e990e231bd9f81a921",
        "extra_sources": TRANSCRIPT_AVAILABLE_RUNTIME_SOURCES,
        "local_status": "rust_exact_available_translations_d1_r2_local_contract",
        "rust_authority": "frame-application exact ordered translation catalog + control-plane optional-auth public policy and bounded private R2 listing",
        "auth": "optional_session_or_share_capability",
        "policy": "video_media.v1",
        "completion": {
            "decision": "serve_frame_exact_business",
            "local_work": "complete",
            "protected_gates": [],
            "retirement_decision": "not_proposed",
            "production_behavior": "serve_exact_d1_r2",
        },
    },
    ("route", "POST", "/api/invite/accept"): {
        "id": "cap-v1-447e3212d20351f6",
        "source_path": "apps/web/app/api/invite/accept/route.ts",
        "source_sha256": "e45eaf177c0608bc6cbfe41792da56fbb3397d0a43c2c85ae532d6240876f790",
        "extra_sources": INVITE_ACCEPT_RUNTIME_SOURCES,
        "local_status": "rust_exact_invite_accept_d1_adapter_local_contract",
        "rust_authority": "frame-application exact invite lifecycle + control-plane atomic D1 route ingress",
        "auth": "session",
        "policy": "auth_session.v1",
        "completion": {
            "decision": "serve_frame_exact_business",
            "local_work": "complete",
            "protected_gates": [],
            "retirement_decision": "not_proposed",
            "production_behavior": "serve_exact_d1",
        },
    },
    ("route", "POST", "/api/invite/decline"): {
        "id": "cap-v1-cddad884de1190b1",
        "source_path": "apps/web/app/api/invite/decline/route.ts",
        "source_sha256": "df4e61e983c8691e359d5adb053b4c342ac2bd184f937740214c5ab345ef9c3e",
        "extra_sources": INVITE_DECLINE_RUNTIME_SOURCES,
        "local_status": "rust_exact_invite_decline_d1_adapter_local_contract",
        "rust_authority": "frame-application exact invite lifecycle + control-plane atomic D1 route ingress",
        "auth": "session",
        "policy": "auth_session.v1",
        "completion": {
            "decision": "serve_frame_exact_business",
            "local_work": "complete",
            "protected_gates": [],
            "retirement_decision": "not_proposed",
            "production_behavior": "serve_exact_d1",
        },
    },
    ("route", "GET", "/api/extension/auth/start"): {
        "id": "cap-v1-249fbd2f77ee7209",
        "source_path": "packages/web-domain/src/Extension.ts",
        "source_sha256": "d1bc68b7e302bc098d16c17bd991fe942a7361ffa88675574ce45980395582ba",
        "extra_sources": EXTENSION_AUTH_RUNTIME_SOURCES,
        "local_status": "rust_exact_extension_auth_start_local_contract",
        "rust_authority": "frame-application source-pinned extension consent contract + control-plane side-effect-free HTTP ingress",
        "auth": "public_or_flow_token",
        "policy": "auth_session.v1",
        "completion": {
            "decision": "serve_frame_exact_business",
            "local_work": "complete",
            "protected_gates": [],
            "retirement_decision": "not_proposed",
            "production_behavior": "serve_exact_d1",
        },
    },
    ("route", "POST", "/api/extension/auth/approve"): {
        "id": "cap-v1-96499b6c8e845b35",
        "source_path": "packages/web-domain/src/Extension.ts",
        "source_sha256": "d1bc68b7e302bc098d16c17bd991fe942a7361ffa88675574ce45980395582ba",
        "extra_sources": EXTENSION_AUTH_RUNTIME_SOURCES,
        "local_status": "rust_exact_extension_auth_approve_d1_local_contract",
        "rust_authority": "frame-application source-pinned extension approval contract + control-plane atomic digest-only D1 mint",
        "auth": "public_or_flow_token",
        "policy": "auth_session.v1",
        "completion": {
            "decision": "serve_frame_exact_business",
            "local_work": "complete",
            "protected_gates": [],
            "retirement_decision": "not_proposed",
            "production_behavior": "serve_exact_d1",
        },
    },
    ("route", "POST", "/api/extension/auth/revoke"): {
        "id": "cap-v1-ed715d4d23e82181",
        "source_path": "packages/web-domain/src/Extension.ts",
        "source_sha256": "d1bc68b7e302bc098d16c17bd991fe942a7361ffa88675574ce45980395582ba",
        "extra_sources": EXTENSION_AUTH_RUNTIME_SOURCES,
        "local_status": "rust_exact_extension_auth_revoke_d1_local_contract",
        "rust_authority": "frame-application source-pinned extension middleware contract + control-plane actor-owned D1 key deletion",
        "auth": "session_or_api_key",
        "policy": "auth_session.v1",
        "completion": {
            "decision": "serve_frame_exact_business",
            "local_work": "complete",
            "protected_gates": [],
            "retirement_decision": "not_proposed",
            "production_behavior": "serve_exact_d1",
        },
    },
    ("route", "GET", "/api/extension/bootstrap"): {
        "id": "cap-v1-12159b1acbaeba7a",
        "source_path": "packages/web-domain/src/Extension.ts",
        "source_sha256": "d1bc68b7e302bc098d16c17bd991fe942a7361ffa88675574ce45980395582ba",
        "extra_sources": EXTENSION_AUTH_RUNTIME_SOURCES,
        "local_status": "rust_exact_extension_bootstrap_d1_local_contract",
        "rust_authority": "frame-application source-pinned extension bootstrap contract + control-plane deterministic D1 organization repair",
        "auth": "session_or_api_key",
        "policy": "client_compatibility.v1",
        "completion": {
            "decision": "serve_frame_exact_business",
            "local_work": "complete",
            "protected_gates": [],
            "retirement_decision": "not_proposed",
            "production_behavior": "serve_exact_d1",
        },
    },
    ("route", "POST", "/api/extension/instant-recordings"): {
        "id": "cap-v1-00422c50f4d39053",
        "source_path": "packages/web-domain/src/Extension.ts",
        "source_sha256": "d1bc68b7e302bc098d16c17bd991fe942a7361ffa88675574ce45980395582ba",
        "extra_sources": EXTENSION_INSTANT_RUNTIME_SOURCES,
        "local_status": "rust_exact_extension_instant_create_d1_r2_local_contract",
        "rust_authority": "frame-application source-pinned instant-recording contract + control-plane atomic D1 alias/native write and scoped R2 PUT",
        "auth": "session_or_api_key",
        "policy": "video_media.v1",
        "completion": {
            "decision": "serve_frame_exact_business",
            "local_work": "complete",
            "protected_gates": [],
            "retirement_decision": "not_proposed",
            "production_behavior": "serve_exact_d1",
        },
    },
    ("route", "POST", "/api/extension/instant-recordings/progress"): {
        "id": "cap-v1-82dec55d0fbea3db",
        "source_path": "packages/web-domain/src/Extension.ts",
        "source_sha256": "d1bc68b7e302bc098d16c17bd991fe942a7361ffa88675574ce45980395582ba",
        "extra_sources": EXTENSION_INSTANT_RUNTIME_SOURCES,
        "local_status": "rust_exact_extension_instant_progress_d1_local_contract",
        "rust_authority": "frame-application source-pinned progress contract + control-plane owner-scoped monotonic D1 upload projection",
        "auth": "session_or_api_key",
        "policy": "video_media.v1",
        "completion": {
            "decision": "serve_frame_exact_business",
            "local_work": "complete",
            "protected_gates": [],
            "retirement_decision": "not_proposed",
            "production_behavior": "serve_exact_d1",
        },
    },
    ("route", "DELETE", "/api/extension/instant-recordings/:videoId"): {
        "id": "cap-v1-8fd4741d6e52465e",
        "source_path": "packages/web-domain/src/Extension.ts",
        "source_sha256": "d1bc68b7e302bc098d16c17bd991fe942a7361ffa88675574ce45980395582ba",
        "extra_sources": EXTENSION_INSTANT_RUNTIME_SOURCES,
        "local_status": "rust_exact_extension_instant_delete_d1_r2_local_contract",
        "rust_authority": "frame-application source-pinned delete contract + control-plane durable tombstone and strongly-consistent R2 prefix cleanup",
        "auth": "session_or_api_key",
        "policy": "video_media.v1",
        "completion": {
            "decision": "serve_frame_exact_business",
            "local_work": "complete",
            "protected_gates": [],
            "retirement_decision": "not_proposed",
            "production_behavior": "serve_exact_d1",
        },
    },
    ("route", "GET", "/api/status"): {
        "id": "cap-v1-05b6ba3f76daac22",
        "source_path": "apps/web/app/api/status/route.ts",
        "source_sha256": "ba3eb1177da489a10f74c9dbc68e0db8324b695c82499e35d6f8d9da8aaf5797",
        "local_status": "rust_exact_status_adapter_local_success_contract",
        "rust_authority": "frame-control-plane legacy status semantic adapter",
        "auth": "public_or_flow_token",
        "policy": "service_misc.v1",
    },
    ("route", "GET", "/media-server"): {
        "id": "cap-v1-ff19008f47194c43",
        "source_path": "apps/media-server/src/app.ts",
        "source_sha256": "b3ba5fc1c8e93bd6896aa4399c283cc33a73e7777275816a11334fd71b75fc57",
        "local_status": "rust_exact_media_server_root_adapter_local_contract",
        "rust_authority": "frame-control-plane legacy media-server root semantic adapter",
        "auth": "public_or_flow_token",
        "policy": "service_misc.v1",
    },
    ("route", "GET", "/api/changelog/status"): {
        "id": "cap-v1-a1b180c5d123c870",
        "source_path": "apps/web/app/api/changelog/status/route.ts",
        "source_sha256": "c2a3c107fce46765286e5a5e14fc3b21959e22b50070ecdc45f3d3d16ea5541b",
        "extra_sources": (
            {
                "path": "apps/web/utils/changelog.ts",
                "symbol": "getChangelogPosts",
                "sha256": "30e6361fb869f87654cdfdb6b5d7f1533d86359ea1820efc1818b4a517759141",
            },
            {
                "path": "apps/web/content/changelog/99.mdx",
                "symbol": "latest-version:0.5.6;max-numeric-slug:99",
                "sha256": "e67f4f451c30e040bbffb70b9cbbb0e107e1aa2723b629c878c6ceeaef7e567e",
            },
        ),
        "local_status": "rust_exact_changelog_status_adapter_local_contract",
        "rust_authority": "frame-control-plane legacy changelog status semantic adapter",
        "auth": "public_or_flow_token",
        "policy": "client_compatibility.v1",
    },
    ("route", "OPTIONS", "/api/changelog/status"): {
        "id": "cap-v1-16668b858461f386",
        "source_path": "apps/web/app/api/changelog/status/route.ts",
        "source_sha256": "c2a3c107fce46765286e5a5e14fc3b21959e22b50070ecdc45f3d3d16ea5541b",
        "local_status": "rust_exact_changelog_status_cors_adapter_local_contract",
        "rust_authority": "frame-control-plane legacy changelog CORS semantic adapter",
        "auth": "public_or_flow_token",
        "policy": "service_misc.v1",
    },
    ("route", "GET", "/api/changelog"): {
        "id": "cap-v1-0fa8384f3666825b",
        "source_path": "apps/web/app/api/changelog/route.ts",
        "source_sha256": "b47371ce19a03def1b675996615e1c48af41651bc48ca479d0a97bd9e7167b04",
        "extra_sources": (
            {
                "path": "apps/web/utils/changelog.ts",
                "symbol": "getChangelogPosts",
                "sha256": "30e6361fb869f87654cdfdb6b5d7f1533d86359ea1820efc1818b4a517759141",
            },
            {
                "path": "apps/web/utils/cors.ts",
                "symbol": "getCorsHeaders",
                "sha256": "fff2797f5845e2fcd6c2941c166a91d616af547cba95bf74291706e541f32edc",
            },
        ),
        "local_status": "rust_exact_changelog_feed_adapter_local_contract",
        "rust_authority": "frame-control-plane legacy changelog feed semantic adapter",
        "auth": "public_or_flow_token",
        "policy": "client_compatibility.v1",
    },
    ("route", "OPTIONS", "/api/changelog"): {
        "id": "cap-v1-237f41f3086a2d67",
        "source_path": "apps/web/app/api/changelog/route.ts",
        "source_sha256": "b47371ce19a03def1b675996615e1c48af41651bc48ca479d0a97bd9e7167b04",
        "extra_sources": (
            {
                "path": "apps/web/utils/cors.ts",
                "symbol": "getCorsHeaders",
                "sha256": "fff2797f5845e2fcd6c2941c166a91d616af547cba95bf74291706e541f32edc",
            },
        ),
        "local_status": "rust_exact_changelog_feed_cors_adapter_local_contract",
        "rust_authority": "frame-control-plane legacy changelog feed CORS semantic adapter",
        "auth": "public_or_flow_token",
        "policy": "service_misc.v1",
    },
    ("route", "GET", "/api/mobile/session/config"): {
        "id": "cap-v1-4f21920a947c4c84",
        "source_path": "apps/web/app/api/mobile/[...route]/route.ts",
        "source_sha256": "02df2ce92dc6e8ae11748b6e082c1304596ba9e4c370b35069867754218f5f79",
        "extra_sources": (
            {
                "path": "packages/web-domain/src/Mobile.ts",
                "symbol": "getAuthConfig",
                "sha256": "331d76900372d62389d729f8682baca1344f3583e3f41f42ad6e3ef2be7a3d5b",
            },
        ),
        "local_status": "rust_exact_mobile_session_config_adapter_local_contract",
        "rust_authority": "frame-control-plane legacy mobile session config semantic adapter",
        "auth": "public_or_flow_token",
        "policy": "client_compatibility.v1",
    },
    ("route", "POST", "/api/mobile/session/email/request"): {
        "id": "cap-v1-e16563e40f697519",
        "source_path": "apps/web/app/api/mobile/[...route]/route.ts",
        "source_sha256": "02df2ce92dc6e8ae11748b6e082c1304596ba9e4c370b35069867754218f5f79",
        "extra_sources": MOBILE_EMAIL_REQUEST_RUNTIME_SOURCES,
        "local_status": "rust_exact_mobile_email_session_request_d1_outbox_local_contract",
        "rust_authority": "frame-application exact mobile normalization and signup policy + control-plane encrypted delivery outbox and D1 challenge replacement",
        "auth": "public_or_flow_token",
        "policy": "auth_session.v1",
        "completion": {
            "decision": "retain_replace_with_provider_effect",
            "local_work": "complete",
            "protected_gates": ["provider_execution"],
            "retirement_decision": "not_proposed",
            "production_behavior": "fail_closed_unavailable",
        },
    },
    ("route", "POST", "/api/mobile/session/email/verify"): {
        "id": "cap-v1-139a189f8a00b38c",
        "source_path": "apps/web/app/api/mobile/[...route]/route.ts",
        "source_sha256": "02df2ce92dc6e8ae11748b6e082c1304596ba9e4c370b35069867754218f5f79",
        "extra_sources": MOBILE_EMAIL_VERIFY_RUNTIME_SOURCES,
        "local_status": "rust_exact_mobile_email_session_verify_d1_local_contract_provider_gated_new_user",
        "rust_authority": "frame-application exact one-use verification and adapter branch semantics + control-plane D1 user/key provisioning and truthful Stripe effect outbox",
        "auth": "public_or_flow_token",
        "policy": "auth_session.v1",
        "completion": {
            "decision": "retain_replace_with_provider_effect",
            "local_work": "complete",
            "protected_gates": ["provider_execution"],
            "retirement_decision": "not_proposed",
            "production_behavior": "fail_closed_unavailable",
        },
    },
    ("route", "GET", "/api/mobile/session/request"): {
        "id": "cap-v1-ea999fdc5829fbd1",
        "source_path": "apps/web/app/api/mobile/[...route]/route.ts",
        "source_sha256": "02df2ce92dc6e8ae11748b6e082c1304596ba9e4c370b35069867754218f5f79",
        "extra_sources": MOBILE_SESSION_REQUEST_RUNTIME_SOURCES,
        "local_status": "rust_exact_mobile_session_request_d1_local_contract",
        "rust_authority": "frame-application strict mobile redirect and login propagation semantics + control-plane optional browser session and replace-all digest-only key adapter",
        "auth": "optional_session_or_share_capability",
        "policy": "auth_session.v1",
        "completion": {
            "decision": "serve_frame_exact_business",
            "local_work": "complete",
            "protected_gates": [],
            "retirement_decision": "not_proposed",
            "production_behavior": "serve_exact_d1",
        },
    },
    ("route", "POST", "/api/mobile/session/revoke"): {
        "id": "cap-v1-1eef72e518a37abd",
        "source_path": "apps/web/app/api/mobile/[...route]/route.ts",
        "source_sha256": "02df2ce92dc6e8ae11748b6e082c1304596ba9e4c370b35069867754218f5f79",
        "extra_sources": MOBILE_SESSION_REVOKE_RUNTIME_SOURCES,
        "local_status": "rust_exact_mobile_session_revoke_d1_local_contract",
        "rust_authority": "frame-application exact middleware and bearer parser split + control-plane authenticated digest-only key deletion",
        "auth": "session_or_api_key",
        "policy": "auth_session.v1",
        "completion": {
            "decision": "serve_frame_exact_business",
            "local_work": "complete",
            "protected_gates": [],
            "retirement_decision": "not_proposed",
            "production_behavior": "serve_exact_d1",
        },
    },
    ("route", "GET", "/api/mobile/bootstrap"): {
        "id": "cap-v1-32a24fe16a4c4a4f",
        "source_path": "apps/web/app/api/mobile/[...route]/route.ts",
        "source_sha256": "02df2ce92dc6e8ae11748b6e082c1304596ba9e4c370b35069867754218f5f79",
        "extra_sources": MOBILE_BOOTSTRAP_RUNTIME_SOURCES,
        "local_status": "rust_exact_mobile_bootstrap_d1_r2_local_contract",
        "rust_authority": "frame-application source-pinned bootstrap projection + control-plane actor-scoped D1 and private R2 image capabilities",
        "auth": "session_or_api_key",
        "policy": "client_compatibility.v1",
        "completion": MOBILE_BOOTSTRAP_CAPS_COMPLETION,
    },
    ("route", "GET", "/api/mobile/caps"): {
        "id": "cap-v1-951ad1523ae9dff4",
        "source_path": "apps/web/app/api/mobile/[...route]/route.ts",
        "source_sha256": "02df2ce92dc6e8ae11748b6e082c1304596ba9e4c370b35069867754218f5f79",
        "extra_sources": MOBILE_CAPS_LIST_RUNTIME_SOURCES,
        "local_status": "rust_exact_mobile_caps_list_d1_r2_local_contract",
        "rust_authority": "frame-application exact pagination/summary semantics + control-plane owner-and-active-organization D1 projection and R2 thumbnails",
        "auth": "session_or_api_key",
        "policy": "client_compatibility.v1",
        "completion": MOBILE_BOOTSTRAP_CAPS_COMPLETION,
    },
    ("route", "DELETE", "/api/mobile/caps/:id"): {
        "id": "cap-v1-6b8a689bf00a9187",
        "source_path": "apps/web/app/api/mobile/[...route]/route.ts",
        "source_sha256": "02df2ce92dc6e8ae11748b6e082c1304596ba9e4c370b35069867754218f5f79",
        "extra_sources": MOBILE_CAP_DELETE_RUNTIME_SOURCES,
        "local_status": "rust_exact_mobile_cap_delete_d1_r2_local_contract",
        "rust_authority": "frame-application owner-hidden delete contract + control-plane atomic D1 tombstone journal followed by scoped R2 cleanup",
        "auth": "session_or_api_key",
        "policy": "client_compatibility.v1",
        "completion": MOBILE_BOOTSTRAP_CAPS_COMPLETION,
    },
    ("route", "GET", "/api/mobile/caps/:id"): {
        "id": "cap-v1-7f0ed5caf3eaf97c",
        "source_path": "apps/web/app/api/mobile/[...route]/route.ts",
        "source_sha256": "02df2ce92dc6e8ae11748b6e082c1304596ba9e4c370b35069867754218f5f79",
        "extra_sources": MOBILE_CAP_GET_RUNTIME_SOURCES,
        "local_status": "rust_exact_mobile_cap_detail_d1_r2_local_contract",
        "rust_authority": "frame-application exact metadata/detail wire contract + control-plane owner-scoped D1 comments and best-effort R2 images",
        "auth": "session_or_api_key",
        "policy": "client_compatibility.v1",
        "completion": MOBILE_BOOTSTRAP_CAPS_COMPLETION,
    },
    ("route", "GET", "/api/mobile/caps/:id/download"): {
        "id": "cap-v1-95fe41c72ce5ca9f",
        "source_path": "apps/web/app/api/mobile/[...route]/route.ts",
        "source_sha256": "02df2ce92dc6e8ae11748b6e082c1304596ba9e4c370b35069867754218f5f79",
        "extra_sources": MOBILE_CAP_DOWNLOAD_RUNTIME_SOURCES,
        "local_status": "rust_exact_mobile_cap_download_d1_r2_local_contract",
        "rust_authority": "frame-application exact screenshot/raw-MP4 selection + control-plane owner-hidden D1 projection and host-only R2 GET capability",
        "auth": "session_or_api_key",
        "policy": "client_compatibility.v1",
        "completion": MOBILE_BOOTSTRAP_CAPS_COMPLETION,
    },
    ("route", "GET", "/api/mobile/caps/:id/playback"): {
        "id": "cap-v1-bde34617e42a8834",
        "source_path": "apps/web/app/api/mobile/[...route]/route.ts",
        "source_sha256": "02df2ce92dc6e8ae11748b6e082c1304596ba9e4c370b35069867754218f5f79",
        "extra_sources": MOBILE_CAP_PLAYBACK_RUNTIME_SOURCES,
        "local_status": "rust_exact_mobile_cap_playback_d1_r2_local_contract",
        "rust_authority": "frame-application exact source-to-playback mapping + control-plane owner-hidden D1 projection and range-compatible R2 GET capability",
        "auth": "session_or_api_key",
        "policy": "client_compatibility.v1",
        "completion": MOBILE_BOOTSTRAP_CAPS_COMPLETION,
    },
    ("route", "POST", "/api/mobile/caps/:id/comments"): {
        "id": "cap-v1-661d23fdcca80bd2",
        "source_path": "apps/web/app/api/mobile/[...route]/route.ts",
        "source_sha256": "02df2ce92dc6e8ae11748b6e082c1304596ba9e4c370b35069867754218f5f79",
        "extra_sources": ({
            "path": "packages/web-domain/src/Mobile.ts",
            "symbol": "createComment",
            "sha256": "331d76900372d62389d729f8682baca1344f3583e3f41f42ad6e3ef2be7a3d5b",
        },),
        "local_status": "rust_exact_mobile_comment_create_d1_local_contract",
        "rust_authority": "frame-application collaboration semantics + control-plane exact mobile ingress and atomic D1 adapter",
        "auth": "session_or_api_key",
        "policy": "collaboration_notifications.v1",
        "completion": {
            "decision": "serve_frame_exact_business",
            "local_work": "complete",
            "protected_gates": [],
            "retirement_decision": "not_proposed",
            "production_behavior": "serve_exact_d1",
        },
    },
    ("route", "POST", "/api/mobile/caps/:id/reactions"): {
        "id": "cap-v1-bd59425c2e7074ae",
        "source_path": "apps/web/app/api/mobile/[...route]/route.ts",
        "source_sha256": "02df2ce92dc6e8ae11748b6e082c1304596ba9e4c370b35069867754218f5f79",
        "extra_sources": ({
            "path": "packages/web-domain/src/Mobile.ts",
            "symbol": "createReaction",
            "sha256": "331d76900372d62389d729f8682baca1344f3583e3f41f42ad6e3ef2be7a3d5b",
        },),
        "local_status": "rust_exact_mobile_reaction_create_d1_local_contract",
        "rust_authority": "frame-application collaboration semantics + control-plane exact mobile ingress and atomic D1 adapter",
        "auth": "session_or_api_key",
        "policy": "collaboration_notifications.v1",
        "completion": {
            "decision": "serve_frame_exact_business",
            "local_work": "complete",
            "protected_gates": [],
            "retirement_decision": "not_proposed",
            "production_behavior": "serve_exact_d1",
        },
    },
    ("route", "DELETE", "/api/mobile/comments/:id"): {
        "id": "cap-v1-b6ec2f719de27105",
        "source_path": "apps/web/app/api/mobile/[...route]/route.ts",
        "source_sha256": "02df2ce92dc6e8ae11748b6e082c1304596ba9e4c370b35069867754218f5f79",
        "extra_sources": ({
            "path": "packages/web-domain/src/Mobile.ts",
            "symbol": "deleteComment",
            "sha256": "331d76900372d62389d729f8682baca1344f3583e3f41f42ad6e3ef2be7a3d5b",
        },),
        "local_status": "rust_exact_mobile_comment_delete_d1_local_contract",
        "rust_authority": "frame-application collaboration semantics + control-plane exact mobile ingress and atomic D1 adapter",
        "auth": "session_or_api_key",
        "policy": "collaboration_notifications.v1",
        "completion": {
            "decision": "serve_frame_exact_business",
            "local_work": "complete",
            "protected_gates": [],
            "retirement_decision": "not_proposed",
            "production_behavior": "serve_exact_d1",
        },
    },
    ("route", "DELETE", "/api/video/comment/delete"): {
        "id": "cap-v1-f3f5e53c019f944a",
        "source_path": "apps/web/app/api/video/comment/delete/route.ts",
        "source_sha256": "14ef1d8346aa29ff90628f0971c78c48d368d966d9e408ea2876ab8aae1df529",
        "local_status": "rust_exact_web_comment_delete_route_d1_local_contract",
        "rust_authority": "frame-application collaboration semantics + control-plane exact route ingress and atomic D1 adapter",
        "auth": "session",
        "policy": "collaboration_notifications.v1",
        "completion": {
            "decision": "serve_frame_exact_business",
            "local_work": "complete",
            "protected_gates": [],
            "retirement_decision": "not_proposed",
            "production_behavior": "serve_exact_d1",
        },
    },
    ("server_action", "ACTION", "action://apps/web/actions/videos/delete-comment.ts#deleteComment"): {
        "id": "cap-v1-f74174457880eadc",
        "source_path": "apps/web/actions/videos/delete-comment.ts",
        "source_sha256": "7e1cf2a1141e56ec28cb256b35cd47583838fae3745750ea4f72db36fc37ff5e",
        "local_status": "rust_exact_web_comment_delete_action_d1_local_contract",
        "rust_authority": "frame-application collaboration semantics + authenticated action ingress and atomic D1 adapter",
        "auth": "session",
        "policy": "collaboration_notifications.v1",
        "completion": {
            "decision": "serve_frame_exact_business",
            "local_work": "complete",
            "protected_gates": [],
            "retirement_decision": "not_proposed",
            "production_behavior": "serve_exact_action",
        },
    },
    ("server_action", "ACTION", "action://apps/web/actions/videos/new-comment.ts#newComment"): {
        "id": "cap-v1-dbe600b35683c827",
        "source_path": "apps/web/actions/videos/new-comment.ts",
        "source_sha256": "66b1386d37d9f0cd04ca37825ecbeef6e57d10a4f9042562bdd655c3badf317e",
        "local_status": "rust_exact_web_comment_create_action_d1_local_contract",
        "rust_authority": "frame-application collaboration semantics + authenticated action ingress and atomic D1 adapter",
        "auth": "session",
        "policy": "collaboration_notifications.v1",
        "completion": {
            "decision": "serve_frame_exact_business",
            "local_work": "complete",
            "protected_gates": [],
            "retirement_decision": "not_proposed",
            "production_behavior": "serve_exact_action",
        },
    },
    ("route", "POST", "/api/mobile/folders"): {
        "id": "cap-v1-7160c4389375c682",
        "source_path": "apps/web/app/api/mobile/[...route]/route.ts",
        "source_sha256": "02df2ce92dc6e8ae11748b6e082c1304596ba9e4c370b35069867754218f5f79",
        "extra_sources": (
            {
                "path": "packages/database/auth/auth-options.ts",
                "symbol": "getServerSession+authOptions",
                "sha256": "22b8923e1cab6b5b1b318609abe664e171fb740ae39817c2c962908ca0dc8595",
            },
            {
                "path": "packages/database/helpers.ts",
                "symbol": "nanoId",
                "sha256": "e976df51a8186737a1c4696a20cd52f2c029aa630b2463d1970b8667b0dd20cd",
            },
            {
                "path": "packages/database/schema.ts",
                "symbol": "folders+organizations+organizationMembers+authApiKeys",
                "sha256": "7fce297f9076be78a9ac6280d9d060bf6e836a62e0f82b5390fa0e42dc7bb9e9",
            },
            {
                "path": "packages/web-backend/src/Auth.ts",
                "symbol": "HttpAuthMiddlewareLive+getCurrentUser+CurrentUser",
                "sha256": "aea054db2b84a8c4bd6684fefe8d0e971a094a9faa9653105b0c33ab52ab824d",
            },
            {
                "path": "packages/web-backend/src/Database.ts",
                "symbol": "Database",
                "sha256": "24500254943ace60c5ea3a7943f40c85ab2c9a8caba36073ff54100ab9488837",
            },
            {
                "path": "packages/web-domain/src/Authentication.ts",
                "symbol": "CurrentUser+HttpAuthMiddleware",
                "sha256": "165c9f652c39d7f1cf3b43a5c66c5a4418bbe97338279ca01d00c19f2026167b",
            },
            {
                "path": "packages/web-domain/src/Mobile.ts",
                "symbol": "createFolder",
                "sha256": "331d76900372d62389d729f8682baca1344f3583e3f41f42ad6e3ef2be7a3d5b",
            },
            {
                "path": "pnpm-lock.yaml",
                "symbol": "nanoid@5.1.6",
                "sha256": "fc0fe122ae5fbea4dcaa7e510bd6275635c14071c4e031996431a54fb7e25e3a",
            },
        ),
        "local_status": "rust_exact_mobile_folder_create_d1_local_contract",
        "rust_authority": "frame-application exact mobile folder semantics + control-plane atomic D1 ingress",
        "auth": "session_or_api_key",
        "policy": "organization_library.v1",
        "completion": {
            "decision": "serve_frame_exact_business",
            "local_work": "complete",
            "protected_gates": [],
            "retirement_decision": "not_proposed",
            "production_behavior": "serve_exact_d1",
        },
    },
    ("rpc", "RPC", "/api/erpc#FolderCreate"): {
        "id": "cap-v1-9e125712cee9ce5a",
        "source_path": "packages/web-domain/src/Folder.ts",
        "source_sha256": "4201376991878efc79979f77901908d542573f5b0f9e1ca6b6b246e04d881e9e",
        "local_status": "rust_effect_rpc_folder_create_d1_local_contract_human_approval_pending",
        "rust_authority": "frame-application exact folder CRUD semantics + control-plane Effect-RPC and atomic D1 ingress",
        "auth": "session",
        "policy": "organization_library.v1",
        "completion": {
            "decision": "serve_frame_exact_business",
            "local_work": "complete",
            "protected_gates": ["human_approval"],
            "retirement_decision": "not_proposed",
            "production_behavior": "fail_closed_unavailable",
        },
    },
    ("rpc", "RPC", "/api/erpc#FolderDelete"): {
        "id": "cap-v1-eea1796482b3af28",
        "source_path": "packages/web-domain/src/Folder.ts",
        "source_sha256": "4201376991878efc79979f77901908d542573f5b0f9e1ca6b6b246e04d881e9e",
        "local_status": "rust_effect_rpc_folder_delete_d1_local_contract_human_approval_pending",
        "rust_authority": "frame-application exact folder CRUD semantics + control-plane Effect-RPC and atomic D1 ingress",
        "auth": "session",
        "policy": "organization_library.v1",
        "completion": {
            "decision": "serve_frame_exact_business",
            "local_work": "complete",
            "protected_gates": ["human_approval"],
            "retirement_decision": "not_proposed",
            "production_behavior": "fail_closed_unavailable",
        },
    },
    ("rpc", "RPC", "/api/erpc#FolderUpdate"): {
        "id": "cap-v1-a193e9e08b2c3f7d",
        "source_path": "packages/web-domain/src/Folder.ts",
        "source_sha256": "4201376991878efc79979f77901908d542573f5b0f9e1ca6b6b246e04d881e9e",
        "local_status": "rust_effect_rpc_folder_update_d1_local_contract_human_approval_pending",
        "rust_authority": "frame-application exact folder CRUD semantics + control-plane Effect-RPC and atomic D1 ingress",
        "auth": "session",
        "policy": "organization_library.v1",
        "completion": {
            "decision": "serve_frame_exact_business",
            "local_work": "complete",
            "protected_gates": ["human_approval"],
            "retirement_decision": "not_proposed",
            "production_behavior": "fail_closed_unavailable",
        },
    },
    ("route", "GET", "/api/desktop/session/request"): {
        "id": "cap-v1-768895bc99380850",
        "source_path": "apps/web/app/api/desktop/[...route]/session.ts",
        "source_sha256": "22c7b789cf901926e6ae4ffe2fcd574e5f6d8474a14c961d94ae3797b3ad45e0",
        "extra_sources": (
            {
                "path": "apps/web/app/api/desktop/[...route]/route.ts",
                "symbol": "desktop mount+GET+OPTIONS+CORS",
                "sha256": "34854ff6fc0839838165990bea1c9ebee86770b1648ec832bbbb786720c9db41",
            },
            {
                "path": "apps/desktop/src/utils/auth.ts",
                "symbol": "createSessionRequestUrl+paramsValidator+processAuthData",
                "sha256": "ae80288f5caac230ff6390a96b3286f6eb961307cb85d3ca9dcc95f99931f914",
            },
            {
                "path": "apps/desktop/src/utils/server-url-routing.ts",
                "symbol": "shouldUseLocalServerSessionForUrl+resolveServerRequestPath",
                "sha256": "3826d1163e4a8a558d199f2202290872d158ef0370f94c4841bebb5e614c46ff",
            },
            {
                "path": "apps/web/app/api/utils.ts",
                "symbol": "getAuth+corsMiddleware",
                "sha256": "241e5259f690ece17b0c50f78a9dc30c3e783082287040fef0f47e56a937bb30",
            },
            {
                "path": "packages/database/auth/auth-options.ts",
                "symbol": "decodeSessionToken+authOptions",
                "sha256": "22b8923e1cab6b5b1b318609abe664e171fb740ae39817c2c962908ca0dc8595",
            },
            {
                "path": "packages/database/auth/session.ts",
                "symbol": "getCurrentUser",
                "sha256": "d526dc9d7a6a1a7cb6a8695c24ab88b843ce09b4444f8e4ade24b7a06cbbc1ee",
            },
            {
                "path": "packages/database/schema.ts",
                "symbol": "users+authApiKeys",
                "sha256": "7fce297f9076be78a9ac6280d9d060bf6e836a62e0f82b5390fa0e42dc7bb9e9",
            },
            {
                "path": "packages/database/index.ts",
                "symbol": "db",
                "sha256": "161c1d1fd2a561fd2846aeceb148f24b58afc58bdaa95175240e48dbe61d9bbb",
            },
            {
                "path": "apps/web/proxy.ts",
                "symbol": "API matcher exclusion",
                "sha256": "7da98445a31f6b48d01b56877c47aaa79ba3af93dff8c015ad06a6e94fb42fcb",
            },
            {
                "path": "packages/env/server.ts",
                "symbol": "WEB_URL+VERCEL_ENV+VERCEL_BRANCH_URL_HOST+NEXTAUTH_SECRET",
                "sha256": "235c2ea66843b610aee61c82cbcafe05086d00193545bc290650d3aa15a2a0a4",
            },
            {
                "path": "pnpm-lock.yaml",
                "symbol": "Hono+Drizzle+NextAuth dependency resolutions",
                "sha256": "fc0fe122ae5fbea4dcaa7e510bd6275635c14071c4e031996431a54fb7e25e3a",
            },
        ),
        "local_status": "rust_exact_desktop_session_handoff_d1_adapter_local_contract",
        "rust_authority": "frame-application source-pinned desktop handoff contract + AuthService session verification + digest-only D1 desktop key mint",
        "auth": "session",
        "policy": "auth_session.v1",
        "completion": {
            "decision": "serve_frame_exact_business",
            "local_work": "complete",
            "protected_gates": [],
            "retirement_decision": "not_proposed",
            "production_behavior": "serve_exact_d1",
        },
    },
    ("route", "GET", "/api/desktop/org-custom-domain"): {
        "id": "cap-v1-ed9957ac480103b9",
        "source_path": "apps/web/app/api/desktop/[...route]/root.ts",
        "source_sha256": "c6f9ca2108849b75a00762b79af45b0523dd246bc118a2805cb57948f6ea2e7a",
        "extra_sources": (
            {
                "path": "apps/desktop/src/utils/queries.ts",
                "symbol": "createCustomDomainQuery",
                "sha256": "6d21daeae4084adbf9c65c67019b5f6b7c3ac6a5566c3b5fbe2fd3abcdcbcc1c",
            },
            {
                "path": "apps/desktop/src/utils/web-api.ts",
                "symbol": "orgCustomDomainClient+protectedHeaders",
                "sha256": "d3655b985a21a54d97b9974b17536aebab490929850baffaa5186d7a5632b45a",
            },
            {
                "path": "apps/web/app/api/desktop/[...route]/route.ts",
                "symbol": "desktop mount+GET+OPTIONS",
                "sha256": "34854ff6fc0839838165990bea1c9ebee86770b1648ec832bbbb786720c9db41",
            },
            {
                "path": "apps/web/app/api/utils.ts",
                "symbol": "getAuth+withAuth+corsMiddleware",
                "sha256": "241e5259f690ece17b0c50f78a9dc30c3e783082287040fef0f47e56a937bb30",
            },
            {
                "path": "packages/database/schema.ts",
                "symbol": "users+organizations+authApiKeys",
                "sha256": "7fce297f9076be78a9ac6280d9d060bf6e836a62e0f82b5390fa0e42dc7bb9e9",
            },
        ),
        "local_status": "rust_exact_desktop_org_custom_domain_d1_adapter_local_contract",
        "rust_authority": "frame-application source-pinned desktop custom-domain contract + control-plane authenticated D1 projection",
        "auth": "session_or_api_key",
        "policy": "client_compatibility.v1",
        "completion": {
            "decision": "serve_frame_exact_business",
            "local_work": "complete",
            "protected_gates": [],
            "retirement_decision": "not_proposed",
            "production_behavior": "serve_exact_d1",
        },
    },
    ("route", "GET", "/api/desktop/organizations"): {
        "id": "cap-v1-ab49cf36a3f243ac",
        "source_path": "apps/web/app/api/desktop/[...route]/root.ts",
        "source_sha256": "c6f9ca2108849b75a00762b79af45b0523dd246bc118a2805cb57948f6ea2e7a",
        "extra_sources": DESKTOP_ORGANIZATIONS_RUNTIME_SOURCES,
        "local_status": "rust_exact_desktop_organizations_d1_local_contract",
        "rust_authority": "frame-application source-pinned organization projection + control-plane actor-scoped D1 desktop ingress",
        "auth": "session_or_api_key",
        "policy": "organization_library.v1",
        "completion": {
            "decision": "serve_frame_exact_business",
            "local_work": "complete",
            "protected_gates": [],
            "retirement_decision": "not_proposed",
            "production_behavior": "serve_exact_d1",
        },
    },
    ("route", "PATCH", "/api/desktop/organizations/:organizationId/branding"): {
        "id": "cap-v1-cdfdf7db0f5cb243",
        "source_path": "apps/web/app/api/desktop/[...route]/root.ts",
        "source_sha256": "c6f9ca2108849b75a00762b79af45b0523dd246bc118a2805cb57948f6ea2e7a",
        "extra_sources": DESKTOP_BRANDING_RUNTIME_SOURCES,
        "local_status": "rust_exact_desktop_organization_branding_d1_local_contract",
        "rust_authority": "frame-application exact branding normalization + tenant-authorized atomic D1 metadata and logo projection",
        "auth": "session_or_api_key",
        "policy": "organization_library.v1",
        "completion": {
            "decision": "serve_frame_exact_business",
            "local_work": "complete",
            "protected_gates": [],
            "retirement_decision": "not_proposed",
            "production_behavior": "serve_exact_d1",
        },
    },
    ("route", "POST", "/api/desktop/storage/set-active"): {
        "id": "cap-v1-a77171e54b2ba955",
        "source_path": "apps/web/app/api/desktop/[...route]/storage.ts",
        "source_sha256": "5e6fb13fe1f1176349a455d8c4ee4f1fea56fb53c095599b0aa990113ebd0886",
        "extra_sources": DESKTOP_STORAGE_RUNTIME_SOURCES,
        "local_status": "rust_exact_desktop_storage_set_active_d1_local_contract",
        "rust_authority": "frame-application exact provider selection + actor-owned personal storage D1 projection",
        "auth": "session_or_api_key",
        "policy": "upload_storage.v1",
        "completion": {
            "decision": "serve_frame_exact_business",
            "local_work": "complete",
            "protected_gates": [],
            "retirement_decision": "not_proposed",
            "production_behavior": "serve_exact_d1",
        },
    },
    ("route", "GET", "/api/desktop/user/profile"): {
        "id": "cap-v1-7508c5a7da637a0b",
        "source_path": "apps/web/app/api/desktop/[...route]/root.ts",
        "source_sha256": "c6f9ca2108849b75a00762b79af45b0523dd246bc118a2805cb57948f6ea2e7a",
        "extra_sources": DESKTOP_PROFILE_RUNTIME_SOURCES,
        "local_status": "rust_exact_desktop_user_profile_d1_local_contract",
        "rust_authority": "frame-application exact desktop profile projection + actor-scoped D1 ingress",
        "auth": "session_or_api_key",
        "policy": "client_compatibility.v1",
        "completion": {
            "decision": "serve_frame_exact_business",
            "local_work": "complete",
            "protected_gates": [],
            "retirement_decision": "not_proposed",
            "production_behavior": "serve_exact_d1",
        },
    },
    ("route", "DELETE", "/api/desktop/video/delete"): {
        "id": "cap-v1-acc98d2d5e8ff345",
        "source_path": "apps/web/app/api/desktop/[...route]/video.ts",
        "source_sha256": "03e50223fb6968dafdbaa8a8c8cb537c46be27a0c88b9c92e004afa95f7c013d",
        "extra_sources": DESKTOP_VIDEO_DELETE_RUNTIME_SOURCES,
        "local_status": "rust_exact_desktop_video_delete_d1_r2_local_contract",
        "rust_authority": "frame-application exact delete contract + owner-fenced D1 tombstone and resumable R2 provider continuation",
        "auth": "session_or_api_key",
        "policy": "video_media.v1",
        "completion": {
            "decision": "serve_frame_exact_business",
            "local_work": "complete",
            "protected_gates": [],
            "retirement_decision": "not_proposed",
            "production_behavior": "serve_exact_d1",
        },
    },
    ("route", "POST", "/api/desktop/video/progress"): {
        "id": "cap-v1-117b0cb801816693",
        "source_path": "apps/web/app/api/desktop/[...route]/video.ts",
        "source_sha256": "03e50223fb6968dafdbaa8a8c8cb537c46be27a0c88b9c92e004afa95f7c013d",
        "extra_sources": DESKTOP_VIDEO_PROGRESS_RUNTIME_SOURCES,
        "local_status": "rust_exact_desktop_video_progress_d1_local_contract",
        "rust_authority": "frame-application source timestamp arbitration + owner-fenced atomic D1 upload progress projection",
        "auth": "session_or_api_key",
        "policy": "video_media.v1",
        "completion": {
            "decision": "serve_frame_exact_business",
            "local_work": "complete",
            "protected_gates": [],
            "retirement_decision": "not_proposed",
            "production_behavior": "serve_exact_d1",
        },
    },
    ("route", "GET", "/api/video/domain-info"): {
        "id": "cap-v1-10e17d0e86b49830",
        "source_path": "apps/web/app/api/video/domain-info/route.ts",
        "source_sha256": "07e0373bace84adabaf409bc1f3360221d01ed7143e1ab49514730d893b66bc5",
        "extra_sources": (
            {
                "path": "packages/database/schema.ts",
                "symbol": "videos+sharedVideos+organizations",
                "sha256": "7fce297f9076be78a9ac6280d9d060bf6e836a62e0f82b5390fa0e42dc7bb9e9",
            },
            {
                "path": "packages/web-domain/src/Video.ts",
                "symbol": "Video.VideoId",
                "sha256": "adc3db0eded2670b1ed89969e7bc85993e04021acb303082b3d015f0afb1c9a7",
            },
            {
                "path": "packages/database/index.ts",
                "symbol": "db",
                "sha256": "161c1d1fd2a561fd2846aeceb148f24b58afc58bdaa95175240e48dbe61d9bbb",
            },
            {
                "path": "apps/web/package.json",
                "symbol": "next+drizzle dependencies",
                "sha256": "c1358cd1880ac5dc9d659760c2788cedd5c4f61fec2cb0dd1b60cbc9bb8af920",
            },
            {
                "path": "pnpm-lock.yaml",
                "symbol": "dependency lock",
                "sha256": "fc0fe122ae5fbea4dcaa7e510bd6275635c14071c4e031996431a54fb7e25e3a",
            },
        ),
        "local_status": "rust_exact_anonymous_video_domain_info_d1_local_contract",
        "rust_authority": "frame-application source-pinned anonymous domain-info contract + control-plane D1 video/share/owner projection",
        "auth": "anonymous",
        "policy": "video_media.v1",
        "completion": {
            "decision": "serve_frame_exact_business",
            "local_work": "complete",
            "protected_gates": [],
            "retirement_decision": "not_proposed",
            "production_behavior": "serve_exact_d1",
        },
    },
    ("route", "GET", "/api/notifications/preferences"): {
        "id": "cap-v1-d130c840f654bd72",
        "source_path": "apps/web/app/api/notifications/preferences/route.ts",
        "source_sha256": "3692f8854c0c050f5168f89acb1d03dc1c31d4529000e0b5e140078e8d3ce975",
        "extra_sources": (
            {
                "path": "packages/database/auth/session.ts",
                "symbol": "getCurrentUser",
                "sha256": "d526dc9d7a6a1a7cb6a8695c24ab88b843ce09b4444f8e4ade24b7a06cbbc1ee",
            },
            {
                "path": "packages/database/auth/auth-options.ts",
                "symbol": "authOptions session callback",
                "sha256": "22b8923e1cab6b5b1b318609abe664e171fb740ae39817c2c962908ca0dc8595",
            },
            {
                "path": "packages/database/schema.ts",
                "symbol": "users.preferences",
                "sha256": "7fce297f9076be78a9ac6280d9d060bf6e836a62e0f82b5390fa0e42dc7bb9e9",
            },
            {
                "path": "apps/web/proxy.ts",
                "symbol": "API path middleware exclusion",
                "sha256": "7da98445a31f6b48d01b56877c47aaa79ba3af93dff8c015ad06a6e94fb42fcb",
            },
            {
                "path": "apps/web/next.config.mjs",
                "symbol": "Next response runtime configuration",
                "sha256": "c3251d5a5925ee835dbc7cd1eb77eb42335813008a163e27e7823c15b9577b1e",
            },
            {
                "path": "apps/web/package.json",
                "symbol": "next dependency declaration",
                "sha256": "c1358cd1880ac5dc9d659760c2788cedd5c4f61fec2cb0dd1b60cbc9bb8af920",
            },
            {
                "path": "pnpm-lock.yaml",
                "symbol": "next@16.2.1 resolution",
                "sha256": "fc0fe122ae5fbea4dcaa7e510bd6275635c14071c4e031996431a54fb7e25e3a",
            },
        ),
        "local_status": "rust_exact_notification_preferences_d1_adapter_local_contract",
        "rust_authority": "frame-application notification preference semantics + control-plane D1 users.preferences_json adapter",
        "auth": "session",
        "policy": "collaboration_notifications.v1",
        "completion": {
            "decision": "serve_frame_exact_business",
            "local_work": "complete",
            "protected_gates": [],
            "retirement_decision": "not_proposed",
            "production_behavior": "serve_exact_d1",
        },
    },
    ("route", "GET", "/api/notifications"): {
        "id": "cap-v1-14dcca6d36eee6b3",
        "source_path": "apps/web/app/api/notifications/route.ts",
        "source_sha256": "1c0571a385328c53ec106967a717201ed2aa04cbcfd108c419f03f8b51b3ae17",
        "extra_sources": (
            {
                "path": "packages/web-api-contract/src/index.ts",
                "symbol": "GET /notifications",
                "sha256": "98bb2529e27eba0ed1569d286a1f5d4069cbbf23cf9e1dde62fdc1f6a9737e3e",
            },
            {
                "path": "packages/database/auth/session.ts",
                "symbol": "getCurrentUser",
                "sha256": "d526dc9d7a6a1a7cb6a8695c24ab88b843ce09b4444f8e4ade24b7a06cbbc1ee",
            },
            {
                "path": "packages/database/schema.ts",
                "symbol": "notifications+users",
                "sha256": "7fce297f9076be78a9ac6280d9d060bf6e836a62e0f82b5390fa0e42dc7bb9e9",
            },
            {
                "path": "packages/web-backend/src/ImageUploads/index.ts",
                "symbol": "ImageUploads.resolveImageUrl",
                "sha256": "1dc0952ae84d76844128d0fc5cdf2eb63519c26183f932c035638ff0d6463d1c",
            },
        ),
        "local_status": "rust_exact_notification_list_d1_adapter_local_contract",
        "rust_authority": "frame-application notification-list semantics + control-plane scoped D1 projection",
        "auth": "session",
        "policy": "collaboration_notifications.v1",
        "completion": {
            "decision": "serve_frame_exact_business",
            "local_work": "complete",
            "protected_gates": [],
            "retirement_decision": "not_proposed",
            "production_behavior": "serve_exact_d1",
        },
    },
    (
        "server_action",
        "ACTION",
        "action://apps/web/app/(org)/dashboard/_components/Navbar/server.ts#updateActiveOrganization",
    ): {
        "id": "cap-v1-a3b4c805d409bc7c",
        "source_path": "apps/web/app/(org)/dashboard/_components/Navbar/server.ts",
        "source_sha256": "a7ea138516eb20f40dad4ad53913e69b01e4f5ad8b2938eb9f5a9a98ab3a29b3",
        "extra_sources": (
            {
                "path": "packages/database/auth/session.ts",
                "symbol": "getCurrentUser",
                "sha256": "d526dc9d7a6a1a7cb6a8695c24ab88b843ce09b4444f8e4ade24b7a06cbbc1ee",
            },
            {
                "path": "packages/database/schema.ts",
                "symbol": "organization selection persistence schema",
                "sha256": "7fce297f9076be78a9ac6280d9d060bf6e836a62e0f82b5390fa0e42dc7bb9e9",
            },
            {
                "path": "packages/web-domain/src/Organisation.ts",
                "symbol": "OrganisationId",
                "sha256": "14d634ad8910d3921af2ea5b136b9c3d2a8ae26f74b3dcb7a82b9cf19d6a3264",
            },
        ),
        "local_status": "rust_exact_active_organization_action_callable_ingress_client_e2e_pending",
        "rust_authority": "frame-application legacy organization selection + control-plane D1 active-only adapter",
        "auth": "session",
        "policy": "organization_library.v1",
        "completion": {
            "decision": "serve_frame_exact_business",
            "local_work": "complete",
            "protected_gates": ["released_legacy_client_e2e"],
            "retirement_decision": "not_proposed",
            "production_behavior": "fail_closed_unavailable",
        },
    },
    (
        "server_action",
        "ACTION",
        "action://apps/web/app/(org)/dashboard/_components/actions.ts#setTheme",
    ): {
        "id": "cap-v1-7773d3e70d1d5919",
        "source_path": "apps/web/app/(org)/dashboard/_components/actions.ts",
        "source_sha256": "3f73b91e1e105555846014adfbc7498d5c719b536b5edcd8a3876167ed84ad1a",
        "extra_sources": (
            {
                "path": "apps/web/app/(org)/dashboard/layout.tsx",
                "symbol": "authenticated dashboard theme cookie reader",
                "sha256": "65221996b10ee679b5868e6cea9002256fc8043eb95b2dbf6b56a222ce9c1d33",
            },
            {
                "path": "apps/web/app/(org)/dashboard/Contexts.tsx",
                "symbol": "dashboard client-side theme persistence behavior",
                "sha256": "2b2df78eab9dbdde2918a4b5993a31cf73fb8de2465d318c7b27d3d439f2a0cb",
            },
            {
                "path": "apps/web/package.json",
                "symbol": "next dependency declaration",
                "sha256": "c1358cd1880ac5dc9d659760c2788cedd5c4f61fec2cb0dd1b60cbc9bb8af920",
            },
            {
                "path": "pnpm-lock.yaml",
                "symbol": "next@16.2.1 resolution",
                "sha256": "fc0fe122ae5fbea4dcaa7e510bd6275635c14071c4e031996431a54fb7e25e3a",
            },
        ),
        "local_status": "rust_exact_theme_action_production_ingress_contract",
        "rust_authority": "frame-application legacy theme semantics + control-plane response-cookie action ingress",
        "auth": "session",
        "policy": "service_misc.v1",
        "completion": {
            "decision": "serve_frame_exact_business",
            "local_work": "complete",
            "protected_gates": [],
            "retirement_decision": "not_proposed",
            "production_behavior": "serve_exact_action",
        },
    },
    (
        "server_action",
        "ACTION",
        "action://apps/web/actions/folders/add-videos.ts#addVideosToFolder",
    ): {
        "id": "cap-v1-f5daa7be337a2979",
        "source_path": "apps/web/actions/folders/add-videos.ts",
        "source_sha256": "cb4bcfab7d466e54fa77c09fdc4bac24d4041468c5c857b32ea0038f195132aa",
        "extra_sources": FOLDER_ASSIGNMENT_ADD_RUNTIME_SOURCES,
        "local_status": "rust_exact_folder_assignment_add_local_contract_released_client_e2e_pending",
        "rust_authority": "frame-application exact folder assignment + control-plane atomic D1 browser ingress",
        "auth": "session",
        "policy": "organization_library.v1",
        "completion": {
            "decision": "serve_frame_exact_business",
            "local_work": "complete",
            "protected_gates": ["released_legacy_client_e2e"],
            "retirement_decision": "not_proposed",
            "production_behavior": "fail_closed_unavailable",
        },
    },
    (
        "server_action",
        "ACTION",
        "action://apps/web/actions/folders/remove-videos.ts#removeVideosFromFolder",
    ): {
        "id": "cap-v1-1af3645bf2ae7168",
        "source_path": "apps/web/actions/folders/remove-videos.ts",
        "source_sha256": "f4ce4a28ff1c3f8f2fc23779606a7530945f47fc2e44f49536687ed6209a2d5f",
        "extra_sources": FOLDER_ASSIGNMENT_RUNTIME_SOURCES,
        "local_status": "rust_exact_folder_assignment_remove_local_contract_released_client_e2e_pending",
        "rust_authority": "frame-application exact folder assignment + control-plane atomic D1 browser ingress",
        "auth": "session",
        "policy": "organization_library.v1",
        "completion": {
            "decision": "serve_frame_exact_business",
            "local_work": "complete",
            "protected_gates": ["released_legacy_client_e2e"],
            "retirement_decision": "not_proposed",
            "production_behavior": "fail_closed_unavailable",
        },
    },
    (
        "server_action",
        "ACTION",
        "action://apps/web/actions/folders/moveVideoToFolder.ts#moveVideoToFolder",
    ): {
        "id": "cap-v1-eaf277e644aa4b92",
        "source_path": "apps/web/actions/folders/moveVideoToFolder.ts",
        "source_sha256": "08f943871c4bdc0f931e140f994dff77c27f249fa3585cc50c1dbd6b8241c045",
        "extra_sources": FOLDER_ASSIGNMENT_RUNTIME_SOURCES,
        "local_status": "rust_exact_folder_assignment_move_local_contract_released_client_e2e_pending",
        "rust_authority": "frame-application exact folder assignment + control-plane atomic D1 browser ingress",
        "auth": "session",
        "policy": "organization_library.v1",
        "completion": {
            "decision": "serve_frame_exact_business",
            "local_work": "complete",
            "protected_gates": ["released_legacy_client_e2e"],
            "retirement_decision": "not_proposed",
            "production_behavior": "fail_closed_unavailable",
        },
    },
    (
        "server_action",
        "ACTION",
        "action://apps/web/actions/organizations/add-videos.ts#addVideosToOrganization",
    ): {
        "id": "cap-v1-d96a1931942eb83b",
        "source_path": "apps/web/actions/organizations/add-videos.ts",
        "source_sha256": "127ccc6ab701c04082cf8010281dbf70daee2ec6c54c01b3af1ebec5b56310c9",
        "extra_sources": LIBRARY_PLACEMENT_ADD_ORGANIZATION_RUNTIME_SOURCES,
        "local_status": "rust_exact_library_organization_add_local_contract_released_client_e2e_pending",
        "rust_authority": "frame-application exact library placement + control-plane atomic D1 browser ingress",
        "auth": "session",
        "policy": "organization_library.v1",
        "completion": {
            "decision": "serve_frame_exact_business",
            "local_work": "complete",
            "protected_gates": ["released_legacy_client_e2e"],
            "retirement_decision": "not_proposed",
            "production_behavior": "fail_closed_unavailable",
        },
    },
    (
        "server_action",
        "ACTION",
        "action://apps/web/actions/organizations/remove-videos.ts#removeVideosFromOrganization",
    ): {
        "id": "cap-v1-0694e68a64976c9a",
        "source_path": "apps/web/actions/organizations/remove-videos.ts",
        "source_sha256": "c67c82c0d5d64229046075569d384bfee3766fea3f7a4adbd592bba1a204bfac",
        "extra_sources": LIBRARY_PLACEMENT_ORGANIZATION_RUNTIME_SOURCES,
        "local_status": "rust_exact_library_organization_remove_local_contract_released_client_e2e_pending",
        "rust_authority": "frame-application exact library placement + control-plane atomic D1 browser ingress",
        "auth": "session",
        "policy": "organization_library.v1",
        "completion": {
            "decision": "serve_frame_exact_business",
            "local_work": "complete",
            "protected_gates": ["released_legacy_client_e2e"],
            "retirement_decision": "not_proposed",
            "production_behavior": "fail_closed_unavailable",
        },
    },
    (
        "server_action",
        "ACTION",
        "action://apps/web/actions/spaces/add-videos.ts#addVideosToSpace",
    ): {
        "id": "cap-v1-bb55b5eeeb5e31ab",
        "source_path": "apps/web/actions/spaces/add-videos.ts",
        "source_sha256": "b15a27c2e1e522f97dcdfcb1802ffd7c449a34420b311ca2273f8c8f737581fa",
        "extra_sources": LIBRARY_PLACEMENT_ADD_SPACE_RUNTIME_SOURCES,
        "local_status": "rust_exact_library_scope_add_local_contract_released_client_e2e_pending",
        "rust_authority": "frame-application exact library placement + control-plane atomic D1 browser ingress",
        "auth": "session",
        "policy": "organization_library.v1",
        "completion": {
            "decision": "serve_frame_exact_business",
            "local_work": "complete",
            "protected_gates": ["released_legacy_client_e2e"],
            "retirement_decision": "not_proposed",
            "production_behavior": "fail_closed_unavailable",
        },
    },
    (
        "server_action",
        "ACTION",
        "action://apps/web/actions/spaces/remove-videos.ts#removeVideosFromSpace",
    ): {
        "id": "cap-v1-ccbe5f1381eaa1b4",
        "source_path": "apps/web/actions/spaces/remove-videos.ts",
        "source_sha256": "a88805652fd94c0baba35afe2d8e3b46d5cf9100362ec8370cba8f43e9b611dc",
        "extra_sources": LIBRARY_PLACEMENT_SPACE_RUNTIME_SOURCES,
        "local_status": "rust_exact_library_scope_remove_local_contract_released_client_e2e_pending",
        "rust_authority": "frame-application exact library placement + control-plane atomic D1 browser ingress",
        "auth": "session",
        "policy": "organization_library.v1",
        "completion": {
            "decision": "serve_frame_exact_business",
            "local_work": "complete",
            "protected_gates": ["released_legacy_client_e2e"],
            "retirement_decision": "not_proposed",
            "production_behavior": "fail_closed_unavailable",
        },
    },
    (
        "server_action",
        "ACTION",
        "action://apps/web/actions/notifications/mark-as-read.ts#markAsRead",
    ): {
        "id": "cap-v1-74a775753d3863c7",
        "source_path": "apps/web/actions/notifications/mark-as-read.ts",
        "source_sha256": "d25181538c6463e95f787902ed52dbb1eec758b14fcdbaa00a77e2408d35bd49",
        "extra_sources": NOTIFICATION_MARK_READ_RUNTIME_SOURCES,
        "local_status": "rust_exact_notification_mark_read_local_contract_released_client_e2e_pending",
        "rust_authority": "frame-application exact notification action + control-plane atomic D1 browser ingress",
        "auth": "session",
        "policy": "collaboration_notifications.v1",
        "completion": {
            "decision": "serve_frame_exact_business",
            "local_work": "complete",
            "protected_gates": ["released_legacy_client_e2e"],
            "retirement_decision": "not_proposed",
            "production_behavior": "fail_closed_unavailable",
        },
    },
    (
        "server_action",
        "ACTION",
        "action://apps/web/actions/notifications/update-preferences.ts#updatePreferences",
    ): {
        "id": "cap-v1-1f6a43a05f2f297c",
        "source_path": "apps/web/actions/notifications/update-preferences.ts",
        "source_sha256": "c66025cde3f0b179440a60a4570368e335d474a4a323e83c2043111a3baf5ee8",
        "extra_sources": NOTIFICATION_UPDATE_PREFERENCES_RUNTIME_SOURCES,
        "local_status": "rust_exact_notification_preferences_write_local_contract_released_client_e2e_pending",
        "rust_authority": "frame-application exact notification action + control-plane atomic D1 browser ingress",
        "auth": "session",
        "policy": "collaboration_notifications.v1",
        "completion": {
            "decision": "serve_frame_exact_business",
            "local_work": "complete",
            "protected_gates": ["released_legacy_client_e2e"],
            "retirement_decision": "not_proposed",
            "production_behavior": "fail_closed_unavailable",
        },
    },
    (
        "server_action",
        "ACTION",
        "action://apps/web/actions/developers/create-app.ts#createDeveloperApp",
    ): {
        "id": "cap-v1-f303e703a4237888",
        "source_path": "apps/web/actions/developers/create-app.ts",
        "source_sha256": "d2149a30c6a3657b224458dd946b9d621f5fb3b1f84b4293ffd84549738c4a0b",
        "extra_sources": DEVELOPER_CREATE_RUNTIME_SOURCES,
        "local_status": "rust_exact_developer_create_app_local_contract_released_client_e2e_pending",
        "rust_authority": "frame-application exact developer action + control-plane atomic D1 browser ingress",
        "auth": "session",
        "policy": "developer_api.v1",
        "completion": {
            "decision": "serve_frame_exact_business",
            "local_work": "complete",
            "protected_gates": ["released_legacy_client_e2e"],
            "retirement_decision": "not_proposed",
            "production_behavior": "fail_closed_unavailable",
        },
    },
    (
        "server_action",
        "ACTION",
        "action://apps/web/actions/developers/update-app.ts#updateDeveloperApp",
    ): {
        "id": "cap-v1-87fd6af55b891cb9",
        "source_path": "apps/web/actions/developers/update-app.ts",
        "source_sha256": "41a00c87464b6d799ae93bcb5d44b0bc5dfec6adb320ff7e0b93841bb1adb025",
        "extra_sources": DEVELOPER_SETTINGS_RUNTIME_SOURCES,
        "local_status": "rust_exact_developer_update_app_local_contract_released_client_e2e_pending",
        "rust_authority": "frame-application exact developer action + control-plane atomic D1 browser ingress",
        "auth": "session",
        "policy": "developer_api.v1",
        "completion": {
            "decision": "serve_frame_exact_business",
            "local_work": "complete",
            "protected_gates": ["released_legacy_client_e2e"],
            "retirement_decision": "not_proposed",
            "production_behavior": "fail_closed_unavailable",
        },
    },
    (
        "server_action",
        "ACTION",
        "action://apps/web/actions/developers/delete-app.ts#deleteDeveloperApp",
    ): {
        "id": "cap-v1-9833b16bb80a3299",
        "source_path": "apps/web/actions/developers/delete-app.ts",
        "source_sha256": "c708ff132594d8523c160c978ab27812f4eec2afc2ec8cba977d6afe17eb7dcc",
        "extra_sources": DEVELOPER_SETTINGS_RUNTIME_SOURCES,
        "local_status": "rust_exact_developer_delete_app_local_contract_released_client_e2e_pending",
        "rust_authority": "frame-application exact developer action + control-plane atomic D1 browser ingress",
        "auth": "session",
        "policy": "developer_api.v1",
        "completion": {
            "decision": "serve_frame_exact_business",
            "local_work": "complete",
            "protected_gates": ["released_legacy_client_e2e"],
            "retirement_decision": "not_proposed",
            "production_behavior": "fail_closed_unavailable",
        },
    },
    (
        "server_action",
        "ACTION",
        "action://apps/web/actions/developers/add-domain.ts#addDeveloperDomain",
    ): {
        "id": "cap-v1-aa86dd3d5351ec06",
        "source_path": "apps/web/actions/developers/add-domain.ts",
        "source_sha256": "d25987a9c3a0eb4df30576e9e1a1ca21b96876bfb16d9e152f75a96225ee795f",
        "extra_sources": DEVELOPER_ADD_DOMAIN_RUNTIME_SOURCES,
        "local_status": "rust_exact_developer_add_domain_local_contract_released_client_e2e_pending",
        "rust_authority": "frame-application exact developer action + control-plane atomic D1 browser ingress",
        "auth": "session",
        "policy": "developer_api.v1",
        "completion": {
            "decision": "serve_frame_exact_business",
            "local_work": "complete",
            "protected_gates": ["released_legacy_client_e2e"],
            "retirement_decision": "not_proposed",
            "production_behavior": "fail_closed_unavailable",
        },
    },
    (
        "server_action",
        "ACTION",
        "action://apps/web/actions/developers/remove-domain.ts#removeDeveloperDomain",
    ): {
        "id": "cap-v1-f7d8036af53d0eb9",
        "source_path": "apps/web/actions/developers/remove-domain.ts",
        "source_sha256": "7e50b46b02a212315ed60ce357ceba12356cbb05e6193d04c61abc745ddfddee",
        "extra_sources": DEVELOPER_REMOVE_DOMAIN_RUNTIME_SOURCES,
        "local_status": "rust_exact_developer_remove_domain_local_contract_released_client_e2e_pending",
        "rust_authority": "frame-application exact developer action + control-plane atomic D1 browser ingress",
        "auth": "session",
        "policy": "developer_api.v1",
        "completion": {
            "decision": "serve_frame_exact_business",
            "local_work": "complete",
            "protected_gates": ["released_legacy_client_e2e"],
            "retirement_decision": "not_proposed",
            "production_behavior": "fail_closed_unavailable",
        },
    },
    (
        "server_action",
        "ACTION",
        "action://apps/web/actions/developers/regenerate-keys.ts#regenerateDeveloperKeys",
    ): {
        "id": "cap-v1-1f1465957551f1c4",
        "source_path": "apps/web/actions/developers/regenerate-keys.ts",
        "source_sha256": "a64dcc1684ef8327f2d953590e307c83fcf0cd23fc5a604233878bca4e0c46c4",
        "extra_sources": DEVELOPER_KEYS_RUNTIME_SOURCES,
        "local_status": "rust_exact_developer_regenerate_keys_local_contract_released_client_e2e_pending",
        "rust_authority": "frame-application exact developer action + control-plane atomic D1 browser ingress",
        "auth": "session",
        "policy": "developer_api.v1",
        "completion": {
            "decision": "serve_frame_exact_business",
            "local_work": "complete",
            "protected_gates": ["released_legacy_client_e2e"],
            "retirement_decision": "not_proposed",
            "production_behavior": "fail_closed_unavailable",
        },
    },
    (
        "server_action",
        "ACTION",
        "action://apps/web/actions/developers/delete-video.ts#deleteDeveloperVideo",
    ): {
        "id": "cap-v1-8328214ed9647abb",
        "source_path": "apps/web/actions/developers/delete-video.ts",
        "source_sha256": "63d75809a7be974610908e70aee859173d645c64ed997c24889e4b132425fe16",
        "extra_sources": DEVELOPER_VIDEO_RUNTIME_SOURCES,
        "local_status": "rust_exact_developer_delete_video_local_contract_released_client_e2e_pending",
        "rust_authority": "frame-application exact developer action + control-plane atomic D1 browser ingress",
        "auth": "session",
        "policy": "developer_api.v1",
        "completion": {
            "decision": "serve_frame_exact_business",
            "local_work": "complete",
            "protected_gates": ["released_legacy_client_e2e"],
            "retirement_decision": "not_proposed",
            "production_behavior": "fail_closed_unavailable",
        },
    },
    (
        "server_action",
        "ACTION",
        "action://apps/web/actions/developers/update-auto-topup.ts#updateDeveloperAutoTopUp",
    ): {
        "id": "cap-v1-b822700b545118f6",
        "source_path": "apps/web/actions/developers/update-auto-topup.ts",
        "source_sha256": "9e6882d1de03d4418ab45286f7c2d0b5bb17f073062955627e884a0774967420",
        "extra_sources": DEVELOPER_AUTO_TOP_UP_RUNTIME_SOURCES,
        "local_status": "rust_exact_developer_auto_top_up_local_contract_released_client_e2e_pending",
        "rust_authority": "frame-application exact developer action + control-plane atomic D1 browser ingress",
        "auth": "session",
        "policy": "developer_api.v1",
        "completion": {
            "decision": "serve_frame_exact_business",
            "local_work": "complete",
            "protected_gates": ["released_legacy_client_e2e"],
            "retirement_decision": "not_proposed",
            "production_behavior": "fail_closed_unavailable",
        },
    },
    (
        "server_action",
        "ACTION",
        "action://apps/web/actions/organization/remove-invite.ts#removeOrganizationInvite",
    ): {
        "id": "cap-v1-866dbe8fbbfd7887",
        "source_path": "apps/web/actions/organization/remove-invite.ts",
        "source_sha256": "614aed36f22c5187b7ac27d0367b6c5467da1a87f30d83ea2b05582f14d7a5b0",
        "extra_sources": MEMBERSHIP_REMOVE_INVITE_RUNTIME_SOURCES,
        "local_status": "rust_exact_membership_remove_invite_local_contract_released_client_e2e_pending",
        "rust_authority": "frame-application exact membership action + control-plane atomic D1 browser ingress",
        "auth": "session",
        "policy": "organization_library.v1",
        "completion": {
            "decision": "serve_frame_exact_business",
            "local_work": "complete",
            "protected_gates": ["released_legacy_client_e2e"],
            "retirement_decision": "not_proposed",
            "production_behavior": "fail_closed_unavailable",
        },
    },
    (
        "server_action",
        "ACTION",
        "action://apps/web/app/(org)/dashboard/spaces/[spaceId]/actions.ts#addSpaceMember",
    ): {
        "id": "cap-v1-455046db3d6ef019",
        "source_path": "apps/web/app/(org)/dashboard/spaces/[spaceId]/actions.ts",
        "source_sha256": "e8d738b63989d18c47cad13309de6728080df7a943b53b10fd45f19c05420745",
        "extra_sources": MEMBERSHIP_SPACE_RUNTIME_SOURCES,
        "local_status": "rust_exact_membership_add_space_member_local_contract_released_client_e2e_pending",
        "rust_authority": "frame-application exact membership action + control-plane atomic D1 browser ingress",
        "auth": "session",
        "policy": "organization_library.v1",
        "completion": {
            "decision": "serve_frame_exact_business",
            "local_work": "complete",
            "protected_gates": ["released_legacy_client_e2e"],
            "retirement_decision": "not_proposed",
            "production_behavior": "fail_closed_unavailable",
        },
    },
    (
        "server_action",
        "ACTION",
        "action://apps/web/app/(org)/dashboard/spaces/[spaceId]/actions.ts#setSpaceMembers",
    ): {
        "id": "cap-v1-9fc80bdec80fb248",
        "source_path": "apps/web/app/(org)/dashboard/spaces/[spaceId]/actions.ts",
        "source_sha256": "e8d738b63989d18c47cad13309de6728080df7a943b53b10fd45f19c05420745",
        "extra_sources": MEMBERSHIP_SET_SPACE_RUNTIME_SOURCES,
        "local_status": "rust_exact_membership_set_space_members_local_contract_released_client_e2e_pending",
        "rust_authority": "frame-application exact membership action + control-plane atomic D1 browser ingress",
        "auth": "session",
        "policy": "organization_library.v1",
        "completion": {
            "decision": "serve_frame_exact_business",
            "local_work": "complete",
            "protected_gates": ["released_legacy_client_e2e"],
            "retirement_decision": "not_proposed",
            "production_behavior": "fail_closed_unavailable",
        },
    },
    (
        "server_action",
        "ACTION",
        "action://apps/web/app/(org)/dashboard/spaces/[spaceId]/actions.ts#addSpaceMembers",
    ): {
        "id": "cap-v1-b177854e2386c877",
        "source_path": "apps/web/app/(org)/dashboard/spaces/[spaceId]/actions.ts",
        "source_sha256": "e8d738b63989d18c47cad13309de6728080df7a943b53b10fd45f19c05420745",
        "extra_sources": MEMBERSHIP_SPACE_RUNTIME_SOURCES,
        "local_status": "rust_exact_membership_add_space_members_local_contract",
        "rust_authority": "frame-application exact membership action + control-plane atomic D1 browser ingress",
        "auth": "session",
        "policy": "organization_library.v1",
        "completion": {
            "decision": "serve_frame_exact_business",
            "local_work": "complete",
            "protected_gates": [],
            "retirement_decision": "not_proposed",
            "production_behavior": "serve_exact_action",
        },
    },
    (
        "server_action",
        "ACTION",
        "action://apps/web/app/(org)/dashboard/spaces/[spaceId]/actions.ts#batchRemoveSpaceMembers",
    ): {
        "id": "cap-v1-38aff8e7221d0260",
        "source_path": "apps/web/app/(org)/dashboard/spaces/[spaceId]/actions.ts",
        "source_sha256": "e8d738b63989d18c47cad13309de6728080df7a943b53b10fd45f19c05420745",
        "extra_sources": MEMBERSHIP_SPACE_RUNTIME_SOURCES,
        "local_status": "rust_exact_membership_batch_remove_space_members_local_contract",
        "rust_authority": "frame-application exact membership action + control-plane atomic D1 browser ingress",
        "auth": "session",
        "policy": "organization_library.v1",
        "completion": {
            "decision": "serve_frame_exact_business",
            "local_work": "complete",
            "protected_gates": [],
            "retirement_decision": "not_proposed",
            "production_behavior": "serve_exact_action",
        },
    },
    (
        "server_action",
        "ACTION",
        "action://apps/web/app/(org)/dashboard/spaces/[spaceId]/actions.ts#removeSpaceMember",
    ): {
        "id": "cap-v1-135614e516c47bf4",
        "source_path": "apps/web/app/(org)/dashboard/spaces/[spaceId]/actions.ts",
        "source_sha256": "e8d738b63989d18c47cad13309de6728080df7a943b53b10fd45f19c05420745",
        "extra_sources": MEMBERSHIP_SPACE_RUNTIME_SOURCES,
        "local_status": "rust_exact_membership_remove_space_member_local_contract",
        "rust_authority": "frame-application exact membership action + control-plane atomic D1 browser ingress",
        "auth": "session",
        "policy": "organization_library.v1",
        "completion": {
            "decision": "serve_frame_exact_business",
            "local_work": "complete",
            "protected_gates": [],
            "retirement_decision": "not_proposed",
            "production_behavior": "serve_exact_action",
        },
    },
    ("route", "POST", "/api/settings/user/name"): {
        "id": "cap-v1-fdc3d5d49bb5ad6d",
        "source_path": "apps/web/app/api/settings/user/name/route.ts",
        "source_sha256": "0185e704e578084d1b1ab63b012a26cda5f0e64af098ba1ccf39cb33dadeefd6",
        "extra_sources": USER_ACCOUNT_LOCAL_RUNTIME_SOURCES,
        "local_status": "rust_exact_user_name_route_local_contract",
        "rust_authority": "frame-application exact user/account contract + control-plane atomic D1 ingress",
        "auth": "session",
        "policy": "organization_library.v1",
        "completion": {
            "decision": "serve_frame_exact_business",
            "local_work": "complete",
            "protected_gates": [],
            "retirement_decision": "not_proposed",
            "production_behavior": "serve_exact_d1",
        },
    },
    ("rpc", "RPC", "/api/erpc#UserCompleteOnboardingStep"): {
        "id": "cap-v1-c7827a1de563f856",
        "source_path": "apps/web/app/api/erpc/route.ts",
        "source_sha256": "01a2dee0518e44fe6137513f117100e6a626b904e4ee4608fc0be6d69e210783",
        "extra_sources": USER_ONBOARDING_RUNTIME_SOURCES,
        "local_status": "rust_exact_user_onboarding_local_contract_provider_execution_pending",
        "rust_authority": "frame-application exact user/account contract + control-plane atomic D1 and R2-gated ingress",
        "auth": "session",
        "policy": "organization_library.v1",
        "completion": {
            "decision": "serve_frame_exact_business",
            "local_work": "complete",
            "protected_gates": ["provider_execution"],
            "retirement_decision": "not_proposed",
            "production_behavior": "fail_closed_unavailable",
        },
    },
    ("rpc", "RPC", "/api/erpc#UserUpdate"): {
        "id": "cap-v1-295a3eb4ba9ffe6f",
        "source_path": "apps/web/app/api/erpc/route.ts",
        "source_sha256": "01a2dee0518e44fe6137513f117100e6a626b904e4ee4608fc0be6d69e210783",
        "extra_sources": USER_UPDATE_RUNTIME_SOURCES,
        "local_status": "rust_exact_user_update_local_contract_provider_execution_pending",
        "rust_authority": "frame-application exact user/account contract + control-plane atomic D1 and R2-gated ingress",
        "auth": "session",
        "policy": "organization_library.v1",
        "completion": {
            "decision": "serve_frame_exact_business",
            "local_work": "complete",
            "protected_gates": ["provider_execution"],
            "retirement_decision": "not_proposed",
            "production_behavior": "fail_closed_unavailable",
        },
    },
    (
        "server_action",
        "ACTION",
        "action://apps/web/app/(org)/dashboard/settings/account/server.ts#patchAccountSettings",
    ): {
        "id": "cap-v1-fdf4d6473b7f6608",
        "source_path": "apps/web/app/(org)/dashboard/settings/account/server.ts",
        "source_sha256": "87980903a1f08bc10d826529aee964db5fdc8832cdf58af8aac4aec8d63e3d7c",
        "extra_sources": USER_ACCOUNT_LOCAL_RUNTIME_SOURCES,
        "local_status": "rust_exact_account_patch_local_contract",
        "rust_authority": "frame-application exact user/account action + control-plane atomic D1 browser ingress",
        "auth": "session",
        "policy": "organization_library.v1",
        "completion": {
            "decision": "serve_frame_exact_business",
            "local_work": "complete",
            "protected_gates": [],
            "retirement_decision": "not_proposed",
            "production_behavior": "serve_exact_action",
        },
    },
    (
        "server_action",
        "ACTION",
        "action://apps/web/app/(org)/dashboard/settings/account/server.ts#signOutAllDevices",
    ): {
        "id": "cap-v1-c067d69850110640",
        "source_path": "apps/web/app/(org)/dashboard/settings/account/server.ts",
        "source_sha256": "87980903a1f08bc10d826529aee964db5fdc8832cdf58af8aac4aec8d63e3d7c",
        "extra_sources": USER_ACCOUNT_LOCAL_RUNTIME_SOURCES,
        "local_status": "rust_exact_account_sign_out_all_local_contract",
        "rust_authority": "frame-application exact user/account action + control-plane atomic D1 browser ingress",
        "auth": "session",
        "policy": "organization_library.v1",
        "completion": {
            "decision": "serve_frame_exact_business",
            "local_work": "complete",
            "protected_gates": [],
            "retirement_decision": "not_proposed",
            "production_behavior": "serve_exact_action",
        },
    },
    (
        "server_action",
        "ACTION",
        "action://apps/web/app/Layout/devtoolsServer.ts#demoteFromPro",
    ): {
        "id": "cap-v1-3d28eb7593bd4b1e",
        "source_path": "apps/web/app/Layout/devtoolsServer.ts",
        "source_sha256": "04b103a4435195608fbe7e6476b5b486ea114530da073d4db553351a76d18343",
        "extra_sources": USER_ACCOUNT_LOCAL_RUNTIME_SOURCES,
        "local_status": "rust_exact_account_demote_devtool_local_contract_human_approval_pending",
        "rust_authority": "frame-application exact user/account devtool + control-plane atomic D1 browser ingress",
        "auth": "session",
        "policy": "organization_library.v1",
        "completion": {
            "decision": "serve_frame_exact_business",
            "local_work": "complete",
            "protected_gates": ["human_approval"],
            "retirement_decision": "not_proposed",
            "production_behavior": "fail_closed_unavailable",
        },
    },
    (
        "server_action",
        "ACTION",
        "action://apps/web/app/Layout/devtoolsServer.ts#promoteToPro",
    ): {
        "id": "cap-v1-e0040a01322ea19e",
        "source_path": "apps/web/app/Layout/devtoolsServer.ts",
        "source_sha256": "04b103a4435195608fbe7e6476b5b486ea114530da073d4db553351a76d18343",
        "extra_sources": USER_ACCOUNT_LOCAL_RUNTIME_SOURCES,
        "local_status": "rust_exact_account_promote_devtool_local_contract_human_approval_pending",
        "rust_authority": "frame-application exact user/account devtool + control-plane atomic D1 browser ingress",
        "auth": "session",
        "policy": "organization_library.v1",
        "completion": {
            "decision": "serve_frame_exact_business",
            "local_work": "complete",
            "protected_gates": ["human_approval"],
            "retirement_decision": "not_proposed",
            "production_behavior": "fail_closed_unavailable",
        },
    },
    (
        "server_action",
        "ACTION",
        "action://apps/web/app/Layout/devtoolsServer.ts#restartOnboarding",
    ): {
        "id": "cap-v1-859bad07650343aa",
        "source_path": "apps/web/app/Layout/devtoolsServer.ts",
        "source_sha256": "04b103a4435195608fbe7e6476b5b486ea114530da073d4db553351a76d18343",
        "extra_sources": USER_ACCOUNT_LOCAL_RUNTIME_SOURCES,
        "local_status": "rust_exact_account_restart_onboarding_devtool_local_contract_human_approval_pending",
        "rust_authority": "frame-application exact user/account devtool + control-plane atomic D1 browser ingress",
        "auth": "session",
        "policy": "organization_library.v1",
        "completion": {
            "decision": "serve_frame_exact_business",
            "local_work": "complete",
            "protected_gates": ["human_approval"],
            "retirement_decision": "not_proposed",
            "production_behavior": "fail_closed_unavailable",
        },
    },
    (
        "server_action",
        "ACTION",
        "action://apps/web/actions/folders/get-folder-videos.ts#getFolderVideoIds",
    ): {
        "id": "cap-v1-b1027c7caafb92e2",
        "source_path": "apps/web/actions/folders/get-folder-videos.ts",
        "source_sha256": "ff36b7e0c86d6dbb44096b342c2beaf8d3b50e31924c6ac7b41681b7e2f47d43",
        "extra_sources": LIBRARY_ID_READ_FOLDER_RUNTIME_SOURCES,
        "local_status": "rust_exact_folder_video_ids_read_local_contract",
        "rust_authority": "frame-application exact library ID read + control-plane tenant-scoped D1 browser ingress",
        "auth": "session",
        "policy": "organization_library.v1",
        "completion": {
            "decision": "serve_frame_exact_business",
            "local_work": "complete",
            "protected_gates": [],
            "retirement_decision": "not_proposed",
            "production_behavior": "serve_exact_action",
        },
    },
    (
        "server_action",
        "ACTION",
        "action://apps/web/actions/organizations/get-organization-videos.ts#getOrganizationVideoIds",
    ): {
        "id": "cap-v1-cc52545598164806",
        "source_path": "apps/web/actions/organizations/get-organization-videos.ts",
        "source_sha256": "96bf7670e2f5b8664c4cfc31b71faf9f10a3558f0460d3ffbd3b5f81c70b16d0",
        "extra_sources": LIBRARY_ID_READ_ORGANIZATION_RUNTIME_SOURCES,
        "local_status": "rust_exact_organization_video_ids_read_local_contract",
        "rust_authority": "frame-application exact library ID read + control-plane tenant-scoped D1 browser ingress",
        "auth": "session",
        "policy": "organization_library.v1",
        "completion": {
            "decision": "serve_frame_exact_business",
            "local_work": "complete",
            "protected_gates": [],
            "retirement_decision": "not_proposed",
            "production_behavior": "serve_exact_action",
        },
    },
    (
        "server_action",
        "ACTION",
        "action://apps/web/actions/spaces/get-space-videos.ts#getSpaceVideoIds",
    ): {
        "id": "cap-v1-a8ace95c6ab712f6",
        "source_path": "apps/web/actions/spaces/get-space-videos.ts",
        "source_sha256": "a1968a5dbf067c86a8146df3240cb8d44ce120c0508324fb44fcc82d698c7da0",
        "extra_sources": LIBRARY_ID_READ_SPACE_RUNTIME_SOURCES,
        "local_status": "rust_exact_space_video_ids_read_local_contract",
        "rust_authority": "frame-application exact library ID read + control-plane tenant-scoped D1 browser ingress",
        "auth": "session",
        "policy": "organization_library.v1",
        "completion": {
            "decision": "serve_frame_exact_business",
            "local_work": "complete",
            "protected_gates": [],
            "retirement_decision": "not_proposed",
            "production_behavior": "serve_exact_action",
        },
    },
    (
        "server_action",
        "ACTION",
        "action://apps/web/actions/spaces/get-user-videos.ts#getUserVideos",
    ): {
        "id": "cap-v1-17a71c3e18600d06",
        "source_path": "apps/web/actions/spaces/get-user-videos.ts",
        "source_sha256": "c6607b999cc7ed0bc687d94fb791c55c2a97c0a6142c9ba60b977aac05d80a5e",
        "extra_sources": LIBRARY_DETAIL_GET_USER_VIDEOS_RUNTIME_SOURCES,
        "local_status": "rust_exact_user_video_detail_read_local_contract",
        "rust_authority": "frame-application exact library detail semantics + control-plane tenant-scoped D1 browser ingress",
        "auth": "session",
        "policy": "organization_library.v1",
        "completion": {
            "decision": "serve_frame_exact_business",
            "local_work": "complete",
            "protected_gates": [],
            "retirement_decision": "not_proposed",
            "production_behavior": "serve_exact_action",
        },
    },
    (
        "server_action",
        "ACTION",
        "action://apps/web/app/(org)/dashboard/_components/Navbar/search.ts#searchDashboardVideos",
    ): {
        "id": "cap-v1-39e8966f308c1528",
        "source_path": "apps/web/app/(org)/dashboard/_components/Navbar/search.ts",
        "source_sha256": "210244ffd7180d957960f27f1d7b7f420bf301daf29dc2d389fa51125fd2c44f",
        "extra_sources": LIBRARY_DETAIL_SEARCH_RUNTIME_SOURCES,
        "local_status": "rust_exact_dashboard_video_search_local_contract",
        "rust_authority": "frame-application exact dashboard search semantics + control-plane tenant-scoped D1 browser ingress",
        "auth": "session",
        "policy": "organization_library.v1",
        "completion": {
            "decision": "serve_frame_exact_business",
            "local_work": "complete",
            "protected_gates": [],
            "retirement_decision": "not_proposed",
            "production_behavior": "serve_exact_action",
        },
    },
    (
        "server_action",
        "ACTION",
        "action://apps/web/actions/organization/space-authorization.ts#getSpaceAccess",
    ): {
        "id": "cap-v1-5595a9d384765e76",
        "source_path": "apps/web/actions/organization/space-authorization.ts",
        "source_sha256": "2a656f25f7c73f2342104127d818a56fffd7d05768d787489b65e08f70a43445",
        "extra_sources": SPACE_ACCESS_ROLE_RUNTIME_SOURCES,
        "local_status": "rust_exact_space_access_read_local_contract",
        "rust_authority": "frame-application exact space role semantics + control-plane tenant-scoped D1 browser ingress",
        "auth": "session",
        "policy": "organization_library.v1",
        "completion": {
            "decision": "serve_frame_exact_business",
            "local_work": "complete",
            "protected_gates": [],
            "retirement_decision": "not_proposed",
            "production_behavior": "serve_exact_action",
        },
    },
    (
        "server_action",
        "ACTION",
        "action://apps/web/actions/organization/space-authorization.ts#requireSpaceManager",
    ): {
        "id": "cap-v1-14cb48febfd0fa5a",
        "source_path": "apps/web/actions/organization/space-authorization.ts",
        "source_sha256": "2a656f25f7c73f2342104127d818a56fffd7d05768d787489b65e08f70a43445",
        "extra_sources": SPACE_MANAGER_ROLE_RUNTIME_SOURCES,
        "local_status": "rust_exact_require_space_manager_read_local_contract",
        "rust_authority": "frame-application exact space role semantics + control-plane tenant-scoped D1 browser ingress",
        "auth": "session",
        "policy": "organization_library.v1",
        "completion": {
            "decision": "serve_frame_exact_business",
            "local_work": "complete",
            "protected_gates": [],
            "retirement_decision": "not_proposed",
            "production_behavior": "serve_exact_action",
        },
    },
    ("route", "PATCH", "/api/mobile/caps/:id/password"): {
        "id": "cap-v1-2cfe7fc40a6f5a78",
        "source_path": "apps/web/app/api/mobile/[...route]/route.ts",
        "source_sha256": "02df2ce92dc6e8ae11748b6e082c1304596ba9e4c370b35069867754218f5f79",
        "extra_sources": (
            VIDEO_PROPERTIES_SCHEMA_SOURCE,
            VIDEO_PROPERTIES_CRYPTO_SOURCE,
            VIDEO_PROPERTIES_PROVIDER_READ_SOURCE,
        ),
        "local_status": "rust_exact_mobile_video_password_d1_local_contract_provider_pending",
        "rust_authority": "frame-application exact video-property semantics + control-plane atomic D1 mobile ingress",
        "auth": "session_or_api_key",
        "policy": "share_playback.v1",
        "completion": VIDEO_PROPERTIES_PROVIDER_COMPLETION,
    },
    ("route", "PATCH", "/api/mobile/caps/:id/sharing"): {
        "id": "cap-v1-5fdf332d1448aedc",
        "source_path": "apps/web/app/api/mobile/[...route]/route.ts",
        "source_sha256": "02df2ce92dc6e8ae11748b6e082c1304596ba9e4c370b35069867754218f5f79",
        "extra_sources": (
            VIDEO_PROPERTIES_SCHEMA_SOURCE,
            VIDEO_PROPERTIES_PROVIDER_READ_SOURCE,
        ),
        "local_status": "rust_exact_mobile_video_sharing_d1_local_contract_provider_pending",
        "rust_authority": "frame-application exact video-property semantics + control-plane atomic D1 mobile ingress",
        "auth": "session_or_api_key",
        "policy": "client_compatibility.v1",
        "completion": VIDEO_PROPERTIES_PROVIDER_COMPLETION,
    },
    ("route", "PATCH", "/api/mobile/caps/:id/title"): {
        "id": "cap-v1-b2db0e7ec51f7898",
        "source_path": "apps/web/app/api/mobile/[...route]/route.ts",
        "source_sha256": "02df2ce92dc6e8ae11748b6e082c1304596ba9e4c370b35069867754218f5f79",
        "extra_sources": (
            VIDEO_PROPERTIES_SCHEMA_SOURCE,
            VIDEO_PROPERTIES_PROVIDER_READ_SOURCE,
        ),
        "local_status": "rust_exact_mobile_video_title_d1_local_contract_provider_pending",
        "rust_authority": "frame-application exact video-property semantics + control-plane atomic D1 mobile ingress",
        "auth": "session_or_api_key",
        "policy": "client_compatibility.v1",
        "completion": VIDEO_PROPERTIES_PROVIDER_COMPLETION,
    },
    ("route", "PUT", "/api/video/metadata"): {
        "id": "cap-v1-5b36dac105856ede",
        "source_path": "apps/web/app/api/video/metadata/route.ts",
        "source_sha256": "cbd25bc1150aa53dea5f5b8a120c899c36b151134af307f89992108e81b17812",
        "extra_sources": (
            VIDEO_PROPERTIES_SESSION_SOURCE,
            VIDEO_PROPERTIES_SCHEMA_SOURCE,
        ),
        "local_status": "rust_exact_video_metadata_replace_d1_local_contract",
        "rust_authority": "frame-application exact video-property semantics + control-plane atomic D1 browser ingress",
        "auth": "session",
        "policy": "video_media.v1",
        "completion": VIDEO_PROPERTIES_D1_COMPLETION,
    },
    (
        "server_action",
        "ACTION",
        "action://apps/web/actions/videos/edit-date.ts#editDate",
    ): {
        "id": "cap-v1-96c52e9330f9a131",
        "source_path": "apps/web/actions/videos/edit-date.ts",
        "source_sha256": "f54229f4aed3648988529a310a0cb831c530aac54177766312abe029d8326d78",
        "extra_sources": (
            VIDEO_PROPERTIES_SESSION_SOURCE,
            VIDEO_PROPERTIES_SCHEMA_SOURCE,
            VIDEO_PROPERTIES_DATE_METADATA_SOURCE,
        ),
        "local_status": "rust_exact_video_edit_date_action_local_contract_human_approval_pending",
        "rust_authority": "frame-application exact video-property semantics + control-plane atomic D1 browser ingress",
        "auth": "session",
        "policy": "video_media.v1",
        "completion": VIDEO_PROPERTIES_HUMAN_COMPLETION,
    },
    (
        "server_action",
        "ACTION",
        "action://apps/web/actions/videos/edit-title.ts#editTitle",
    ): {
        "id": "cap-v1-6e9f3d370f1ce239",
        "source_path": "apps/web/actions/videos/edit-title.ts",
        "source_sha256": "7991386a504054dbafa40b46ff46a1f0fa11791b36adf503623fc17a55a6ecf8",
        "extra_sources": (
            VIDEO_PROPERTIES_SESSION_SOURCE,
            VIDEO_PROPERTIES_SCHEMA_SOURCE,
            VIDEO_PROPERTIES_TITLE_METADATA_SOURCE,
        ),
        "local_status": "rust_exact_video_edit_title_action_local_contract",
        "rust_authority": "frame-application exact video-property semantics + control-plane atomic D1 browser ingress",
        "auth": "session",
        "policy": "video_media.v1",
        "completion": VIDEO_PROPERTIES_ACTION_COMPLETION,
    },
    (
        "server_action",
        "ACTION",
        "action://apps/web/actions/videos/password.ts#removeVideoPassword",
    ): {
        "id": "cap-v1-ab11637faa2de45e",
        "source_path": "apps/web/actions/videos/password.ts",
        "source_sha256": "13a240f004a307bba1e0b66b4341036dcab941aa1037ae16dfdcbfcbd485b119",
        "extra_sources": (
            VIDEO_PROPERTIES_SESSION_SOURCE,
            VIDEO_PROPERTIES_SCHEMA_SOURCE,
        ),
        "local_status": "rust_exact_video_remove_password_action_local_contract",
        "rust_authority": "frame-application exact video-property semantics + control-plane atomic D1 browser ingress",
        "auth": "session",
        "policy": "share_playback.v1",
        "completion": VIDEO_PROPERTIES_ACTION_COMPLETION,
    },
    (
        "server_action",
        "ACTION",
        "action://apps/web/actions/videos/password.ts#setVideoPassword",
    ): {
        "id": "cap-v1-455e6a1b82e647d9",
        "source_path": "apps/web/actions/videos/password.ts",
        "source_sha256": "13a240f004a307bba1e0b66b4341036dcab941aa1037ae16dfdcbfcbd485b119",
        "extra_sources": (
            VIDEO_PROPERTIES_SESSION_SOURCE,
            VIDEO_PROPERTIES_SCHEMA_SOURCE,
            VIDEO_PROPERTIES_CRYPTO_SOURCE,
        ),
        "local_status": "rust_exact_video_set_password_action_local_contract",
        "rust_authority": "frame-application exact video-property semantics + control-plane atomic D1 browser ingress",
        "auth": "session",
        "policy": "share_playback.v1",
        "completion": VIDEO_PROPERTIES_ACTION_COMPLETION,
    },
    (
        "server_action",
        "ACTION",
        "action://apps/web/actions/videos/password.ts#verifyVideoPassword",
    ): {
        "id": "cap-v1-0a2c44d7a626a1fe",
        "source_path": "apps/web/actions/videos/password.ts",
        "source_sha256": "13a240f004a307bba1e0b66b4341036dcab941aa1037ae16dfdcbfcbd485b119",
        "extra_sources": (
            VIDEO_PROPERTIES_SCHEMA_SOURCE,
            VIDEO_PROPERTIES_CRYPTO_SOURCE,
            VIDEO_PROPERTIES_EFFECTIVE_RULES_SOURCE,
            VIDEO_PROPERTIES_PASSWORD_COOKIE_SOURCE,
        ),
        "local_status": "rust_exact_anonymous_video_password_verify_action_local_contract",
        "rust_authority": "frame-application exact video-property semantics + control-plane atomic D1 browser ingress and encrypted cookie carrier",
        "auth": "anonymous",
        "policy": "share_playback.v1",
        "completion": VIDEO_PROPERTIES_ACTION_COMPLETION,
    },
    (
        "server_action",
        "ACTION",
        "action://apps/web/actions/videos/settings.ts#updateVideoSettings",
    ): {
        "id": "cap-v1-49dba3fbc7c4a74c",
        "source_path": "apps/web/actions/videos/settings.ts",
        "source_sha256": "c6dcfca09bcde824b071c56432124ccec9fa2c690b528afd131284d69c0bf78c",
        "extra_sources": (
            VIDEO_PROPERTIES_SESSION_SOURCE,
            VIDEO_PROPERTIES_SCHEMA_SOURCE,
            VIDEO_PROPERTIES_PLAYBACK_SPEED_SOURCE,
        ),
        "local_status": "rust_exact_video_settings_action_local_contract",
        "rust_authority": "frame-application exact video-property semantics + control-plane atomic D1 browser ingress",
        "auth": "session",
        "policy": "video_media.v1",
        "completion": VIDEO_PROPERTIES_ACTION_COMPLETION,
    },
}


def adapter_source_identities(adapter: dict[str, Any]) -> list[tuple[str, str]]:
    return [
        (adapter["source_path"], adapter["source_sha256"]),
        *[
            (source["path"], source["sha256"])
            for source in adapter.get("extra_sources", ())
        ],
    ]


def sha256_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def stable_operation_id(row: dict[str, Any]) -> str:
    identity = "\0".join((row["kind"], row["method"], row["legacy_path"]))
    return f"cap-v1-{sha256_bytes(identity.encode())[:16]}"


def completion_for(
    disposition: str,
    family: str,
    searchable: str,
    *,
    endpoint_promoted: bool,
    operation_id: str,
) -> dict[str, Any]:
    """Return the exact local/protected residual without hiding local adapter work."""
    if endpoint_promoted:
        return {
            "decision": "serve_frame_exact_static",
            "local_work": "complete",
            "protected_gates": [],
            "retirement_decision": "not_proposed",
            "production_behavior": "serve_exact_static",
        }

    if operation_id in DECLARATION_ONLY_OWNER_DECISION_IDS:
        decision = "declaration_only_owner_disposition_required"
        local_work = "complete"
        protected_gates = ["human_approval"]
        retirement_decision = "repository_owner_pending"
    elif operation_id in PROVIDER_BACKED_IDS:
        decision = "retain_replace_with_provider_effect"
        local_work = "exact_adapter_and_provider_effect_orchestration_required"
        protected_gates = ["provider_execution"]
        retirement_decision = "not_proposed"
    elif operation_id in MESSENGER_RETIREMENT_IDENTITIES:
        decision = "retirement_response_contract_ready"
        local_work = "complete"
        protected_gates = ["human_approval"]
        retirement_decision = "repository_owner_pending"
    elif disposition == "retire":
        decision = "retirement_proposed"
        local_work = "retirement_response_required"
        protected_gates = ["human_approval"]
        retirement_decision = "repository_owner_pending"
    elif disposition == "protected_parity_required":
        decision = "protected_parity"
        local_work = "exact_adapter_and_business_effect_required"
        protected_gates = ["human_approval", "provider_execution"]
        retirement_decision = "not_proposed"
    elif disposition == "migrate":
        decision = "migrate"
        local_work = "exact_adapter_and_business_effect_required"
        protected_gates = ["provider_execution"]
        retirement_decision = "not_proposed"
    elif "organisationsoftdelete" in searchable:
        decision = "retain_replace_with_provider_effect"
        local_work = "exact_adapter_and_provider_effect_orchestration_required"
        protected_gates = ["provider_execution"]
        retirement_decision = "not_proposed"
    else:
        decision = "retain_replace"
        local_work = "exact_adapter_and_business_effect_required"
        if family != "video_media":
            protected_gates = []
        elif "/api/webhooks/media-server/" in searchable:
            protected_gates = ["provider_execution"]
        elif "/media-server/" in searchable:
            protected_gates = ["hardware_execution"]
        elif any(
            marker in searchable
            for marker in (
                "desktop-segments",
                "generate-ai",
                "generateai",
                "processing",
                "retry-ai",
                "save-edits",
                "thumbnail",
                "transcrib",
                "/api/video/ai",
                "/api/video/preview",
                "workflow://",
            )
        ):
            protected_gates = ["hardware_execution", "provider_execution"]
        else:
            protected_gates = []
        retirement_decision = "not_proposed"
    return {
        "decision": decision,
        "local_work": local_work,
        "protected_gates": protected_gates,
        "retirement_decision": retirement_decision,
        "production_behavior": "fail_closed_unavailable",
    }


def source_ref(path: str, symbol: str) -> dict[str, str]:
    source = CAP / path
    return {
        "path": path,
        "symbol": symbol,
        "sha256": sha256_bytes(source.read_bytes()),
    }


def transitive_source_specs(row: dict[str, Any]) -> list[tuple[str, str]]:
    """Return implementation-bearing sources omitted by declaration extraction."""
    specs: list[tuple[str, str]] = []
    operation_id = stable_operation_id(row)
    source_paths = {source["path"] for source in row["sources"]}
    operations = row["operations"]

    if row["kind"] == "route" and MOBILE_DECLARATION_SOURCE in source_paths:
        specs.extend(
            (MOBILE_HANDLER_SOURCE, f"mobile handler:{operation}")
            for operation in operations
        )

    if row["kind"] == "rpc":
        specs.extend(EFFECT_RPC_TRANSPORT_SOURCES)
        for operation in operations:
            implementation_sources = EFFECT_RPC_IMPLEMENTATION_SOURCES.get(operation)
            if implementation_sources is None:
                raise RuntimeError(
                    f"Effect RPC operation lacks a pinned implementation graph: {operation}"
                )
            specs.extend(implementation_sources)

    if row["kind"] == "server_action":
        for operation in operations:
            specs.extend(ORGANIZATION_ACTION_TRANSITIVE_SOURCES.get(operation, ()))

    specs.extend(OPERATION_TRANSITIVE_SOURCE_SPECS.get(operation_id, ()))
    specs.extend(PROVIDER_BACKED_SOURCE_SPECS.get(operation_id, ()))

    return sorted(set(specs))


def add_transitive_source_refs(row: dict[str, Any]) -> None:
    known = {
        (source["path"], source["symbol"], source["sha256"])
        for source in row["sources"]
    }
    for path, symbol in transitive_source_specs(row):
        source = source_ref(path, symbol)
        identity = (source["path"], source["symbol"], source["sha256"])
        if identity not in known:
            row["sources"].append(source)
            known.add(identity)
    row["sources"].sort(key=lambda source: (source["path"], source["symbol"]))


def classification_searchable(row: dict[str, Any]) -> str:
    """Keep implementation-graph pins from accidentally changing route taxonomy."""
    transitive = set(transitive_source_specs(row))
    primary_source_paths = [
        source["path"]
        for source in row["sources"]
        if (source["path"], source["symbol"]) not in transitive
    ]
    return " ".join(
        [row["legacy_path"], *row["operations"], *primary_source_paths]
    ).lower()


def build_changelog_feed() -> tuple[str, list[dict[str, str]], str]:
    """Reproduce the pinned TypeScript frontmatter parser and JSON.stringify body."""
    changelog_root = CAP / "apps" / "web" / "content" / "changelog"
    posts: list[tuple[int, dict[str, str]]] = []
    sources: list[dict[str, str]] = []
    manifest = hashlib.sha256()
    for source in sorted(changelog_root.glob("*.mdx"), key=lambda item: int(item.stem)):
        if not source.stem.isdigit():
            raise RuntimeError(f"non-numeric changelog source: {source.name}")
        relative = source.relative_to(CAP).as_posix()
        source_digest = sha256_bytes(source.read_bytes())
        manifest.update(f"{relative}\0{source_digest}\n".encode())
        sources.append(source_ref(relative, f"changelog-post:{source.stem}"))

        file_content = source.read_text(encoding="utf-8")
        match = re.search(r"---\s*([\s\S]*?)\s*---", file_content)
        content = re.sub(
            r"---\s*([\s\S]*?)\s*---", "", file_content, count=1
        ).strip()
        metadata: dict[str, str] = {}
        if match is not None:
            for line in match.group(1).strip().split("\n"):
                key, *values = line.split(": ")
                if not key:
                    continue
                value = ": ".join(values).strip()
                value = re.sub(r'''^['\"](.*)['\"]$''', r"\1", value)
                metadata[key.strip()] = value
        metadata["content"] = content
        posts.append((int(source.stem), metadata))

    slugs = [slug for slug, _ in posts]
    if slugs != list(range(1, 100)):
        raise RuntimeError("pinned changelog sources are not the exact numeric 1..99 set")
    body = json.dumps(
        [metadata for _, metadata in sorted(posts, reverse=True)],
        ensure_ascii=False,
        separators=(",", ":"),
    )
    return body, sources, manifest.hexdigest()


def join_path(prefix: str, suffix: str) -> str:
    if suffix == "/":
        return prefix or "/"
    return f"{prefix.rstrip('/')}/{suffix.lstrip('/')}"


def filesystem_api_path(source: Path) -> str:
    relative = source.relative_to(CAP / "apps" / "web" / "app").parent
    parts: list[str] = []
    for part in relative.parts:
        catch_all = re.fullmatch(r"\[\.\.\.([^]]+)\]", part)
        optional_catch_all = re.fullmatch(r"\[\[\.\.\.([^]]+)\]\]", part)
        dynamic = re.fullmatch(r"\[([^]]+)\]", part)
        if catch_all or optional_catch_all:
            parts.append(f":{(catch_all or optional_catch_all).group(1)}*")
        elif dynamic:
            parts.append(f":{dynamic.group(1)}")
        else:
            parts.append(part)
    return "/" + "/".join(parts)


def collect_raw() -> list[dict[str, Any]]:
    if not CAP.is_dir():
        raise RuntimeError("the pinned .tmp/cap reference checkout is missing")

    rows: list[dict[str, Any]] = []

    # Next route handlers that are not catch-all transport wrappers.
    api_root = CAP / "apps" / "web" / "app" / "api"
    for source in sorted((*api_root.rglob("route.ts"), *api_root.rglob("route.tsx"))):
        relative = source.relative_to(CAP).as_posix()
        if relative in TRANSPORT_WRAPPERS:
            continue
        text = source.read_text(encoding="utf-8")
        methods = set(
            re.findall(
                r"export\s+(?:(?:async\s+)?function|const)\s+"
                r"(GET|POST|PUT|PATCH|DELETE|HEAD|OPTIONS)\b",
                text,
            )
        )
        for export_block in re.findall(r"export\s*\{([^}]+)\}", text, re.DOTALL):
            methods.update(
                re.findall(
                    r"\bas\s+(GET|POST|PUT|PATCH|DELETE|HEAD|OPTIONS)\b",
                    export_block,
                )
            )
        for method in sorted(methods):
            rows.append(
                raw_row(
                    "route",
                    method,
                    filesystem_api_path(source),
                    method,
                    source_ref(relative, method),
                )
            )

    # Mounted Hono services, including the legacy media-server control API.
    for relative, prefix, receivers in HONO_MOUNTS:
        source = CAP / relative
        text = source.read_text(encoding="utf-8")
        receiver_pattern = "|".join(re.escape(value) for value in receivers)
        pattern = re.compile(
            rf"(?:\b(?:{receiver_pattern})|new\s+Hono\(\))\s*\.\s*"
            r"(get|post|put|patch|delete|head|options)\s*\(\s*[\"']([^\"']+)[\"']",
            re.DOTALL,
        )
        for match in pattern.finditer(text):
            method = match.group(1).upper()
            path = join_path(prefix, match.group(2))
            rows.append(
                raw_row(
                    "route",
                    method,
                    path,
                    f"{match.group(1)}:{match.group(2)}",
                    source_ref(relative, f"{method} {match.group(2)}"),
                )
            )

    # Effect HttpApi definitions. Prefixes are applied where their host API mounts them.
    endpoint_pattern = re.compile(
        r"HttpApiEndpoint\.(get|post|del|put|patch)\(\s*"
        r"[\"']([^\"']+)[\"']\s*,\s*(?:[\"']([^\"']+)[\"']|`([^`]+)`)",
        re.DOTALL,
    )
    for relative, prefix in EFFECT_SOURCES:
        source = CAP / relative
        text = source.read_text(encoding="utf-8")
        for match in endpoint_pattern.finditer(text):
            method = {"del": "DELETE"}.get(match.group(1), match.group(1).upper())
            operation = match.group(2)
            endpoint = (match.group(3) or match.group(4)).replace(
                "${INSTANT_RECORDINGS_PATH}", "/instant-recordings"
            )
            rows.append(
                raw_row(
                    "route",
                    method,
                    join_path(prefix, endpoint),
                    operation,
                    source_ref(relative, operation),
                )
            )

    # The older ts-rest snapshot remains a supported-client contract oracle.
    # Merge it into the route identity rather than counting it as another route.
    ts_rest_pattern = re.compile(
        r"method:\s*[\"'](GET|POST|PUT|PATCH|DELETE|HEAD|OPTIONS)[\"']\s*,\s*"
        r"path:\s*[\"']([^\"']+)[\"']",
        re.DOTALL,
    )
    for relative in TS_REST_CONTRACT_SOURCES:
        text = (CAP / relative).read_text(encoding="utf-8")
        for method, endpoint in ts_rest_pattern.findall(text):
            rows.append(
                raw_row(
                    "route",
                    method,
                    join_path("/api", endpoint),
                    f"ts-rest:{method}:{endpoint}",
                    source_ref(relative, f"{method} {endpoint}"),
                )
            )

    # Extension paths are deliberately centralized as constants in the source,
    # so they cannot all be recovered from a literal-only HttpApi regex.
    extension_relative = "packages/web-domain/src/Extension.ts"
    for method, endpoint, operation in EXTENSION_ENDPOINTS:
        rows.append(
            raw_row(
                "route",
                method,
                join_path("/api/extension", endpoint),
                operation,
                source_ref(extension_relative, operation),
            )
        )

    # Next server actions. Files without `use server` are ordinary helpers.
    web_root = CAP / "apps" / "web"
    action_pattern = re.compile(
        r"export\s+(?:async\s+function\s+([A-Za-z_$][\w$]*)|"
        r"const\s+([A-Za-z_$][\w$]*)\s*=\s*async\b)"
    )
    for source in sorted((*web_root.rglob("*.ts"), *web_root.rglob("*.tsx"))):
        text = source.read_text(encoding="utf-8")
        if not re.search(r"[\"']use server[\"']", text):
            continue
        relative = source.relative_to(CAP).as_posix()
        for match in action_pattern.finditer(text):
            operation = match.group(1) or match.group(2)
            rows.append(
                raw_row(
                    "server_action",
                    "ACTION",
                    f"action://{relative}#{operation}",
                    operation,
                    source_ref(relative, operation),
                )
            )
        # An inline action is not exported, but Next still turns it into a
        # remotely invocable server action when the function body carries the directive.
        for operation in re.findall(
            r"async\s+function\s+([A-Za-z_$][\w$]*)\s*\([^)]*\)\s*\{\s*"
            r"[\"']use server[\"']",
            text,
            re.DOTALL,
        ):
            rows.append(
                raw_row(
                    "server_action",
                    "ACTION",
                    f"action://{relative}#{operation}",
                    operation,
                    source_ref(relative, operation),
                )
            )

    # Durable workflow entrypoints in the web workflow directory.
    workflows_root = CAP / "apps" / "web" / "workflows"
    workflow_pattern = re.compile(
        r"export\s+async\s+function\s+([A-Za-z_$][\w$]*Workflow)\b"
    )
    for source in sorted(workflows_root.rglob("*.ts")):
        text = source.read_text(encoding="utf-8")
        relative = source.relative_to(CAP).as_posix()
        for operation in workflow_pattern.findall(text):
            rows.append(
                raw_row(
                    "workflow",
                    "WORKFLOW",
                    f"workflow://{relative}#{operation}",
                    operation,
                    source_ref(relative, operation),
                )
            )

    for relative, operation in CURATED_WORKFLOW_ENTRYPOINTS:
        text = (CAP / relative).read_text(encoding="utf-8")
        if not re.search(
            rf"export\s+(?:async\s+function|const)\s+{re.escape(operation)}\b",
            text,
        ):
            raise RuntimeError(f"curated workflow entrypoint drifted: {relative}#{operation}")
        rows.append(
            raw_row(
                "workflow",
                "WORKFLOW",
                f"workflow://{relative}#{operation}",
                operation,
                source_ref(relative, operation),
            )
        )

    # Effect RPC operations carried over the /api/erpc transport.
    domain_root = CAP / "packages" / "web-domain" / "src"
    for source in sorted(domain_root.rglob("*.ts")):
        text = source.read_text(encoding="utf-8")
        relative = source.relative_to(CAP).as_posix()
        for operation in re.findall(r"Rpc\.make\(\s*[\"']([^\"']+)[\"']", text):
            rows.append(
                raw_row(
                    "rpc",
                    "RPC",
                    f"/api/erpc#{operation}",
                    operation,
                    source_ref(relative, operation),
                )
            )

    # Effect durable workflows outside apps/web/workflows.
    loom_relative = "packages/web-domain/src/Loom.ts"
    loom_text = (CAP / loom_relative).read_text(encoding="utf-8")
    for operation in re.findall(
        r"Workflow\.make\(\s*\{\s*name:\s*[\"']([^\"']+)[\"']", loom_text, re.DOTALL
    ):
        rows.append(
            raw_row(
                "workflow",
                "WORKFLOW",
                f"workflow://{loom_relative}#{operation}",
                operation,
                source_ref(loom_relative, operation),
            )
        )

    for row in rows:
        add_transitive_source_refs(row)

    return merge_routes(rows)


def raw_row(
    kind: str,
    method: str,
    legacy_path: str,
    operation: str,
    source: dict[str, str],
) -> dict[str, Any]:
    return {
        "kind": kind,
        "method": method,
        "legacy_path": legacy_path,
        "operations": [operation],
        "sources": [source],
    }


def merge_routes(rows: list[dict[str, Any]]) -> list[dict[str, Any]]:
    merged: dict[tuple[str, str, str], dict[str, Any]] = {}
    for row in rows:
        key = (row["kind"], row["method"], row["legacy_path"])
        if row["kind"] != "route" or key not in merged:
            if key in merged:
                raise RuntimeError(f"duplicate non-route inventory identity: {key}")
            merged[key] = row
            continue
        current = merged[key]
        current["operations"] = sorted(
            set(current["operations"]) | set(row["operations"])
        )
        known_sources = {
            (item["path"], item["symbol"], item["sha256"]) for item in current["sources"]
        }
        for source in row["sources"]:
            identity = (source["path"], source["symbol"], source["sha256"])
            if identity not in known_sources:
                current["sources"].append(source)
        current["sources"].sort(key=lambda item: (item["path"], item["symbol"]))
    return sorted(
        merged.values(),
        key=lambda row: (row["kind"], row["legacy_path"], row["method"]),
    )


def classify(row: dict[str, Any]) -> dict[str, Any]:
    identity_tuple = (row["kind"], row["method"], row["legacy_path"])
    adapter = LOCAL_ENDPOINT_ADAPTERS.get(identity_tuple)
    if adapter:
        if identity_tuple == ("route", "GET", "/api/changelog"):
            body, content_sources, source_manifest = build_changelog_feed()
            if (
                len(body.encode()) != CHANGELOG_FEED_BODY_BYTES
                or sha256_bytes(body.encode()) != CHANGELOG_FEED_BODY_SHA256
                or source_manifest != CHANGELOG_FEED_SOURCE_MANIFEST_SHA256
            ):
                raise RuntimeError("pinned changelog feed body or source manifest drifted")
            known_sources = {
                (source["path"], source["symbol"], source["sha256"])
                for source in row["sources"]
            }
            for source in content_sources:
                identity = (source["path"], source["symbol"], source["sha256"])
                if identity not in known_sources:
                    row["sources"].append(source)
        if identity_tuple == ("route", "GET", "/api/changelog/status"):
            changelog_root = CAP / "apps" / "web" / "content" / "changelog"
            numeric_posts = [
                (int(source.stem), source)
                for source in changelog_root.glob("*.mdx")
                if source.stem.isdigit()
            ]
            latest_slug, latest_source = max(numeric_posts)
            latest_text = latest_source.read_text(encoding="utf-8")
            if (
                latest_slug != 99
                or latest_source.name != "99.mdx"
                or not re.search(r"(?m)^version:\s*0\.5\.6\s*$", latest_text)
            ):
                raise RuntimeError("pinned changelog latest-version input drifted")
        for expected in adapter.get("extra_sources", ()):
            actual = source_ref(expected["path"], expected["symbol"])
            if actual != expected:
                raise RuntimeError(
                    f"local endpoint adapter source drifted: {expected['path']}"
                )
            if actual not in row["sources"]:
                row["sources"].append(actual)
        row["sources"].sort(key=lambda source: (source["path"], source["symbol"]))

    searchable = classification_searchable(row)
    path = row["legacy_path"].lower()
    method = row["method"]

    if "messenger" in searchable:
        family = "messenger_support"
    # Classify the authority before the transport. A Stripe callback and a
    # media workflow are not made low-risk merely because both use a webhook
    # or workflow wrapper.
    elif any(value in searchable for value in ("billing", "stripe", "checkout", "subscribe")):
        family = "billing_admin"
    elif "admin" in searchable:
        family = "billing_admin"
    elif any(value in searchable for value in ("loom", "import")):
        family = "imports_integrations"
    elif any(
        value in searchable
        for value in ("organization", "organisation", "space", "folder", "collection")
    ):
        family = "organization_library"
    elif any(
        value in searchable
        for value in (
            "/auth",
            "auth/",
            "startauth",
            "approveauth",
            "revokeauth",
            "oauth",
            "session",
            "invite",
        )
    ):
        family = "auth_session"
    elif any(
        value in searchable
        for value in ("upload", "storage", "s3", "google-drive", "download")
    ):
        family = "upload_storage"
    elif any(
        value in searchable
        for value in ("comment", "notification", "transcript", "reaction")
    ):
        family = "collaboration_notifications"
    elif any(value in searchable for value in ("share", "password", "playlist")):
        family = "share_playback"
    elif any(value in searchable for value in ("developer", "commercial", "credit", "usage")):
        family = "developer_api"
    elif "analytics" in searchable:
        family = "analytics_consent"
    elif any(
        value in searchable
        for value in (
            "video",
            "thumbnail",
            "audio",
            "transcrib",
            "recording",
            "media-server",
            "generateai",
            "generate-ai",
            "desktop-segments",
        )
    ):
        family = "video_media"
    elif any(value in searchable for value in ("webhook", "cron/", "workflow://")):
        family = "callbacks_webhooks_workflows"
    elif any(value in searchable for value in ("desktop", "mobile", "extension")):
        family = "client_compatibility"
    else:
        family = "service_misc"

    clients = clients_for(searchable, row["kind"])
    auth = auth_for(path, searchable, family)
    disposition, local_status, deprecation = disposition_for(family, searchable)
    owners, authority = authority_for(family)
    idempotency = (
        "forbidden"
        if method in {"GET", "HEAD", "OPTIONS"}
        else "required"
    )
    body_limit = 0 if method in {"GET", "HEAD", "OPTIONS"} else 256 * 1024
    accepted_content_types = [] if body_limit == 0 else ["application/json"]
    if family == "upload_storage":
        body_limit = 8 * 1024 * 1024
    if path.startswith(HMAC_WEBHOOK_PATHS):
        body_limit = 1024 * 1024

    evidence_default = (
        "protected_evidence_required"
        if disposition == "protected_parity_required"
        else "dependency_pending"
        if family == "video_media"
        else "retirement_contract_pending_approval"
        if disposition == "retire"
        else "family_contract"
    )
    endpoint_success = (
        "retirement_contract_pending_approval"
        if disposition == "retire"
        else "protected_evidence_required"
        if disposition == "protected_parity_required"
        else "endpoint_adapter_pending"
    )
    retry_evidence = (
        "local_contract"
        if row["kind"] == "workflow" or family == "callbacks_webhooks_workflows"
        else evidence_default
    )

    identity_tuple = (row["kind"], method, row["legacy_path"])
    identity = "\0".join(identity_tuple).encode()
    operation_id = f"cap-v1-{sha256_bytes(identity)[:16]}"
    operation_override = OPERATION_CONTRACT_OVERRIDES.get(operation_id, {})
    auth = operation_override.get("auth", auth)
    idempotency = operation_override.get("idempotency", idempotency)
    body_limit = operation_override.get("max_body_bytes", body_limit)
    accepted_content_types = operation_override.get(
        "accepted_content_types", accepted_content_types
    )
    if operation_id == LICENSING_DECLARATION_ONLY_ID:
        local_status = "contract_declarations_audited_owner_disposition_pending"
        authority = "no concrete Frame commercial licensing authority"
        evidence_default = "dependency_pending"
        retry_evidence = "dependency_pending"
    elif operation_id == WEB_CUSTOM_DOMAIN_DECLARATION_ONLY_ID:
        local_status = "contract_declarations_audited_owner_disposition_pending"
        authority = "no executable boolean custom-domain authority"
        evidence_default = "dependency_pending"
        retry_evidence = "dependency_pending"
    elif operation_id in MESSENGER_RETIREMENT_IDENTITIES:
        local_status = "rust_retirement_response_complete_owner_approval_pending"
        authority = (
            "frame-application deterministic retirement response and privacy-safe export authority"
        )
    adapter = LOCAL_ENDPOINT_ADAPTERS.get(identity_tuple)
    if adapter:
        source_identities = {
            (source["path"], source["sha256"]) for source in row["sources"]
        }
        if operation_id != adapter["id"] or not all(
            identity in source_identities
            for identity in adapter_source_identities(adapter)
        ):
            raise RuntimeError("local endpoint adapter lost its pinned source identity")
        endpoint_success = "local_contract"
        auth = adapter["auth"]
        family = adapter["policy"].removesuffix(".v1")
        local_status = adapter["local_status"]
        authority = adapter["rust_authority"]
        evidence_default = "local_contract"
        retry_evidence = "local_contract"
    return {
        "id": operation_id,
        "kind": row["kind"],
        "legacy_path": row["legacy_path"],
        "method": method,
        "operations": row["operations"],
        "sources": row["sources"],
        "clients": clients,
        "auth": auth,
        "policy": f"{family}.v1",
        "contract_version": CONTRACT_VERSION,
        "owners": owners,
        "implementation": {
            "rust_authority": authority,
            "local_status": local_status,
        },
        "disposition": disposition,
        "security": {
            "max_body_bytes": body_limit,
            "accepted_content_types": accepted_content_types,
            "rate_limit_bucket": f"{family}.v1",
            "idempotency": idempotency,
            "tenant_non_disclosure": operation_override.get(
                "tenant_non_disclosure",
                family not in {"service_misc", "analytics_consent"},
            ),
        },
        "contract_evidence": {
            "success": endpoint_success,
            "validation": evidence_default,
            "authorization": evidence_default,
            "idempotency_retry": retry_evidence,
            "failure": evidence_default,
        },
        "completion": (
            adapter["completion"]
            if adapter and "completion" in adapter
            else OPERATION_COMPLETION_OVERRIDES[operation_id]
            if operation_id in OPERATION_COMPLETION_OVERRIDES
            else completion_for(
                disposition,
                family,
                searchable,
                endpoint_promoted=adapter is not None,
                operation_id=operation_id,
            )
        ),
        "deprecation": deprecation,
    }


def clients_for(searchable: str, kind: str) -> list[str]:
    clients: list[str] = []
    for token, client in (
        ("desktop", "desktop"),
        ("mobile", "mobile"),
        ("extension", "extension"),
        ("developer", "developer"),
        ("sdk", "developer"),
        ("webhook", "provider"),
        ("cron", "scheduler"),
        ("media-server", "internal_worker"),
    ):
        if token in searchable and client not in clients:
            clients.append(client)
    if kind == "workflow" and "scheduler" not in clients:
        clients.append("scheduler")
    if kind == "server_action" and "web" not in clients:
        clients.append("web")
    if not clients:
        clients.append("web")
    return sorted(clients)


def auth_for(path: str, searchable: str, family: str) -> str:
    if path.startswith("/api/webhooks/"):
        return "signed_webhook"
    if "/cron/" in path:
        return "scheduler_secret"
    if path.startswith("/media-server/"):
        return "internal_service"
    if "developer/sdk" in searchable or "developer/v1" in searchable:
        return "developer_api_key"
    if family == "billing_admin" or "admin" in searchable:
        return "admin_session"
    if any(
        marker in path
        for marker in (
            "/status",
            "/changelog",
            "/releases/",
            "/auth/",
            "/commercial/",
        )
    ):
        return "public_or_flow_token"
    if any(marker in path for marker in ("/thumbnail", "/preview", "/playlist", "/download")):
        return "optional_session_or_share_capability"
    return "session"


def disposition_for(family: str, searchable: str) -> tuple[str, str, dict[str, Any]]:
    no_deprecation = {
        "state": "not_deprecated",
        "earliest_removal": None,
        "migration_path": None,
        "approval": None,
    }
    if family == "messenger_support":
        return (
            "retire",
            "retirement_response_and_owner_approval_pending",
            {
                "state": "retirement_proposed",
                "earliest_removal": None,
                "migration_path": "privacy-safe export; product surface remains off by default",
                "approval": "repository_owner_pending",
            },
        )
    if family == "billing_admin":
        return (
            "protected_parity_required",
            "provider_sandbox_and_ledger_reconciliation_pending",
            no_deprecation,
        )
    if family == "imports_integrations" or any(
        token in searchable for token in ("s3", "google-drive")
    ):
        return (
            "migrate",
            "migration_authority_present_provider_adapter_pending",
            no_deprecation,
        )
    if family == "video_media":
        return (
            "replace",
            "rust_authority_present_issue_28_adapter_or_protected_evidence_pending",
            no_deprecation,
        )
    return "replace", "rust_authority_present_endpoint_adapter_pending", no_deprecation


def authority_for(family: str) -> tuple[list[str], str]:
    mapping = {
        "auth_session": (["13", "30"], "frame-application::identity + api_workflow"),
        "organization_library": (
            ["14", "30"],
            "frame-application::organization + business",
        ),
        "upload_storage": (
            ["18", "19", "20", "21", "30"],
            "frame-application storage/multipart/backfill/governance",
        ),
        "collaboration_notifications": (
            ["15", "30"],
            "frame-application::business durable aggregates/outbox",
        ),
        "share_playback": (
            ["15", "21", "30", "32"],
            "frame-application::business + governed storage",
        ),
        "developer_api": (
            ["13", "15", "30", "36"],
            "frame-application::identity + business developer ledger",
        ),
        "billing_admin": (
            ["15", "30"],
            "frame-application::business ledger; provider authority not promoted",
        ),
        "analytics_consent": (
            ["15", "30"],
            "frame-application::business consent/usage aggregates",
        ),
        "imports_integrations": (
            ["15", "20", "30"],
            "frame-application::backfill + business import workflow",
        ),
        "video_media": (
            ["07", "15", "28", "29", "30"],
            "frame-media job contracts + control-plane capability router",
        ),
        "callbacks_webhooks_workflows": (
            ["15", "28", "30"],
            "frame-application::api_workflow + control-plane D1 replay + business outbox",
        ),
        "messenger_support": (
            ["15", "30"],
            "business export authority; retained product adapter intentionally absent",
        ),
        "client_compatibility": (
            ["30", "36"],
            "frame-domain::api_workflow compatibility policy",
        ),
        "service_misc": (["30"], "frame-domain/application API contracts"),
    }
    return mapping[family]


def build_report() -> dict[str, Any]:
    raw = collect_raw()
    entries = [classify(row) for row in raw]
    entries.sort(key=lambda row: (row["kind"], row["legacy_path"], row["method"]))
    by_kind = Counter(row["kind"] for row in entries)
    by_disposition = Counter(row["disposition"] for row in entries)
    by_status = Counter(row["implementation"]["local_status"] for row in entries)
    return {
        "schema_version": SCHEMA_VERSION,
        "reference": {
            "repository": "CapSoftware/Cap",
            "commit": REFERENCE_COMMIT,
        },
        "contract_version": CONTRACT_VERSION,
        "generated_by": "scripts/ci/check-api-workflow-parity.py",
        "scope": {
            "included": [
                "Next API route handlers",
                "mounted Hono routes",
                "Effect HttpApi endpoints",
                "Next server actions",
                "Effect RPC operations",
                "durable workflow entrypoints",
            ],
            "transport_wrappers": list(TRANSPORT_WRAPPERS),
            "excluded": [
                "ordinary helper exports without use server",
                "UI routes and pixel behavior owned by issues 31-33",
                "native desktop IPC owned by issue 33",
            ],
        },
        "summary": {
            "total": len(entries),
            "by_kind": dict(sorted(by_kind.items())),
            "by_disposition": dict(sorted(by_disposition.items())),
            "by_local_status": dict(sorted(by_status.items())),
            "endpoint_success_proven": sum(
                row["contract_evidence"]["success"] == "local_contract" for row in entries
            ),
            "endpoint_success_pending": sum(
                row["contract_evidence"]["success"] != "local_contract" for row in entries
            ),
        },
        "entries": entries,
    }


def canonical_json_sha256(value: Any) -> str:
    return sha256_bytes(
        json.dumps(value, sort_keys=True, separators=(",", ":")).encode()
    )


def operation_contract_profile(row: dict[str, Any]) -> dict[str, Any]:
    """Describe the required five-axis oracle without fabricating execution evidence."""
    retirement = row["disposition"] == "retire"
    last_write_wins_active_selection = row["id"] == "cap-v1-a3b4c805d409bc7c"
    repeatable_theme_action = row["id"] == "cap-v1-7773d3e70d1d5919"
    safe_read = row["kind"] == "route" and row["method"] in {
        "GET",
        "HEAD",
        "OPTIONS",
    }
    if retirement:
        success_result = "owner_approved_retirement_response_no_business_effect"
        effect_cardinality = "none"
    elif row["kind"] == "workflow":
        success_result = "source_equivalent_terminal_receipt"
        effect_cardinality = "at_most_once_per_effect_key"
    elif row["kind"] == "rpc":
        success_result = "source_equivalent_rpc_result"
        effect_cardinality = "at_most_once_per_effect_key"
    elif row["kind"] == "server_action":
        success_result = "source_equivalent_action_result"
        effect_cardinality = (
            "repeatable_last_write_wins_cookie_replacement"
            if repeatable_theme_action
            else "repeatable_last_write_wins_active_selection"
            if last_write_wins_active_selection
            else "at_most_once_per_effect_key"
        )
    elif safe_read:
        success_result = "source_equivalent_response"
        effect_cardinality = "none"
    else:
        success_result = "source_equivalent_response_after_committed_effect"
        effect_cardinality = "at_most_once_per_effect_key"

    idempotency = row["security"]["idempotency"]
    if idempotency == "required":
        retry = {
            "mode": "required",
            "missing_key": "invalid_request",
            "exact_replay": "original_receipt_without_reexecution",
            "conflicting_reuse": "conflict",
        }
    elif idempotency == "optional":
        retry = {
            "mode": "optional",
            "missing_key": "accepted_without_replay_guarantee",
            "exact_replay": "original_receipt_when_key_supplied",
            "conflicting_reuse": "conflict_when_key_supplied",
        }
    else:
        retry = {
            "mode": "forbidden",
            "missing_key": "accepted",
            "exact_replay": (
                "same_value_repeat_is_safe_last_write_wins"
                if repeatable_theme_action
                else "fresh_server_operation_each_retry_last_write_wins"
                if last_write_wins_active_selection
                else "safe_transport_retry_without_business_mutation"
                if safe_read
                else "server_derived_operation_identity_only"
            ),
            "conflicting_reuse": "supplied_key_is_invalid_request",
        }

    anonymous_allowed = row["auth"] in {
        "public_or_flow_token",
        "optional_session_or_share_capability",
    }
    return {
        "success_or_retirement": {
            "case": "valid_source_pinned_request",
            "semantic_oracle": "operation_source_manifest_sha256",
            "expected": success_result,
            "effect_cardinality": effect_cardinality,
            "receipt_binding": "operation_scope_request_fingerprint",
        },
        "validation": {
            "case": "exact_transport_identity_and_request_shape",
            "max_body_bytes": row["security"]["max_body_bytes"],
            "accepted_content_types": row["security"]["accepted_content_types"],
            "malformed_or_oversized": "invalid_request",
            "wrong_method_or_identity": "not_found",
        },
        "authorization": {
            "case": "authenticate_then_authorize",
            "auth": row["auth"],
            "anonymous": "allowed" if anonymous_allowed else "unauthenticated",
            "unknown_or_forbidden_resource": "not_found",
            "tenant_non_disclosure": row["security"]["tenant_non_disclosure"],
        },
        "idempotency_or_retry": {
            "case": "bounded_replay_and_retry",
            "in_flight": (
                "last_completed_cookie_write_wins"
                if repeatable_theme_action
                else "last_committed_active_selection_wins"
                if last_write_wins_active_selection
                else "conflict_or_indeterminate_without_resubmission"
            ),
            **retry,
        },
        "failure": {
            "case": "closed_redacted_public_error",
            "stable_codes": [
                "invalid_request",
                "unauthenticated",
                "not_found",
                "conflict",
                "rate_limited",
                "unsupported",
                "upgrade_required",
                "temporarily_unavailable",
                "indeterminate",
                "internal",
            ],
            "missing_authority": "temporarily_unavailable",
            "unknown_provider_outcome": (
                "indeterminate_without_resubmission"
                if "provider_execution" in row["completion"]["protected_gates"]
                else "not_applicable"
            ),
            "secret_or_resource_detail": "redacted",
        },
    }


def build_operation_contract_catalog(report: dict[str, Any]) -> dict[str, Any]:
    profiles: dict[str, dict[str, Any]] = {}
    operations: list[dict[str, Any]] = []
    for row in report["entries"]:
        profile = operation_contract_profile(row)
        profile_id = f"frame-contract-v1-{canonical_json_sha256(profile)[:16]}"
        profiles.setdefault(profile_id, {"id": profile_id, **profile})
        complete = row["completion"]["local_work"] == "complete"
        protected_gates_clear = not row["completion"]["protected_gates"]
        locally_tested = row["contract_evidence"]["success"] == "local_contract"
        operations.append(
            {
                "operation_id": row["id"],
                "identity": {
                    "kind": row["kind"],
                    "method": row["method"],
                    "legacy_path": row["legacy_path"],
                },
                "profile_id": profile_id,
                "source_manifest_sha256": canonical_json_sha256(row["sources"]),
                "source_count": len(row["sources"]),
                "execution_evidence": {
                    "endpoint_success": row["contract_evidence"]["success"],
                    "remaining_local_work": row["completion"]["local_work"],
                    "protected_gates": row["completion"]["protected_gates"],
                    "retirement_approval": row["deprecation"]["approval"],
                    "production_behavior": row["completion"]["production_behavior"],
                    "promotion_authorized": (
                        locally_tested and complete and protected_gates_clear
                    ),
                },
            }
        )
    total = len(operations)
    locally_tested = sum(
        row["contract_evidence"]["success"] == "local_contract"
        for row in report["entries"]
    )
    return {
        "schema_version": OPERATION_CONTRACT_CATALOG_SCHEMA_VERSION,
        "catalog_role": "specification_only_not_endpoint_execution_evidence",
        "reference": report["reference"],
        "contract_version": CONTRACT_VERSION,
        "report_sha256": canonical_json_sha256(report),
        "summary": {
            "total": total,
            "profiles": len(profiles),
            "retained_contracts": sum(
                row["disposition"] != "retire" for row in report["entries"]
            ),
            "retirement_contracts": sum(
                row["disposition"] == "retire" for row in report["entries"]
            ),
            "locally_tested_success": locally_tested,
            "promotion_authorized": sum(
                operation["execution_evidence"]["promotion_authorized"]
                for operation in operations
            ),
            "endpoint_or_retirement_pending": total
            - sum(
                operation["execution_evidence"]["promotion_authorized"]
                for operation in operations
            ),
            "specified_axis_contracts": {
                "success_or_retirement": total,
                "validation": total,
                "authorization": total,
                "idempotency_or_retry": total,
                "failure": total,
            },
        },
        "profiles": [profiles[key] for key in sorted(profiles)],
        "operations": operations,
    }


def validate_operation_contract_catalog(
    catalog: dict[str, Any], report: dict[str, Any]
) -> list[str]:
    errors: list[str] = []
    if catalog != build_operation_contract_catalog(report):
        errors.append(
            "operation contract catalog differs from the exhaustive source-pinned report"
        )
    required_top = {
        "schema_version",
        "catalog_role",
        "reference",
        "contract_version",
        "report_sha256",
        "summary",
        "profiles",
        "operations",
    }
    if set(catalog) != required_top:
        return [*errors, "operation contract catalog top-level keys drifted"]
    if catalog["catalog_role"] != "specification_only_not_endpoint_execution_evidence":
        errors.append("operation contract catalog overclaims its evidence role")
    profiles = catalog.get("profiles")
    operations = catalog.get("operations")
    if not isinstance(profiles, list) or not isinstance(operations, list):
        return [*errors, "operation contract profiles and operations must be arrays"]
    axis_keys = {
        "success_or_retirement",
        "validation",
        "authorization",
        "idempotency_or_retry",
        "failure",
    }
    profile_ids: set[str] = set()
    for index, profile in enumerate(profiles):
        if not isinstance(profile, dict) or set(profile) != {"id", *axis_keys}:
            errors.append(f"operation contract profile {index} keys drifted")
            continue
        profile_id = profile["id"]
        expected_id = "frame-contract-v1-" + canonical_json_sha256(
            {axis: profile[axis] for axis in axis_keys}
        )[:16]
        if profile_id != expected_id or profile_id in profile_ids:
            errors.append(f"operation contract profile {index} identity drifted")
        profile_ids.add(profile_id)
        if profile["success_or_retirement"].get("semantic_oracle") != (
            "operation_source_manifest_sha256"
        ):
            errors.append(f"operation contract profile {index} lost its semantic oracle")
        if profile["authorization"].get("unknown_or_forbidden_resource") != "not_found":
            errors.append(f"operation contract profile {index} lost non-disclosure")
        if profile["failure"].get("secret_or_resource_detail") != "redacted":
            errors.append(f"operation contract profile {index} lost redaction")
    operation_ids: set[str] = set()
    for index, operation in enumerate(operations):
        if not isinstance(operation, dict):
            errors.append(f"operation contract {index} is not an object")
            continue
        operation_id = operation.get("operation_id")
        if operation_id in operation_ids:
            errors.append(f"operation contract {index} repeats an operation")
        operation_ids.add(operation_id)
        if operation.get("profile_id") not in profile_ids:
            errors.append(f"operation contract {index} references an unknown profile")
        if not re.fullmatch(r"[0-9a-f]{64}", operation.get("source_manifest_sha256", "")):
            errors.append(f"operation contract {index} has an unsafe source oracle")
        if not isinstance(operation.get("source_count"), int) or operation["source_count"] < 1:
            errors.append(f"operation contract {index} has no pinned semantic source")
        evidence = operation.get("execution_evidence", {})
        if evidence.get("promotion_authorized") and (
            evidence.get("endpoint_success") != "local_contract"
            or evidence.get("remaining_local_work") != "complete"
            or evidence.get("protected_gates")
        ):
            errors.append(f"operation contract {index} promotes incomplete evidence")
        if evidence.get("endpoint_success") != "local_contract" and evidence.get(
            "production_behavior"
        ) != "fail_closed_unavailable":
            errors.append(f"operation contract {index} does not fail closed while pending")
    if operation_ids != {row["id"] for row in report["entries"]}:
        errors.append("operation contract catalog does not cover every inventory identity")
    expected_axes = {axis: len(report["entries"]) for axis in axis_keys}
    if catalog.get("summary", {}).get("specified_axis_contracts") != expected_axes:
        errors.append("operation contract catalog does not specify all five axes for every row")
    return errors


def render_doc(report: dict[str, Any]) -> str:
    summary = report["summary"]
    catalog_summary = build_operation_contract_catalog(report)["summary"]
    entries = report["entries"]
    route_entries = [row for row in entries if row["kind"] == "route"]
    evidence_values = (
        "local_contract",
        "family_contract",
        "dependency_pending",
        "endpoint_adapter_pending",
        "protected_evidence_required",
        "retirement_contract_pending_approval",
    )
    evidence_axes = (
        "success",
        "validation",
        "authorization",
        "idempotency_retry",
        "failure",
    )
    client_counts = Counter(client for row in entries for client in row["clients"])
    client_success = Counter(
        client
        for row in entries
        if row["contract_evidence"]["success"] == "local_contract"
        for client in row["clients"]
    )
    local_work_pending = sum(
        row["completion"]["local_work"] != "complete" for row in entries
    )
    protected_overlap = sum(bool(row["completion"]["protected_gates"]) for row in entries)
    local_only_pending = sum(
        row["completion"]["local_work"] != "complete"
        and not row["completion"]["protected_gates"]
        for row in entries
    )
    protected_only_pending = sum(
        row["completion"]["local_work"] == "complete"
        and bool(row["completion"]["protected_gates"])
        for row in entries
    )
    mobile_handler_pinned = sum(
        any(source["path"] == MOBILE_HANDLER_SOURCE for source in row["sources"])
        for row in entries
    )
    rpc_implementation_pinned = sum(
        row["kind"] == "rpc"
        and any(
            source["path"].startswith("packages/web-backend/src/")
            for source in row["sources"]
        )
        for row in entries
    )
    provider_graph_pinned = sum(row["id"] in PROVIDER_BACKED_IDS for row in entries)
    lines = [
        "# Generated API and workflow parity report",
        "",
        "<!-- Generated by scripts/ci/check-api-workflow-parity.py; do not edit by hand. -->",
        "",
        f"Reference: `CapSoftware/Cap@{REFERENCE_COMMIT}`. Target contract: `{CONTRACT_VERSION}`.",
        "",
        "This is an inventory and gap report, not a production-parity attestation. "
        "A mapped Rust authority does not prove that its legacy endpoint adapter or protected "
        "provider path has passed E2E tests.",
        "",
        "## Summary",
        "",
        f"- Total retained/retirement decisions inventoried: **{summary['total']}**",
        f"- Endpoint-level success contracts proven locally: **{summary['endpoint_success_proven']}**",
        f"- Endpoint-level success or retirement approval still pending: **{summary['endpoint_success_pending']}**",
        "- Kinds: " + ", ".join(f"`{key}` {value}" for key, value in summary["by_kind"].items()),
        "- Dispositions: "
        + ", ".join(f"`{key}` {value}" for key, value in summary["by_disposition"].items()),
        "",
        "## Executable coverage boundary",
        "",
        f"The {len(route_entries)} HTTP method rows represent "
        f"{len({row['legacy_path'] for row in route_entries})} unique legacy paths. "
        f"Exactly {sum(row['legacy_path'].startswith('/api/v1') for row in route_entries)} "
        "are already under `/api/v1`; a target-native Frame route is not counted as a legacy "
        "adapter unless its inventory row has endpoint-level evidence.",
        "",
        "| Kind | Inventory rows | Endpoint success proven | Endpoint success pending |",
        "|---|---:|---:|---:|",
    ]
    for kind, count in summary["by_kind"].items():
        proven = sum(
            row["kind"] == kind
            and row["contract_evidence"]["success"] == "local_contract"
            for row in entries
        )
        lines.append(f"| `{kind}` | {count} | {proven} | {count - proven} |")
    lines.extend(
        [
            "",
            "Evidence values below are row counts, not endpoint passes inferred from a shared "
            "family contract.",
            "",
            "| Evidence axis | `local_contract` | `family_contract` | `dependency_pending` | "
            "`endpoint_adapter_pending` | `protected_evidence_required` | "
            "`retirement_contract_pending_approval` |",
            "|---|---:|---:|---:|---:|---:|---:|",
        ]
    )
    for axis in evidence_axes:
        counts = Counter(row["contract_evidence"][axis] for row in entries)
        values = " | ".join(str(counts[value]) for value in evidence_values)
        lines.append(f"| `{axis}` | {values} |")
    lines.extend(
        [
            "",
            "Every row also carries an exact completion decision. "
            f"{local_work_pending} rows still name repository-local adapter or retirement-response "
            f"work; {protected_overlap} rows name genuine protected gates, including "
            f"{protected_only_pending} whose repository-local implementation is complete but whose "
            f"released-client/provider/approval proof is not. {local_only_pending} are local-only. "
            "Protected evidence never converts an unproven journey into a completed route.",
            "",
            "The normalized `operation-contract-catalog.json` assigns every identity a "
            "source-manifest-bound success-or-retirement, validation, authorization, "
            "idempotency/retry, and failure specification. It is deliberately marked "
            "`specification_only_not_endpoint_execution_evidence`: validating a profile cannot "
            "promote an adapter or approve a retirement. "
            f"It contains {catalog_summary['profiles']} shared profiles for "
            f"{catalog_summary['total']} operations; "
            f"{catalog_summary['locally_tested_success']} have local success evidence, "
            f"{catalog_summary['promotion_authorized']} have no remaining local/protected work, "
            f"and {catalog_summary['endpoint_or_retirement_pending']} endpoint or retirement "
            "gates remain.",
            "",
            f"Source closure pins the concrete catch-all implementation for all "
            f"{mobile_handler_pinned} Mobile rows and the HTTP transport, RPC layer, family handler, "
            f"and called service/policy sources for all {rpc_implementation_pinned} Effect RPC rows. "
            f"`{ORGANISATION_SOFT_DELETE_ID}` additionally pins its S3 and Tinybird services and "
            "therefore retains an explicit `provider_execution` gate alongside unfinished local "
            "adapter/orchestration work.",
            "",
            f"A second exact-ID audit pins the minimal provider-bearing implementation graph for "
            f"{provider_graph_pinned} additional rows and gives each an explicit "
            "`provider_execution` completion gate without changing its route taxonomy. "
            f"`{LICENSING_DECLARATION_ONLY_ID}` and "
            f"`{WEB_CUSTOM_DOMAIN_DECLARATION_ONLY_ID}` are exhaustive declaration-only audits. "
            "Neither pins an executable handler or authority, so repository-local investigation "
            "is complete while production remains fail-closed behind an explicit repository-owner "
            "implementation-or-retirement decision; no behavior is invented from a schema.",
            "",
            "Client counts are associations and can overlap when one operation serves multiple "
            "client families. They do not claim a current or N-1 client journey.",
            "",
            "| Client family | Operation associations | Endpoint success proven | "
            "Endpoint success pending |",
            "|---|---:|---:|---:|",
        ]
    )
    for client, count in sorted(client_counts.items()):
        proven = client_success[client]
        lines.append(f"| `{client}` | {count} | {proven} | {count - proven} |")
    lines.extend(
        [
            "",
            "## Central compatibility registry",
            "",
            "`frame-application::LegacyCompatibilityRegistryV1` decodes this exact report in its "
            "focused test suite. It registers all 288 identities and exercises the common "
            "compatibility, request validation, authentication/non-disclosure, rate-limit, "
            "idempotency-header, stable-error, trace-label, and audit-label boundary for every "
            "retained row. Its compatibility fixture supplies synthetic fallback availability "
            "to prove routing decisions; it does not claim an external deployment is reachable. "
            "The test-only evidence-enabled case proves that no row can bypass the common admission "
            "path. Every retained row also reaches one atomic execution port contract "
            "that binds its operation ID, request fingerprint, idempotency key, audit labels, "
            "and durable receipt; replay, conflicting reuse, in-flight work, and closed execution "
            "failures are covered. All 288 stable identities and all 138 raw HTTP method patterns "
            "resolve through the same registry without URL decoding; hostile encoded, dot, empty, "
            "backslash, semicolon, and control-character paths fail closed.",
            "",
            "The control-plane runtime constructs that registry behind a raw HTTP transport and "
            "implements the execution port with a digest-only D1 claim, fenced intent, completion, "
            "and append-only audit journal. Provider-free SQLite conformance covers a two-contender "
            "race, restart replay, conflicting key reuse, losing-reservation partial writes, "
            "tenant scoping, and immutable rows. Its durable semantic-adapter allowlist remains "
            "empty. The enabled semantic adapters are the source-pinned `GET /api/status` "
            "contract (`cap-v1-05b6ba3f76daac22`) and `GET /media-server` metadata contract "
            "(`cap-v1-ff19008f47194c43`), plus `GET /api/changelog/status` "
            "(`cap-v1-a1b180c5d123c870`) and its exact `OPTIONS` preflight "
            "(`cap-v1-16668b858461f386`), and the full `GET /api/changelog` feed "
            "(`cap-v1-0fa8384f3666825b`) with its exact `OPTIONS` preflight "
            "(`cap-v1-237f41f3086a2d67`), and `GET /api/mobile/session/config` "
            "(`cap-v1-4f21920a947c4c84`), plus the session-authenticated D1 "
            "`GET /api/notifications/preferences` contract (`cap-v1-d130c840f654bd72`). "
            "The notification adapter pins `getCurrentUser`, `users.preferences`, the API middleware "
            "exclusion, and Next 16.2.1 response runtime; it preserves Cap's whole-object fallback, "
            "optional `pauseAnonViews` default, compact field order, actor-only query, exact 401 and "
            "preference-query 500 JSON while keeping auth-infrastructure failures outside that custom "
            "query error. The mobile adapter pins both the Effect endpoint "
            "declaration and handler, derives its two booleans from non-empty Worker bindings "
            "with JavaScript string truthiness, "
            "and binds all four configurations into its fingerprint and exact compact JSON. "
            "The feed adapter pins all 99 MDX sources and the exact "
            "88,817-byte `JSON.stringify` body. The seven static response adapters have no D1 "
            "business-data dependency, but every production ingress enforces its report bucket through "
            "the bounded keyed-digest authority in migration `0034_compatibility_rate_limits.sql`; "
            "missing authority fails closed and saturation reaches the typed rate-limit error. All eight "
            "semantic adapters return their exact pinned status, content type, body, headers, "
            "and response digest. Per-operation "
            "path, method, query semantics, "
            "empty-body, forbidden-idempotency, authorization, retry, source-SHA, response, and "
            "stable-failure tests guard all seven static promotions and the exact D1 preference read. "
            "A separate exact business "
            "registration pins the Navbar `updateActiveOrganization` server action "
            "(`cap-v1-a3b4c805d409bc7c`) as `server_action`/`ACTION`, binds the actor to a trusted "
            "session principal, maps the Cap NanoID deterministically, and executes an atomic D1 "
            "active-only update that preserves the default organization and derives the revision "
            "server-side. It yields an internal `/dashboard` invalidation-then-void effect and never "
            "synthesizes an HTTP path. An authenticated same-origin compatibility-action ingress "
            "and browser NanoID method are implemented, and a local-evidence registry proves the "
            "effect and atomic browser-proof consumption, but the production registry keeps the "
            "operation unavailable while the released-client gate remains. The Leptos dashboard picker "
            "continues to use Frame's native UUID/revision mutation. The callable NanoID client "
            "method has no released Cap-client E2E journey yet, so that protected client gate "
            "remains explicit. The exact `setTheme` action "
            "(`cap-v1-7773d3e70d1d5919`) uses the same transport-neutral admission, accepts only "
            "`light` or `dark`, forbids client idempotency, and consumes its repeatable "
            "last-write-wins effect as the exact `theme={value}; Path=/` response cookie plus a "
            "no-store void response. Frame's authenticated hydration toggle calls that ingress "
            "and its bootstrap reapplies only an exact `light`/`dark` theme cookie on reload; this "
            "is distinct from Cap's pinned `Contexts.tsx` JS-cookie persistence behavior, which "
            "does not import the otherwise-unused server action. The three exact folder-assignment "
            "actions (`cap-v1-f5daa7be337a2979`, `cap-v1-1af3645bf2ae7168`, and "
            "`cap-v1-eaf277e644aa4b92`) use a bounded authenticated browser carrier and an atomic "
            "D1 business adapter. Add/remove require every canonical video to be actor-owned; move "
            "requires manager authority for the selected context and a tenant video. Folder, space, "
            "active-tenant, membership, and video snapshots are reasserted with the normalized "
            "mutation, typed storage postcondition, tenant/actor/action-scoped receipt, audit, cache "
            "effects, and one-use browser-grant consumption. Their local evidence remains production "
            "fail-closed behind `released_legacy_client_e2e`. Additional source-pinned exact business "
            "families cover library placement, notification actions, developer administration, and "
            "membership actions; each row below remains authoritative for its distinct protected-gate "
            "and production-promotion state. The mobile "
            "`PATCH /api/mobile/user/active-organization` row remains unpromoted and provider-gated: "
            "its exact fresh bootstrap still requires provider image signing and nullable-space root "
            "folder semantics. Production fallback availability stays false, so every unpromoted "
            "operation returns a closed unavailable error rather than manufacturing a business "
            "success or a legacy fallback.",
            "",
            "The registry exercises current and previous release decisions for all 267 "
            "release-managed client associations and rejects older releases. This is local "
            "registry evidence, not a released client binary/build. Endpoint success is therefore "
            f"limited to the {summary['endpoint_success_proven']} exact contracts enumerated below; "
            "the remaining "
            f"{summary['endpoint_success_pending']} per-operation request/response and side-effect "
            "semantics, transport promotions, released-client runs, protected providers, and "
            "accountable retirement approvals remain explicit gates.",
            "",
            "## Contract inventory",
            "",
            "| Method | Legacy path / operation | Clients | Auth | Policy | Disposition | Local status |",
            "|---|---|---|---|---|---|---|",
        ]
    )
    for row in entries:
        path = row["legacy_path"].replace("|", "\\|")
        clients = ", ".join(row["clients"])
        lines.append(
            f"| `{row['method']}` | `{path}` | {clients} | `{row['auth']}` | "
            f"`{row['policy']}` | `{row['disposition']}` | "
            f"`{row['implementation']['local_status']}` |"
        )
    lines.extend(
        [
            "",
            "## Reading the evidence fields",
            "",
            "Each machine-readable row has independent success, validation, authorization, "
            "idempotency/retry, and failure evidence states. `family_contract` means the shared "
            "Rust authority has focused tests; `endpoint_adapter_pending` means the legacy-shaped "
            "transport has not earned redirect authority. Protected provider and retirement rows "
            "remain explicitly pending until their named reviewer evidence exists.",
            "",
        ]
    )
    return "\n".join(lines)


def validate_report(report: dict[str, Any], *, compare_reference: bool) -> list[str]:
    errors: list[str] = []
    required_top = {
        "schema_version",
        "reference",
        "contract_version",
        "generated_by",
        "scope",
        "summary",
        "entries",
    }
    if set(report) != required_top:
        errors.append("report top-level keys drifted")
        return errors
    if report["schema_version"] != SCHEMA_VERSION:
        errors.append("report schema version drifted")
    if report["reference"] != {
        "repository": "CapSoftware/Cap",
        "commit": REFERENCE_COMMIT,
    }:
        errors.append("reference identity drifted")
    if report["contract_version"] != CONTRACT_VERSION:
        errors.append("target API contract version drifted")
    entries = report.get("entries")
    if not isinstance(entries, list) or not entries:
        errors.append("report entries must be a non-empty array")
        return errors

    entry_keys = {
        "id",
        "kind",
        "legacy_path",
        "method",
        "operations",
        "sources",
        "clients",
        "auth",
        "policy",
        "contract_version",
        "owners",
        "implementation",
        "disposition",
        "security",
        "contract_evidence",
        "completion",
        "deprecation",
    }
    identities: set[tuple[str, str, str]] = set()
    ids: set[str] = set()
    for index, row in enumerate(entries):
        label = f"entry {index}"
        if not isinstance(row, dict) or set(row) != entry_keys:
            errors.append(f"{label}: keys drifted")
            continue
        if not re.fullmatch(r"cap-v1-[0-9a-f]{16}", row["id"]):
            errors.append(f"{label}: invalid ID")
        expected_id = "cap-v1-" + sha256_bytes(
            "\0".join((row["kind"], row["method"], row["legacy_path"])).encode()
        )[:16]
        if row["id"] != expected_id:
            errors.append(f"{label}: ID does not match its stable identity")
        if row["id"] in ids:
            errors.append(f"{label}: duplicate ID")
        ids.add(row["id"])
        identity = (row["kind"], row["method"], row["legacy_path"])
        if identity in identities:
            errors.append(f"{label}: duplicate route/action/workflow identity")
        identities.add(identity)
        adapter = LOCAL_ENDPOINT_ADAPTERS.get(identity)
        if adapter:
            if row["id"] != adapter["id"]:
                errors.append(f"{label}: local adapter ID drifted")
            if row["implementation"] != {
                "rust_authority": adapter["rust_authority"],
                "local_status": adapter["local_status"],
            }:
                errors.append(f"{label}: local adapter implementation evidence drifted")
            if row["contract_evidence"] != {
                "success": "local_contract",
                "validation": "local_contract",
                "authorization": "local_contract",
                "idempotency_retry": "local_contract",
                "failure": "local_contract",
            }:
                errors.append(f"{label}: local adapter lost an exact evidence axis")
            if row["auth"] != adapter["auth"] or row["policy"] != adapter["policy"]:
                errors.append(f"{label}: local adapter request policy drifted")
            source_identities = {
                (source["path"], source["sha256"]) for source in row["sources"]
            }
            if not all(
                identity in source_identities
                for identity in adapter_source_identities(adapter)
            ):
                errors.append(f"{label}: local adapter source identity drifted")
        elif row["contract_evidence"]["success"] == "local_contract":
            errors.append(f"{label}: endpoint success lacks an explicit local adapter")
        if row["method"] not in ALL_METHODS:
            errors.append(f"{label}: unsupported method")
        if row["contract_version"] != CONTRACT_VERSION:
            errors.append(f"{label}: contract version drifted")
        for field in ("operations", "sources", "clients", "owners"):
            if not isinstance(row[field], list) or not row[field]:
                errors.append(f"{label}: {field} must be non-empty")
        if row["disposition"] not in {
            "replace",
            "migrate",
            "retire",
            "protected_parity_required",
        }:
            errors.append(f"{label}: disposition is invalid")
        searchable = classification_searchable(row)
        if "messenger" not in searchable and any(
            token in searchable
            for token in ("billing", "stripe", "checkout", "subscribe")
        ) and row["disposition"] != "protected_parity_required":
            errors.append(f"{label}: billing/provider authority lost its protected gate")
        if row["legacy_path"].startswith("/api/webhooks/") and row["auth"] != "signed_webhook":
            errors.append(f"{label}: webhook is not bound to signed-webhook auth")
        if set(row["implementation"]) != {"rust_authority", "local_status"}:
            errors.append(f"{label}: implementation keys drifted")
        elif (
            adapter is None
            and "/api/webhooks/media-server/" in row["legacy_path"]
            and row["implementation"]["local_status"]
            != "rust_authority_present_issue_28_adapter_or_protected_evidence_pending"
        ):
            errors.append(f"{label}: media callback lost its issue 28 evidence gate")
        if set(row["security"]) != {
            "max_body_bytes",
            "accepted_content_types",
            "rate_limit_bucket",
            "idempotency",
            "tenant_non_disclosure",
        }:
            errors.append(f"{label}: security keys drifted")
        if not isinstance(row["security"]["max_body_bytes"], int) or not 0 <= row["security"][
            "max_body_bytes"
        ] <= 8 * 1024 * 1024:
            errors.append(f"{label}: body limit is invalid")
        if row["security"]["idempotency"] not in {"forbidden", "optional", "required"}:
            errors.append(f"{label}: idempotency policy is invalid")
        accepted_content_types = row["security"]["accepted_content_types"]
        if (
            not isinstance(accepted_content_types, list)
            or len(accepted_content_types) > 8
            or len(accepted_content_types) != len(set(accepted_content_types))
            or any(
                not isinstance(value, str)
                or not re.fullmatch(r"[a-z0-9.+-]+/[a-z0-9.+-]+", value)
                for value in accepted_content_types
            )
            or (row["security"]["max_body_bytes"] == 0 and accepted_content_types)
        ):
            errors.append(f"{label}: accepted content types are invalid")
        contract_override = OPERATION_CONTRACT_OVERRIDES.get(row["id"])
        if contract_override and any(
            row["security"].get(key) != value
            for key, value in contract_override.items()
            if key != "auth"
        ):
            errors.append(f"{label}: operation-level transport contract drifted")
        if contract_override and row["auth"] != contract_override.get("auth", row["auth"]):
            errors.append(f"{label}: operation-level auth contract drifted")
        if set(row["contract_evidence"]) != {
            "success",
            "validation",
            "authorization",
            "idempotency_retry",
            "failure",
        }:
            errors.append(f"{label}: evidence axes drifted")
        elif any(value not in EVIDENCE_VALUES for value in row["contract_evidence"].values()):
            errors.append(f"{label}: evidence value is invalid")
        expected_completion = (
            adapter["completion"]
            if adapter and "completion" in adapter
            else OPERATION_COMPLETION_OVERRIDES[row["id"]]
            if row["id"] in OPERATION_COMPLETION_OVERRIDES
            else completion_for(
                row["disposition"],
                row["policy"].removesuffix(".v1"),
                searchable,
                endpoint_promoted=adapter is not None,
                operation_id=row["id"],
            )
        )
        if row["completion"] != expected_completion:
            errors.append(f"{label}: exact local/protected completion blockers drifted")
        elif (
            any(
                gate not in PROTECTED_COMPLETION_GATES
                for gate in row["completion"]["protected_gates"]
            )
            or len(row["completion"]["protected_gates"])
            != len(set(row["completion"]["protected_gates"]))
        ):
            errors.append(f"{label}: completion gates are invalid")
        if (
            row["id"] == ORGANISATION_SOFT_DELETE_ID
            and adapter is None
            and row["completion"] != {
            "decision": "retain_replace_with_provider_effect",
            "local_work": "exact_adapter_and_provider_effect_orchestration_required",
            "protected_gates": ["provider_execution"],
            "retirement_decision": "not_proposed",
            "production_behavior": "fail_closed_unavailable",
            }
        ):
            errors.append(
                f"{label}: organisation soft-delete lost its S3/Tinybird provider gate"
            )
        if (
            row["id"] in PROVIDER_BACKED_IDS
            and adapter is None
            and row["id"] not in OPERATION_COMPLETION_OVERRIDES
            and row["completion"]
            != {
            "decision": "retain_replace_with_provider_effect",
            "local_work": "exact_adapter_and_provider_effect_orchestration_required",
            "protected_gates": ["provider_execution"],
            "retirement_decision": "not_proposed",
            "production_behavior": "fail_closed_unavailable",
            }
        ):
            errors.append(f"{label}: transitive provider operation lost its protected gate")
        if row["id"] == LICENSING_DECLARATION_ONLY_ID:
            if row["implementation"] != {
                "rust_authority": "no concrete Frame commercial licensing authority",
                "local_status": "contract_declarations_audited_owner_disposition_pending",
            }:
                errors.append(f"{label}: commercial licensing authority is overclaimed")
            if row["contract_evidence"] != {
                "success": "endpoint_adapter_pending",
                "validation": "dependency_pending",
                "authorization": "dependency_pending",
                "idempotency_retry": "dependency_pending",
                "failure": "dependency_pending",
            }:
                errors.append(f"{label}: commercial licensing dependency evidence drifted")
            if row["completion"] != {
                "decision": "declaration_only_owner_disposition_required",
                "local_work": "complete",
                "protected_gates": ["human_approval"],
                "retirement_decision": "repository_owner_pending",
                "production_behavior": "fail_closed_unavailable",
            }:
                errors.append(f"{label}: commercial licensing completion is overclaimed")
        if set(row["deprecation"]) != {
            "state",
            "earliest_removal",
            "migration_path",
            "approval",
        }:
            errors.append(f"{label}: deprecation keys drifted")
        if row["disposition"] == "retire" and (
            not row["deprecation"]["migration_path"] or not row["deprecation"]["approval"]
        ):
            errors.append(f"{label}: retirement lacks migration/approval state")
        try:
            required_source_specs = set(transitive_source_specs(row))
        except RuntimeError as error:
            errors.append(f"{label}: {error}")
            required_source_specs = set()
        actual_source_specs = {
            (source.get("path"), source.get("symbol"))
            for source in row["sources"]
            if isinstance(source, dict)
        }
        missing_source_specs = required_source_specs - actual_source_specs
        if missing_source_specs:
            errors.append(
                f"{label}: transitive implementation source pins are missing: "
                + ", ".join(
                    f"{path}#{symbol}"
                    for path, symbol in sorted(missing_source_specs)
                )
            )
        for source in row["sources"]:
            if set(source) != {"path", "symbol", "sha256"}:
                errors.append(f"{label}: source keys drifted")
                continue
            if source["path"].startswith(("/", "../")) or not re.fullmatch(
                r"[0-9a-f]{64}", source["sha256"]
            ):
                errors.append(f"{label}: source identity is unsafe")
            source_path = CAP / source["path"]
            if compare_reference and source_path.is_file():
                if sha256_bytes(source_path.read_bytes()) != source["sha256"]:
                    errors.append(f"{label}: source checksum drifted for {source['path']}")

    if not ORGANIZATION_AUDIT_IDS.issubset(ids):
        errors.append("the exact 14-row organization audit identity set is incomplete")
    if not PROVIDER_BACKED_IDS.issubset(ids) or len(PROVIDER_BACKED_IDS) != 36:
        errors.append("the exact 36-row transitive provider identity set is incomplete")
    if LICENSING_DECLARATION_ONLY_ID not in ids:
        errors.append("the commercial licensing declaration-only identity is missing")

    expected_summary = build_summary(entries)
    if report["summary"] != expected_summary:
        errors.append("summary does not match entries")
    if entries != sorted(entries, key=lambda row: (row["kind"], row["legacy_path"], row["method"])):
        errors.append("entries are not deterministically sorted")

    if compare_reference:
        try:
            expected = build_report()
        except (OSError, RuntimeError, ValueError) as error:
            errors.append(f"reference extraction failed: {error}")
        else:
            if expected != report:
                errors.append("committed report differs from the pinned Cap extraction")
    return errors


def validate_changelog_feed(*, compare_reference: bool) -> list[str]:
    errors: list[str] = []
    try:
        body = CHANGELOG_FEED.read_bytes()
    except OSError as error:
        return [f"unable to load pinned changelog feed fixture: {error}"]
    if len(body) != CHANGELOG_FEED_BODY_BYTES:
        errors.append("pinned changelog feed fixture byte length drifted")
    if sha256_bytes(body) != CHANGELOG_FEED_BODY_SHA256:
        errors.append("pinned changelog feed fixture digest drifted")
    try:
        decoded = json.loads(body)
    except json.JSONDecodeError:
        errors.append("pinned changelog feed fixture is not valid JSON")
    else:
        if not isinstance(decoded, list) or len(decoded) != 99:
            errors.append("pinned changelog feed fixture lost its exact 99-entry shape")
    if compare_reference:
        try:
            expected, _, manifest = build_changelog_feed()
        except (OSError, RuntimeError, ValueError) as error:
            errors.append(f"pinned changelog feed extraction failed: {error}")
        else:
            if expected.encode() != body:
                errors.append("pinned changelog feed differs from the Cap JSON.stringify body")
            if manifest != CHANGELOG_FEED_SOURCE_MANIFEST_SHA256:
                errors.append("pinned changelog source manifest digest drifted")
    return errors


def validate_theme_action_fixture(report: dict[str, Any]) -> list[str]:
    """Keep the transport selector separate from the exact abstract ACTION."""
    try:
        fixture = json.loads(THEME_ACTION_FIXTURE.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        return [f"unable to load theme action fixture: {error}"]
    errors: list[str] = []
    operation_id = "cap-v1-7773d3e70d1d5919"
    expected_identity = (
        "action://apps/web/app/(org)/dashboard/_components/actions.ts#setTheme"
    )
    operation = fixture.get("operation", {})
    transport = fixture.get("transport", {})
    cookie = fixture.get("cookie", {})
    retry = fixture.get("retry", {})
    input_contract = fixture.get("input", {})
    if fixture.get("schema_version") != 1:
        errors.append("theme action fixture schema drifted")
    if operation != {
        "id": operation_id,
        "kind": "server_action",
        "method": "ACTION",
        "legacy_identity": expected_identity,
        "abstract_body_bytes": 0,
        "abstract_content_types": [],
        "output": "void_with_response_cookie",
    }:
        errors.append("theme fixture lost its exact abstract ACTION contract")
    if (
        transport.get("method") != "POST"
        or transport.get("path")
        != f"/api/v1/web/compatibility-actions/{operation_id}"
        or transport.get("role") != "frame_selector_not_legacy_identity"
        or transport.get("request_schema")
        != "frame.web-compatibility-action-request.v1"
        or transport.get("authentication") != "host_only_session"
        or transport.get("same_origin") != "required"
        or transport.get("csrf") != "double_submit_required"
        or transport.get("client_idempotency") != "forbidden"
        or transport.get("success_status") != 204
        or transport.get("success_body") != "empty"
        or transport.get("cache_control") != "no-store, max-age=0"
    ):
        errors.append("theme fixture lost its exact Frame ingress contract")
    if input_contract.get("accepted") != ["light", "dark"]:
        errors.append("theme fixture input enum drifted")
    if cookie != {
        "name": "theme",
        "path": "/",
        "set_cookie": ["theme=light; Path=/", "theme=dark; Path=/"],
        "http_only": False,
        "secure": False,
        "same_site": None,
        "max_age": None,
    }:
        errors.append("theme fixture cookie serialization drifted")
    if retry != {
        "mode": "last_write_wins_without_client_idempotency",
        "same_value_repeat": "safe_cookie_replacement",
        "concurrent_writes": "last_completed_cookie_write_wins",
    }:
        errors.append("theme fixture retry semantics drifted")
    rows = [row for row in report.get("entries", []) if row.get("id") == operation_id]
    if len(rows) != 1:
        errors.append("theme fixture operation is not unique in the inventory")
    else:
        row = rows[0]
        fixture_sources = {
            (source.get("path"), source.get("sha256"))
            for source in fixture.get("sources", [])
        }
        report_sources = {
            (source.get("path"), source.get("sha256")) for source in row.get("sources", [])
        }
        if fixture_sources != report_sources or len(fixture_sources) != 5:
            errors.append("theme fixture source closure differs from the report")
        if (
            row.get("kind") != "server_action"
            or row.get("method") != "ACTION"
            or row.get("legacy_path") != expected_identity
            or row.get("security", {}).get("idempotency") != "forbidden"
            or row.get("contract_evidence", {}).get("success") != "local_contract"
        ):
            errors.append("theme report row is not the promoted exact action")
    return errors


def validate_folder_assignment_fixture(report: dict[str, Any]) -> list[str]:
    """Bind the Frame carrier, atomic D1 proof, and three frozen ACTION rows."""
    try:
        fixture = json.loads(FOLDER_ASSIGNMENT_FIXTURE.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        return [f"unable to load folder-assignment action fixture: {error}"]
    errors: list[str] = []
    expected = {
        "cap-v1-f5daa7be337a2979": (
            "action://apps/web/actions/folders/add-videos.ts#addVideosToFolder",
            "object_success_message_addedCount",
            8,
        ),
        "cap-v1-1af3645bf2ae7168": (
            "action://apps/web/actions/folders/remove-videos.ts#removeVideosFromFolder",
            "object_success_message_removedCount",
            7,
        ),
        "cap-v1-eaf277e644aa4b92": (
            "action://apps/web/actions/folders/moveVideoToFolder.ts#moveVideoToFolder",
            "empty_204_void",
            7,
        ),
    }
    transport = fixture.get("transport", {})
    atomicity = fixture.get("atomicity", {})
    if fixture.get("schema_version") != 1:
        errors.append("folder-assignment fixture schema drifted")
    if (
        transport.get("method") != "POST"
        or transport.get("path_prefix")
        != "/api/v1/web/compatibility-actions/"
        or transport.get("role") != "frame_selector_not_legacy_identity"
        or transport.get("request_schema")
        != "frame.web-folder-assignment-request.v1"
        or transport.get("max_body_bytes") != 256 * 1024
        or transport.get("authentication") != "host_only_session"
        or transport.get("same_origin") != "required"
        or transport.get("csrf") != "double_submit_one_use_grant_required"
        or transport.get("client_idempotency")
        != "required_header_and_body_exact_match"
        or transport.get("rate_limit_bucket") != "organization_library.v1"
        or transport.get("cache_control") != "no-store, max-age=0"
    ):
        errors.append("folder-assignment fixture lost its Frame ingress contract")
    required_atomic_steps = {
        "live_actor_active_tenant_and_membership_reassertion",
        "folder_space_video_and_role_preconditions",
        "normalized_storage_mutation",
        "typed_storage_postcondition",
        "tenant_actor_action_scoped_idempotency_receipt",
        "business_audit",
        "validated_cache_effects",
        "one_use_browser_grant_consumption",
    }
    if set(atomicity.get("one_transaction", [])) != required_atomic_steps or any(
        atomicity.get(key) != value
        for key, value in {
            "same_key_same_fingerprint": "replay_original_receipt",
            "same_key_different_fingerprint": "conflict",
            "race": "one_apply_one_replay",
            "cross_tenant_or_stale_authority": "not_found_without_mutation",
        }.items()
    ):
        errors.append("folder-assignment fixture lost its atomic retry contract")
    operations = fixture.get("operations", [])
    if not isinstance(operations, list) or {row.get("id") for row in operations} != set(
        expected
    ):
        errors.append("folder-assignment fixture operation set drifted")
        return errors
    report_by_id = {row.get("id"): row for row in report.get("entries", [])}
    source_closure = fixture.get("source_closure", {})
    for operation in operations:
        operation_id = operation["id"]
        identity, success, source_count = expected[operation_id]
        row = report_by_id.get(operation_id, {})
        if (
            operation.get("kind") != "server_action"
            or operation.get("method") != "ACTION"
            or operation.get("legacy_identity") != identity
            or operation.get("success") != success
            or operation.get("protected_gates") != ["released_legacy_client_e2e"]
            or operation.get("production_behavior") != "fail_closed_unavailable"
        ):
            errors.append(f"folder-assignment fixture contract drifted: {operation_id}")
        if (
            row.get("kind") != "server_action"
            or row.get("method") != "ACTION"
            or row.get("legacy_path") != identity
            or row.get("contract_evidence", {}).get("success") != "local_contract"
            or row.get("completion", {}).get("local_work") != "complete"
            or row.get("completion", {}).get("protected_gates")
            != ["released_legacy_client_e2e"]
            or row.get("completion", {}).get("production_behavior")
            != "fail_closed_unavailable"
            or len(row.get("sources", [])) != source_count
            or source_closure.get(operation_id) != source_count
        ):
            errors.append(f"folder-assignment report evidence drifted: {operation_id}")
    return errors


def validate_library_placement_fixture(report: dict[str, Any]) -> list[str]:
    """Bind four library-root ACTION identities to one atomic browser carrier."""
    try:
        fixture = json.loads(LIBRARY_PLACEMENT_FIXTURE.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        return [f"unable to load library-placement action fixture: {error}"]
    errors: list[str] = []
    expected = {
        "cap-v1-d96a1931942eb83b": (
            "action://apps/web/actions/organizations/add-videos.ts#addVideosToOrganization",
            "organization_id",
            "object_success_message_organization_root",
            7,
        ),
        "cap-v1-0694e68a64976c9a": (
            "action://apps/web/actions/organizations/remove-videos.ts#removeVideosFromOrganization",
            "organization_id",
            "object_success_removed_or_no_matching_message",
            6,
        ),
        "cap-v1-bb55b5eeeb5e31ab": (
            "action://apps/web/actions/spaces/add-videos.ts#addVideosToSpace",
            "scope_id",
            "object_success_message_scope_label",
            12,
        ),
        "cap-v1-ccbe5f1381eaa1b4": (
            "action://apps/web/actions/spaces/remove-videos.ts#removeVideosFromSpace",
            "scope_id",
            "object_success_message_deletedCount",
            11,
        ),
    }
    transport = fixture.get("transport", {})
    if fixture.get("schema_version") != 1 or transport != {
        "method": "POST",
        "path_prefix": "/api/v1/web/compatibility-actions/",
        "role": "frame_selector_not_legacy_identity",
        "request_schema": "frame.web-library-placement-request.v1",
        "max_body_bytes": 256 * 1024,
        "authentication": "host_only_session",
        "same_origin": "required",
        "csrf": "double_submit_one_use_grant_required",
        "client_idempotency": "required_header_and_body_exact_match",
        "rate_limit_bucket": "organization_library.v1",
        "cache_control": "no-store, max-age=0",
    }:
        errors.append("library-placement fixture lost its Frame ingress contract")
    if fixture.get("authorization") != {
        "all_actions": "live_session_actor_active_tenant_manager",
        "add_organization_add_space_remove_space": "every_target_video_actor_owned",
        "remove_organization": "delete_only_matching_tenant_shares",
        "cross_tenant_or_stale_authority": "not_found_without_mutation",
    }:
        errors.append("library-placement fixture authorization contract drifted")
    required_atomic_steps = {
        "live_actor_active_tenant_and_membership_reassertion",
        "organization_or_space_scope_preconditions",
        "operation_specific_video_ownership_or_share_preconditions",
        "normalized_share_space_and_folder_mutation",
        "typed_storage_postcondition",
        "tenant_actor_action_scoped_idempotency_receipt",
        "business_audit",
        "validated_cache_effects",
        "one_use_browser_grant_consumption",
    }
    atomicity = fixture.get("atomicity", {})
    if set(atomicity.get("one_transaction", [])) != required_atomic_steps or any(
        atomicity.get(key) != value
        for key, value in {
            "same_key_same_fingerprint": "replay_original_receipt",
            "same_key_different_fingerprint": "conflict",
            "race": "one_apply_one_replay",
            "rollback": "no_partial_mutation_or_consumed_grant",
        }.items()
    ):
        errors.append("library-placement fixture lost its atomic retry contract")
    operations = fixture.get("operations", [])
    if not isinstance(operations, list) or {row.get("id") for row in operations} != set(
        expected
    ):
        errors.append("library-placement fixture operation set drifted")
        return errors
    report_by_id = {row.get("id"): row for row in report.get("entries", [])}
    source_closure = fixture.get("source_closure", {})
    for operation in operations:
        operation_id = operation["id"]
        identity, request_scope_field, success, source_count = expected[operation_id]
        row = report_by_id.get(operation_id, {})
        if (
            operation.get("kind") != "server_action"
            or operation.get("method") != "ACTION"
            or operation.get("legacy_identity") != identity
            or operation.get("request_scope_field") != request_scope_field
            or operation.get("success") != success
            or operation.get("protected_gates") != ["released_legacy_client_e2e"]
            or operation.get("production_behavior") != "fail_closed_unavailable"
        ):
            errors.append(f"library-placement fixture contract drifted: {operation_id}")
        if (
            row.get("kind") != "server_action"
            or row.get("method") != "ACTION"
            or row.get("legacy_path") != identity
            or row.get("contract_evidence", {}).get("success") != "local_contract"
            or row.get("completion", {}).get("local_work") != "complete"
            or row.get("completion", {}).get("protected_gates")
            != ["released_legacy_client_e2e"]
            or row.get("completion", {}).get("production_behavior")
            != "fail_closed_unavailable"
            or len(row.get("sources", [])) != source_count
            or source_closure.get(operation_id) != source_count
        ):
            errors.append(f"library-placement report evidence drifted: {operation_id}")
    return errors


def validate_notification_actions_fixture(report: dict[str, Any]) -> list[str]:
    """Bind notification mutations to exact presence, void, and atomic contracts."""
    try:
        fixture = json.loads(NOTIFICATION_ACTIONS_FIXTURE.read_text(encoding="utf-8"))
        application = APPLICATION_NOTIFICATION_ACTIONS.read_text(encoding="utf-8")
        application_lib = APPLICATION_LIB.read_text(encoding="utf-8")
        d1_runtime = CONTROL_NOTIFICATION_ACTIONS_RUNTIME.read_text(encoding="utf-8")
        ingress = CONTROL_NOTIFICATION_WEB_RUNTIME.read_text(encoding="utf-8")
        migration = NOTIFICATION_ACTIONS_MIGRATION.read_text(encoding="utf-8")
        conformance = NOTIFICATION_ACTIONS_CONFORMANCE.read_text(encoding="utf-8")
        control_lib = CONTROL_LIB.read_text(encoding="utf-8")
        browser_client = WEB_BROWSER_CLIENT.read_text(encoding="utf-8")
        query_names = {
            path.name for path in NOTIFICATION_ACTIONS_QUERY_ROOT.glob("*.sql")
        }
    except (OSError, json.JSONDecodeError) as error:
        return [f"unable to load notification action fixture: {error}"]
    errors: list[str] = []
    expected = {
        "cap-v1-74a775753d3863c7": (
            "action://apps/web/actions/notifications/mark-as-read.ts#markAsRead",
            "optional_notification_id_missing_means_active_tenant_bulk",
            "live_actor_active_tenant_recipient_scope",
            14,
        ),
        "cap-v1-1f6a43a05f2f297c": (
            "action://apps/web/actions/notifications/update-preferences.ts#updatePreferences",
            "four_required_booleans_and_optional_pauseAnonViews_property",
            "live_actor_global_preferences_row",
            12,
        ),
    }
    transport = fixture.get("transport", {})
    if fixture.get("schema_version") != 1 or transport != {
        "method": "POST",
        "path_prefix": "/api/v1/web/compatibility-actions/",
        "role": "frame_selector_not_legacy_identity",
        "request_schema": "frame.web-notification-action-request.v1",
        "max_body_bytes": 256 * 1024,
        "authentication": "host_only_session",
        "same_origin": "required",
        "csrf": "double_submit_one_use_grant_required",
        "client_idempotency": "required_header_and_body_exact_match",
        "rate_limit_bucket": "collaboration_notifications.v1",
        "success_status": 204,
        "success_body": "empty",
        "cache_control": "no-store, max-age=0",
    }:
        errors.append("notification action fixture lost its Frame ingress contract")
    if fixture.get("presence") != {
        "mark_notification_id": "missing_or_cap_nanoid; explicit_null_rejected",
        "pauseAnonViews": "missing_or_boolean; explicit_null_rejected",
    }:
        errors.append("notification action fixture presence contract drifted")
    required_application_tokens = (
        '"cap-v1-74a775753d3863c7"',
        '"cap-v1-1f6a43a05f2f297c"',
        "LEGACY_MARK_NOTIFICATIONS_READ_SOURCES",
        "LEGACY_UPDATE_NOTIFICATION_PREFERENCES_SOURCES",
        "LegacyNotificationAtomicPortV1",
        "LegacyNotificationAdapterV1",
        'LEGACY_NOTIFICATION_ACTION_PROTECTED_GATES: &[&str] = &["released_legacy_client_e2e"]',
        "preference_flags_and_optional_property_presence_are_preserved_exactly",
        "receipt_for_a_different_idempotency_key_is_rejected",
    )
    for token in required_application_tokens:
        if token not in application:
            errors.append(f"notification application proof lost token: {token}")
    if (
        "mod legacy_notification_actions;" not in application_lib
        or "pub use legacy_notification_actions::*;" not in application_lib
    ):
        errors.append("notification action semantics are not exported by frame-application")

    required_ingress_tokens = (
        '"frame.web-notification-action-request.v1"',
        "LEGACY_MARK_NOTIFICATIONS_READ_OPERATION_ID",
        "LEGACY_UPDATE_NOTIFICATION_PREFERENCES_OPERATION_ID",
        "OptionalJsonFieldV1::Missing",
        "OptionalJsonFieldV1::Present(serde_json::Value::Null)",
        'request.headers().get("idempotency-key")',
        "trusted_active_organization_id",
        "D1LegacyNotificationAtomicPortV1::new",
    )
    for token in required_ingress_tokens:
        if token not in ingress:
            errors.append(f"notification ingress proof lost token: {token}")
    for token in (
        "mod legacy_notification_actions_runtime;",
        "mod legacy_notification_web_runtime;",
        "legacy_notification_web_runtime::is_action",
        "legacy_notification_action_response",
        "Response::empty()?.with_status(204)",
    ):
        if token not in control_lib:
            errors.append(f"notification route wiring lost token: {token}")

    required_runtime_tokens = (
        "D1LegacyNotificationAtomicPortV1",
        "impl LegacyNotificationAtomicPortV1",
        'include_str!("../queries/legacy_notification_actions/',
        "LegacyNotificationAtomicOutcomeV1::Applied",
        "LegacyNotificationAtomicOutcomeV1::Replay",
        "browser_grant_delete_returning.sql",
        "mark_out_of_scope_assert.sql",
        "preferences_postcondition_assert.sql",
    )
    for token in required_runtime_tokens:
        if token not in d1_runtime:
            errors.append(f"notification D1 runtime lost token: {token}")
    expected_queries = {
        "assertion_cleanup.sql",
        "audit_insert.sql",
        "browser_grant_assert.sql",
        "browser_grant_delete_returning.sql",
        "changes_assert.sql",
        "clock_now.sql",
        "durable_receipt_assert.sql",
        "effect_insert.sql",
        "mark_authority_assert.sql",
        "mark_authority_snapshot.sql",
        "mark_matching_count.sql",
        "mark_out_of_scope_assert.sql",
        "mark_postcondition_assert.sql",
        "mark_precondition_assert.sql",
        "mark_update.sql",
        "operation_by_key.sql",
        "operation_claim.sql",
        "operation_complete.sql",
        "preferences_authority_assert.sql",
        "preferences_other_actor_assert.sql",
        "preferences_postcondition_assert.sql",
        "preferences_snapshot.sql",
        "preferences_update.sql",
        "proof_insert.sql",
        "receipt_insert.sql",
    }
    if query_names != expected_queries:
        errors.append("notification D1 query surface drifted")
    for token in (
        "legacy_notification_action_operations_v1",
        "legacy_notification_action_receipts_v1",
        "legacy_notification_action_effects_v1",
        "legacy_notification_action_audit_events_v1",
        "legacy_notification_action_proof_consumptions_v1",
        "frame_legacy_notification_operation_immutable_v1",
    ):
        if token not in migration:
            errors.append(f"notification migration proof lost token: {token}")
    for token in (
        '"schema_version": "frame.legacy-notification-actions-sqlite-conformance.v1"',
        '"checked_in_queries_executed": len(SQL)',
        '"race_applied": 1',
        '"race_replayed": 1',
        'parser.add_argument("--evidence", type=Path)',
    ):
        if token not in conformance:
            errors.append(f"notification SQLite conformance lost token: {token}")
    for token in (
        '"frame.web-notification-action-request.v1"',
        "LEGACY_MARK_NOTIFICATIONS_READ_ACTION_ID",
        "LEGACY_UPDATE_NOTIFICATION_PREFERENCES_ACTION_ID",
        "mark_notifications_read",
        "update_notification_preferences",
        "notification_actions_preserve_optional_fields_and_exact_idempotent_transport",
        '"invalid_compatibility_action"',
    ):
        if token not in browser_client:
            errors.append(f"notification browser client proof lost token: {token}")
    required_atomic_steps = {
        "live_actor_and_action_specific_authority_reassertion",
        "tenant_recipient_notification_selection_or_actor_preferences_selection",
        "exact_mark_count_read_time_and_out_of_scope_postcondition",
        "notifications_branch_merge_and_sibling_json_preservation_digest",
        "tenant_actor_action_scoped_idempotency_receipt",
        "business_audit",
        "dashboard_invalidation_effect",
        "one_use_browser_grant_consumption",
    }
    atomicity = fixture.get("atomicity", {})
    if set(atomicity.get("one_transaction", [])) != required_atomic_steps or any(
        atomicity.get(key) != value
        for key, value in {
            "same_key_same_fingerprint": "replay_original_receipt",
            "same_key_different_fingerprint": "conflict",
            "race": "one_apply_one_replay",
            "rollback": "no_partial_mutation_or_consumed_grant",
        }.items()
    ):
        errors.append("notification action fixture lost its atomic retry contract")
    operations = fixture.get("operations", [])
    if not isinstance(operations, list) or {row.get("id") for row in operations} != set(
        expected
    ):
        errors.append("notification action fixture operation set drifted")
        return errors
    report_by_id = {row.get("id"): row for row in report.get("entries", [])}
    source_closure = fixture.get("source_closure", {})
    for operation in operations:
        operation_id = operation["id"]
        identity, input_contract, authority, source_count = expected[operation_id]
        row = report_by_id.get(operation_id, {})
        if (
            operation.get("kind") != "server_action"
            or operation.get("method") != "ACTION"
            or operation.get("legacy_identity") != identity
            or operation.get("input") != input_contract
            or operation.get("authority") != authority
            or operation.get("success")
            != "empty_204_void_and_dashboard_invalidation"
            or operation.get("protected_gates") != ["released_legacy_client_e2e"]
            or operation.get("production_behavior") != "fail_closed_unavailable"
        ):
            errors.append(f"notification action fixture contract drifted: {operation_id}")
        if (
            row.get("kind") != "server_action"
            or row.get("method") != "ACTION"
            or row.get("legacy_path") != identity
            or row.get("contract_evidence", {}).get("success") != "local_contract"
            or row.get("completion", {}).get("local_work") != "complete"
            or row.get("completion", {}).get("protected_gates")
            != ["released_legacy_client_e2e"]
            or row.get("completion", {}).get("production_behavior")
            != "fail_closed_unavailable"
            or len(row.get("sources", [])) != source_count
            or source_closure.get(operation_id) != source_count
        ):
            errors.append(f"notification action report evidence drifted: {operation_id}")
    return errors


def validate_developer_actions_fixture(report: dict[str, Any]) -> list[str]:
    """Bind all eight user-owned developer ACTIONs to one secret-safe carrier."""
    try:
        fixture = json.loads(DEVELOPER_ACTIONS_FIXTURE.read_text(encoding="utf-8"))
        application = APPLICATION_DEVELOPER_ACTIONS.read_text(encoding="utf-8")
        application_lib = APPLICATION_LIB.read_text(encoding="utf-8")
        d1_runtime = CONTROL_DEVELOPER_ACTIONS_RUNTIME.read_text(encoding="utf-8")
        ingress = CONTROL_DEVELOPER_WEB_RUNTIME.read_text(encoding="utf-8")
        migration = DEVELOPER_ACTIONS_MIGRATION.read_text(encoding="utf-8")
        conformance = DEVELOPER_ACTIONS_CONFORMANCE.read_text(encoding="utf-8")
        control_lib = CONTROL_LIB.read_text(encoding="utf-8")
        browser_client = WEB_BROWSER_CLIENT.read_text(encoding="utf-8")
        query_names = {
            path.name for path in DEVELOPER_ACTIONS_QUERY_ROOT.glob("*.sql")
        }
    except (OSError, json.JSONDecodeError) as error:
        return [f"unable to load developer action fixture: {error}"]
    errors: list[str] = []
    expected = {
        "cap-v1-f303e703a4237888": (
            "action://apps/web/actions/developers/create-app.ts#createDeveloperApp",
            "name_and_development_or_production_environment",
            "live_actor_owns_new_app",
            "appId_publicKey_secretKey_object",
            18,
        ),
        "cap-v1-87fd6af55b891cb9": (
            "action://apps/web/actions/developers/update-app.ts#updateDeveloperApp",
            "appId_and_optional_name_environment_logoUrl_patch",
            "live_actor_owns_app",
            "success_true_object",
            13,
        ),
        "cap-v1-9833b16bb80a3299": (
            "action://apps/web/actions/developers/delete-app.ts#deleteDeveloperApp",
            "appId",
            "live_actor_owns_app",
            "success_true_object",
            13,
        ),
        "cap-v1-aa86dd3d5351ec06": (
            "action://apps/web/actions/developers/add-domain.ts#addDeveloperDomain",
            "appId_and_full_origin_domain",
            "live_actor_owns_app",
            "success_true_object",
            13,
        ),
        "cap-v1-f7d8036af53d0eb9": (
            "action://apps/web/actions/developers/remove-domain.ts#removeDeveloperDomain",
            "appId_and_domainId",
            "live_actor_owns_app",
            "success_true_object_even_when_no_domain_row_matches",
            13,
        ),
        "cap-v1-1f1465957551f1c4": (
            "action://apps/web/actions/developers/regenerate-keys.ts#regenerateDeveloperKeys",
            "appId",
            "live_actor_owns_app",
            "publicKey_secretKey_object",
            18,
        ),
        "cap-v1-8328214ed9647abb": (
            "action://apps/web/actions/developers/delete-video.ts#deleteDeveloperVideo",
            "appId_and_videoId",
            "live_actor_owns_app",
            "success_true_object_even_when_no_video_row_matches",
            13,
        ),
        "cap-v1-b822700b545118f6": (
            "action://apps/web/actions/developers/update-auto-topup.ts#updateDeveloperAutoTopUp",
            "appId_enabled_and_optional_thresholdMicroCredits_amountCents",
            "live_actor_owns_app",
            "success_true_object",
            12,
        ),
    }
    transport = fixture.get("transport", {})
    if fixture.get("schema_version") != 1 or transport != {
        "method": "POST",
        "path_prefix": "/api/v1/web/compatibility-actions/",
        "role": "frame_selector_not_legacy_identity",
        "request_schema": "frame.web-developer-action-request.v1",
        "max_body_bytes": 256 * 1024,
        "authentication": "host_only_session",
        "same_origin": "required",
        "csrf": "double_submit_one_use_grant_required",
        "client_idempotency": "required_header_and_body_exact_match",
        "rate_limit_bucket": "developer_api.v1",
        "success_status": 200,
        "cache_control": "no-store, max-age=0",
    }:
        errors.append("developer action fixture lost its Frame ingress contract")
    if fixture.get("presence") != {
        "update_name": "missing_or_nonempty_string; explicit_null_rejected",
        "update_environment": "missing_or_development_or_production; explicit_null_rejected",
        "update_logoUrl": "missing_preserves_existing; null_clears; string_replaces",
        "update_empty_patch": "successful_no_op_with_atomic_receipt",
        "auto_top_up_enabled": "required_boolean",
        "auto_top_up_thresholdMicroCredits": "missing_preserves_existing; explicit_null_rejected",
        "auto_top_up_amountCents": "missing_preserves_existing; explicit_null_rejected",
    }:
        errors.append("developer action fixture presence contract drifted")
    if fixture.get("secret_safety") != {
        "authority": "versioned_local_aead_key_from_exact_64_lowercase_nonzero_hex",
        "storage": "public_and_secret_keys_hashed_and_encrypted_at_rest",
        "plaintext": "returned_once_on_apply_or_unsealed_only_for_same_fingerprint_replay",
        "debug": "all_key_bearing_types_redacted",
        "missing_or_invalid_key": "fail_closed_unavailable_without_mutation",
    }:
        errors.append("developer action fixture secret-safety contract drifted")

    required_application_tokens = (
        "LEGACY_DEVELOPER_PROFILES",
        "LEGACY_CREATE_DEVELOPER_APP_PROFILE",
        "LEGACY_UPDATE_DEVELOPER_APP_PROFILE",
        "LEGACY_DELETE_DEVELOPER_APP_PROFILE",
        "LEGACY_ADD_DEVELOPER_DOMAIN_PROFILE",
        "LEGACY_REMOVE_DEVELOPER_DOMAIN_PROFILE",
        "LEGACY_REGENERATE_DEVELOPER_KEYS_PROFILE",
        "LEGACY_DELETE_DEVELOPER_VIDEO_PROFILE",
        "LEGACY_UPDATE_DEVELOPER_AUTO_TOP_UP_PROFILE",
        "LegacyDeveloperAtomicPortV1",
        "LegacyDeveloperSecretAuthorityV1",
        "LegacyDeveloperAdapterV1",
        'LEGACY_DEVELOPER_PROTECTED_GATES: &[&str] = &["released_legacy_client_e2e"]',
        "update_freezes_missing_null_and_value_logo_semantics",
        "create_replay_uses_journaled_envelope_without_generating_new_keys",
        "execution_debug_redacts_revealed_credentials",
    )
    for token in required_application_tokens:
        if token not in application:
            errors.append(f"developer application proof lost token: {token}")
    if (
        "mod legacy_developer_actions;" not in application_lib
        or "pub use legacy_developer_actions::*;" not in application_lib
    ):
        errors.append("developer action semantics are not exported by frame-application")

    required_ingress_tokens = (
        '"frame.web-developer-action-request.v1"',
        "LEGACY_CREATE_DEVELOPER_APP_OPERATION_ID",
        "LEGACY_UPDATE_DEVELOPER_APP_OPERATION_ID",
        "LEGACY_DELETE_DEVELOPER_APP_OPERATION_ID",
        "LEGACY_ADD_DEVELOPER_DOMAIN_OPERATION_ID",
        "LEGACY_REMOVE_DEVELOPER_DOMAIN_OPERATION_ID",
        "LEGACY_REGENERATE_DEVELOPER_KEYS_OPERATION_ID",
        "LEGACY_DELETE_DEVELOPER_VIDEO_OPERATION_ID",
        "LEGACY_UPDATE_DEVELOPER_AUTO_TOP_UP_OPERATION_ID",
        "OptionalJsonFieldV1::Missing",
        "OptionalJsonFieldV1::Present(serde_json::Value::Null)",
        'request.headers().get("idempotency-key")',
        "LegacyAuthenticatedContextV1::principal_only",
        "D1LegacyDeveloperAtomicPortV1::new",
        "LocalLegacyDeveloperSecretAuthorityV1::from_hex",
        'env.secret("FRAME_LEGACY_DEVELOPER_SECRET_HEX_V1")',
        "consume_session_grant_or_confirm_absent",
    )
    for token in required_ingress_tokens:
        if token not in ingress:
            errors.append(f"developer ingress proof lost token: {token}")
    for token in (
        "mod legacy_developer_actions_runtime;",
        "mod legacy_developer_web_runtime;",
        "legacy_developer_web_runtime::is_action",
        "legacy_developer_action_response",
    ):
        if token not in control_lib:
            errors.append(f"developer route wiring lost token: {token}")

    required_runtime_tokens = (
        "D1LegacyDeveloperAtomicPortV1",
        "impl LegacyDeveloperAtomicPortV1",
        "LocalLegacyDeveloperSecretAuthorityV1",
        "impl LegacyDeveloperSecretAuthorityV1",
        'include_str!("../queries/legacy_developer_actions/',
        "Aes256Gcm",
        "Zeroizing::new(Vec::with_capacity",
        "LegacyDeveloperAtomicOutcomeV1::Applied",
        "LegacyDeveloperAtomicOutcomeV1::Replay",
        "browser_grant_delete_returning.sql",
        "durable_receipt_assert.sql",
    )
    for token in required_runtime_tokens:
        if token not in d1_runtime:
            errors.append(f"developer D1 runtime lost token: {token}")
    expected_queries = {
        "app_authority_assert.sql",
        "app_authority_snapshot.sql",
        "assertion_cleanup.sql",
        "audit_insert.sql",
        "auto_top_up_postcondition_assert.sql",
        "auto_top_up_update.sql",
        "browser_grant_assert.sql",
        "browser_grant_delete_returning.sql",
        "changes_assert.sql",
        "clock_now.sql",
        "create_app_insert.sql",
        "create_postcondition_assert.sql",
        "credit_insert.sql",
        "delete_app.sql",
        "delete_app_postcondition_assert.sql",
        "domain_add_postcondition_assert.sql",
        "domain_delete.sql",
        "domain_insert.sql",
        "domain_remove_postcondition_assert.sql",
        "domain_target_count.sql",
        "durable_receipt_assert.sql",
        "effect_insert.sql",
        "key_insert.sql",
        "operation_by_key.sql",
        "operation_claim.sql",
        "operation_complete.sql",
        "proof_insert.sql",
        "receipt_insert.sql",
        "regenerate_postcondition_assert.sql",
        "revoke_active_keys.sql",
        "update_app.sql",
        "update_postcondition_assert.sql",
        "video_delete.sql",
        "video_postcondition_assert.sql",
        "video_target_count.sql",
    }
    if query_names != expected_queries:
        errors.append("developer D1 query surface drifted")
    for token in (
        "legacy_developer_action_operations_v1",
        "legacy_developer_action_receipts_v1",
        "legacy_developer_action_effects_v1",
        "legacy_developer_action_audit_events_v1",
        "legacy_developer_action_proof_consumptions_v1",
        "frame_legacy_developer_operation_immutable_v1",
        "frame_legacy_developer_receipt_immutable_v1",
    ):
        if token not in migration:
            errors.append(f"developer migration proof lost token: {token}")
    for token in (
        "frame.legacy-developer-actions-sqlite-conformance.v1",
        'parser.add_argument("--evidence", type=Path)',
    ):
        if token not in conformance:
            errors.append(f"developer SQLite conformance lost token: {token}")
    for token in (
        '"frame.web-developer-action-request.v1"',
        "LEGACY_CREATE_DEVELOPER_APP_ACTION_ID",
        "LEGACY_UPDATE_DEVELOPER_APP_ACTION_ID",
        "LEGACY_DELETE_DEVELOPER_APP_ACTION_ID",
        "LEGACY_ADD_DEVELOPER_DOMAIN_ACTION_ID",
        "LEGACY_REMOVE_DEVELOPER_DOMAIN_ACTION_ID",
        "LEGACY_REGENERATE_DEVELOPER_KEYS_ACTION_ID",
        "LEGACY_DELETE_DEVELOPER_VIDEO_ACTION_ID",
        "LEGACY_UPDATE_DEVELOPER_AUTO_TOP_UP_ACTION_ID",
        "create_developer_app",
        "update_developer_app",
        "regenerate_developer_keys",
        "BrowserDeveloperKeyPairV1([redacted])",
        "developer_actions_preserve_optional_fields_keys_and_exact_idempotent_transport",
    ):
        if token not in browser_client:
            errors.append(f"developer browser client proof lost token: {token}")
    required_atomic_steps = {
        "live_actor_and_app_owner_authority_reassertion",
        "action_specific_developer_mutation_and_exact_postcondition",
        "protected_api_key_generation_storage_or_revocation",
        "actor_action_scoped_idempotency_receipt",
        "business_audit",
        "dashboard_invalidation_effect_when_source_declares_it",
        "sealed_secret_replay_receipt_for_key_returning_actions",
        "one_use_browser_grant_consumption",
    }
    atomicity = fixture.get("atomicity", {})
    if set(atomicity.get("one_transaction", [])) != required_atomic_steps or any(
        atomicity.get(key) != value
        for key, value in {
            "same_key_same_fingerprint": "replay_original_receipt_and_exact_same_key_pair_when_applicable",
            "same_key_different_fingerprint": "conflict",
            "race": "one_apply_one_replay",
            "rollback": "no_partial_mutation_secret_receipt_or_consumed_grant",
        }.items()
    ):
        errors.append("developer action fixture lost its atomic retry contract")
    operations = fixture.get("operations", [])
    if not isinstance(operations, list) or {row.get("id") for row in operations} != set(
        expected
    ):
        errors.append("developer action fixture operation set drifted")
        return errors
    report_by_id = {row.get("id"): row for row in report.get("entries", [])}
    source_closure = fixture.get("source_closure", {})
    for operation in operations:
        operation_id = operation["id"]
        identity, input_contract, authority, success, source_count = expected[operation_id]
        row = report_by_id.get(operation_id, {})
        if (
            operation.get("kind") != "server_action"
            or operation.get("method") != "ACTION"
            or operation.get("legacy_identity") != identity
            or operation.get("input") != input_contract
            or operation.get("authority") != authority
            or operation.get("success") != success
            or operation.get("protected_gates") != ["released_legacy_client_e2e"]
            or operation.get("production_behavior") != "fail_closed_unavailable"
        ):
            errors.append(f"developer action fixture contract drifted: {operation_id}")
        if (
            row.get("kind") != "server_action"
            or row.get("method") != "ACTION"
            or row.get("legacy_path") != identity
            or row.get("contract_evidence", {}).get("success") != "local_contract"
            or row.get("completion", {}).get("local_work") != "complete"
            or row.get("completion", {}).get("protected_gates")
            != ["released_legacy_client_e2e"]
            or row.get("completion", {}).get("production_behavior")
            != "fail_closed_unavailable"
            or len(row.get("sources", [])) != source_count
            or source_closure.get(operation_id) != source_count
        ):
            errors.append(f"developer action report evidence drifted: {operation_id}")
    return errors


def validate_membership_actions_fixture(report: dict[str, Any]) -> list[str]:
    """Bind six tenant-scoped membership ACTIONs to one atomic authority fence."""
    try:
        fixture = json.loads(MEMBERSHIP_ACTIONS_FIXTURE.read_text(encoding="utf-8"))
        application = APPLICATION_MEMBERSHIP_ACTIONS.read_text(encoding="utf-8")
        application_lib = APPLICATION_LIB.read_text(encoding="utf-8")
        d1_runtime = CONTROL_MEMBERSHIP_ACTIONS_RUNTIME.read_text(encoding="utf-8")
        ingress = CONTROL_MEMBERSHIP_WEB_RUNTIME.read_text(encoding="utf-8")
        migration = MEMBERSHIP_ACTIONS_MIGRATION.read_text(encoding="utf-8")
        conformance = MEMBERSHIP_ACTIONS_CONFORMANCE.read_text(encoding="utf-8")
        control_lib = CONTROL_LIB.read_text(encoding="utf-8")
        browser_client = WEB_BROWSER_CLIENT.read_text(encoding="utf-8")
        query_names = {
            path.name for path in MEMBERSHIP_ACTIONS_QUERY_ROOT.glob("*.sql")
        }
    except (OSError, json.JSONDecodeError) as error:
        return [f"unable to load membership action fixture: {error}"]
    errors: list[str] = []
    expected = {
        "cap-v1-866dbe8fbbfd7887": (
            "action://apps/web/actions/organization/remove-invite.ts#removeOrganizationInvite",
            "inviteId_and_organizationId",
            "live_actor_active_tenant_owner_or_admin_and_scoped_invite",
            "success_true_object",
            11,
            ["released_legacy_client_e2e"],
            "fail_closed_unavailable",
        ),
        "cap-v1-455046db3d6ef019": (
            "action://apps/web/app/(org)/dashboard/spaces/[spaceId]/actions.ts#addSpaceMember",
            "spaceId_userId_and_admin_or_member_role",
            "live_actor_active_tenant_space_manager_and_target_organization_member",
            "success_true_object",
            12,
            ["released_legacy_client_e2e"],
            "fail_closed_unavailable",
        ),
        "cap-v1-9fc80bdec80fb248": (
            "action://apps/web/app/(org)/dashboard/spaces/[spaceId]/actions.ts#setSpaceMembers",
            "spaceId_userIds_optional_role_and_optional_members",
            "live_actor_active_tenant_space_manager_and_all_targets_organization_members",
            "success_true_and_creator_inclusive_count_object",
            13,
            ["released_legacy_client_e2e"],
            "fail_closed_unavailable",
        ),
        "cap-v1-b177854e2386c877": (
            "action://apps/web/app/(org)/dashboard/spaces/[spaceId]/actions.ts#addSpaceMembers",
            "spaceId_userIds_and_admin_or_member_role",
            "live_actor_active_tenant_space_manager_and_all_targets_organization_members",
            "success_true_added_and_all_existing_members_object",
            12,
            [],
            "serve_exact_action",
        ),
        "cap-v1-38aff8e7221d0260": (
            "action://apps/web/app/(org)/dashboard/spaces/[spaceId]/actions.ts#batchRemoveSpaceMembers",
            "memberIds_array",
            "live_actor_active_tenant_single_space_manager_and_creator_protected",
            "success_true_and_submitted_removed_ids_object",
            12,
            [],
            "serve_exact_action",
        ),
        "cap-v1-135614e516c47bf4": (
            "action://apps/web/app/(org)/dashboard/spaces/[spaceId]/actions.ts#removeSpaceMember",
            "memberId",
            "live_actor_active_tenant_space_manager_and_creator_protected",
            "success_true_object",
            12,
            [],
            "serve_exact_action",
        ),
    }
    transport = fixture.get("transport", {})
    if fixture.get("schema_version") != 1 or transport != {
        "method": "POST",
        "path_prefix": "/api/v1/web/compatibility-actions/",
        "role": "frame_selector_not_legacy_identity",
        "request_schema": "frame.web-membership-action-request.v1",
        "max_body_bytes": 256 * 1024,
        "authentication": "host_only_session",
        "same_origin": "required",
        "csrf": "double_submit_one_use_grant_required",
        "client_idempotency": "required_header_and_body_exact_match",
        "rate_limit_bucket": "organization_library.v1",
        "success_status": 200,
        "cache_control": "no-store, max-age=0",
    }:
        errors.append("membership action fixture lost its Frame ingress contract")
    if fixture.get("presence") != {
        "add_many_userIds": "required_array; duplicate_new_ids_conflict_without_partial_insert",
        "batch_remove_memberIds": "required_array; empty_or_no_match_returns_empty_removed",
        "set_role": "missing_defaults_to_member; explicit_null_rejected",
        "set_members": "missing_selects_userIds; present_array_wins_even_when_empty; explicit_null_rejected",
        "set_userIds": "required_array_and_validated_even_when_members_is_present",
        "member_role": "admin_or_Admin_or_member_normalized_to_admin_or_member",
    }:
        errors.append("membership action fixture presence contract drifted")
    if fixture.get("authorization") != {
        "remove_invite": "active_tenant_owner_or_admin_and_invite_scoped_to_active_tenant",
        "space_actions": "active_tenant_owner_or_admin_or_space_admin",
        "target_members": "organization_owner_or_current_organization_member",
        "set_creator": "space_creator_forced_to_admin_and_cannot_be_removed",
        "remove_creator": "space_creator_cannot_be_removed",
        "batch_remove_scope": "all_discovered_rows_must_belong_to_one_space",
        "cross_tenant_or_stale_authority": "not_found_without_mutation",
    }:
        errors.append("membership action fixture authorization contract drifted")

    for token in (
        "LEGACY_REMOVE_ORGANIZATION_INVITE_OPERATION_ID",
        "LEGACY_ADD_SPACE_MEMBER_OPERATION_ID",
        "LEGACY_SET_SPACE_MEMBERS_OPERATION_ID",
        "LEGACY_ADD_SPACE_MEMBERS_OPERATION_ID",
        "LEGACY_BATCH_REMOVE_SPACE_MEMBERS_OPERATION_ID",
        "LEGACY_REMOVE_SPACE_MEMBER_OPERATION_ID",
        "LEGACY_REMOVE_ORGANIZATION_INVITE_SOURCES",
        "LEGACY_ADD_SPACE_MEMBER_SOURCES",
        "LEGACY_SET_SPACE_MEMBERS_SOURCES",
        "LEGACY_ADD_SPACE_MEMBERS_SOURCES",
        "LEGACY_BATCH_REMOVE_SPACE_MEMBERS_SOURCES",
        "LEGACY_REMOVE_SPACE_MEMBER_SOURCES",
        "LegacyMembershipAtomicPortV1",
        "LegacyMembershipAdapterV1",
        'LEGACY_MEMBERSHIP_PROTECTED_GATES: &[&str] = &["released_legacy_client_e2e"]',
        "LEGACY_MEMBERSHIP_NO_PROTECTED_GATES",
        "production_promoted: true",
        "profiles_pin_exact_provider_free_source_closures",
        "set_fingerprint_uses_the_effective_creator_inclusive_set",
        "replay_returns_the_original_result_without_a_new_projection",
    ):
        if token not in application:
            errors.append(f"membership application proof lost token: {token}")
    if (
        "mod legacy_membership_actions;" not in application_lib
        or "pub use legacy_membership_actions::*;" not in application_lib
    ):
        errors.append("membership action semantics are not exported by frame-application")

    for token in (
        '"frame.web-membership-action-request.v1"',
        "LEGACY_REMOVE_ORGANIZATION_INVITE_OPERATION_ID",
        "LEGACY_ADD_SPACE_MEMBER_OPERATION_ID",
        "LEGACY_SET_SPACE_MEMBERS_OPERATION_ID",
        "LEGACY_ADD_SPACE_MEMBERS_OPERATION_ID",
        "LEGACY_BATCH_REMOVE_SPACE_MEMBERS_OPERATION_ID",
        "LEGACY_REMOVE_SPACE_MEMBER_OPERATION_ID",
        "OptionalJsonFieldV1::Missing",
        "OptionalJsonFieldV1::Present(_)",
        '"role":null',
        '"members":null',
        'request.headers().get("idempotency-key")',
        "trusted_active_organization_id",
        "D1LegacyMembershipAtomicPortV1::new",
        "consume_session_grant_or_confirm_absent",
    ):
        if token not in ingress:
            errors.append(f"membership ingress proof lost token: {token}")
    for token in (
        "mod legacy_membership_actions_runtime;",
        "mod legacy_membership_web_runtime;",
        "legacy_membership_web_runtime::is_action",
        "legacy_membership_action_response",
    ):
        if token not in control_lib:
            errors.append(f"membership route wiring lost token: {token}")

    for token in (
        "D1LegacyMembershipAtomicPortV1",
        "impl LegacyMembershipAtomicPortV1",
        'include_str!("../queries/legacy_membership_actions/',
        "LegacyMembershipAtomicOutcomeV1::Applied",
        "LegacyMembershipAtomicOutcomeV1::Replay",
        "browser_grant_delete_returning.sql",
        "durable_receipt_assert.sql",
        "authority_generation_postcondition_assert.sql",
        "revoked_grant_postcondition_assert.sql",
    ):
        if token not in d1_runtime:
            errors.append(f"membership D1 runtime lost token: {token}")
    expected_queries = {
        "add_absent_assert.sql",
        "add_insert.sql",
        "add_postcondition_assert.sql",
        "aliases_added_changes_assert.sql",
        "aliases_final_changes_assert.sql",
        "aliases_previous_changes_assert.sql",
        "assertion_cleanup.sql",
        "audit_insert.sql",
        "authority_generation_changes_assert.sql",
        "authority_generation_postcondition_assert.sql",
        "authority_generation_upsert.sql",
        "authority_subject_insert.sql",
        "authority_subject_insert_added.sql",
        "authority_subject_insert_removed.sql",
        "authority_subjects_by_operation.sql",
        "browser_grant_assert.sql",
        "browser_grant_delete_returning.sql",
        "bulk_add_duplicate_assert.sql",
        "bulk_add_insert.sql",
        "bulk_add_postcondition_assert.sql",
        "bulk_added_changes_assert.sql",
        "changes_assert.sql",
        "clock_now.sql",
        "creator_graph_assert.sql",
        "durable_receipt_assert.sql",
        "effect_insert.sql",
        "effect_insert_bulk_add.sql",
        "final_changes_assert.sql",
        "final_creator_upsert.sql",
        "final_members_by_operation.sql",
        "final_members_insert.sql",
        "invite_authority_assert.sql",
        "invite_authority_snapshot.sql",
        "invite_delete.sql",
        "invite_postcondition_assert.sql",
        "invite_target_assert.sql",
        "member_alias_added_postcondition_assert.sql",
        "member_alias_insert_added.sql",
        "member_alias_insert_all.sql",
        "member_alias_postcondition_assert.sql",
        "member_alias_remove_previous.sql",
        "member_alias_targets.sql",
        "operation_by_key.sql",
        "operation_claim.sql",
        "operation_complete.sql",
        "out_of_scope_assert.sql",
        "previous_aliases_complete_assert.sql",
        "previous_bound_assert.sql",
        "previous_changes_assert.sql",
        "previous_members_by_operation.sql",
        "previous_snapshot_insert.sql",
        "proof_insert.sql",
        "receipt_insert_add.sql",
        "receipt_insert_batch_remove.sql",
        "receipt_insert_bulk_add.sql",
        "receipt_insert_invite.sql",
        "receipt_insert_remove_member.sql",
        "receipt_insert_set.sql",
        "removal_changes_assert.sql",
        "removal_creator_assert.sql",
        "removal_delete.sql",
        "removal_no_match_assert.sql",
        "removal_postcondition_assert.sql",
        "removal_targets_assert.sql",
        "removal_targets_insert.sql",
        "removed_target_graph_assert.sql",
        "revoke_grants.sql",
        "revoked_grant_changes_assert.sql",
        "revoked_grant_postcondition_assert.sql",
        "revoked_grant_snapshot_insert.sql",
        "set_delete.sql",
        "set_insert.sql",
        "set_postcondition_assert.sql",
        "space_authority_assert.sql",
        "space_authority_snapshot.sql",
        "target_graph_assert.sql",
        "tenant_authority_assert.sql",
        "tenant_authority_snapshot.sql",
    }
    if query_names != expected_queries:
        errors.append("membership D1 query surface drifted")
    for token in (
        "legacy_membership_action_operations_v1",
        "legacy_membership_action_receipts_v1",
        "legacy_membership_action_effects_v1",
        "legacy_membership_action_audit_events_v1",
        "legacy_membership_action_proof_consumptions_v1",
        "legacy_membership_action_authority_subjects_v1",
        "legacy_membership_action_revoked_grants_v1",
        "frame_legacy_membership_operation_immutable_v1",
        "frame_legacy_membership_receipt_immutable_v1",
    ):
        if token not in migration:
            errors.append(f"membership migration proof lost token: {token}")
    for token in (
        "frame.legacy-membership-actions-sqlite-conformance.v2",
        'parser.add_argument("--evidence", type=Path)',
    ):
        if token not in conformance:
            errors.append(f"membership SQLite conformance lost token: {token}")
    for token in (
        '"frame.web-membership-action-request.v1"',
        "LEGACY_REMOVE_ORGANIZATION_INVITE_ACTION_ID",
        "LEGACY_ADD_SPACE_MEMBER_ACTION_ID",
        "LEGACY_SET_SPACE_MEMBERS_ACTION_ID",
        "LEGACY_ADD_SPACE_MEMBERS_ACTION_ID",
        "LEGACY_BATCH_REMOVE_SPACE_MEMBERS_ACTION_ID",
        "LEGACY_REMOVE_SPACE_MEMBER_ACTION_ID",
        "remove_organization_invite",
        "add_space_member",
        "set_space_members",
        "add_space_members",
        "batch_remove_space_members",
        "remove_space_member",
        "membership_actions_preserve_presence_and_exact_idempotent_transport",
    ):
        if token not in browser_client:
            errors.append(f"membership browser client proof lost token: {token}")
    required_atomic_steps = {
        "live_actor_active_tenant_and_membership_authority_reassertion",
        "invite_or_space_target_graph_precondition",
        "exact_single_or_bulk_insert_delete_or_creator_inclusive_replacement",
        "authority_generation_bump_and_grant_revocation_for_changed_membership",
        "tenant_actor_action_scoped_idempotency_receipt",
        "business_audit",
        "validated_dashboard_invalidation_effect",
        "one_use_browser_grant_consumption",
    }
    atomicity = fixture.get("atomicity", {})
    if set(atomicity.get("one_transaction", [])) != required_atomic_steps or any(
        atomicity.get(key) != value
        for key, value in {
            "same_key_same_fingerprint": "replay_original_receipt",
            "same_key_different_fingerprint": "conflict",
            "race": "one_apply_one_replay",
            "rollback": "no_partial_membership_authority_or_consumed_grant",
        }.items()
    ):
        errors.append("membership action fixture lost its atomic retry contract")
    operations = fixture.get("operations", [])
    if not isinstance(operations, list) or {row.get("id") for row in operations} != set(
        expected
    ):
        errors.append("membership action fixture operation set drifted")
        return errors
    report_by_id = {row.get("id"): row for row in report.get("entries", [])}
    source_closure = fixture.get("source_closure", {})
    for operation in operations:
        operation_id = operation["id"]
        (
            identity,
            input_contract,
            authority,
            success,
            source_count,
            protected_gates,
            production_behavior,
        ) = expected[operation_id]
        row = report_by_id.get(operation_id, {})
        if (
            operation.get("kind") != "server_action"
            or operation.get("method") != "ACTION"
            or operation.get("legacy_identity") != identity
            or operation.get("input") != input_contract
            or operation.get("authority") != authority
            or operation.get("success") != success
            or operation.get("protected_gates") != protected_gates
            or operation.get("production_behavior") != production_behavior
        ):
            errors.append(f"membership action fixture contract drifted: {operation_id}")
        if (
            row.get("kind") != "server_action"
            or row.get("method") != "ACTION"
            or row.get("legacy_path") != identity
            or row.get("contract_evidence", {}).get("success") != "local_contract"
            or row.get("completion", {}).get("local_work") != "complete"
            or row.get("completion", {}).get("protected_gates") != protected_gates
            or row.get("completion", {}).get("production_behavior")
            != production_behavior
            or len(row.get("sources", [])) != source_count
            or source_closure.get(operation_id) != source_count
        ):
            errors.append(f"membership action report evidence drifted: {operation_id}")
    return errors


def validate_folder_crud_fixture(report: dict[str, Any]) -> list[str]:
    """Bind Cap's mobile/Effect wire carriers to the atomic folder D1 port."""
    try:
        fixture = json.loads(FOLDER_CRUD_FIXTURE.read_text(encoding="utf-8"))
        application = APPLICATION_FOLDER_CRUD.read_text(encoding="utf-8")
        application_lib = APPLICATION_LIB.read_text(encoding="utf-8")
        d1_runtime = CONTROL_FOLDER_CRUD_RUNTIME.read_text(encoding="utf-8")
        ingress = CONTROL_FOLDER_CRUD_WEB_RUNTIME.read_text(encoding="utf-8")
        migration = FOLDER_CRUD_MIGRATION.read_text(encoding="utf-8")
        conformance = FOLDER_CRUD_CONFORMANCE.read_text(encoding="utf-8")
        control_lib = CONTROL_LIB.read_text(encoding="utf-8")
        routing = CONTROL_ROUTING.read_text(encoding="utf-8")
        workflow = parity_workflow_contract()
        query_names = {path.name for path in FOLDER_CRUD_QUERY_ROOT.glob("*.sql")}
    except (OSError, json.JSONDecodeError) as error:
        return [f"unable to load folder CRUD fixture: {error}"]

    errors: list[str] = []
    expected = {
        "cap-v1-7160c4389375c682": (
            "route",
            "POST",
            "/api/mobile/folders",
            9,
            "fd34af459bb9b5bac46118ad808e9065887fa4201dd2db67dde80bc295d17897",
            [],
            "serve_exact_d1",
            "optional",
        ),
        "cap-v1-9e125712cee9ce5a": (
            "rpc",
            "RPC",
            "/api/erpc#FolderCreate",
            18,
            "4bd7c5ed8e4c94c34649ea2b23b6366d3c40a79f77d8e67021ad36e37f914407",
            ["human_approval"],
            "fail_closed_unavailable",
            "optional",
        ),
        "cap-v1-eea1796482b3af28": (
            "rpc",
            "RPC",
            "/api/erpc#FolderDelete",
            7,
            "798c68c4b062c78e6cdb845338adc4c45e5bd20bda9d9f942957795695be12a4",
            ["human_approval"],
            "fail_closed_unavailable",
            "forbidden",
        ),
        "cap-v1-a193e9e08b2c3f7d": (
            "rpc",
            "RPC",
            "/api/erpc#FolderUpdate",
            18,
            "2881b2ca39ba24c104349eebcc87bdc5d9427b0d3b2e2dc71c41837371d61102",
            ["human_approval"],
            "fail_closed_unavailable",
            "optional",
        ),
    }
    if fixture.get("schema_version") != 1 or fixture.get("family") != "folder_crud.v1":
        errors.append("folder CRUD fixture schema drifted")
    if fixture.get("reference_commit") != REFERENCE_COMMIT:
        errors.append("folder CRUD fixture reference commit drifted")
    if fixture.get("mobile") != {
        "identity": "POST /api/mobile/folders",
        "request": {
            "content_type": "application/json",
            "shape": "{name:string,color?:normal|blue|red|yellow}",
            "excess_fields": "stripped",
            "null_optional_fields": "invalid",
        },
        "authentication": "host_session_or_36_character_legacy_api_key",
        "success": "{id,name,color,parentId:null,videoCount:0}",
        "idempotency": "optional_Idempotency-Key_bound_when_supplied_server_generated_when_absent",
    }:
        errors.append("folder CRUD mobile wire contract drifted")
    effect_rpc = fixture.get("effect_rpc", {})
    if any(
        effect_rpc.get(key) != value
        for key, value in {
            "identity": "POST /api/erpc",
            "serialization": "@effect/rpc@0.71.2 RpcSerialization.layerJson",
            "request_cardinality": "one_tagged_request_object_per_post_no_batching",
            "authentication": "host_session_cookie_only",
            "success": "[{_tag:'Exit',requestId,exit:{_tag:'Success'}}]",
            "malformed_payload": "Exit.Failure.Cause.Die",
            "unknown_tag": "top_level_Defect",
            "idempotency_header": "absent",
        }.items()
    ):
        errors.append("folder CRUD Effect-RPC wire contract drifted")
    if fixture.get("scope_invariant", {}).get("classification") != (
        "rpc_local_contract_human_approval_required_not_production_exact"
    ):
        errors.append("folder CRUD cross-namespace protected gate drifted")

    operations = fixture.get("operations", [])
    if not isinstance(operations, list) or {row.get("id") for row in operations} != set(expected):
        errors.append("folder CRUD fixture operation set drifted")
        return errors
    report_by_id = {row.get("id"): row for row in report.get("entries", [])}
    for operation in operations:
        operation_id = operation["id"]
        kind, method, identity, count, manifest, gates, behavior, idempotency = expected[
            operation_id
        ]
        row = report_by_id.get(operation_id, {})
        if (
            operation.get("kind") != kind
            or operation.get("method") != method
            or operation.get("legacy_identity") != identity
            or operation.get("source_count") != count
            or operation.get("source_manifest_sha256") != manifest
            or operation.get("protected_gates") != gates
            or operation.get("production_behavior") != behavior
        ):
            errors.append(f"folder CRUD fixture contract drifted: {operation_id}")
        if (
            row.get("kind") != kind
            or row.get("method") != method
            or row.get("legacy_path") != identity
            or len(row.get("sources", [])) != count
            or canonical_json_sha256(row.get("sources", [])) != manifest
            or row.get("security", {}).get("idempotency") != idempotency
            or row.get("contract_evidence", {}).get("success") != "local_contract"
            or set(row.get("contract_evidence", {}).values()) != {"local_contract"}
            or row.get("completion", {}).get("local_work") != "complete"
            or row.get("completion", {}).get("protected_gates") != gates
            or row.get("completion", {}).get("production_behavior") != behavior
        ):
            errors.append(f"folder CRUD report evidence drifted: {operation_id}")

    for token in (
        "LEGACY_MOBILE_CREATE_FOLDER_SOURCES",
        "LEGACY_RPC_FOLDER_CREATE_SOURCES",
        "LEGACY_RPC_FOLDER_DELETE_SOURCES",
        "LEGACY_RPC_FOLDER_UPDATE_SOURCES",
        "ForbiddenServerGenerated",
        "mapped_source_id",
        "ParentMissing",
        "RecursiveDefinition",
        "ScopeConflict",
        "profiles_pin_exact_inventory_identities_and_source_closures",
    ):
        if token not in application:
            errors.append(f"folder CRUD application proof lost token: {token}")
    if (
        "mod legacy_folder_crud;" not in application_lib
        or "pub use legacy_folder_crud::*;" not in application_lib
    ):
        errors.append("folder CRUD semantics are not exported by frame-application")
    for token in (
        "decode_mobile_create",
        'request.headers().get("idempotency-key")',
        "authenticate_host_only_browser_session",
        "split(' ').nth(1)",
        "FolderCreate",
        "FolderDelete",
        "FolderUpdate",
        "LegacyFolderParentPatchV1::Absent",
        "LegacyFolderParentPatchV1::Root",
        "LegacyFolderParentPatchV1::Parent",
        "rpc_success",
        "rpc_typed_failure",
        "rpc_die",
        "rpc_defect",
        "public_page_strips_logo_url_but_validates_effect_schema_fields",
        "rpc_transport_rejects_batching_and_unknown_tags_without_json_rpc",
    ):
        if token not in ingress:
            errors.append(f"folder CRUD HTTP ingress proof lost token: {token}")
    for token in (
        "D1LegacyFolderCrudAtomicPortV1",
        "PARENT_SENTINEL",
        "CYCLE_SENTINEL",
        "SCOPE_SENTINEL",
        "EFFECT_INSERT_SQL",
        "AUDIT_INSERT_SQL",
        ".batch(statements)",
    ):
        if token not in d1_runtime:
            errors.append(f"folder CRUD D1 runtime proof lost token: {token}")
    for token in (
        "legacy_folder_crud_operations_v1",
        "legacy_folder_crud_receipts_v1",
        "legacy_folder_crud_effects_v1",
        "legacy_folder_crud_audit_events_v1",
        "frame_legacy_folder_crud_parent_v1",
        "frame_legacy_folder_crud_cycle_v1",
        "frame_legacy_folder_crud_scope_v1",
    ):
        if token not in migration:
            errors.append(f"folder CRUD migration lost token: {token}")
    if len(query_names) != 29:
        errors.append("folder CRUD checked-in SQL closure must contain exactly 29 queries")
    for token in (
        "test_mobile_create_exact_response_replay_and_conflict",
        "test_rpc_create_scopes_owner_pro_and_tenant_non_disclosure",
        "test_update_presence_merge_cycle_and_descendant_depth",
        "test_recursive_delete_reparents_personal_space_and_organization_products",
        "test_scope_guards_stale_authority_and_atomic_rollback",
        "test_durable_evidence_is_immutable_and_plaintext_free",
        "assert len(SQL) == 29",
    ):
        if token not in conformance:
            errors.append(f"folder CRUD SQLite proof lost token: {token}")
    for token in (
        "mod legacy_folder_crud_runtime;",
        "mod legacy_folder_crud_web_runtime;",
        "legacy_folder_crud_web_runtime::mobile_create_response",
        "legacy_folder_crud_web_runtime::effect_rpc_response",
    ):
        if token not in control_lib:
            errors.append(f"folder CRUD control-plane wiring lost token: {token}")
    for token in (
        "LegacyMobileFolders",
        "LegacyEffectRpc",
        '"/api/mobile/folders"',
        '"/api/erpc"',
    ):
        if token not in routing:
            errors.append(f"folder CRUD raw routing lost token: {token}")
    for token in (
        "legacy-folder-crud-sqlite-conformance.py",
        "legacy-folder-crud-sqlite-conformance.json",
        "fixtures/api-parity/v1/folder-crud.json",
        "frame-application --lib legacy_folder_crud",
        "frame-control-plane --lib legacy_folder_crud",
    ):
        if token not in workflow:
            errors.append(f"folder CRUD workflow proof lost token: {token}")
    return errors


def validate_collaboration_fixture(report: dict[str, Any]) -> list[str]:
    """Bind all six asymmetric Cap collaboration mutations to exact ingress and D1."""
    try:
        fixture = json.loads(COLLABORATION_FIXTURE.read_text(encoding="utf-8"))
        application = APPLICATION_COLLABORATION.read_text(encoding="utf-8")
        application_lib = APPLICATION_LIB.read_text(encoding="utf-8")
        d1_runtime = CONTROL_COLLABORATION_RUNTIME.read_text(encoding="utf-8")
        ingress = CONTROL_COLLABORATION_WEB_RUNTIME.read_text(encoding="utf-8")
        migration = COLLABORATION_MIGRATION.read_text(encoding="utf-8")
        conformance = COLLABORATION_CONFORMANCE.read_text(encoding="utf-8")
        control_lib = CONTROL_LIB.read_text(encoding="utf-8")
        routing = CONTROL_ROUTING.read_text(encoding="utf-8")
        registry = CONTROL_RUNTIME.read_text(encoding="utf-8")
        workflow = parity_workflow_contract()
        query_names = {path.name for path in COLLABORATION_QUERY_ROOT.glob("*.sql")}
    except (OSError, json.JSONDecodeError) as error:
        return [f"unable to load collaboration fixture: {error}"]

    errors: list[str] = []
    expected = {
        "cap-v1-661d23fdcca80bd2": (
            "route", "POST", "/api/mobile/caps/:id/comments", 2,
            "9ff2a74a8cedde6c7164ca890a85733a38623ef5de0dedffae49b7d6947b2b60",
            "session_or_api_key", "serve_exact_d1",
        ),
        "cap-v1-bd59425c2e7074ae": (
            "route", "POST", "/api/mobile/caps/:id/reactions", 2,
            "8cf2f03083338bc9a28948ed7399533d66fcbcced7d02138801337b8710720d8",
            "session_or_api_key", "serve_exact_d1",
        ),
        "cap-v1-b6ec2f719de27105": (
            "route", "DELETE", "/api/mobile/comments/:id", 2,
            "43c02de6f1b9b5b7b028f878f019ba2d4b593ea2b177949d1dc7824e49e151e7",
            "session_or_api_key", "serve_exact_d1",
        ),
        "cap-v1-f3f5e53c019f944a": (
            "route", "DELETE", "/api/video/comment/delete", 1,
            "b61bb2da1d688d3b291755938dd79def4c67c74443674339f2c7873180999907",
            "session", "serve_exact_d1",
        ),
        "cap-v1-f74174457880eadc": (
            "server_action", "ACTION",
            "action://apps/web/actions/videos/delete-comment.ts#deleteComment", 1,
            "8bb9e6475b36c3950f0715d4bf0ab7ed2349dc921680235ddef4e96e19165f91",
            "session", "serve_exact_action",
        ),
        "cap-v1-dbe600b35683c827": (
            "server_action", "ACTION",
            "action://apps/web/actions/videos/new-comment.ts#newComment", 1,
            "9918435d3881d495be8733f7593f8d4438bf176e9a64cfbdcb4723ed8beed033",
            "session", "serve_exact_action",
        ),
    }
    if (
        fixture.get("schema_version") != 1
        or fixture.get("family") != "collaboration_comments.v1"
        or fixture.get("reference_commit") != REFERENCE_COMMIT
    ):
        errors.append("collaboration fixture schema or reference drifted")
    if fixture.get("transport") != {
        "mobile_authentication": "host_session_or_authorization_second_space_token_exactly_36_characters",
        "web_route_authentication": "host_only_session_missing_session_is_source_400_missing_required_data",
        "action_path_prefix": "/api/v1/web/compatibility-actions/",
        "action_request_schema": "frame.web-collaboration-action-request.v1",
        "action_same_origin": "required",
        "action_csrf": "double_submit_one_use_grant_required",
        "client_idempotency": "required_Idempotency-Key_and_action_body_exact_match",
        "max_body_bytes": 262144,
        "rate_limit_bucket": "collaboration_notifications.v1",
        "cache_control": "no-store, max-age=0",
    }:
        errors.append("collaboration transport contract drifted")
    semantics = fixture.get("source_semantics", {})
    for token in (
        "JS_trim_rejects_empty",
        "video_owner_or_active_organization_shared_video_member",
        "authored_target_only_without_video_child_or_notification_scope",
        "commentId_presence_required_but_empty_value_reaches_lookup",
        "same_actor_direct_replies_only",
        "caller_parent_branch_selects_notification_cleanup",
        "preserve_whitespace_allow_empty_root_and_orphan_or_cross_video_parent",
        "after_insert_best_effort_failure_swallowed",
        "same_transaction_failure_rolls_back_comment",
    ):
        if token not in "\n".join(str(value) for value in semantics.values()):
            errors.append(f"collaboration source semantics lost token: {token}")
    atomicity = fixture.get("atomicity", {})
    if (
        atomicity.get("same_key_same_fingerprint") != "replay_original_projection"
        or atomicity.get("same_key_different_fingerprint") != "conflict"
        or atomicity.get("delete_bound") != 100000
        or atomicity.get("row_100001") != "abort_without_partial_delete"
        or atomicity.get("immutable_evidence") is not True
    ):
        errors.append("collaboration atomic retry or bounded-delete contract drifted")

    operations = fixture.get("operations", [])
    if not isinstance(operations, list) or {row.get("id") for row in operations} != set(expected):
        errors.append("collaboration fixture operation set drifted")
        return errors
    report_by_id = {row.get("id"): row for row in report.get("entries", [])}
    for operation in operations:
        operation_id = operation["id"]
        kind, method, identity, source_count, manifest, auth, behavior = expected[operation_id]
        row = report_by_id.get(operation_id, {})
        if (
            operation.get("kind") != kind
            or operation.get("method") != method
            or operation.get("legacy_identity") != identity
            or operation.get("source_count") != source_count
            or operation.get("source_manifest_sha256") != manifest
            or operation.get("protected_gates") != []
            or operation.get("production_behavior") != behavior
        ):
            errors.append(f"collaboration fixture contract drifted: {operation_id}")
        if (
            row.get("kind") != kind
            or row.get("method") != method
            or row.get("legacy_path") != identity
            or row.get("auth") != auth
            or len(row.get("sources", [])) != source_count
            or canonical_json_sha256(row.get("sources", [])) != manifest
            or row.get("security", {}).get("idempotency") != "required"
            or row.get("security", {}).get("rate_limit_bucket")
            != "collaboration_notifications.v1"
            or set(row.get("contract_evidence", {}).values()) != {"local_contract"}
            or row.get("completion", {}).get("local_work") != "complete"
            or row.get("completion", {}).get("protected_gates") != []
            or row.get("completion", {}).get("production_behavior") != behavior
        ):
            errors.append(f"collaboration report five-axis evidence drifted: {operation_id}")

    for token in (
        "LEGACY_MOBILE_CREATE_COMMENT_SOURCES",
        "LEGACY_MOBILE_CREATE_REACTION_SOURCES",
        "LEGACY_MOBILE_DELETE_COMMENT_SOURCES",
        "LEGACY_WEB_DELETE_COMMENT_ROUTE_SOURCES",
        "LEGACY_WEB_DELETE_COMMENT_ACTION_SOURCES",
        "LEGACY_WEB_NEW_COMMENT_ACTION_SOURCES",
        "trim_ecmascript",
        "caller_parent_id",
        "LEGACY_COLLABORATION_MAX_DELETE_ROWS",
        "mobile_accepts_api_keys_but_web_surfaces_remain_session_only",
    ):
        if token not in application:
            errors.append(f"collaboration application proof lost token: {token}")
    if (
        "mod legacy_collaboration;" not in application_lib
        or "pub use legacy_collaboration::*;" not in application_lib
    ):
        errors.append("collaboration semantics are not exported by frame-application")
    for token in (
        "frame.web-collaboration-action-request.v1",
        "split(' ').nth(1)",
        "authenticate_host_only_browser_session",
        "trusted_active_organization_id",
        "commentId",
        "RequiredNullableString",
        "consume_session_grant_or_confirm_absent",
        "iso_utc",
        "mobile_wire_requires_nullable_timestamp_and_strips_excess_fields",
        "timestamp_projection_matches_ecmascript_json_dates",
    ):
        if token not in ingress:
            errors.append(f"collaboration ingress proof lost token: {token}")
    for token in (
        "D1LegacyCollaborationAtomicPortV1",
        "NOTIFICATION_ATTEMPT_INSERT_SQL",
        "notification_failure_rolls_back_core",
        ".batch(statements)",
        "decode_existing",
    ):
        if token not in d1_runtime:
            errors.append(f"collaboration D1 runtime proof lost token: {token}")
    for token in (
        "legacy_collaboration_operations_v1",
        "legacy_collaboration_receipts_v1",
        "legacy_collaboration_effects_v1",
        "legacy_collaboration_audit_events_v1",
        "legacy_collaboration_notification_attempts_v1",
        "frame_legacy_collaboration_operation_immutable_v1",
        "frame_legacy_collaboration_receipt_immutable_v1",
    ):
        if token not in migration:
            errors.append(f"collaboration migration proof lost token: {token}")
    if len(query_names) != 28:
        errors.append("collaboration checked-in SQL closure must contain exactly 28 queries")
    for token in (
        "test_mobile_owner_and_shared_authority",
        "test_create_notification_failure_is_post_commit_and_swallowed",
        "test_web_route_deletes_only_authored_target_and_authored_direct_replies",
        "test_action_reply_selector_uses_caller_parent_not_database_parent",
        "test_action_notification_delete_failure_rolls_back_comment_and_operation",
        "test_delete_and_notification_staging_bounds_fail_closed",
        "test_replay_identity_conflict_and_receipt_immutability",
        'parser.add_argument("--evidence", "--evidence-out"',
    ):
        if token not in conformance:
            errors.append(f"collaboration SQLite proof lost token: {token}")
    for token in (
        "mod legacy_collaboration_runtime;",
        "mod legacy_collaboration_web_runtime;",
        "mobile_create_comment_response",
        "mobile_create_reaction_response",
        "mobile_delete_comment_response",
        "web_delete_comment_response",
        "legacy_collaboration_action_response",
    ):
        if token not in control_lib:
            errors.append(f"collaboration control-plane wiring lost token: {token}")
    for token in (
        "LegacyMobileCapComments",
        "LegacyMobileCapReactions",
        "LegacyMobileComment",
        "LegacyWebCommentDelete",
        '"/api/video/comment/delete"',
    ):
        if token not in routing:
            errors.append(f"collaboration raw routing lost token: {token}")
    for token in (
        "LegacyRegistrationSourcesV1::Collaboration",
        "LegacyCollaborationInvocationV1",
        "dispatch_collaboration",
        "LEGACY_MOBILE_CREATE_COMMENT_OPERATION_ID",
        "LEGACY_WEB_NEW_COMMENT_ACTION_OPERATION_ID",
    ):
        if token not in registry:
            errors.append(f"collaboration registry proof lost token: {token}")
    for token in (
        "legacy-collaboration-sqlite-conformance.py",
        "legacy-collaboration-sqlite-conformance.json",
        "fixtures/api-parity/v1/collaboration-actions.json",
        "frame-application --lib legacy_collaboration",
        "frame-control-plane --lib legacy_collaboration",
    ):
        if token not in workflow:
            errors.append(f"collaboration workflow proof lost token: {token}")
    return errors


def validate_user_account_fixture(report: dict[str, Any]) -> list[str]:
    """Bind all eight user/account identities to exact carriers and atomic D1."""
    try:
        fixture = json.loads(USER_ACCOUNT_FIXTURE.read_text(encoding="utf-8"))
        application = APPLICATION_USER_ACCOUNT.read_text(encoding="utf-8")
        application_lib = APPLICATION_LIB.read_text(encoding="utf-8")
        d1_runtime = CONTROL_USER_ACCOUNT_RUNTIME.read_text(encoding="utf-8")
        ingress = CONTROL_USER_ACCOUNT_WEB_RUNTIME.read_text(encoding="utf-8")
        migration = USER_ACCOUNT_MIGRATION.read_text(encoding="utf-8")
        conformance = USER_ACCOUNT_CONFORMANCE.read_text(encoding="utf-8")
        control_lib = CONTROL_LIB.read_text(encoding="utf-8")
        folder_rpc_ingress = CONTROL_FOLDER_CRUD_WEB_RUNTIME.read_text(encoding="utf-8")
        routing = CONTROL_ROUTING.read_text(encoding="utf-8")
        workflow = parity_workflow_contract()
        query_names = {path.name for path in USER_ACCOUNT_QUERY_ROOT.glob("*.sql")}
    except (OSError, json.JSONDecodeError) as error:
        return [f"unable to load user/account fixture: {error}"]

    errors: list[str] = []
    expected = {
        "cap-v1-fdc3d5d49bb5ad6d": (
            "route",
            "POST",
            "/api/settings/user/name",
            4,
            "8bb2b5b6398a8d197418c621a5de8fdded75dc397fdb79e5d655df839861a197",
            [],
            "serve_exact_d1",
            "forbidden",
        ),
        "cap-v1-c7827a1de563f856": (
            "rpc",
            "RPC",
            "/api/erpc#UserCompleteOnboardingStep",
            19,
            "1005767c78e0243c7a9909fb9ff75f03104f317d9c4176cda437fe51c87a8cdb",
            ["provider_execution"],
            "fail_closed_unavailable",
            "forbidden",
        ),
        "cap-v1-295a3eb4ba9ffe6f": (
            "rpc",
            "RPC",
            "/api/erpc#UserUpdate",
            15,
            "1600e83d8be61c1613fba8181414bcb6e3546edd0f4454da8aec39536cc20dfc",
            ["provider_execution"],
            "fail_closed_unavailable",
            "forbidden",
        ),
        "cap-v1-fdf4d6473b7f6608": (
            "server_action",
            "ACTION",
            "action://apps/web/app/(org)/dashboard/settings/account/server.ts#patchAccountSettings",
            4,
            "448bdcde83147aefc1c713367f96fe17d8566b3798d3d85e056e71e2a7159554",
            [],
            "serve_exact_action",
            "required",
        ),
        "cap-v1-c067d69850110640": (
            "server_action",
            "ACTION",
            "action://apps/web/app/(org)/dashboard/settings/account/server.ts#signOutAllDevices",
            4,
            "448bdcde83147aefc1c713367f96fe17d8566b3798d3d85e056e71e2a7159554",
            [],
            "serve_exact_action",
            "required",
        ),
        "cap-v1-3d28eb7593bd4b1e": (
            "server_action",
            "ACTION",
            "action://apps/web/app/Layout/devtoolsServer.ts#demoteFromPro",
            4,
            "0dd6130474181804819d88cca837f652e7bcd633f2530fe6161b71ec27b796af",
            ["human_approval"],
            "fail_closed_unavailable",
            "required",
        ),
        "cap-v1-e0040a01322ea19e": (
            "server_action",
            "ACTION",
            "action://apps/web/app/Layout/devtoolsServer.ts#promoteToPro",
            4,
            "0dd6130474181804819d88cca837f652e7bcd633f2530fe6161b71ec27b796af",
            ["human_approval"],
            "fail_closed_unavailable",
            "required",
        ),
        "cap-v1-859bad07650343aa": (
            "server_action",
            "ACTION",
            "action://apps/web/app/Layout/devtoolsServer.ts#restartOnboarding",
            4,
            "0dd6130474181804819d88cca837f652e7bcd633f2530fe6161b71ec27b796af",
            ["human_approval"],
            "fail_closed_unavailable",
            "required",
        ),
    }
    if fixture.get("schema_version") != 1 or fixture.get("family") != "user_account.v1":
        errors.append("user/account fixture schema drifted")
    if fixture.get("reference_commit") != REFERENCE_COMMIT:
        errors.append("user/account fixture reference commit drifted")
    if fixture.get("name_route") != {
        "identity": "POST /api/settings/user/name",
        "authentication": "host_session_cookie_only",
        "parse_order": "json_body_before_explicit_unauthenticated_branch",
        "request": "{firstName?:string|null,lastName?:string|null}",
        "presence": "missing_no_patch_null_persisted_empty_string_preserved",
        "success": "HTTP 200 JSON true",
        "idempotency": "forbidden",
    }:
        errors.append("user/account name-route wire contract drifted")
    if fixture.get("effect_rpc", {}).get("serialization") != (
        "@effect/rpc@0.71.2 RpcSerialization.layerJson"
    ) or fixture.get("effect_rpc", {}).get("idempotency_header") != "forbidden":
        errors.append("user/account Effect-RPC wire contract drifted")
    if fixture.get("server_actions", {}).get("request_schema") != (
        "frame.web-user-account-action-request.v1"
    ) or "one_use_mutation_grant" not in fixture.get("server_actions", {}).get(
        "authentication", ""
    ):
        errors.append("user/account action carrier contract drifted")

    operations = fixture.get("operations", [])
    if not isinstance(operations, list) or {row.get("id") for row in operations} != set(expected):
        errors.append("user/account fixture operation set drifted")
        return errors
    report_by_id = {row.get("id"): row for row in report.get("entries", [])}
    for operation in operations:
        operation_id = operation["id"]
        kind, method, identity, count, manifest, gates, behavior, idempotency = expected[
            operation_id
        ]
        row = report_by_id.get(operation_id, {})
        if (
            operation.get("kind") != kind
            or operation.get("method") != method
            or operation.get("legacy_identity") != identity
            or operation.get("source_count") != count
            or operation.get("source_manifest_sha256") != manifest
            or operation.get("protected_gates") != gates
            or operation.get("production_behavior") != behavior
        ):
            errors.append(f"user/account fixture contract drifted: {operation_id}")
        if (
            row.get("kind") != kind
            or row.get("method") != method
            or row.get("legacy_path") != identity
            or len(row.get("sources", [])) != count
            or row.get("auth") != "session"
            or row.get("policy") != "organization_library.v1"
            or row.get("security", {}).get("max_body_bytes") != 256 * 1024
            or row.get("security", {}).get("accepted_content_types")
            != ["application/json"]
            or row.get("security", {}).get("idempotency") != idempotency
            or set(row.get("contract_evidence", {}).values()) != {"local_contract"}
            or row.get("completion", {}).get("local_work") != "complete"
            or row.get("completion", {}).get("protected_gates") != gates
            or row.get("completion", {}).get("production_behavior") != behavior
        ):
            errors.append(f"user/account report evidence drifted: {operation_id}")

    for token in (
        "LEGACY_USER_NAME_SOURCES",
        "LEGACY_USER_ONBOARDING_SOURCES",
        "LEGACY_USER_UPDATE_SOURCES",
        "LEGACY_ACCOUNT_ACTION_SOURCES",
        "LEGACY_DEVTOOL_ACTION_SOURCES",
        "LegacyNullableTextPatchV1::Absent",
        "LegacyOptionalImageUpdateV1::None",
        "execute_web_action",
        "organization_icon_content_type_allowed",
        "profiles_cover_exact_inventory_and_source_observability",
    ):
        if token not in application:
            errors.append(f"user/account application proof lost token: {token}")
    if (
        "mod legacy_user_account;" not in application_lib
        or "pub use legacy_user_account::*;" not in application_lib
    ):
        errors.append("user/account semantics are not exported by frame-application")
    for token in (
        "D1LegacyUserAccountAtomicPortV1",
        "BROWSER_GRANT_ASSERT_SQL",
        "BROWSER_GRANT_DELETE_SQL",
        "BROWSER_ASSERTION_CLEANUP_SQL",
        "ORGANIZATION_PROJECTION_ASSERT_SQL",
        "SIGN_OUT_V2_MUTATION_GRANTS_DELETE_SQL",
        ".batch(statements)",
        "consume_browser_fence",
    ):
        if token not in d1_runtime:
            errors.append(f"user/account D1 runtime proof lost token: {token}")
    for token in (
        "WEB_USER_ACCOUNT_ACTION_REQUEST_SCHEMA_V1",
        "authenticate_host_only_browser_session",
        "authenticate_compatibility_mutation",
        "decode_name_route",
        "decode_rpc_request",
        "UserCompleteOnboardingStep",
        "UserUpdate",
        "rpc_typed_failure",
        "consume_or_confirm_absent",
        "environment guard before any authentication lookup",
    ):
        if token not in ingress:
            errors.append(f"user/account ingress proof lost token: {token}")
    for token in (
        "legacy_user_account_organization_ids_v1",
        "legacy_user_account_operations_v1",
        "legacy_user_account_receipts_v1",
        "legacy_user_account_effects_v1",
        "legacy_user_account_audit_events_v1",
        "frame_legacy_user_account_authority_v1",
        "frame_legacy_user_account_forbidden_v1",
        "frame_legacy_user_account_projection_v1",
    ):
        if token not in migration:
            errors.append(f"user/account migration lost token: {token}")
    if len(query_names) != 40:
        errors.append("user/account checked-in SQL closure must contain exactly 40 queries")
    for token in (
        "test_name_presence_replay_and_conflict",
        "test_welcome_merge_and_exact_default_organization_rename",
        "test_organization_setup_whitespace_projection_and_best_effort_icon",
        "test_user_update_absent_and_provider_fail_closed",
        "test_patch_account_owner_or_any_membership_and_atomic_denial",
        "test_sign_out_all_revokes_every_credential_family_atomically",
        "test_devtools_and_evidence_immutability",
        "test_browser_action_proof_is_consumed_atomically",
        "assert len(SQL) == 40",
    ):
        if token not in conformance:
            errors.append(f"user/account SQLite proof lost token: {token}")
    for token in (
        "mod legacy_user_account_runtime;",
        "mod legacy_user_account_web_runtime;",
        "legacy_user_account_web_runtime::name_route_response",
        "legacy_user_account_web_runtime::mutate_action",
    ):
        if token not in control_lib:
            errors.append(f"user/account control-plane wiring lost token: {token}")
    if "legacy_user_account_web_runtime::is_user_rpc_request" not in folder_rpc_ingress:
        errors.append("user/account RPC tag is not composed into the shared Effect carrier")
    for token in ("LegacyUserName", '"/api/settings/user/name"', '"/api/erpc"'):
        if token not in routing:
            errors.append(f"user/account raw routing lost token: {token}")
    for token in (
        "legacy-user-account-sqlite-conformance.py",
        "legacy-user-account-sqlite-conformance.json",
        "fixtures/api-parity/v1/user-account.json",
        "frame-application --lib legacy_user_account",
        "frame-control-plane --lib legacy_user_account",
    ):
        if token not in workflow:
            errors.append(f"user/account workflow proof lost token: {token}")
    return errors


def validate_library_id_reads_fixture(report: dict[str, Any]) -> list[str]:
    """Bind the three source-pinned ID reads to tenant-scoped D1 ingress."""
    try:
        fixture = json.loads(LIBRARY_ID_READS_FIXTURE.read_text(encoding="utf-8"))
        application = APPLICATION_LIBRARY_ID_READS.read_text(encoding="utf-8")
        application_lib = APPLICATION_LIB.read_text(encoding="utf-8")
        d1_runtime = CONTROL_LIBRARY_ID_READ_RUNTIME.read_text(encoding="utf-8")
        ingress = CONTROL_LIBRARY_ID_READ_WEB_RUNTIME.read_text(encoding="utf-8")
        migration = LIBRARY_ID_READ_MIGRATION.read_text(encoding="utf-8")
        conformance = LIBRARY_ID_READ_CONFORMANCE.read_text(encoding="utf-8")
        control_lib = CONTROL_LIB.read_text(encoding="utf-8")
        registry = CONTROL_RUNTIME.read_text(encoding="utf-8")
        workflow = parity_workflow_contract()
        query_names = {path.name for path in LIBRARY_ID_READ_QUERY_ROOT.glob("*.sql")}
    except (OSError, json.JSONDecodeError) as error:
        return [f"unable to load library-ID read fixture: {error}"]

    errors: list[str] = []
    expected = {
        "cap-v1-b1027c7caafb92e2": (
            "action://apps/web/actions/folders/get-folder-videos.ts#getFolderVideoIds",
            10,
            "bab8979189df2978ae381303e209a0b4ee5832e3fba5949761976ab7e3b5d41b",
            "actor_active_organization_and_exact_folder_namespace",
        ),
        "cap-v1-cc52545598164806": (
            "action://apps/web/actions/organizations/get-organization-videos.ts#getOrganizationVideoIds",
            9,
            "cef5fc71ef1b95f9747d010bd3a04a744d33200f3d4427fb1a16fbc4fe72722b",
            "requested_organization_must_be_actor_active_organization",
        ),
        "cap-v1-a8ace95c6ab712f6": (
            "action://apps/web/actions/spaces/get-space-videos.ts#getSpaceVideoIds",
            9,
            "0bf24570c914feabd6be1be0f83ff7c3567059e86098fc6f4e1ce0f5a948dcb4",
            "actor_active_organization_and_live_space_access",
        ),
    }
    expected_transport = {
        "method": "POST",
        "path_prefix": "/api/v1/web/compatibility-actions/",
        "role": "frame_selector_not_legacy_identity",
        "request_schema": "frame.web-library-id-read-request.v1",
        "max_body_bytes": 1024,
        "authentication": "host_only_browser_session",
        "same_origin": "required",
        "csrf": "not_required_for_read_only_action",
        "client_idempotency": "forbidden",
        "rate_limit_bucket": "organization_library.v1",
        "cache_control": "no-store, max-age=0",
    }
    if (
        fixture.get("schema_version") != "frame.legacy-library-id-reads.v1"
        or fixture.get("reference_commit") != REFERENCE_COMMIT
        or fixture.get("transport") != expected_transport
    ):
        errors.append("library-ID read fixture transport or source reference drifted")
    if fixture.get("wire_contract") != {
        "success": "HTTP 200 application/json {success:true,data:string[]}",
        "failure": "HTTP 200 application/json {success:false,error:string}",
        "ordering": "source_unspecified_no_order_fabricated",
        "retry": "read_only_and_safe_without_an_idempotency_key",
    }:
        errors.append("library-ID read source-shaped response contract drifted")
    authorization_closure = fixture.get("authorization_closure", {})
    for token in (
        "without a tenant authorization predicate",
        "active organization",
        "never returns membership IDs",
    ):
        if token not in "\n".join(str(value) for value in authorization_closure.values()):
            errors.append(f"library-ID read authorization closure lost token: {token}")

    operations = fixture.get("operations", [])
    if not isinstance(operations, list) or {row.get("id") for row in operations} != set(expected):
        errors.append("library-ID read fixture operation set drifted")
        return errors
    report_by_id = {row.get("id"): row for row in report.get("entries", [])}
    for operation in operations:
        operation_id = operation["id"]
        identity, source_count, rust_manifest, authorization = expected[operation_id]
        row = report_by_id.get(operation_id, {})
        if (
            operation.get("kind") != "server_action"
            or operation.get("method") != "ACTION"
            or operation.get("legacy_identity") != identity
            or operation.get("authorization") != authorization
            or operation.get("source_count") != source_count
            or operation.get("rust_source_manifest_sha256") != rust_manifest
            or operation.get("protected_gates") != []
            or operation.get("production_behavior") != "serve_exact_action"
        ):
            errors.append(f"library-ID read fixture contract drifted: {operation_id}")
        if (
            row.get("kind") != "server_action"
            or row.get("method") != "ACTION"
            or row.get("legacy_path") != identity
            or row.get("auth") != "session"
            or len(row.get("sources", [])) != source_count
            or row.get("security", {}).get("max_body_bytes") != 1024
            or row.get("security", {}).get("idempotency") != "forbidden"
            or row.get("security", {}).get("rate_limit_bucket")
            != "organization_library.v1"
            or set(row.get("contract_evidence", {}).values()) != {"local_contract"}
            or row.get("completion", {}).get("local_work") != "complete"
            or row.get("completion", {}).get("protected_gates") != []
            or row.get("completion", {}).get("production_behavior") != "serve_exact_action"
        ):
            errors.append(f"library-ID read report five-axis evidence drifted: {operation_id}")
        if rust_manifest not in application:
            errors.append(f"library-ID read Rust source manifest drifted: {operation_id}")

    for token in (
        "LEGACY_GET_FOLDER_VIDEO_IDS_SOURCES",
        "LEGACY_GET_ORGANIZATION_VIDEO_IDS_SOURCES",
        "LEGACY_GET_SPACE_VIDEO_IDS_SOURCES",
        "read_only_client_key_forbidden_retry_safe",
        "source_order_is_not_fabricated_for_unordered_id_queries",
        "authority_and_storage_failures_are_stable_and_non_disclosing",
    ):
        if token not in application:
            errors.append(f"library-ID read application proof lost token: {token}")
    if (
        "mod legacy_library_id_reads;" not in application_lib
        or "pub use legacy_library_id_reads::*;" not in application_lib
    ):
        errors.append("library-ID read semantics are not exported by frame-application")
    for token in (
        "D1LegacyLibraryIdReadPortV1",
        "impl LegacyLibraryIdReadPortV1",
        "principal_for_actor",
        "reassert_principal",
        "valid_legacy_nanoid",
        'include_str!("../queries/legacy_library_id_reads/',
    ):
        if token not in d1_runtime:
            errors.append(f"library-ID read D1 proof lost token: {token}")
    expected_queries = {
        "principal_scope.sql",
        "folder_authority.sql",
        "folder_video_ids_organization.sql",
        "folder_video_ids_space.sql",
        "organization_authority.sql",
        "organization_video_ids.sql",
        "space_authority.sql",
        "space_video_ids.sql",
    }
    if query_names != expected_queries:
        errors.append("library-ID read checked-in SQL closure drifted")
    for token in (
        "legacy_library_space_aliases_v1",
        "legacy_library_space_aliases_no_update_v1",
        "frame_legacy_library_alias_immutable_v1",
        "legacy_library_folder_alias_scope_read_v1",
    ):
        if token not in migration:
            errors.append(f"library-ID read migration proof lost token: {token}")
    for token in (
        "active_legacy_organization_id",
        "organization_video_ids.sql",
        "space_video_ids.sql",
        "folder_video_ids_organization.sql",
        "folder_video_ids_space.sql",
        "cross-tenant",
        "non-disclosure",
        "immutable scope aliases",
    ):
        if token not in conformance:
            errors.append(f"library-ID read SQLite proof lost token: {token}")
    for token in (
        '"frame.web-library-id-read-request.v1"',
        'request.headers().get("idempotency-key")',
        "authenticate_compatibility_read",
        "CompatibilityRateLimitBucketV1::OrganizationLibrary",
        "dispatch_web_library_id_read",
    ):
        if token not in ingress:
            errors.append(f"library-ID read browser ingress lost token: {token}")
    for token in (
        "mod legacy_library_id_read_runtime;",
        "mod legacy_library_id_read_web_runtime;",
        "legacy_library_id_read_web_runtime::is_action",
        "legacy_library_id_read_response",
    ):
        if token not in control_lib:
            errors.append(f"library-ID read route wiring lost token: {token}")
    for token in (
        "LegacyRegistrationSourcesV1::LibraryIdRead",
        "LegacyWebLibraryIdReadInvocationV1",
        "dispatch_web_library_id_read",
        "LEGACY_GET_FOLDER_VIDEO_IDS_OPERATION_ID",
        "LEGACY_GET_ORGANIZATION_VIDEO_IDS_OPERATION_ID",
        "LEGACY_GET_SPACE_VIDEO_IDS_OPERATION_ID",
    ):
        if token not in registry:
            errors.append(f"library-ID read registry proof lost token: {token}")
    for token in (
        "legacy-library-id-reads-sqlite-conformance.py",
        "fixtures/api-parity/v1/library-id-reads.json",
        "frame-application --lib legacy_library_id_reads",
        "frame-control-plane --lib legacy_library_id_read",
    ):
        if token not in workflow:
            errors.append(f"library-ID read workflow proof lost token: {token}")
    return errors


def validate_library_detail_reads_fixture(report: dict[str, Any]) -> list[str]:
    """Bind user-video detail and dashboard search to exact tenant-safe D1 reads."""
    try:
        fixture = json.loads(
            LIBRARY_DETAIL_READS_FIXTURE.read_text(encoding="utf-8")
        )
        application = APPLICATION_LIBRARY_DETAIL_READS.read_text(encoding="utf-8")
        application_lib = APPLICATION_LIB.read_text(encoding="utf-8")
        d1_runtime = CONTROL_LIBRARY_DETAIL_READ_RUNTIME.read_text(encoding="utf-8")
        ingress = CONTROL_LIBRARY_DETAIL_READ_WEB_RUNTIME.read_text(encoding="utf-8")
        migration = LIBRARY_DETAIL_READ_MIGRATION.read_text(encoding="utf-8")
        conformance = LIBRARY_DETAIL_READ_CONFORMANCE.read_text(encoding="utf-8")
        control_lib = CONTROL_LIB.read_text(encoding="utf-8")
        registry = CONTROL_RUNTIME.read_text(encoding="utf-8")
        workflow = parity_workflow_contract()
        query_names = {
            path.name for path in LIBRARY_DETAIL_READ_QUERY_ROOT.glob("*.sql")
        }
    except (OSError, json.JSONDecodeError) as error:
        return [f"unable to load library-detail read fixture: {error}"]

    errors: list[str] = []
    expected = {
        "cap-v1-17a71c3e18600d06": (
            "action://apps/web/actions/spaces/get-user-videos.ts#getUserVideos",
            15,
            "960be699e80eb782d5a04cc7880310b03bf762983372efee39e07efbec355bbf",
        ),
        "cap-v1-39e8966f308c1528": (
            "action://apps/web/app/(org)/dashboard/_components/Navbar/search.ts#searchDashboardVideos",
            11,
            "4a760c534a6b32ad0d75602d86c18ada4331c7ff5e8409c3792f6ccae0ece544",
        ),
    }
    expected_transport = {
        "method": "POST",
        "path_prefix": "/api/v1/web/compatibility-actions/",
        "role": "frame_selector_not_legacy_identity",
        "request_schema": "frame.web-library-detail-read-request.v1",
        "max_body_bytes": 2048,
        "authentication": "host_only_browser_session",
        "same_origin": "required",
        "csrf": "not_required_for_read_only_action",
        "client_idempotency": "forbidden",
        "rate_limit_bucket": "organization_library.v1",
        "cache_control": "no-store, max-age=0",
    }
    if (
        fixture.get("schema_version") != "frame.legacy-library-detail-reads.v1"
        or fixture.get("reference_commit") != REFERENCE_COMMIT
        or fixture.get("transport") != expected_transport
    ):
        errors.append("library-detail fixture transport or source reference drifted")
    closure = "\n".join(
        str(value)
        for section in (
            fixture.get("authorization_closure", {}),
            fixture.get("projection_closure", {}),
        )
        for value in section.values()
    )
    for token in (
        "unrelated tenants",
        "live active organization",
        "legacy_effective_created_at_ms",
        "legacy_collaboration_comments_v1",
    ):
        if token not in closure:
            errors.append(f"library-detail closure lost token: {token}")

    operations = fixture.get("operations", [])
    if not isinstance(operations, list) or {
        row.get("id") for row in operations
    } != set(expected):
        errors.append("library-detail fixture operation set drifted")
        return errors
    report_by_id = {row.get("id"): row for row in report.get("entries", [])}
    for operation in operations:
        operation_id = operation["id"]
        identity, source_count, rust_manifest = expected[operation_id]
        row = report_by_id.get(operation_id, {})
        if (
            operation.get("kind") != "server_action"
            or operation.get("method") != "ACTION"
            or operation.get("legacy_identity") != identity
            or operation.get("source_count") != source_count
            or operation.get("rust_source_manifest_sha256") != rust_manifest
            or operation.get("protected_gates") != []
            or operation.get("production_behavior") != "serve_exact_action"
        ):
            errors.append(f"library-detail fixture contract drifted: {operation_id}")
        if (
            row.get("kind") != "server_action"
            or row.get("method") != "ACTION"
            or row.get("legacy_path") != identity
            or row.get("auth") != "session"
            or len(row.get("sources", [])) != source_count
            or row.get("security", {}).get("max_body_bytes") != 2048
            or row.get("security", {}).get("idempotency") != "forbidden"
            or row.get("security", {}).get("rate_limit_bucket")
            != "organization_library.v1"
            or set(row.get("contract_evidence", {}).values()) != {"local_contract"}
            or row.get("completion", {}).get("local_work") != "complete"
            or row.get("completion", {}).get("protected_gates") != []
            or row.get("completion", {}).get("production_behavior")
            != "serve_exact_action"
        ):
            errors.append(f"library-detail report evidence drifted: {operation_id}")
        if rust_manifest not in application:
            errors.append(f"library-detail Rust source manifest drifted: {operation_id}")

    for token in (
        "LEGACY_GET_USER_VIDEOS_SOURCES",
        "LEGACY_SEARCH_DASHBOARD_VIDEOS_SOURCES",
        "normalize_dashboard_video_query",
        "effective_created_at_ms",
        "total_comments",
        "total_reactions",
        "has_active_upload",
    ):
        if token not in application:
            errors.append(f"library-detail application proof lost token: {token}")
    if (
        "mod legacy_library_detail_reads;" not in application_lib
        or "pub use legacy_library_detail_reads::*;" not in application_lib
    ):
        errors.append("library-detail semantics are not exported by frame-application")
    for token in (
        "D1LegacyLibraryDetailReadPortV1",
        "impl LegacyLibraryDetailReadPortV1",
        "principal_for_actor",
        "reassert_principal",
        "require_scope",
        'include_str!("../queries/legacy_library_detail_reads/',
    ):
        if token not in d1_runtime:
            errors.append(f"library-detail D1 proof lost token: {token}")
    if query_names != {
        "principal_scope.sql",
        "scope_authority.sql",
        "get_user_videos_organization.sql",
        "get_user_videos_space.sql",
        "search_dashboard_videos.sql",
    }:
        errors.append("library-detail checked-in SQL closure drifted")
    for token in (
        "legacy_is_screenshot",
        "legacy_duration_seconds",
        "legacy_effective_created_at_ms",
        "GENERATED ALWAYS AS",
        "legacy_library_detail_search_order_v1",
    ):
        if token not in migration:
            errors.append(f"library-detail migration proof lost token: {token}")
    for token in (
        "effective-date and prefix",
        "metadata/duration/screenshot/folder/count/upload projection",
        "LIKE",
        "cross-tenant non-disclosure",
    ):
        if token not in conformance:
            errors.append(f"library-detail SQLite proof lost token: {token}")
    for token in (
        '"frame.web-library-detail-read-request.v1"',
        'request.headers().get("idempotency-key")',
        "authenticate_compatibility_read",
        "CompatibilityRateLimitBucketV1::OrganizationLibrary",
        "dispatch_web_library_detail_read",
    ):
        if token not in ingress:
            errors.append(f"library-detail browser ingress lost token: {token}")
    for token in (
        "mod legacy_library_detail_read_runtime;",
        "mod legacy_library_detail_read_web_runtime;",
        "legacy_library_detail_read_web_runtime::is_action",
        "legacy_library_detail_read_response",
        "legacy_library_detail_iso",
    ):
        if token not in control_lib:
            errors.append(f"library-detail route wiring lost token: {token}")
    for token in (
        "LegacyRegistrationSourcesV1::LibraryDetailRead",
        "LegacyWebLibraryDetailReadInvocationV1",
        "dispatch_web_library_detail_read",
        "LEGACY_GET_USER_VIDEOS_OPERATION_ID",
        "LEGACY_SEARCH_DASHBOARD_VIDEOS_OPERATION_ID",
    ):
        if token not in registry:
            errors.append(f"library-detail registry proof lost token: {token}")
    for token in (
        "legacy-library-detail-reads-sqlite-conformance.py",
        "fixtures/api-parity/v1/library-detail-reads.json",
        "frame-application --lib legacy_library_detail_reads",
        "frame-control-plane --lib legacy_library_detail_read",
    ):
        if token not in workflow:
            errors.append(f"library-detail workflow proof lost token: {token}")
    return errors


def validate_space_authorization_fixture(report: dict[str, Any]) -> list[str]:
    """Bind both space-authorization ACTIONs to exact roles and tenant-safe D1."""
    try:
        fixture = json.loads(SPACE_AUTHORIZATION_FIXTURE.read_text(encoding="utf-8"))
        application = APPLICATION_SPACE_AUTHORIZATION.read_text(encoding="utf-8")
        application_lib = APPLICATION_LIB.read_text(encoding="utf-8")
        d1_runtime = CONTROL_SPACE_AUTHORIZATION_RUNTIME.read_text(encoding="utf-8")
        ingress = CONTROL_SPACE_AUTHORIZATION_WEB_RUNTIME.read_text(encoding="utf-8")
        conformance = SPACE_AUTHORIZATION_CONFORMANCE.read_text(encoding="utf-8")
        control_lib = CONTROL_LIB.read_text(encoding="utf-8")
        registry = CONTROL_RUNTIME.read_text(encoding="utf-8")
        workflow = parity_workflow_contract()
        query_text = {
            path.name: path.read_text(encoding="utf-8")
            for path in SPACE_AUTHORIZATION_QUERY_ROOT.glob("*.sql")
        }
    except (OSError, json.JSONDecodeError) as error:
        return [f"unable to load space-authorization fixture: {error}"]

    errors: list[str] = []
    expected = {
        "cap-v1-5595a9d384765e76": (
            "action://apps/web/actions/organization/space-authorization.ts#getSpaceAccess",
            "b3cc205e302c0a50208b4a31e2c40144f438ba07965eabf0769e6458881e7183",
        ),
        "cap-v1-14cb48febfd0fa5a": (
            "action://apps/web/actions/organization/space-authorization.ts#requireSpaceManager",
            "69438e783f7cac4aeae4faa061188d6a38ea2ca2e23b9d42205c592c64ad3667",
        ),
    }
    expected_transport = {
        "method": "POST",
        "path_prefix": "/api/v1/web/compatibility-actions/",
        "role": "frame_selector_not_legacy_identity",
        "request_schema": "frame.web-space-authorization-request.v1",
        "max_body_bytes": 1024,
        "authentication": "host_only_browser_session",
        "same_origin": "required",
        "csrf": "not_required_for_read_only_action",
        "client_idempotency": "forbidden",
        "rate_limit_bucket": "organization_library.v1",
        "cache_control": "no-store, max-age=0",
    }
    if (
        fixture.get("schema_version") != "frame.legacy-space-authorization.v1"
        or fixture.get("reference_commit") != REFERENCE_COMMIT
        or fixture.get("transport") != expected_transport
    ):
        errors.append("space-authorization fixture transport or source reference drifted")
    wire = fixture.get("wire_contract", {})
    for token in (
        "SpaceAccess | null",
        "Space not found",
        "Only space admins, organization admins, and owners can manage this space",
        "without_an_idempotency_key",
    ):
        if token not in "\n".join(str(value) for value in wire.values()):
            errors.append(f"space-authorization wire contract lost token: {token}")
    closure = "\n".join(
        str(value)
        for key in ("role_closure", "authorization_closure", "wire_identity_closure")
        for value in fixture.get(key, {}).values()
    )
    for token in (
        "non-owner membership role of owner normalizes to member",
        "manager -> admin; viewer -> member; contributor -> null",
        "caller-supplied userId",
        "eight bounded collision retries",
        "persisted globally unique",
    ):
        if token not in closure:
            errors.append(f"space-authorization closure lost token: {token}")

    operations = fixture.get("operations", [])
    if not isinstance(operations, list) or {row.get("id") for row in operations} != set(expected):
        errors.append("space-authorization fixture operation set drifted")
        return errors
    report_by_id = {row.get("id"): row for row in report.get("entries", [])}
    for operation in operations:
        operation_id = operation["id"]
        identity, manifest = expected[operation_id]
        row = report_by_id.get(operation_id, {})
        if (
            operation.get("kind") != "server_action"
            or operation.get("method") != "ACTION"
            or operation.get("legacy_identity") != identity
            or operation.get("source_count") != 2
            or operation.get("rust_source_manifest_sha256") != manifest
            or operation.get("protected_gates") != []
            or operation.get("production_behavior") != "serve_exact_action"
        ):
            errors.append(f"space-authorization fixture contract drifted: {operation_id}")
        if (
            row.get("kind") != "server_action"
            or row.get("method") != "ACTION"
            or row.get("legacy_path") != identity
            or row.get("auth") != "session"
            or len(row.get("sources", [])) != 2
            or row.get("security", {}).get("max_body_bytes") != 1024
            or row.get("security", {}).get("idempotency") != "forbidden"
            or row.get("security", {}).get("rate_limit_bucket")
            != "organization_library.v1"
            or set(row.get("contract_evidence", {}).values()) != {"local_contract"}
            or row.get("completion", {}).get("local_work") != "complete"
            or row.get("completion", {}).get("protected_gates") != []
            or row.get("completion", {}).get("production_behavior")
            != "serve_exact_action"
        ):
            errors.append(f"space-authorization report evidence drifted: {operation_id}")
        if manifest not in application:
            errors.append(f"space-authorization Rust source manifest drifted: {operation_id}")

    for token in (
        "LEGACY_GET_SPACE_ACCESS_SOURCES",
        "LEGACY_REQUIRE_SPACE_MANAGER_SOURCES",
        "effective_organization_role",
        "effective_space_role",
        "can_manage_space",
        "LEGACY_SPACE_NOT_FOUND_MESSAGE",
        "LEGACY_SPACE_MANAGER_REQUIRED_MESSAGE",
        "read_only_client_key_forbidden_retry_safe",
    ):
        if token not in application:
            errors.append(f"space-authorization application proof lost token: {token}")
    if (
        "mod legacy_space_authorization;" not in application_lib
        or "pub use legacy_space_authorization::*;" not in application_lib
    ):
        errors.append("space-authorization semantics are not exported by frame-application")
    for token in (
        "D1LegacySpaceAuthorizationPortV1",
        "impl LegacySpaceAuthorizationPortV1",
        "principal_for_actor",
        "ensure_persisted_alias",
        "native_alias_candidate",
        "NATIVE_ALIAS_ATTEMPTS",
        "legacy_collaboration_user_aliases_v1",
        'include_str!("../queries/legacy_space_authorization/',
    ):
        if token not in d1_runtime:
            errors.append(f"space-authorization D1 proof lost token: {token}")
    if set(query_text) != {
        "principal_scope.sql",
        "access_read.sql",
        "clock_now.sql",
        "user_alias_insert.sql",
        "user_alias_read.sql",
    }:
        errors.append("space-authorization checked-in SQL closure drifted")
    joined_queries = "\n".join(query_text.values())
    for token in (
        "legacy_library_space_aliases_v1",
        "space.organization_id = organization.id",
        "organization.tombstoned_at_ms IS NULL",
        "space.deleted_at_ms IS NULL",
        "WHEN 'manager' THEN 'admin'",
        "WHEN 'viewer' THEN 'member'",
        "INSERT OR IGNORE INTO legacy_collaboration_user_aliases_v1",
    ):
        if token not in joined_queries:
            errors.append(f"space-authorization SQL closure lost token: {token}")
    if "substr(replace" in query_text.get("access_read.sql", ""):
        errors.append("space-authorization projection silently truncates a UUID")
    for token in (
        "deterministic collision retry/non-drift",
        "active-tenant alias scope",
        "manager/viewer role translation",
        "missing/deleted/tombstoned non-disclosure",
    ):
        if token not in conformance:
            errors.append(f"space-authorization SQLite proof lost token: {token}")
    for token in (
        '"frame.web-space-authorization-request.v1"',
        'request.headers().get("idempotency-key")',
        "authenticate_compatibility_read",
        "CompatibilityRateLimitBucketV1::OrganizationLibrary",
        "dispatch_web_space_authorization",
    ):
        if token not in ingress:
            errors.append(f"space-authorization browser ingress lost token: {token}")
    for token in (
        "mod legacy_space_authorization_runtime;",
        "mod legacy_space_authorization_web_runtime;",
        "legacy_space_authorization_web_runtime::is_action",
        "legacy_space_authorization_response",
    ):
        if token not in control_lib:
            errors.append(f"space-authorization route wiring lost token: {token}")
    for token in (
        "LegacyRegistrationSourcesV1::SpaceAuthorization",
        "LegacyWebSpaceAuthorizationInvocationV1",
        "dispatch_web_space_authorization",
        "LEGACY_GET_SPACE_ACCESS_OPERATION_ID",
        "LEGACY_REQUIRE_SPACE_MANAGER_OPERATION_ID",
    ):
        if token not in registry:
            errors.append(f"space-authorization registry proof lost token: {token}")
    for token in (
        "legacy-space-authorization-sqlite-conformance.py",
        "fixtures/api-parity/v1/space-authorization.json",
        "frame-application --lib legacy_space_authorization",
        "frame-control-plane --lib legacy_space_authorization",
    ):
        if token not in workflow:
            errors.append(f"space-authorization workflow proof lost token: {token}")
    return errors


def validate_video_properties_fixture(report: dict[str, Any]) -> list[str]:
    """Bind ten asymmetric video-property mutations to exact D1 and HTTP carriers."""
    try:
        fixture = json.loads(VIDEO_PROPERTIES_FIXTURE.read_text(encoding="utf-8"))
        application = APPLICATION_VIDEO_PROPERTIES.read_text(encoding="utf-8")
        application_lib = APPLICATION_LIB.read_text(encoding="utf-8")
        d1_runtime = CONTROL_VIDEO_PROPERTIES_RUNTIME.read_text(encoding="utf-8")
        ingress = CONTROL_VIDEO_PROPERTIES_WEB_RUNTIME.read_text(encoding="utf-8")
        migration = VIDEO_PROPERTIES_MIGRATION.read_text(encoding="utf-8")
        conformance = VIDEO_PROPERTIES_CONFORMANCE.read_text(encoding="utf-8")
        control_lib = CONTROL_LIB.read_text(encoding="utf-8")
        routing = CONTROL_ROUTING.read_text(encoding="utf-8")
        registry = CONTROL_RUNTIME.read_text(encoding="utf-8")
        workflow = parity_workflow_contract()
        query_names = {
            path.name for path in VIDEO_PROPERTIES_QUERY_ROOT.glob("*.sql")
        }
        query_text = {
            path.name: path.read_text(encoding="utf-8")
            for path in VIDEO_PROPERTIES_QUERY_ROOT.glob("*.sql")
        }
    except (OSError, json.JSONDecodeError) as error:
        return [f"unable to load video-properties fixture: {error}"]

    expected = {
        "cap-v1-2cfe7fc40a6f5a78": {
            "kind": "route",
            "method": "PATCH",
            "identity": "/api/mobile/caps/:id/password",
            "source_count": 5,
            "manifest": "24e4585815c8a22ce021d49c460f5c27cc1c038431a9345e4f2c3bcf08d5175a",
            "auth": "session_or_api_key",
            "policy": "share_playback.v1",
            "gates": ["provider_execution"],
            "behavior": "fail_closed_unavailable",
        },
        "cap-v1-5fdf332d1448aedc": {
            "kind": "route",
            "method": "PATCH",
            "identity": "/api/mobile/caps/:id/sharing",
            "source_count": 4,
            "manifest": "07fd9bbfc4674bdd69b0d983dd5e13635ff229b8f6c15dff7a7d27e92a33c46a",
            "auth": "session_or_api_key",
            "policy": "client_compatibility.v1",
            "gates": ["provider_execution"],
            "behavior": "fail_closed_unavailable",
        },
        "cap-v1-b2db0e7ec51f7898": {
            "kind": "route",
            "method": "PATCH",
            "identity": "/api/mobile/caps/:id/title",
            "source_count": 4,
            "manifest": "048f7393a88ed0317ab1ecd4f3c493848c23493b11ae25932802d2870cd1c292",
            "auth": "session_or_api_key",
            "policy": "client_compatibility.v1",
            "gates": ["provider_execution"],
            "behavior": "fail_closed_unavailable",
        },
        "cap-v1-5b36dac105856ede": {
            "kind": "route",
            "method": "PUT",
            "identity": "/api/video/metadata",
            "source_count": 3,
            "manifest": "01a4d4f059a7d9c28d4c0580f0dccb19e3752808a4bf29899a6edd14e78873a7",
            "auth": "session",
            "policy": "video_media.v1",
            "gates": [],
            "behavior": "serve_exact_d1",
        },
        "cap-v1-96c52e9330f9a131": {
            "kind": "server_action",
            "method": "ACTION",
            "identity": "action://apps/web/actions/videos/edit-date.ts#editDate",
            "source_count": 4,
            "manifest": "13d31ba77997d6b33ef14c1dfac6c6cc31e192368ebba11c9f66e6b31f9d5ce1",
            "auth": "session",
            "policy": "video_media.v1",
            "gates": ["human_approval"],
            "behavior": "fail_closed_unavailable",
        },
        "cap-v1-6e9f3d370f1ce239": {
            "kind": "server_action",
            "method": "ACTION",
            "identity": "action://apps/web/actions/videos/edit-title.ts#editTitle",
            "source_count": 4,
            "manifest": "537d4de6afbbd653fceedcc90864d1e46d4083a58db693c5b2d954d04cb879b1",
            "auth": "session",
            "policy": "video_media.v1",
            "gates": [],
            "behavior": "serve_exact_action",
        },
        "cap-v1-ab11637faa2de45e": {
            "kind": "server_action",
            "method": "ACTION",
            "identity": "action://apps/web/actions/videos/password.ts#removeVideoPassword",
            "source_count": 3,
            "manifest": "334eecf86628602d944d8af6f4125f25379510a28259b396cc48c92febf7afdd",
            "auth": "session",
            "policy": "share_playback.v1",
            "gates": [],
            "behavior": "serve_exact_action",
        },
        "cap-v1-455e6a1b82e647d9": {
            "kind": "server_action",
            "method": "ACTION",
            "identity": "action://apps/web/actions/videos/password.ts#setVideoPassword",
            "source_count": 4,
            "manifest": "3a96a2b46b7c5f7516a88225b56a8b3513d03c90980d1154fad7c65bf8dcbe2d",
            "auth": "session",
            "policy": "share_playback.v1",
            "gates": [],
            "behavior": "serve_exact_action",
        },
        "cap-v1-0a2c44d7a626a1fe": {
            "kind": "server_action",
            "method": "ACTION",
            "identity": "action://apps/web/actions/videos/password.ts#verifyVideoPassword",
            "source_count": 5,
            "manifest": "f08afa095fb760b933f6299fdcf414327925b7b3e9957e2924e9827fd3e87945",
            "auth": "anonymous",
            "policy": "share_playback.v1",
            "gates": [],
            "behavior": "serve_exact_action",
        },
        "cap-v1-49dba3fbc7c4a74c": {
            "kind": "server_action",
            "method": "ACTION",
            "identity": "action://apps/web/actions/videos/settings.ts#updateVideoSettings",
            "source_count": 4,
            "manifest": "95c1c1f551e17bceaec86e945b6f8b92a780d372de89974e57f00c6b3950fffa",
            "auth": "session",
            "policy": "video_media.v1",
            "gates": [],
            "behavior": "serve_exact_action",
        },
    }
    errors: list[str] = []
    if (
        fixture.get("schema_version") != 1
        or fixture.get("family") != "video_properties.v1"
        or fixture.get("reference_commit") != REFERENCE_COMMIT
    ):
        errors.append("video-properties fixture schema or reference drifted")
    if fixture.get("transport") != {
        "mobile_authentication": "host_session_or_authorization_second_space_token_exactly_36_characters",
        "browser_authentication": "host_only_session_except_anonymous_verifyVideoPassword",
        "action_path_prefix": "/api/v1/web/compatibility-actions/",
        "action_request_schema": "frame.web-video-property-action-request.v1",
        "action_same_origin": "required",
        "action_csrf": "double_submit_one_use_grant_required_except_anonymous_verifyVideoPassword",
        "idempotency": "optional_client_key_bound_when_supplied_server_generated_when_absent",
        "max_body_bytes": 262144,
        "content_type": "application/json",
        "cache_control": "no-store, max-age=0",
    }:
        errors.append("video-properties transport contract drifted")
    semantics = "\n".join(
        str(value) for value in fixture.get("source_semantics", {}).values()
    )
    for token in (
        "ECMAScript_trim_then_reject_blank_without_titleManuallyEdited",
        "null_empty_or_whitespace_clears",
        "provider_failures_fall_back_to_null_and_zero",
        "arbitrary_truthy_JSON_replaces_legacy_metadata_only",
        "safe_ISO_subset_future_rejected",
        "whitespace_preserved_metadata_object_spread_sets_titleManuallyEdited",
        "without_trim_whitespace_password_is_hashed",
        "video_hash_then_joined_space_hashes_in_insertion_order",
        "normalize_playback_speed_to_nearest_allowed_value_ties_choose_earlier",
    ):
        if token not in semantics:
            errors.append(f"video-properties source semantics lost token: {token}")
    if fixture.get("password_contract") != {
        "algorithm": "PBKDF2-HMAC-SHA256",
        "iterations": 100000,
        "salt_bytes": 16,
        "derived_bytes": 32,
        "wire": "standard_base64_of_salt_then_hash_48_bytes_64_characters",
        "cookie": "AES-256-GCM_encrypted_x-cap-password_httpOnly_secure_sameSite_lax",
        "cookie_secret": "FRAME_LEGACY_PASSWORD_COOKIE_KEY_V1",
        "cookie_max_hashes": 10,
    }:
        errors.append("video-properties password or encrypted-cookie contract drifted")
    atomicity = fixture.get("atomicity", {})
    if (
        atomicity.get("same_key_same_fingerprint") != "replay_original_projection"
        or atomicity.get("same_key_different_fingerprint") != "conflict"
        or atomicity.get("stale_authority")
        != "abort_without_partial_mutation_or_evidence"
        or atomicity.get("native_metadata_isolation")
        != "legacy_metadata_json_never_overwrites_checksummed_native_metadata_json"
        or atomicity.get("immutable_evidence") is not True
    ):
        errors.append("video-properties atomicity or native metadata isolation drifted")

    operations = fixture.get("operations", [])
    if (
        not isinstance(operations, list)
        or {row.get("id") for row in operations} != set(expected)
    ):
        errors.append("video-properties fixture operation set drifted")
        return errors
    report_by_id = {row.get("id"): row for row in report.get("entries", [])}
    for operation in operations:
        operation_id = operation["id"]
        contract = expected[operation_id]
        row = report_by_id.get(operation_id, {})
        if (
            operation.get("kind") != contract["kind"]
            or operation.get("method") != contract["method"]
            or operation.get("legacy_identity") != contract["identity"]
            or operation.get("source_count") != contract["source_count"]
            or operation.get("source_manifest_sha256") != contract["manifest"]
            or operation.get("protected_gates") != contract["gates"]
            or operation.get("production_behavior") != contract["behavior"]
        ):
            errors.append(f"video-properties fixture contract drifted: {operation_id}")
        if (
            row.get("kind") != contract["kind"]
            or row.get("method") != contract["method"]
            or row.get("legacy_path") != contract["identity"]
            or row.get("auth") != contract["auth"]
            or row.get("policy") != contract["policy"]
            or len(row.get("sources", [])) != contract["source_count"]
            or canonical_json_sha256(row.get("sources", [])) != contract["manifest"]
            or row.get("security", {}).get("idempotency") != "optional"
            or row.get("security", {}).get("rate_limit_bucket") != contract["policy"]
            or set(row.get("contract_evidence", {}).values()) != {"local_contract"}
            or row.get("completion", {}).get("local_work") != "complete"
            or row.get("completion", {}).get("protected_gates") != contract["gates"]
            or row.get("completion", {}).get("production_behavior")
            != contract["behavior"]
        ):
            errors.append(
                f"video-properties report five-axis evidence drifted: {operation_id}"
            )

    for token in (
        "LEGACY_VIDEO_PROPERTY_PROFILES",
        "LEGACY_MOBILE_VIDEO_PASSWORD_SOURCES",
        "LEGACY_MOBILE_VIDEO_SHARING_SOURCES",
        "LEGACY_MOBILE_VIDEO_TITLE_SOURCES",
        "LEGACY_VIDEO_METADATA_SOURCES",
        "LEGACY_EDIT_VIDEO_DATE_SOURCES",
        "LEGACY_EDIT_VIDEO_TITLE_SOURCES",
        "LEGACY_REMOVE_VIDEO_PASSWORD_SOURCES",
        "LEGACY_SET_VIDEO_PASSWORD_SOURCES",
        "LEGACY_VERIFY_VIDEO_PASSWORD_SOURCES",
        "LEGACY_UPDATE_VIDEO_SETTINGS_SOURCES",
        "trim_ecmascript",
        "javascript_truthy_json",
        "javascript_object_spread",
        "normalize_playback_speed",
        "profiles_pin_ten_distinct_contracts_and_password_parameters",
    ):
        if token not in application:
            errors.append(f"video-properties application proof lost token: {token}")
    for contract in expected.values():
        if contract["manifest"] not in application:
            errors.append(
                "video-properties application source-manifest closure drifted: "
                + contract["manifest"]
            )
    if (
        "mod legacy_video_properties;" not in application_lib
        or "pub use legacy_video_properties::*;" not in application_lib
    ):
        errors.append("video-properties semantics are not exported by frame-application")
    for token in (
        "D1LegacyVideoPropertiesAtomicPortV1",
        "pbkdf2_hmac::<Sha256>",
        "VERIFICATION_CANDIDATES_SQL",
        "javascript_object_spread",
        "property_revision",
        ".batch(statements)",
        "pbkdf2_wire_is_source_exact_and_verifies_without_trimming",
        "millisecond_dates_match_javascript_iso_shape",
    ):
        if token not in d1_runtime:
            errors.append(f"video-properties D1 runtime proof lost token: {token}")
    for query_name, token in (
        ("video_snapshot.sql", "v.legacy_property_revision AS property_revision"),
        (
            "mutation_apply.sql",
            "legacy_property_revision = legacy_property_revision + 1",
        ),
        ("owner_assert.sql", "v.legacy_property_revision = ?10"),
        ("verification_assert.sql", "v.legacy_property_revision = ?4"),
    ):
        if token not in query_text.get(query_name, ""):
            errors.append(
                "video-properties D1 property-revision proof drifted: "
                f"{query_name} lost {token}"
            )
    for token in (
        "frame.web-video-property-action-request.v1",
        "optional_header_idempotency",
        "authenticate_mobile",
        "authenticate_host_only_browser_session",
        "action.anonymous()",
        "consume_session_grant_or_confirm_absent",
        "Aes256Gcm",
        "FRAME_LEGACY_PASSWORD_COOKIE_KEY_V1",
        "x-cap-password",
        "MAX_VERIFIED_HASHES",
        "SameSite=Lax",
        "action_wire_allows_internal_idempotency_and_preserves_password_whitespace",
        "cookie_envelope_primitives_are_canonical_and_bounded",
    ):
        if token not in ingress:
            errors.append(f"video-properties HTTP ingress proof lost token: {token}")
    for token in (
        "legacy_video_property_operations_v1",
        "legacy_video_property_receipts_v1",
        "legacy_video_property_effects_v1",
        "legacy_video_property_audit_v1",
        "legacy_video_property_assertions_v1",
        "legacy_metadata_json",
        "legacy_property_revision",
        "frame_legacy_video_property_evidence_immutable_v1",
    ):
        if token not in migration:
            errors.append(f"video-properties migration proof lost token: {token}")
    if len(query_names) != 16:
        errors.append("video-properties checked-in SQL closure must contain exactly 16 queries")
    for token in (
        "test_static_contract",
        "test_all_mutations_replay_and_password_order",
        "test_stale_snapshot_rolls_back_and_evidence_is_immutable",
        "assert len(SQL) == 16",
        'parser.add_argument("--evidence", "--evidence-out"',
    ):
        if token not in conformance:
            errors.append(f"video-properties SQLite proof lost token: {token}")
    for token in (
        "mod legacy_video_properties_runtime;",
        "mod legacy_video_properties_web_runtime;",
        "legacy_video_properties_web_runtime::mobile_response",
        "legacy_video_properties_web_runtime::metadata_response",
        "legacy_video_properties_web_runtime::action_response",
    ):
        if token not in control_lib:
            errors.append(f"video-properties control-plane wiring lost token: {token}")
    for token in (
        "LegacyMobileCapPassword",
        "LegacyMobileCapSharing",
        "LegacyMobileCapTitle",
        "LegacyVideoMetadata",
        '"/api/video/metadata"',
    ):
        if token not in routing:
            errors.append(f"video-properties raw routing lost token: {token}")
    for token in (
        "LegacyRegistrationSourcesV1::VideoProperties",
        "LegacyVideoPropertiesInvocationV1",
        "dispatch_video_properties",
        "LEGACY_MOBILE_VIDEO_PASSWORD_OPERATION_ID",
        "LEGACY_UPDATE_VIDEO_SETTINGS_OPERATION_ID",
    ):
        if token not in registry:
            errors.append(f"video-properties registry proof lost token: {token}")
    for token in (
        "legacy-video-properties-sqlite-conformance.py",
        "legacy-video-properties-sqlite-conformance.json",
        "fixtures/api-parity/v1/video-properties.json",
        "frame-application --lib legacy_video_properties",
        "frame-control-plane --lib legacy_video_properties",
    ):
        if token not in workflow:
            errors.append(f"video-properties workflow proof lost token: {token}")
    return errors


def validate_notification_read_fixture(report: dict[str, Any]) -> list[str]:
    """Prove the exact scoped D1 notification-list route and null avatar fallback."""
    try:
        fixture = json.loads(NOTIFICATION_READ_FIXTURE.read_text(encoding="utf-8"))
        application = APPLICATION_NOTIFICATION_READ.read_text(encoding="utf-8")
        application_lib = APPLICATION_LIB.read_text(encoding="utf-8")
        d1_runtime = CONTROL_NOTIFICATION_READ_RUNTIME.read_text(encoding="utf-8")
        control_lib = CONTROL_LIB.read_text(encoding="utf-8")
        compatibility_runtime = CONTROL_RUNTIME.read_text(encoding="utf-8")
        routing = CONTROL_ROUTING.read_text(encoding="utf-8")
        conformance = NOTIFICATION_READ_CONFORMANCE.read_text(encoding="utf-8")
        workflow = parity_workflow_contract()
        query_names = {
            path.name for path in NOTIFICATION_READ_QUERY_ROOT.glob("*.sql")
        }
    except (OSError, json.JSONDecodeError) as error:
        return [f"unable to load notification-read fixture: {error}"]

    errors: list[str] = []
    expected_operation = {
        "id": "cap-v1-14dcca6d36eee6b3",
        "kind": "route",
        "method": "GET",
        "path": "/api/notifications",
        "auth": "host_only_browser_session",
        "policy": "collaboration_notifications.v1",
        "idempotency": "forbidden",
        "protected_gates": [],
        "production_behavior": "serve_exact_route",
    }
    if (
        fixture.get("schema_version") != "frame.legacy-notification-read.v1"
        or fixture.get("reference_commit") != REFERENCE_COMMIT
        or fixture.get("operation") != expected_operation
    ):
        errors.append("notification-read fixture identity or completion drifted")
    wire = fixture.get("wire_contract", {})
    if wire.get("success") != {
        "status": 200,
        "content_type": "application/json",
        "body": {
            "notifications": "source-shaped notification array",
            "count": {
                "view": "number; includes view + anon_view",
                "comment": "number",
                "reply": "number",
                "reaction": "number",
            },
        },
    }:
        errors.append("notification-read success wire contract drifted")
    if wire.get("unauthenticated") != {
        "status": 401,
        "content_type": "text/plain;charset=UTF-8",
        "body": '{"error":"Unauthorized"}',
    } or wire.get("database_failure") != {
        "status": 500,
        "content_type": "application/json",
        "body": '{"error":"Failed to fetch notifications"}',
    }:
        errors.append("notification-read failure wire contract drifted")
    semantics = fixture.get("semantic_contract", {})
    for key, value in {
        "scope": "recipient actor plus actor active organization",
        "ordering": "unread first, then createdAt descending",
        "authored_rows": "join authorId unless type is anon_view; missing authors are omitted",
        "invalid_rows": "each invalid API notification is omitted without failing siblings",
        "avatar_resolution": "resolve when possible; provider failures return avatar null",
        "dates": "JavaScript Date JSON ISO-8601 with millisecond precision",
    }.items():
        if semantics.get(key) != value:
            errors.append(f"notification-read semantic contract drifted: {key}")

    sources = fixture.get("sources", [])
    expected_source_paths = {
        "apps/web/app/api/notifications/route.ts",
        "packages/web-api-contract/src/index.ts",
        "packages/database/schema.ts",
        "packages/database/auth/session.ts",
        "packages/web-backend/src/ImageUploads/index.ts",
    }
    if (
        not isinstance(sources, list)
        or {source.get("path") for source in sources} != expected_source_paths
        or any(
            not isinstance(source.get("sha256"), str)
            or len(source["sha256"]) != 64
            for source in sources
        )
    ):
        errors.append("notification-read source closure drifted")

    row = next(
        (
            candidate
            for candidate in report.get("entries", [])
            if candidate.get("id") == expected_operation["id"]
        ),
        {},
    )
    fixture_source_digests = {
        source["path"]: source["sha256"] for source in sources if isinstance(source, dict)
    }
    report_source_digests = {
        source.get("path"): source.get("sha256") for source in row.get("sources", [])
    }
    if (
        row.get("kind") != "route"
        or row.get("method") != "GET"
        or row.get("legacy_path") != "/api/notifications"
        or row.get("auth") != "session"
        or row.get("policy") != "collaboration_notifications.v1"
        or report_source_digests != fixture_source_digests
        or row.get("implementation", {}).get("local_status")
        != "rust_exact_notification_list_d1_adapter_local_contract"
        or set(row.get("contract_evidence", {}).values()) != {"local_contract"}
        or row.get("security", {}).get("idempotency") != "forbidden"
        or row.get("completion")
        != {
            "decision": "serve_frame_exact_business",
            "local_work": "complete",
            "protected_gates": [],
            "retirement_decision": "not_proposed",
            "production_behavior": "serve_exact_d1",
        }
    ):
        errors.append("notification-read report evidence drifted")

    for token in (
        "LEGACY_NOTIFICATION_READ_SOURCES",
        "unread_first_then_created_at_desc",
        "omit_individually",
        "return_null_avatar",
        "AnonymousView",
        "from_grouped_counts",
    ):
        if token not in application:
            errors.append(f"notification-read application proof lost token: {token}")
    if (
        "mod legacy_notification_read;" not in application_lib
        or "pub use legacy_notification_read::*;" not in application_lib
    ):
        errors.append("notification-read semantics are not exported by frame-application")
    for token in (
        "READ_ROWS_SQL",
        "READ_COUNTS_SQL",
        "read_exact_json",
        "project_row",
        '"Anonymous Viewer"',
        "iso_timestamp",
        "avatar: author.resolved_avatar",
    ):
        if token not in d1_runtime:
            errors.append(f"notification-read D1 runtime proof lost token: {token}")
    if query_names != {"read_rows.sql", "read_counts.sql"}:
        errors.append("notification-read SQL closure must contain exactly two named reads")
    for token in (
        "n.recipient_user_id = actor.id",
        "n.organization_id = actor.active_organization_id",
        "n.read_at_ms IS NULL DESC",
        "n.created_at_ms DESC",
        "assert counts ==",
    ):
        if token not in conformance and token not in "\n".join(
            path.read_text(encoding="utf-8")
            for path in NOTIFICATION_READ_QUERY_ROOT.glob("*.sql")
        ):
            errors.append(f"notification-read SQLite proof lost token: {token}")
    for token in (
        "mod legacy_notification_read_runtime;",
        "Route::LegacyNotifications",
        "legacy_notifications_response",
        "legacy_notification_read_runtime::read_exact_json",
    ):
        if token not in control_lib:
            errors.append(f"notification-read control-plane wiring lost token: {token}")
    for token in ('LegacyNotifications', '"/api/notifications"'):
        if token not in routing:
            errors.append(f"notification-read raw routing lost token: {token}")
    for token in (
        "NotificationReadGet",
        "LEGACY_NOTIFICATION_READ_OPERATION_ID",
        "LEGACY_NOTIFICATION_READ_RUNTIME_SOURCES",
        "notification_read_body",
    ):
        if token not in compatibility_runtime:
            errors.append(f"notification-read registry proof lost token: {token}")
    for token in (
        "legacy-notification-read-sqlite-conformance.py",
        "notification-read.json",
        "frame-application --lib legacy_notification_read",
        "frame-control-plane --lib legacy_notification_read_runtime",
    ):
        if token not in workflow:
            errors.append(f"notification-read workflow proof lost token: {token}")
    return errors


def validate_declaration_only_dispositions_fixture(
    report: dict[str, Any],
) -> list[str]:
    """Keep schema-only identities fail-closed behind an owner disposition."""
    try:
        fixture = json.loads(
            DECLARATION_ONLY_DISPOSITIONS_FIXTURE.read_text(encoding="utf-8")
        )
        compatibility_runtime = CONTROL_RUNTIME.read_text(encoding="utf-8")
        routing = CONTROL_ROUTING.read_text(encoding="utf-8")
        workflow = parity_workflow_contract()
    except (OSError, json.JSONDecodeError) as error:
        return [f"unable to load declaration-only disposition evidence: {error}"]

    errors: list[str] = []
    policy = fixture.get("policy", {})
    if (
        fixture.get("schema_version") != "frame.declaration-only-dispositions.v1"
        or fixture.get("reference_commit") != REFERENCE_COMMIT
        or policy.get("local_work") != "complete"
        or policy.get("protected_gate") != "human_approval"
        or policy.get("production_behavior") != "fail_closed_unavailable"
        or "never promoted by inventing behavior" not in policy.get("rule", "")
    ):
        errors.append("declaration-only disposition policy drifted")

    operations = fixture.get("operations", [])
    by_id = {
        operation.get("id"): operation
        for operation in operations
        if isinstance(operation, dict)
    }
    if set(by_id) != DECLARATION_ONLY_OWNER_DECISION_IDS:
        errors.append("declaration-only disposition operation set drifted")
    expected_contract_sources = {
        "packages/web-api-contract-effect/src/index.ts":
            "9c2185ebf12be4c9d231d42938c975ea6ad596a0031ed8a0aca2bb1cbec3c7a0",
        "packages/web-api-contract/src/index.ts":
            "98bb2529e27eba0ed1569d286a1f5d4069cbbf23cf9e1dde62fdc1f6a9737e3e",
    }
    for operation_id, operation in by_id.items():
        sources = {
            source.get("path"): source.get("sha256")
            for source in operation.get("contract_sources", [])
        }
        if sources != expected_contract_sources or operation.get("executable_handler") is not None:
            errors.append(f"declaration-only source audit drifted: {operation_id}")
        if not operation.get("missing_authority") or not operation.get("owner_decision"):
            errors.append(f"declaration-only decision rationale missing: {operation_id}")

    commercial = by_id.get(LICENSING_DECLARATION_ONLY_ID, {})
    expected_callers = {
        "apps/desktop/src/utils/web-api.ts":
            "d3655b985a21a54d97b9974b17536aebab490929850baffaa5186d7a5632b45a",
        "apps/desktop/src/routes/(window-chrome)/upgrade.tsx":
            "c5c22b8f4d113eeac0e1e08603130b363791c7970ad15d2dc23d93aa095eff32",
        "apps/desktop/src/routes/(window-chrome)/settings/license.tsx":
            "dfb1addc2818cb7ede7e65740d5dee0d20646986651473633adf01245aff316a",
    }
    if {
        source.get("path"): source.get("sha256")
        for source in commercial.get("caller_sources", [])
    } != expected_callers:
        errors.append("commercial declaration-only caller closure drifted")
    if commercial.get("declared_request") != {
        "headers": {"licensekey": "string", "instanceid": "string"},
        "body": {"reset": "boolean|absent"},
    } or commercial.get("declared_success") != {
        "message": "string",
        "expiryDate": "number|absent",
        "refresh": "number",
    }:
        errors.append("commercial declaration-only wire declaration drifted")

    rows = {row.get("id"): row for row in report.get("entries", [])}
    for operation_id in DECLARATION_ONLY_OWNER_DECISION_IDS:
        row = rows.get(operation_id, {})
        if (
            row.get("contract_evidence", {}).get("success") != "endpoint_adapter_pending"
            or row.get("completion")
            != {
                "decision": "declaration_only_owner_disposition_required",
                "local_work": "complete",
                "protected_gates": ["human_approval"],
                "retirement_decision": "repository_owner_pending",
                "production_behavior": "fail_closed_unavailable",
            }
        ):
            errors.append(f"declaration-only protected disposition drifted: {operation_id}")
        if operation_id in compatibility_runtime:
            errors.append(f"declaration-only identity entered production registry: {operation_id}")
    for route_token in ('"/api/org-custom-domain"', '"/api/commercial/activate"'):
        if route_token in routing:
            errors.append(f"declaration-only path entered raw routing: {route_token}")
    if "declaration-only-dispositions.json" not in workflow:
        errors.append("declaration-only disposition fixture is not retained by CI")
    return errors


def validate_desktop_compatibility_fixture(report: dict[str, Any]) -> list[str]:
    """Bind six retained desktop carriers to exact D1/R2 evidence."""
    errors: list[str] = []
    try:
        fixture = json.loads(DESKTOP_COMPATIBILITY_FIXTURE.read_text(encoding="utf-8"))
        application = APPLICATION_DESKTOP_COMPATIBILITY.read_text(encoding="utf-8")
        runtime = CONTROL_DESKTOP_COMPATIBILITY_RUNTIME.read_text(encoding="utf-8")
        web = CONTROL_DESKTOP_COMPATIBILITY_WEB_RUNTIME.read_text(encoding="utf-8")
        migration = DESKTOP_COMPATIBILITY_MIGRATION.read_text(encoding="utf-8")
        routing = CONTROL_ROUTING.read_text(encoding="utf-8")
        control = CONTROL_LIB.read_text(encoding="utf-8")
        registry = CONTROL_RUNTIME.read_text(encoding="utf-8")
        conformance = DESKTOP_COMPATIBILITY_CONFORMANCE.read_text(encoding="utf-8")
        workflow = parity_workflow_contract()
    except (OSError, json.JSONDecodeError) as error:
        return [f"unable to load desktop-compatibility evidence: {error}"]

    expected = {
        "cap-v1-ab49cf36a3f243ac": ("GET", "/api/desktop/organizations", 10, "57c38458f638ec3715c6ea08845e9e7ed34fa594244deca966d71734ce83d8e3", 0, (), "forbidden", "organization_library.v1"),
        "cap-v1-cdfdf7db0f5cb243": ("PATCH", "/api/desktop/organizations/:organizationId/branding", 12, "644d6f74e135bcab7978f4921563b9454b1213345acce07049f360aec3333492", 1_500_000, ("application/json",), "optional", "organization_library.v1"),
        "cap-v1-a77171e54b2ba955": ("POST", "/api/desktop/storage/set-active", 8, "12fd70ac75df2f65fa8daaeaa1f27b0e622e973745b02b199531251dd0bf5ae8", 256 * 1024, ("application/json",), "optional", "upload_storage.v1"),
        "cap-v1-7508c5a7da637a0b": ("GET", "/api/desktop/user/profile", 9, "d3b9e425a3c603aa946511e3418ff607fc0654fe0bdbfa08a8a1942713437cdd", 0, (), "forbidden", "client_compatibility.v1"),
        "cap-v1-acc98d2d5e8ff345": ("DELETE", "/api/desktop/video/delete", 9, "d79e69c91b9fddae0fb55620cde75649e2ab85ad18d1cc16756f5a241d071822", 0, (), "optional", "video_media.v1"),
        "cap-v1-117b0cb801816693": ("POST", "/api/desktop/video/progress", 7, "80fab5666369f7d185f9c4aec1c7a16973a33e9948005078ca1e6b6b2018a135", 256 * 1024, ("application/json",), "optional", "video_media.v1"),
    }
    completion = {
        "decision": "serve_frame_exact_business",
        "local_work": "complete",
        "protected_gates": [],
        "production_behavior": "serve_exact_d1",
    }
    if (
        fixture.get("schema_version") != "frame.legacy-desktop-compatibility.v1"
        or fixture.get("cap_commit") != REFERENCE_COMMIT
        or fixture.get("completion") != completion
    ):
        errors.append("desktop-compatibility fixture identity or completion drifted")
    fixture_operations = {
        operation.get("id"): operation for operation in fixture.get("operations", [])
    }
    report_operations = {
        row.get("id"): row for row in report.get("entries", []) if row.get("id") in expected
    }
    if set(fixture_operations) != set(expected) or set(report_operations) != set(expected):
        errors.append("desktop-compatibility operation closure drifted")
    for operation_id, contract in expected.items():
        method, path, source_count, manifest, max_body, content_types, idempotency, bucket = contract
        operation = fixture_operations.get(operation_id, {})
        row = report_operations.get(operation_id, {})
        security = row.get("security", {})
        if (
            operation.get("method") != method
            or operation.get("path") != path
            or operation.get("source_count") != source_count
            or operation.get("source_manifest_sha256") != manifest
            or operation.get("max_body_bytes") != max_body
            or tuple(operation.get("accepted_content_types", [])) != content_types
            or operation.get("idempotency") != idempotency
            or operation.get("rate_limit_bucket") != bucket
        ):
            errors.append(f"desktop-compatibility fixture contract drifted: {operation_id}")
        if (
            row.get("kind") != "route"
            or row.get("method") != method
            or row.get("legacy_path") != path
            or row.get("clients") != ["desktop"]
            or row.get("auth") != "session_or_api_key"
            or row.get("policy") != bucket
            or len(row.get("sources", [])) != source_count
            or canonical_json_sha256(row.get("sources", [])) != manifest
            or security.get("max_body_bytes") != max_body
            or tuple(security.get("accepted_content_types", [])) != content_types
            or security.get("idempotency") != idempotency
            or security.get("rate_limit_bucket") != bucket
            or set(row.get("contract_evidence", {}).values()) != {"local_contract"}
            or row.get("completion") != {**completion, "retirement_decision": "not_proposed"}
        ):
            errors.append(f"desktop-compatibility report evidence drifted: {operation_id}")
        if manifest not in application:
            errors.append(f"desktop-compatibility application manifest lost: {operation_id}")

    authentication = fixture.get("authentication", {})
    cors = fixture.get("cors", {})
    if (
        authentication.get("class") != "session_or_api_key"
        or authentication.get("unauthenticated_status") != 401
        or authentication.get("unauthenticated_body") != "User not authenticated"
        or "exactly 36 characters" not in authentication.get("selector", "")
        or cors.get("credentials") is not True
        or cors.get("allow_methods") != "GET, POST, PATCH, DELETE, OPTIONS"
        or cors.get("allow_headers") != "Content-Type, Authorization, sentry-trace, baggage"
        or len(cors.get("allowed_origins", [])) != 6
    ):
        errors.append("desktop-compatibility authentication or CORS contract drifted")
    idempotency = fixture.get("idempotency", {})
    if (
        idempotency.get("released_clients_send_header") is not False
        or "optional" not in idempotency.get("mutations", "")
        or idempotency.get("reads") != "forbidden"
        or idempotency.get("same_key_different_request") != "409 conflict"
        or "effect_pending" not in idempotency.get("video_delete_continuation", "")
    ):
        errors.append("desktop-compatibility idempotency contract drifted")
    for token in (
        "LegacyDesktopCompatibilityAdapterV1",
        "desktop_profile_name",
        "merge_organization_branding_metadata",
        "LEGACY_DESKTOP_LOGO_MAX_BYTES",
        "desktop-auto:",
    ):
        if token not in application:
            errors.append(f"desktop-compatibility application proof lost token: {token}")
    for token in (
        "effect_pending",
        "VIDEO_DELETE_PENDING_OBJECTS_SQL",
        "object_legal_holds",
        "VIDEO_DELETE_OBJECT_COMPLETE_SQL",
        "legacy_desktop_personal_storage_integrations_v1",
    ):
        if token not in runtime and token not in migration and token not in "".join(
            path.read_text(encoding="utf-8") for path in DESKTOP_COMPATIBILITY_QUERY_ROOT.glob("*.sql")
        ):
            errors.append(f"desktop-compatibility D1/R2 proof lost token: {token}")
    for token in (
        "authenticate(request, env, now_ms)",
        "LegacyCompatibilityTransportV1::new_fail_closed",
        "CompatibilityRateLimitBucketV1::UploadStorage",
        "User not authenticated",
    ):
        if token not in web:
            errors.append(f"desktop-compatibility HTTP proof lost token: {token}")
    for token in (
        "LegacyDesktopOrganizations",
        "LegacyDesktopOrganizationBranding",
        "LegacyDesktopStorageSetActive",
        "LegacyDesktopUserProfile",
        "LegacyDesktopVideoDelete",
        "LegacyDesktopVideoProgress",
    ):
        if token not in routing or token not in control:
            errors.append(f"desktop-compatibility raw routing lost token: {token}")
    for token in (
        "LEGACY_DESKTOP_ORGANIZATIONS_OPERATION_ID",
        "LEGACY_DESKTOP_ORGANIZATION_BRANDING_OPERATION_ID",
        "LEGACY_DESKTOP_STORAGE_SET_ACTIVE_OPERATION_ID",
        "LEGACY_DESKTOP_USER_PROFILE_OPERATION_ID",
        "LEGACY_DESKTOP_VIDEO_DELETE_OPERATION_ID",
        "LEGACY_DESKTOP_VIDEO_PROGRESS_OPERATION_ID",
    ):
        if token not in registry:
            errors.append(f"desktop-compatibility central registry lost token: {token}")
    for token in (
        "Organization reads are live, actor-scoped",
        "Profile projection preserves nullability",
        "timestamp-arbitrated progress",
        "resumable D1/R2 video deletion",
    ):
        if token not in conformance:
            errors.append(f"desktop-compatibility SQLite proof lost token: {token}")
    for token in (
        "legacy-desktop-compatibility-sqlite-conformance.py",
        "desktop-compatibility.json",
        "frame-application --lib legacy_desktop_compatibility",
        "frame-control-plane --lib legacy_desktop_compatibility",
    ):
        if token not in workflow:
            errors.append(f"desktop-compatibility workflow proof lost token: {token}")
    return errors


def validate_desktop_session_fixture(report: dict[str, Any]) -> list[str]:
    """Bind the desktop sign-in bridge to exact AuthService and D1 evidence."""
    try:
        fixture = json.loads(DESKTOP_SESSION_FIXTURE.read_text(encoding="utf-8"))
        application = APPLICATION_DESKTOP_SESSION.read_text(encoding="utf-8")
        application_lib = APPLICATION_LIB.read_text(encoding="utf-8")
        browser_runtime = CONTROL_BROWSER_RUNTIME.read_text(encoding="utf-8")
        runtime = CONTROL_DESKTOP_SESSION_RUNTIME.read_text(encoding="utf-8")
        web_runtime = CONTROL_DESKTOP_SESSION_WEB_RUNTIME.read_text(encoding="utf-8")
        control_lib = CONTROL_LIB.read_text(encoding="utf-8")
        routing = CONTROL_ROUTING.read_text(encoding="utf-8")
        compatibility_runtime = CONTROL_RUNTIME.read_text(encoding="utf-8")
        conformance = DESKTOP_SESSION_CONFORMANCE.read_text(encoding="utf-8")
        workflow = parity_workflow_contract()
        query_names = {path.name for path in DESKTOP_SESSION_QUERY_ROOT.glob("*.sql")}
        query_text = "\n".join(
            path.read_text(encoding="utf-8")
            for path in DESKTOP_SESSION_QUERY_ROOT.glob("*.sql")
        )
    except (OSError, json.JSONDecodeError) as error:
        return [f"unable to load desktop-session evidence: {error}"]

    errors: list[str] = []
    expected_operation = {
        "id": "cap-v1-768895bc99380850",
        "kind": "route",
        "method": "GET",
        "path": "/api/desktop/session/request",
        "auth": "session",
        "policy": "auth_session.v1",
        "idempotency": "forbidden",
        "protected_gates": [],
        "production_behavior": "serve_exact_d1",
    }
    if (
        fixture.get("schema_version") != "frame.legacy-desktop-session.v1"
        or fixture.get("reference_commit") != REFERENCE_COMMIT
        or fixture.get("operation") != expected_operation
    ):
        errors.append("desktop-session fixture identity or completion drifted")

    sources = fixture.get("sources", [])
    expected_paths = {
        "apps/web/app/api/desktop/[...route]/session.ts",
        "apps/web/app/api/desktop/[...route]/route.ts",
        "apps/desktop/src/utils/auth.ts",
        "apps/desktop/src/utils/server-url-routing.ts",
        "apps/web/app/api/utils.ts",
        "apps/web/proxy.ts",
        "packages/database/auth/auth-options.ts",
        "packages/database/auth/session.ts",
        "packages/database/schema.ts",
        "packages/database/index.ts",
        "packages/env/server.ts",
        "pnpm-lock.yaml",
    }
    if (
        not isinstance(sources, list)
        or {source.get("path") for source in sources} != expected_paths
        or any(
            not isinstance(source.get("sha256"), str)
            or len(source["sha256"]) != 64
            for source in sources
        )
    ):
        errors.append("desktop-session source closure drifted")

    query_contract = fixture.get("query_contract", {})
    for token in (
        "decimal TCP port 1..65535",
        "default web",
        "default session",
        "400 Bad Request",
    ):
        if token not in json.dumps(query_contract, sort_keys=True):
            errors.append(f"desktop-session query contract lost token: {token}")
    wire = json.dumps(fixture.get("wire_contract", {}), sort_keys=True)
    for token in (
        "__Host-frame_session",
        "type=token&token=<raw>&expires=<seconds>&user_id=<actor>",
        "type=api_key&api_key=<uuid>&user_id=<actor>",
        "http://127.0.0.1:<validated-port>",
        "cap-desktop://signin",
        "1800ms",
        "no-store",
    ):
        if token not in wire:
            errors.append(f"desktop-session wire contract lost token: {token}")

    row = next(
        (
            entry
            for entry in report.get("entries", [])
            if entry.get("id") == expected_operation["id"]
        ),
        {},
    )
    fixture_digests = {
        source["path"]: source["sha256"]
        for source in sources
        if isinstance(source, dict)
    }
    report_digests = {
        source.get("path"): source.get("sha256") for source in row.get("sources", [])
    }
    if (
        row.get("kind") != "route"
        or row.get("method") != "GET"
        or row.get("legacy_path") != "/api/desktop/session/request"
        or row.get("clients") != ["desktop"]
        or row.get("auth") != "session"
        or row.get("policy") != "auth_session.v1"
        or report_digests != fixture_digests
        or row.get("implementation", {}).get("local_status")
        != "rust_exact_desktop_session_handoff_d1_adapter_local_contract"
        or set(row.get("contract_evidence", {}).values()) != {"local_contract"}
        or row.get("security", {}).get("max_body_bytes") != 0
        or row.get("security", {}).get("idempotency") != "forbidden"
        or row.get("completion")
        != {
            "decision": "serve_frame_exact_business",
            "local_work": "complete",
            "protected_gates": [],
            "retirement_decision": "not_proposed",
            "production_behavior": "serve_exact_d1",
        }
    ):
        errors.append("desktop-session report five-axis evidence drifted")

    for token in (
        "LEGACY_DESKTOP_SESSION_SOURCES",
        "LEGACY_DESKTOP_SESSION_SOURCE_MANIFEST_SHA256",
        "parse_legacy_desktop_session_query",
        "legacy_desktop_session_destination",
        "legacy_desktop_login_url",
        "render_legacy_desktop_redirect_page",
        "port == 0",
    ):
        if token not in application:
            errors.append(f"desktop-session application proof lost token: {token}")
    if (
        "mod legacy_desktop_session;" not in application_lib
        or "pub use legacy_desktop_session::*;" not in application_lib
    ):
        errors.append("desktop-session contract is not exported by frame-application")
    for token in (
        "authenticate_host_only_browser_session_export",
        "auth_sessions_v2",
        "idle_expires_at_ms < absolute_expires_at_ms",
        "admission.session_id",
        "unique_cookie(request, SESSION_COOKIE_NAME)",
    ):
        if token not in browser_runtime:
            errors.append(f"desktop-session AuthService export lost token: {token}")
    if query_names != {"mint_desktop_key.sql"}:
        errors.append("desktop-session checked-in SQL closure drifted")
    for token in (
        "INSERT INTO auth_api_keys",
        "key_digest",
        "'desktop'",
        "u.status = 'active'",
        "u.deleted_at_ms IS NULL",
        "RETURNING id",
    ):
        if token not in query_text:
            errors.append(f"desktop-session SQL authority lost token: {token}")
    for token in (
        "mint_desktop_key",
        "Uuid::new_v4",
        "sha256_hex",
        "row.id == row_id",
    ):
        if token not in runtime:
            errors.append(f"desktop-session D1 runtime lost token: {token}")
    for token in (
        "LegacyDesktopSessionCredentialTypeV1::Session",
        "LegacyDesktopSessionCredentialTypeV1::ApiKey",
        "div_euclid(1_000)",
        "HybridPage",
        'get("idempotency-key")',
        "content-security-policy",
        "VERCEL_BRANCH_URL_HOST",
    ):
        if token not in web_runtime:
            errors.append(f"desktop-session HTTP carrier lost token: {token}")
    for token in (
        "mod legacy_desktop_session_runtime;",
        "mod legacy_desktop_session_web_runtime;",
        "Route::LegacyDesktopSessionRequest",
        "legacy_desktop_session_web_runtime::response",
    ):
        if token not in control_lib:
            errors.append(f"desktop-session control-plane wiring lost token: {token}")
    for token in ("LegacyDesktopSessionRequest", '"/api/desktop/session/request"'):
        if token not in routing:
            errors.append(f"desktop-session raw routing lost token: {token}")
    for token in (
        "LEGACY_DESKTOP_SESSION_OPERATION_ID",
        "LEGACY_DESKTOP_SESSION_SOURCES",
        "LegacyRegistrationSourcesV1::DesktopSession",
    ):
        if token not in compatibility_runtime:
            errors.append(f"desktop-session registry proof lost token: {token}")
    for token in (
        "digest-only desktop UUID mint",
        "session-id-bound minimum expiry",
        "duplicate raw desktop key digest",
    ):
        if token not in conformance:
            errors.append(f"desktop-session SQLite proof lost token: {token}")
    for token in (
        "legacy-desktop-session-sqlite-conformance.py",
        "desktop-session.json",
        "frame-application --lib legacy_desktop_session",
        "frame-control-plane --lib legacy_desktop_session",
    ):
        if token not in workflow:
            errors.append(f"desktop-session workflow proof lost token: {token}")
    return errors


def validate_org_custom_domain_fixture(report: dict[str, Any]) -> list[str]:
    """Prove the desktop runtime without promoting declaration-only web semantics."""
    try:
        fixture = json.loads(ORG_CUSTOM_DOMAIN_FIXTURE.read_text(encoding="utf-8"))
        application = APPLICATION_ORG_CUSTOM_DOMAIN.read_text(encoding="utf-8")
        application_lib = APPLICATION_LIB.read_text(encoding="utf-8")
        runtime = CONTROL_ORG_CUSTOM_DOMAIN_RUNTIME.read_text(encoding="utf-8")
        web_runtime = CONTROL_ORG_CUSTOM_DOMAIN_WEB_RUNTIME.read_text(encoding="utf-8")
        migration = ORG_CUSTOM_DOMAIN_MIGRATION.read_text(encoding="utf-8")
        conformance = ORG_CUSTOM_DOMAIN_CONFORMANCE.read_text(encoding="utf-8")
        control_lib = CONTROL_LIB.read_text(encoding="utf-8")
        routing = CONTROL_ROUTING.read_text(encoding="utf-8")
        compatibility_runtime = CONTROL_RUNTIME.read_text(encoding="utf-8")
        workflow = parity_workflow_contract()
        query_names = {path.name for path in ORG_CUSTOM_DOMAIN_QUERY_ROOT.glob("*.sql")}
        query_text = "\n".join(
            path.read_text(encoding="utf-8")
            for path in ORG_CUSTOM_DOMAIN_QUERY_ROOT.glob("*.sql")
        )
    except (OSError, json.JSONDecodeError) as error:
        return [f"unable to load organization custom-domain evidence: {error}"]

    errors: list[str] = []
    expected_desktop = {
        "id": "cap-v1-ed9957ac480103b9",
        "kind": "route",
        "method": "GET",
        "path": "/api/desktop/org-custom-domain",
        "auth": "session_or_api_key",
        "policy": "client_compatibility.v1",
        "idempotency": "forbidden",
        "protected_gates": [],
        "production_behavior": "serve_exact_d1",
    }
    if (
        fixture.get("schema_version") != "frame.legacy-org-custom-domain.v1"
        or fixture.get("reference_commit") != REFERENCE_COMMIT
        or fixture.get("desktop_operation") != expected_desktop
    ):
        errors.append("desktop organization custom-domain identity or completion drifted")

    desktop_sources = fixture.get("desktop_sources", [])
    expected_desktop_source_paths = {
        "apps/desktop/src/utils/queries.ts",
        "apps/desktop/src/utils/web-api.ts",
        "apps/web/app/api/desktop/[...route]/root.ts",
        "apps/web/app/api/desktop/[...route]/route.ts",
        "apps/web/app/api/utils.ts",
        "packages/database/schema.ts",
    }
    if (
        not isinstance(desktop_sources, list)
        or {source.get("path") for source in desktop_sources}
        != expected_desktop_source_paths
        or any(
            not isinstance(source.get("sha256"), str)
            or len(source["sha256"]) != 64
            for source in desktop_sources
        )
    ):
        errors.append("desktop organization custom-domain source closure drifted")

    wire = fixture.get("desktop_wire_contract", {})
    if wire.get("success") != {
        "status": 200,
        "content_type": "application/json",
        "body": {
            "custom_domain": "string|null; prepend https:// unless case-sensitive http:// or https:// prefix already exists",
            "domain_verified": "JavaScript Date JSON ISO-8601 string|null",
        },
    }:
        errors.append("desktop organization custom-domain success wire drifted")
    if wire.get("unauthenticated") != {
        "status": 401,
        "content_type": "text/plain; charset=UTF-8",
        "body": "User not authenticated",
    } or wire.get("database_failure") != {
        "status": 500,
        "content_type": "application/json",
        "body": '{"error":"Failed to fetch custom domain"}',
    }:
        errors.append("desktop organization custom-domain failure wire drifted")
    if wire.get("cors") != {
        "allowed_origins": "configured web origin plus localhost:3000, localhost:3001, and three Tauri origins",
        "credentials": True,
        "methods": "GET, POST, PATCH, DELETE, OPTIONS",
        "headers": "Content-Type, Authorization, sentry-trace, baggage",
    }:
        errors.append("desktop organization custom-domain CORS contract drifted")

    semantics = fixture.get("desktop_semantics", {})
    for key, value in {
        "api_key_selector": "second literal-space authorization segment with exactly 36 characters; otherwise session fallback",
        "scope": "authenticated actor active organization derived in D1",
        "projection": "lossless independent customDomain and domainVerified ISO fields",
        "missing_actor_or_active_organization": "both response fields null",
        "missing_projection_for_existing_organization": "fail closed as corrupt import",
    }.items():
        if semantics.get(key) != value:
            errors.append(f"desktop organization custom-domain semantic drifted: {key}")

    desktop_row = next(
        (
            row
            for row in report.get("entries", [])
            if row.get("id") == expected_desktop["id"]
        ),
        {},
    )
    desktop_fixture_digests = {
        source["path"]: source["sha256"]
        for source in desktop_sources
        if isinstance(source, dict)
    }
    desktop_report_digests = {
        source.get("path"): source.get("sha256")
        for source in desktop_row.get("sources", [])
    }
    if (
        desktop_row.get("kind") != "route"
        or desktop_row.get("method") != "GET"
        or desktop_row.get("legacy_path") != "/api/desktop/org-custom-domain"
        or desktop_row.get("clients") != ["desktop"]
        or desktop_row.get("auth") != "session_or_api_key"
        or desktop_row.get("policy") != "client_compatibility.v1"
        or desktop_report_digests != desktop_fixture_digests
        or desktop_row.get("implementation", {}).get("local_status")
        != "rust_exact_desktop_org_custom_domain_d1_adapter_local_contract"
        or set(desktop_row.get("contract_evidence", {}).values()) != {"local_contract"}
        or desktop_row.get("security", {}).get("idempotency") != "forbidden"
        or desktop_row.get("completion")
        != {
            "decision": "serve_frame_exact_business",
            "local_work": "complete",
            "protected_gates": [],
            "retirement_decision": "not_proposed",
            "production_behavior": "serve_exact_d1",
        }
    ):
        errors.append("desktop organization custom-domain report evidence drifted")

    web = fixture.get("web_declaration_only", {})
    expected_web_sources = {
        "packages/web-api-contract-effect/src/index.ts": "9c2185ebf12be4c9d231d42938c975ea6ad596a0031ed8a0aca2bb1cbec3c7a0",
        "packages/web-api-contract/src/index.ts": "98bb2529e27eba0ed1569d286a1f5d4069cbbf23cf9e1dde62fdc1f6a9737e3e",
    }
    web_fixture_sources = {
        source.get("path"): source.get("sha256") for source in web.get("sources", [])
    }
    if (
        web.get("id") != "cap-v1-9323d0178c5a63b5"
        or web.get("path") != "/api/org-custom-domain"
        or web.get("auth")
        != "authorization_string_declaration_with_pinned_bearer_client"
        or web.get("bearer_client_source")
        != {
            "path": "apps/desktop/src/utils/web-api.ts",
            "symbol": "protectedHeaders",
            "sha256": "d3655b985a21a54d97b9974b17536aebab490929850baffaa5186d7a5632b45a",
        }
        or web.get("declared_success")
        != {"custom_domain": "string|null", "domain_verified": "boolean|null"}
        or web.get("executable_handler") is not None
        or web.get("boolean_derivation") is not None
        or web.get("completion") != "declaration_only_owner_disposition_required"
        or web.get("local_work") != "complete"
        or web.get("protected_gates") != ["human_approval"]
        or web.get("retirement_decision") != "repository_owner_pending"
        or web.get("production_behavior") != "fail_closed_unavailable"
        or web_fixture_sources != expected_web_sources
    ):
        errors.append("web organization custom-domain declaration audit drifted")
    web_row = next(
        (
            row
            for row in report.get("entries", [])
            if row.get("id") == "cap-v1-9323d0178c5a63b5"
        ),
        {},
    )
    web_report_sources = {
        source.get("path"): source.get("sha256")
        for source in web_row.get("sources", [])
    }
    if (
        web_report_sources != expected_web_sources
        or web_row.get("implementation")
        != {
            "rust_authority": "no executable boolean custom-domain authority",
            "local_status": "contract_declarations_audited_owner_disposition_pending",
        }
        or web_row.get("contract_evidence")
        != {
            "success": "endpoint_adapter_pending",
            "validation": "dependency_pending",
            "authorization": "dependency_pending",
            "idempotency_retry": "dependency_pending",
            "failure": "dependency_pending",
        }
        or web_row.get("completion")
        != {
            "decision": "declaration_only_owner_disposition_required",
            "local_work": "complete",
            "protected_gates": ["human_approval"],
            "retirement_decision": "repository_owner_pending",
            "production_behavior": "fail_closed_unavailable",
        }
    ):
        errors.append("declaration-only web custom-domain row was silently promoted")

    for token in (
        "LEGACY_DESKTOP_ORG_CUSTOM_DOMAIN_SOURCES",
        "nullable_iso_timestamp_string",
        "nullable_boolean_declaration",
        "declaration_only_no_handler_or_boolean_derivation",
        "LEGACY_DESKTOP_ORG_CUSTOM_DOMAIN_SOURCE_MANIFEST_SHA256",
    ):
        if token not in application:
            errors.append(f"organization custom-domain application proof lost token: {token}")
    if (
        "mod legacy_org_custom_domain;" not in application_lib
        or "pub use legacy_org_custom_domain::*;" not in application_lib
    ):
        errors.append("organization custom-domain application contract is not exported")
    for token in (
        "READ_FOR_ACTOR_SQL",
        "u.active_organization_id",
        "projection_present",
        "normalize_custom_domain",
        "valid_iso_timestamp",
        "exact_json_body",
    ):
        if token not in runtime:
            errors.append(f"organization custom-domain D1 runtime lost token: {token}")
    for token in (
        "API_KEY_ACTOR_SQL",
        "desktop_api_key_selector",
        "authenticate_host_only_browser_session",
        "legacy_desktop_cors_headers",
        "preflight_response",
        '"GET, POST, PATCH, DELETE, OPTIONS"',
    ):
        if token not in web_runtime:
            errors.append(f"organization custom-domain transport lost token: {token}")
    if query_names != {"api_key_actor.sql", "read_for_actor.sql", "upsert_projection.sql"}:
        errors.append("organization custom-domain SQL closure drifted")
    for token in (
        "legacy_org_custom_domain_projection_v1",
        "domain_verified_iso",
        "source_row_digest",
        "imported_at_ms",
    ):
        if token not in migration:
            errors.append(f"organization custom-domain migration lost token: {token}")
    for token in (
        "k.key_digest = ?1",
        "k.revoked_at_ms IS NULL",
        "u.active_organization_id",
        "p.organization_id = o.id",
    ):
        if token not in query_text:
            errors.append(f"organization custom-domain query proof lost token: {token}")
    for token in ("active_key", "expired_key", "revoked_key", "inactive_key"):
        if token not in conformance:
            errors.append(f"organization custom-domain SQLite proof lost token: {token}")
    for token in (
        "mod legacy_org_custom_domain_runtime;",
        "mod legacy_org_custom_domain_web_runtime;",
        "Route::LegacyDesktopOrgCustomDomain",
        "legacy_desktop_org_custom_domain_response",
    ):
        if token not in control_lib:
            errors.append(f"organization custom-domain control-plane wiring lost token: {token}")
    for token in ("LegacyDesktopOrgCustomDomain", '"/api/desktop/org-custom-domain"'):
        if token not in routing:
            errors.append(f"organization custom-domain raw routing lost token: {token}")
    for token in (
        "DesktopOrgCustomDomainGet",
        "LEGACY_DESKTOP_ORG_CUSTOM_DOMAIN_OPERATION_ID",
        "LEGACY_DESKTOP_ORG_CUSTOM_DOMAIN_RUNTIME_SOURCES",
        "D1LegacyOrganizationCustomDomainAuthorityV1",
    ):
        if token not in compatibility_runtime:
            errors.append(f"organization custom-domain registry proof lost token: {token}")
    for token in (
        "legacy-org-custom-domain-sqlite-conformance.py",
        "org-custom-domain.json",
        "frame-application --lib legacy_org_custom_domain",
        "frame-control-plane --lib legacy_org_custom_domain",
    ):
        if token not in workflow:
            errors.append(f"organization custom-domain workflow proof lost token: {token}")
    return errors


def validate_invite_lifecycle_fixture(report: dict[str, Any]) -> list[str]:
    """Bind invite decisions to exact route shapes and one atomic D1 graph."""
    try:
        fixture = json.loads(INVITE_LIFECYCLE_FIXTURE.read_text(encoding="utf-8"))
        application = APPLICATION_INVITE_LIFECYCLE.read_text(encoding="utf-8")
        application_lib = APPLICATION_LIB.read_text(encoding="utf-8")
        runtime = CONTROL_INVITE_LIFECYCLE_RUNTIME.read_text(encoding="utf-8")
        ingress = CONTROL_INVITE_LIFECYCLE_WEB_RUNTIME.read_text(encoding="utf-8")
        migration = INVITE_LIFECYCLE_MIGRATION.read_text(encoding="utf-8")
        conformance = INVITE_LIFECYCLE_CONFORMANCE.read_text(encoding="utf-8")
        control_lib = CONTROL_LIB.read_text(encoding="utf-8")
        routing = CONTROL_ROUTING.read_text(encoding="utf-8")
        registry = CONTROL_RUNTIME.read_text(encoding="utf-8")
        workflow = parity_workflow_contract()
        query_names = {path.name for path in INVITE_LIFECYCLE_QUERY_ROOT.glob("*.sql")}
        authority_assert = (INVITE_LIFECYCLE_QUERY_ROOT / "authority_assert.sql").read_text(
            encoding="utf-8"
        )
    except (OSError, json.JSONDecodeError) as error:
        return [f"unable to load invite-lifecycle fixture: {error}"]

    errors: list[str] = []
    expected = {
        "cap-v1-447e3212d20351f6": (
            "/api/invite/accept", 11,
            "319060081ae9a039e5068c4ddd7626a320590928ae456f3c08fc9dada1525409",
        ),
        "cap-v1-cddad884de1190b1": (
            "/api/invite/decline", 9,
            "d50a659fa4d5155c15ddd3804165be3124cdb429f351ad90ed3985b7b1f4decb",
        ),
    }
    if (
        fixture.get("schema_version") != "frame.legacy-invite-lifecycle.v1"
        or fixture.get("reference_commit") != REFERENCE_COMMIT
        or fixture.get("transport", {}).get("authentication")
        != "host_only_session_before_json_parse"
        or fixture.get("transport", {}).get("idempotency")
        != "forbidden_source_has_no_client_key"
    ):
        errors.append("invite-lifecycle fixture transport or source reference drifted")
    fixture_text = json.dumps(fixture, sort_keys=True)
    for token in (
        "case-insensitive", "organizationSetup", "customDomain", "inviteTeam",
        "paid seat", "deterministic remaining organization", "retry return 404",
        "400", "401", "403", "404", "500",
    ):
        if token not in fixture_text:
            errors.append(f"invite-lifecycle semantic closure lost token: {token}")
    operations = fixture.get("operations", [])
    if not isinstance(operations, list) or {row.get("id") for row in operations} != set(expected):
        errors.append("invite-lifecycle operation set drifted")
        return errors
    report_by_id = {row.get("id"): row for row in report.get("entries", [])}
    for operation in operations:
        operation_id = operation["id"]
        identity, source_count, manifest = expected[operation_id]
        row = report_by_id.get(operation_id, {})
        if (
            operation.get("kind") != "route"
            or operation.get("method") != "POST"
            or operation.get("legacy_identity") != identity
            or operation.get("source_count") != source_count
            or operation.get("source_manifest_sha256") != manifest
            or operation.get("protected_gates") != []
            or operation.get("production_behavior") != "serve_exact_d1"
        ):
            errors.append(f"invite-lifecycle fixture contract drifted: {operation_id}")
        if (
            row.get("kind") != "route"
            or row.get("method") != "POST"
            or row.get("legacy_path") != identity
            or row.get("auth") != "session"
            or row.get("policy") != "auth_session.v1"
            or len(row.get("sources", [])) != source_count
            or canonical_json_sha256(row.get("sources", [])) != manifest
            or row.get("security", {}).get("idempotency") != "forbidden"
            or row.get("security", {}).get("rate_limit_bucket") != "auth_session.v1"
            or set(row.get("contract_evidence", {}).values()) != {"local_contract"}
            or row.get("completion", {}).get("local_work") != "complete"
            or row.get("completion", {}).get("protected_gates") != []
            or row.get("completion", {}).get("production_behavior") != "serve_exact_d1"
        ):
            errors.append(f"invite-lifecycle report five-axis evidence drifted: {operation_id}")

    for token in (
        "LEGACY_INVITE_ACCEPT_SOURCES", "LEGACY_INVITE_DECLINE_SOURCES",
        "LegacyInviteAdapterV1", "Invalid request body", "Invalid invite ID",
        "Invite not found", "Email mismatch", "Internal server error",
        'LEGACY_INVITE_RATE_LIMIT_BUCKET: &str = "auth_session.v1"',
    ):
        if token not in application:
            errors.append(f"invite-lifecycle application proof lost token: {token}")
    if (
        "mod legacy_invite_lifecycle;" not in application_lib
        or "pub use legacy_invite_lifecycle::*;" not in application_lib
    ):
        errors.append("invite-lifecycle semantics are not exported by frame-application")
    for token in (
        "D1LegacyInviteAtomicPortV1", "to_lowercase", "derived_member_nanoid",
        "ACCEPT_PRO_SEAT_UPDATE_SQL", "DECLINE_SPACE_MEMBERS_DELETE_SQL",
        "POSTCONDITION_ASSERT_SQL", ".batch(statements)",
    ):
        if token not in runtime:
            errors.append(f"invite-lifecycle D1 runtime proof lost token: {token}")
    expected_queries = {
        "snapshot.sql", "operation_insert.sql", "authority_assert.sql",
        "accept_membership_insert.sql", "accept_member_alias_insert.sql",
        "accept_pro_seat_update.sql", "accept_user_update.sql",
        "decline_space_members_delete.sql", "decline_member_alias_update.sql",
        "decline_membership_delete.sql", "decline_user_update.sql", "invite_delete.sql",
        "invite_alias_resolve.sql", "receipt_insert.sql", "audit_insert.sql",
        "operation_complete.sql", "postcondition_assert.sql", "assertion_cleanup.sql",
    }
    if query_names != expected_queries:
        errors.append("invite-lifecycle checked-in SQL closure drifted")
    for token in (
        "owner.legacy_invite_quota IS ?10",
        "owner.legacy_stripe_subscription_id IS ?11",
        "member_alias.mapped_member_id",
        "fallback.organization_id",
        "other_seat.has_pro_seat = 1",
    ):
        if token not in authority_assert:
            errors.append(f"invite-lifecycle atomic authority reassertion lost token: {token}")
    for token in (
        "legacy_invite_lifecycle_invite_aliases_v1",
        "legacy_invite_lifecycle_member_aliases_v1",
        "legacy_third_party_stripe_subscription_id",
        "legacy_invite_quota", "legacy_invite_lifecycle_receipts_v1",
    ):
        if token not in migration:
            errors.append(f"invite-lifecycle migration lost token: {token}")
    for token in (
        "authenticate_host_only_browser_session", "decode_body",
        "CompatibilityRateLimitBucketV1::AuthSession",
        "dispatch_invite_lifecycle", "error_projection",
    ):
        if token not in ingress:
            errors.append(f"invite-lifecycle HTTP ingress lost token: {token}")
    for token in (
        "LegacyInviteLifecycleInvocationV1", "LegacyRegistrationSourcesV1::InviteLifecycle",
        "LEGACY_INVITE_ACCEPT_OPERATION_ID", "LEGACY_INVITE_DECLINE_OPERATION_ID",
        "dispatch_invite_lifecycle",
    ):
        if token not in registry:
            errors.append(f"invite-lifecycle registry proof lost token: {token}")
    for token in (
        "mod legacy_invite_lifecycle_runtime;", "mod legacy_invite_lifecycle_web_runtime;",
        "Route::LegacyInviteAccept", "Route::LegacyInviteDecline",
    ):
        if token not in control_lib:
            errors.append(f"invite-lifecycle route wiring lost token: {token}")
    for token in ("LegacyInviteAccept", "LegacyInviteDecline", '"/api/invite/accept"', '"/api/invite/decline"'):
        if token not in routing:
            errors.append(f"invite-lifecycle raw route classification lost token: {token}")
    for token in (
        ".lower()", "membership_removed", "pro_seat_assigned", "foreign_key_check",
        "stale authority accepted", "case-only alias mutation accepted",
    ):
        if token not in conformance:
            errors.append(f"invite-lifecycle SQLite proof lost token: {token}")
    for token in (
        "legacy-invite-lifecycle-sqlite-conformance.py", "invite-lifecycle.json",
        "frame-application --lib legacy_invite_lifecycle",
        "frame-control-plane --lib legacy_invite_lifecycle",
    ):
        if token not in workflow:
            errors.append(f"invite-lifecycle workflow proof lost token: {token}")
    return errors


def validate_developer_api_fixture(report: dict[str, Any]) -> list[str]:
    errors: list[str] = []
    try:
        fixture = json.loads(DEVELOPER_API_FIXTURE.read_text(encoding="utf-8"))
        application = APPLICATION_DEVELOPER_API.read_text(encoding="utf-8")
        runtime = CONTROL_DEVELOPER_API_RUNTIME.read_text(encoding="utf-8")
        ingress = CONTROL_DEVELOPER_API_WEB_RUNTIME.read_text(encoding="utf-8")
        migration = DEVELOPER_API_MIGRATION.read_text(encoding="utf-8")
        conformance = DEVELOPER_API_CONFORMANCE.read_text(encoding="utf-8")
        registry = CONTROL_RUNTIME.read_text(encoding="utf-8")
        routing = CONTROL_ROUTING.read_text(encoding="utf-8")
        query_names = {path.name for path in DEVELOPER_API_QUERY_ROOT.glob("*.sql")}
    except (OSError, json.JSONDecodeError) as error:
        return [f"developer API fixture could not be loaded: {error}"]

    expected_ids = list(DEVELOPER_API_LOCAL_ENDPOINT_ADAPTERS.values())
    expected_id_set = {adapter["id"] for adapter in expected_ids}
    operations = fixture.get("operations")
    if not isinstance(operations, list):
        return ["developer API fixture operations must be a list"]
    fixture_by_id = {
        operation.get("id"): operation
        for operation in operations
        if isinstance(operation, dict) and isinstance(operation.get("id"), str)
    }
    if set(fixture_by_id) != expected_id_set:
        errors.append("developer API fixture identity set drifted")

    report_by_id = {row["id"]: row for row in report.get("entries", [])}
    for identity, adapter in DEVELOPER_API_LOCAL_ENDPOINT_ADAPTERS.items():
        operation_id = adapter["id"]
        operation = fixture_by_id.get(operation_id)
        row = report_by_id.get(operation_id)
        if operation is None or row is None:
            errors.append(f"developer API operation missing from fixture/report: {operation_id}")
            continue
        kind, method, path = identity
        if (
            kind != "route"
            or operation.get("method") != method
            or operation.get("path") != path
            or operation.get("auth") != adapter["auth"]
            or operation.get("rate_limit_bucket") != adapter["policy"]
            or operation.get("source_count") != len(row.get("sources", []))
            or row.get("contract_evidence", {}).get("success") != "local_contract"
            or row.get("completion", {}).get("local_work") != "complete"
            or row.get("completion", {}).get("protected_gates") != []
            or row.get("completion", {}).get("production_behavior") != "serve_exact_d1"
        ):
            errors.append(f"developer API fixture/report contract drifted: {operation_id}")
        security = row.get("security", {})
        if any(
            operation.get(field) != security.get(field)
            for field in (
                "max_body_bytes",
                "accepted_content_types",
                "idempotency",
                "rate_limit_bucket",
            )
        ):
            errors.append(f"developer API security contract drifted: {operation_id}")

    expected_aliases = {
        "cap-v1-5914c280b14ba739": "cap-v1-5914aa6459d24ff1",
        "cap-v1-5c98d28b577890ab": "cap-v1-5c98b9755e4643ba",
        "cap-v1-0d39abfe8b33690b": "cap-v1-0d3940728bc19e0e",
        "cap-v1-b6fe0d399f94c289": "cap-v1-b6fe5aec600a2e1a",
        "cap-v1-c904df94374ab384": "cap-v1-c904ef9c11983a40",
        "cap-v1-cbf029d42a0b86a1": "cap-v1-cbf22d62a64d3486",
        "cap-v1-6e227329e3b4d880": "cap-v1-6e2296f9695261a3",
        "cap-v1-1cbe6087c0d0fd0e": "cap-v1-1cbfe3ecac36f198",
        "cap-v1-aed8207796eecfc4": "cap-v1-aed411f91e977fe5",
        "cap-v1-71839c3bb617dffe": "cap-v1-718e84b39180c0ac",
    }
    if fixture.get("corrected_issue_aliases") != expected_aliases:
        errors.append("developer API corrected issue aliases drifted")
    if fixture.get("completion") != {
        "decision": "serve_frame_exact_business",
        "local_work": "complete",
        "protected_gates": [],
        "production_behavior": "serve_exact_d1",
    }:
        errors.append("developer API completion disposition drifted")

    required_queries = {
        "auth_key.sql",
        "auth_origin.sql",
        "usage_read.sql",
        "videos_list.sql",
        "video_read.sql",
        "operation_read.sql",
        "operation_claim.sql",
        "operation_effect_pending.sql",
        "operation_complete.sql",
        "receipt_insert.sql",
        "audit_insert.sql",
        "video_create.sql",
        "video_delete.sql",
        "multipart_session_insert.sql",
        "multipart_session_read.sql",
        "multipart_state.sql",
        "credit_debit.sql",
        "video_complete.sql",
        "part_capability_insert.sql",
        "outbox_insert.sql",
        "outbox_attempt.sql",
        "outbox_complete.sql",
        "cron_candidates.sql",
        "cron_snapshot_insert.sql",
        "cron_run_read.sql",
        "cron_run_insert.sql",
    }
    if not required_queries.issubset(query_names):
        errors.append("developer API checked-in D1 query set is incomplete")
    required_tokens = {
        "application": [
            "LEGACY_DEVELOPER_MULTIPART_SOURCES",
            "LEGACY_DEVELOPER_STORAGE_CRON_SOURCES",
            "duration_seconds > 0.0",
            "utf16_len",
        ],
        "runtime": [
            "OUTBOX_INSERT_SQL",
            "sign_legacy_multipart_part",
            "2_500_000.0",
            "LEGACY_DEVELOPER_STORAGE_RATE_NUMERATOR",
            "effect_pending",
        ],
        "ingress": [
            "cpk_",
            "csk_",
            "Origin header required for production apps",
            "CRON_SECRET",
            "dispatch_developer_api",
        ],
        "migration": [
            "legacy_developer_api_operations_v1",
            "legacy_developer_provider_outbox_v1",
            "legacy_developer_credit_transactions_v1",
            "legacy_developer_daily_storage_snapshots_v1",
            "frame_legacy_developer_insufficient_credits_v1",
        ],
        "registry": [
            "LegacyRegistrationSourcesV1::DeveloperApi",
            "dispatch_developer_api",
        ],
        "routing": [
            "LegacyDeveloperStorageCron",
            "LegacyDeveloperMultipartComplete",
            "LegacyDeveloperVideoStatus",
        ],
        "conformance": [
            "completion_size_floor_bytes_per_second",
            "cron_candidates",
            "frame_legacy_developer_insufficient_credits_v1",
        ],
    }
    sources = {
        "application": application,
        "runtime": runtime,
        "ingress": ingress,
        "migration": migration,
        "registry": registry,
        "routing": routing,
        "conformance": conformance,
    }
    for label, tokens in required_tokens.items():
        for token in tokens:
            if token not in sources[label]:
                errors.append(f"developer API {label} lost semantic token: {token}")
    return errors


def validate_transcripts_fixture(report: dict[str, Any]) -> list[str]:
    """Bind transcript retry/read/edit/list/translation to D1/R2 and provider evidence."""
    try:
        fixture = json.loads(TRANSCRIPTS_FIXTURE.read_text(encoding="utf-8"))
        application = APPLICATION_TRANSCRIPTS.read_text(encoding="utf-8")
        application_lib = APPLICATION_LIB.read_text(encoding="utf-8")
        runtime = CONTROL_TRANSCRIPTS_RUNTIME.read_text(encoding="utf-8")
        ingress = CONTROL_TRANSCRIPTS_WEB_RUNTIME.read_text(encoding="utf-8")
        migration = TRANSCRIPTS_MIGRATION.read_text(encoding="utf-8")
        registry = CONTROL_RUNTIME.read_text(encoding="utf-8")
        routing = CONTROL_ROUTING.read_text(encoding="utf-8")
        control = CONTROL_LIB.read_text(encoding="utf-8")
        conformance = TRANSCRIPTS_CONFORMANCE.read_text(encoding="utf-8")
        workflow = parity_workflow_contract()
        query_names = {path.name for path in TRANSCRIPTS_QUERY_ROOT.glob("*.sql")}
        query_text = "\n".join(
            path.read_text(encoding="utf-8")
            for path in TRANSCRIPTS_QUERY_ROOT.glob("*.sql")
        )
    except (OSError, json.JSONDecodeError) as error:
        return [f"unable to load transcript evidence: {error}"]

    errors: list[str] = []
    expected_operations = {
        "cap-v1-c8dffb9b102dd4f7": {
            "kind": "route",
            "method": "POST",
            "identity": "/api/videos/:videoId/retry-transcription",
            "auth": "session",
            "idempotency": "required",
            "max_body_bytes": 0,
            "accepted_content_types": [],
            "tenant_non_disclosure": False,
            "local_status": "rust_exact_transcription_retry_d1_local_contract",
            "completion": {
                "decision": "serve_frame_exact_business",
                "local_work": "complete",
                "protected_gates": [],
                "retirement_decision": "not_proposed",
                "production_behavior": "serve_exact_d1",
            },
        },
        "cap-v1-3db394ae13895b46": {
            "kind": "server_action",
            "method": "ACTION",
            "identity": "action://apps/web/actions/videos/edit-transcript.ts#editTranscriptEntry",
            "auth": "session",
            "idempotency": "required",
            "max_body_bytes": 256 * 1024,
            "accepted_content_types": ["application/json"],
            "tenant_non_disclosure": False,
            "local_status": "rust_exact_transcript_edit_d1_r2_local_contract",
            "completion": {
                "decision": "serve_frame_exact_business",
                "local_work": "complete",
                "protected_gates": [],
                "retirement_decision": "not_proposed",
                "production_behavior": "serve_exact_d1_r2",
            },
        },
        "cap-v1-f2659b43d5ee9162": {
            "kind": "server_action",
            "method": "ACTION",
            "identity": "action://apps/web/actions/videos/get-transcript.ts#getTranscript",
            "auth": "optional_session_or_share_capability",
            "idempotency": "forbidden",
            "max_body_bytes": 256 * 1024,
            "accepted_content_types": ["application/json"],
            "tenant_non_disclosure": True,
            "local_status": "rust_exact_transcript_read_d1_r2_local_contract",
            "completion": {
                "decision": "serve_frame_exact_business",
                "local_work": "complete",
                "protected_gates": [],
                "retirement_decision": "not_proposed",
                "production_behavior": "serve_exact_d1_r2",
            },
        },
        "cap-v1-6f6ece85bd786289": {
            "kind": "server_action",
            "method": "ACTION",
            "identity": "action://apps/web/actions/videos/translate-transcript.ts#translateTranscript",
            "auth": "optional_session_or_share_capability",
            "idempotency": "required",
            "max_body_bytes": 256 * 1024,
            "accepted_content_types": ["application/json"],
            "tenant_non_disclosure": True,
            "local_status": "rust_exact_transcript_translation_d1_r2_provider_outbox_local_contract",
            "completion": {
                "decision": "retain_replace_with_provider_effect",
                "local_work": "complete",
                "protected_gates": ["provider_execution"],
                "retirement_decision": "not_proposed",
                "production_behavior": "fail_closed_unavailable",
            },
        },
        "cap-v1-6c82f3cbe383d92b": {
            "kind": "server_action",
            "method": "ACTION",
            "identity": "action://apps/web/actions/videos/get-available-translations.ts#getAvailableTranslations",
            "policy": "video_media.v1",
            "auth": "optional_session_or_share_capability",
            "idempotency": "forbidden",
            "max_body_bytes": 256 * 1024,
            "accepted_content_types": ["application/json"],
            "tenant_non_disclosure": True,
            "local_status": "rust_exact_available_translations_d1_r2_local_contract",
            "completion": {
                "decision": "serve_frame_exact_business",
                "local_work": "complete",
                "protected_gates": [],
                "retirement_decision": "not_proposed",
                "production_behavior": "serve_exact_d1_r2",
            },
        },
    }
    fixture_operations = {
        item.get("id"): item
        for item in fixture.get("operations", [])
        if isinstance(item, dict)
    }
    if (
        fixture.get("schema_version") != "frame.legacy-transcripts.v1"
        or fixture.get("reference_commit") != REFERENCE_COMMIT
        or fixture.get("carrier_schema")
        != "frame.web-transcript-action-request.v1"
        or set(fixture_operations) != set(expected_operations)
    ):
        errors.append("transcript fixture identity drifted")

    report_entries = {
        row.get("id"): row
        for row in report.get("entries", [])
        if isinstance(row, dict)
    }
    for operation_id, expected in expected_operations.items():
        item = fixture_operations.get(operation_id, {})
        row = report_entries.get(operation_id, {})
        expected_policy = expected.get(
            "policy", "collaboration_notifications.v1"
        )
        expected_sources = {
            (source.get("path"), source.get("symbol"), source.get("sha256"))
            for source in item.get("sources", [])
            if isinstance(source, dict)
        }
        report_sources = {
            (source.get("path"), source.get("symbol"), source.get("sha256"))
            for source in row.get("sources", [])
            if isinstance(source, dict)
        }
        expected_security = {
            "max_body_bytes": expected["max_body_bytes"],
            "accepted_content_types": expected["accepted_content_types"],
            "rate_limit_bucket": expected_policy,
            "idempotency": expected["idempotency"],
            "tenant_non_disclosure": expected["tenant_non_disclosure"],
        }
        if (
            item.get("kind") != expected["kind"]
            or item.get("method") != expected["method"]
            or item.get("legacy_identity") != expected["identity"]
            or item.get("policy") != expected_policy
            or row.get("kind") != expected["kind"]
            or row.get("method") != expected["method"]
            or row.get("legacy_path") != expected["identity"]
            or row.get("clients") != ["web"]
            or row.get("auth") != expected["auth"]
            or row.get("policy") != expected_policy
            or row.get("implementation", {}).get("local_status")
            != expected["local_status"]
            or row.get("security") != expected_security
            or set(row.get("contract_evidence", {}).values()) != {"local_contract"}
            or row.get("completion") != expected["completion"]
            or report_sources != expected_sources
            or not re.fullmatch(r"[0-9a-f]{64}", item.get("source_manifest_sha256", ""))
            or item.get("source_manifest_sha256") not in application
        ):
            errors.append(f"transcript report or source evidence drifted: {operation_id}")

    fixture_text = json.dumps(fixture, sort_keys=True)
    for token in (
        "ECMAScript whitespace collapse+trim",
        "optional auth/public policy",
        "required inference corrected to forbidden for a read",
        "one of 26 pinned codes",
        "D1 operation plus translation outbox is durable and restart-safe",
        "no provider result is fabricated",
        "preserve provider order and duplicates",
    ):
        if token not in fixture_text:
            errors.append(f"transcript fixture lost semantic token: {token}")
    for token in (
        "LEGACY_RETRY_TRANSCRIPTION_SOURCES",
        "LEGACY_EDIT_TRANSCRIPT_SOURCES",
        "LEGACY_GET_TRANSCRIPT_SOURCES",
        "LEGACY_TRANSLATE_TRANSCRIPT_SOURCES",
        "LEGACY_AVAILABLE_TRANSLATIONS_SOURCES",
        "legacy_available_translations_from_keys",
        "legacy_update_vtt_entry_text",
        "legacy_transcript_language_name",
        "legacy_transcript_object_key",
        "LEGACY_TRANSCRIPT_TRANSLATION_PROTECTED_GATES",
    ):
        if token not in application:
            errors.append(f"transcript application proof lost token: {token}")
    if (
        "mod legacy_transcripts;" not in application_lib
        or "pub use legacy_transcripts::*;" not in application_lib
    ):
        errors.append("transcript contract is not exported by frame-application")

    expected_queries = {
        "actor_email.sql",
        "explicit_access.sql",
        "operation_by_key.sql",
        "operation_complete.sql",
        "operation_insert.sql",
        "operation_provider_pending.sql",
        "operation_storage_applied.sql",
        "password_candidates.sql",
        "retry_status_reset.sql",
        "storage_receipt_insert.sql",
        "translation_outbox_insert.sql",
        "video_authority.sql",
    }
    if query_names != expected_queries:
        errors.append("transcript checked SQL closure drifted")
    for token in (
        "legacy_transcript_operations_v1",
        "legacy_transcript_storage_receipts_v1",
        "legacy_transcript_translation_outbox_v1",
        "frame_legacy_transcript_receipt_immutable_v1",
        "legacy_allowed_email_restriction",
    ):
        if token not in migration:
            errors.append(f"transcript migration proof lost token: {token}")
    for token in (
        "state = 'provider_pending'",
        "state = 'storage_applied'",
        "transcription_status = NULL",
        "legacy_password_hash",
        "legacy_allowed_email_restriction",
    ):
        if token not in query_text:
            errors.append(f"transcript checked SQL lost token: {token}")
    for token in (
        "D1LegacyTranscriptAuthorityV1",
        "can_view",
        "claim_operation",
        "mark_storage_applied",
        "queue_translation",
        "reset_retry_status",
    ):
        if token not in runtime:
            errors.append(f"transcript D1 runtime lost token: {token}")
    for token in (
        "frame.web-transcript-action-request.v1",
        "authenticate_host_only_browser_session",
        "optional_actor",
        "CompatibilityRateLimitBucketV1::CollaborationNotifications",
        "CompatibilityRateLimitBucketV1::VideoMedia",
        "legacy_update_vtt_entry_text",
        "legacy_available_translations_from_keys",
        "Translation failed",
        "GROQ_API_KEY",
    ):
        if token not in ingress:
            errors.append(f"transcript HTTP/R2 carrier lost token: {token}")
    for token in (
        "LEGACY_RETRY_TRANSCRIPTION_OPERATION_ID",
        "LEGACY_EDIT_TRANSCRIPT_OPERATION_ID",
        "LEGACY_GET_TRANSCRIPT_OPERATION_ID",
        "LEGACY_TRANSLATE_TRANSCRIPT_OPERATION_ID",
        "LEGACY_AVAILABLE_TRANSLATIONS_OPERATION_ID",
        "Transcripts",
    ):
        if token not in registry:
            errors.append(f"transcript central registry lost token: {token}")
    for token in ("LegacyRetryTranscription", "legacy_transcripts_runtime"):
        if token not in routing and token not in control:
            errors.append(f"transcript route wiring lost token: {token}")
    for token in (
        "Provider-free SQLite proof",
        "frame_legacy_transcript_receipt_immutable_v1",
        "translation_outbox_insert.sql",
        "retry_status_reset.sql",
    ):
        if token not in conformance:
            errors.append(f"transcript SQLite proof lost token: {token}")
    for token in (
        "legacy-transcripts-sqlite-conformance.py",
        "transcripts.json",
        "frame-application --lib legacy_transcripts",
        "frame-control-plane --lib legacy_transcripts",
    ):
        if token not in workflow:
            errors.append(f"transcript workflow proof lost token: {token}")
    return errors


def validate_mobile_bootstrap_caps_fixture(report: dict[str, Any]) -> list[str]:
    """Bind six mobile bootstrap/cap routes to owner-scoped D1 and private R2 evidence."""
    try:
        fixture = json.loads(MOBILE_BOOTSTRAP_CAPS_FIXTURE.read_text(encoding="utf-8"))
        application = APPLICATION_MOBILE_BOOTSTRAP_CAPS.read_text(encoding="utf-8")
        application_lib = APPLICATION_LIB.read_text(encoding="utf-8")
        runtime = CONTROL_MOBILE_BOOTSTRAP_CAPS_RUNTIME.read_text(encoding="utf-8")
        ingress = CONTROL_MOBILE_BOOTSTRAP_CAPS_WEB_RUNTIME.read_text(encoding="utf-8")
        migration = MOBILE_BOOTSTRAP_CAPS_MIGRATION.read_text(encoding="utf-8")
        registry = CONTROL_RUNTIME.read_text(encoding="utf-8")
        routing = CONTROL_ROUTING.read_text(encoding="utf-8")
        control = CONTROL_LIB.read_text(encoding="utf-8")
        conformance = MOBILE_BOOTSTRAP_CAPS_CONFORMANCE.read_text(encoding="utf-8")
        workflow = parity_workflow_contract()
        query_names = {
            path.name for path in MOBILE_BOOTSTRAP_CAPS_QUERY_ROOT.glob("*.sql")
        }
        query_text = "\n".join(
            path.read_text(encoding="utf-8")
            for path in MOBILE_BOOTSTRAP_CAPS_QUERY_ROOT.glob("*.sql")
        )
    except (OSError, json.JSONDecodeError) as error:
        return [f"unable to load mobile bootstrap/caps evidence: {error}"]

    errors: list[str] = []
    expected_operations = {
        "cap-v1-32a24fe16a4c4a4f": ("GET", "/api/mobile/bootstrap", MOBILE_BOOTSTRAP_RUNTIME_SOURCES, "rust_exact_mobile_bootstrap_d1_r2_local_contract"),
        "cap-v1-951ad1523ae9dff4": ("GET", "/api/mobile/caps", MOBILE_CAPS_LIST_RUNTIME_SOURCES, "rust_exact_mobile_caps_list_d1_r2_local_contract"),
        "cap-v1-6b8a689bf00a9187": ("DELETE", "/api/mobile/caps/:id", MOBILE_CAP_DELETE_RUNTIME_SOURCES, "rust_exact_mobile_cap_delete_d1_r2_local_contract"),
        "cap-v1-7f0ed5caf3eaf97c": ("GET", "/api/mobile/caps/:id", MOBILE_CAP_GET_RUNTIME_SOURCES, "rust_exact_mobile_cap_detail_d1_r2_local_contract"),
        "cap-v1-95fe41c72ce5ca9f": ("GET", "/api/mobile/caps/:id/download", MOBILE_CAP_DOWNLOAD_RUNTIME_SOURCES, "rust_exact_mobile_cap_download_d1_r2_local_contract"),
        "cap-v1-bde34617e42a8834": ("GET", "/api/mobile/caps/:id/playback", MOBILE_CAP_PLAYBACK_RUNTIME_SOURCES, "rust_exact_mobile_cap_playback_d1_r2_local_contract"),
    }
    fixture_operations = {
        item.get("id"): (item.get("method"), item.get("path"))
        for item in fixture.get("operations", [])
        if isinstance(item, dict)
    }
    if (
        fixture.get("schema_version") != "frame.legacy-mobile-bootstrap-caps.v1"
        or fixture.get("reference_commit") != REFERENCE_COMMIT
        or fixture_operations
        != {
            operation_id: (method, path)
            for operation_id, (method, path, _, _) in expected_operations.items()
        }
    ):
        errors.append("mobile bootstrap/caps fixture identity drifted")

    fixture_counts = fixture.get("source_counts", {})
    fixture_manifests = fixture.get("source_manifest_sha256", {})
    for operation_id, (method, path, extra_sources, local_status) in expected_operations.items():
        row = next(
            (entry for entry in report.get("entries", []) if entry.get("id") == operation_id),
            {},
        )
        expected_count = 1 + len(extra_sources)
        if (
            row.get("kind") != "route"
            or row.get("method") != method
            or row.get("legacy_path") != path
            or row.get("clients") != ["mobile"]
            or row.get("auth") != "session_or_api_key"
            or row.get("policy") != "client_compatibility.v1"
            or row.get("implementation", {}).get("local_status") != local_status
            or set(row.get("contract_evidence", {}).values()) != {"local_contract"}
            or row.get("security")
            != {
                "max_body_bytes": 0,
                "accepted_content_types": [],
                "rate_limit_bucket": "client_compatibility.v1",
                "idempotency": "forbidden",
                "tenant_non_disclosure": True,
            }
            or row.get("completion") != MOBILE_BOOTSTRAP_CAPS_COMPLETION
            or len(row.get("sources", [])) != expected_count
            or fixture_counts.get(operation_id) != expected_count
            or fixture_manifests.get(operation_id)
            != canonical_json_sha256(row.get("sources", []))
        ):
            errors.append(f"mobile bootstrap/caps report evidence drifted: {operation_id}")

    semantic_text = json.dumps(fixture, sort_keys=True)
    for token in (
        "second segment of exactly 36 bytes",
        "foreign or absent target as 404",
        "Math.trunc",
        "Range is intentionally not signed",
        "result.mp4",
        "transcription.vtt",
        "D1 deletion/tombstone commits before",
    ):
        if token not in semantic_text:
            errors.append(f"mobile bootstrap/caps fixture lost semantic token: {token}")

    for token in (
        "LEGACY_MOBILE_BOOTSTRAP_SOURCES",
        "LEGACY_MOBILE_CAPS_LIST_SOURCES",
        "LEGACY_MOBILE_CAP_DELETE_SOURCES",
        "LEGACY_MOBILE_CAP_GET_SOURCES",
        "LEGACY_MOBILE_CAP_DOWNLOAD_SOURCES",
        "LEGACY_MOBILE_CAP_PLAYBACK_SOURCES",
        "legacy_mobile_positive_integer",
        "legacy_mobile_screenshot_key",
        "LegacyMobileVideoSourceV1",
    ):
        if token not in application:
            errors.append(f"mobile bootstrap/caps application proof lost token: {token}")
    if (
        "mod legacy_mobile_bootstrap_caps;" not in application_lib
        or "pub use legacy_mobile_bootstrap_caps::*;" not in application_lib
    ):
        errors.append("mobile bootstrap/caps contract is not exported by frame-application")

    expected_queries = {
        "actor_profile.sql", "organizations.sql", "root_folders.sql",
        "caps_count.sql", "caps_rows.sql", "cap_row.sql", "comments.sql",
        "delete_snapshot.sql", "delete_apply.sql", "delete_operation_insert.sql",
        "delete_audit_insert.sql", "delete_assert.sql", "delete_complete.sql",
        "delete_cleanup_assert.sql",
    }
    if query_names != expected_queries:
        errors.append("mobile bootstrap/caps checked SQL closure drifted")
    for token in (
        "video.owner_id = ?1",
        "video.organization_id = ?2",
        "legacy_effective_created_at_us DESC",
        "legacy_collaboration_comments_v1",
        "state = 'deleted'",
        "'storage_pending'",
        "'complete'",
    ):
        if token not in query_text:
            errors.append(f"mobile bootstrap/caps checked SQL lost token: {token}")
    for token in (
        "legacy_mobile_cap_media_v1",
        "legacy_mobile_cap_uploads_v1",
        "legacy_mobile_cap_delete_operations_v1",
        "legacy_mobile_cap_delete_assertion_guard_v1",
        "legacy_mobile_cap_delete_assertion_no_update_v1",
        "legacy_mobile_cap_media_alias_insert_v1",
        "legacy_mobile_cap_upload_native_update_v1",
    ):
        if token not in migration:
            errors.append(f"mobile bootstrap/caps migration proof lost token: {token}")
    for token in (
        "D1LegacyMobileBootstrapCapsV1",
        "begin_delete",
        "complete_delete",
        "LegacyMobileBootstrapCapsRuntimeFailureV1::NotFound",
        "valid_prefix",
    ):
        if token not in runtime:
            errors.append(f"mobile bootstrap/caps D1 runtime lost token: {token}")
    for token in (
        "legacy_mobile_middleware_api_key",
        "authenticate_host_only_browser_session",
        "CompatibilityRateLimitBucketV1::ClientCompatibility",
        "X-Amz-SignedHeaders",
        "UNSIGNED-PAYLOAD",
        "delete_multiple",
        "thumbnail_best_effort",
        "object.size() > 0",
        "request.bytes()",
        "TooManyRequests",
    ):
        if token not in ingress:
            errors.append(f"mobile bootstrap/caps HTTP/R2 carrier lost token: {token}")
    for token in (
        "LEGACY_MOBILE_BOOTSTRAP_OPERATION_ID",
        "LEGACY_MOBILE_CAPS_LIST_OPERATION_ID",
        "LEGACY_MOBILE_CAP_DELETE_OPERATION_ID",
        "LEGACY_MOBILE_CAP_GET_OPERATION_ID",
        "LEGACY_MOBILE_CAP_DOWNLOAD_OPERATION_ID",
        "LEGACY_MOBILE_CAP_PLAYBACK_OPERATION_ID",
        "MobileBootstrapCaps",
    ):
        if token not in registry:
            errors.append(f"mobile bootstrap/caps central registry lost token: {token}")
    for token in (
        "LegacyMobileBootstrap", "LegacyMobileCaps", "LegacyMobileCapDownload",
        "LegacyMobileCapPlayback",
    ):
        if token not in routing or token not in control:
            errors.append(f"mobile bootstrap/caps route wiring lost token: {token}")
    for token in (
        "Owner-scoped list/detail projection",
        "Delete commits D1 before provider cleanup",
        "Tenant non-disclosure",
        "Upload/media dual-write triggers",
    ):
        if token not in conformance:
            errors.append(f"mobile bootstrap/caps SQLite proof lost token: {token}")
    for token in (
        "legacy-mobile-bootstrap-caps-sqlite-conformance.py",
        "mobile-bootstrap-caps.json",
        "frame-application --lib legacy_mobile_bootstrap_caps",
        "frame-control-plane --lib legacy_mobile_bootstrap_caps",
    ):
        if token not in workflow:
            errors.append(f"mobile bootstrap/caps workflow proof lost token: {token}")
    return errors


def validate_mobile_uploads_fixture(report: dict[str, Any]) -> list[str]:
    """Bind released mobile create/progress/complete to exact D1/R2 evidence."""
    try:
        fixture = json.loads(MOBILE_UPLOADS_FIXTURE.read_text(encoding="utf-8"))
        application = APPLICATION_MOBILE_UPLOADS.read_text(encoding="utf-8")
        application_lib = APPLICATION_LIB.read_text(encoding="utf-8")
        runtime = CONTROL_MOBILE_UPLOADS_RUNTIME.read_text(encoding="utf-8")
        ingress = CONTROL_MOBILE_UPLOADS_WEB_RUNTIME.read_text(encoding="utf-8")
        migration = MOBILE_UPLOADS_MIGRATION.read_text(encoding="utf-8")
        registry = CONTROL_RUNTIME.read_text(encoding="utf-8")
        routing = CONTROL_ROUTING.read_text(encoding="utf-8")
        control = CONTROL_LIB.read_text(encoding="utf-8")
        conformance = MOBILE_UPLOADS_CONFORMANCE.read_text(encoding="utf-8")
        workflow = parity_workflow_contract()
        query_names = {path.name for path in MOBILE_UPLOADS_QUERY_ROOT.glob("*.sql")}
        query_text = "\n".join(
            path.read_text(encoding="utf-8")
            for path in MOBILE_UPLOADS_QUERY_ROOT.glob("*.sql")
        )
    except (OSError, json.JSONDecodeError) as error:
        return [f"unable to load mobile-upload evidence: {error}"]

    errors: list[str] = []
    expected_operations = {
        "cap-v1-b0116dd82b010477": {
            "path": "/api/mobile/uploads",
            "sources": MOBILE_UPLOAD_CREATE_RUNTIME_SOURCES,
            "source_count": 16,
            "manifest": "de3bf22950e46dfdbe9bc54bc020888fa5f11398353b2c0890883ebfd7ee869c",
            "local_status": "rust_exact_mobile_upload_create_d1_r2_local_contract",
            "completion": {
                "decision": "serve_frame_exact_business",
                "local_work": "complete",
                "protected_gates": [],
                "retirement_decision": "not_proposed",
                "production_behavior": "serve_exact_d1_r2",
            },
        },
        "cap-v1-b43b6ede64a73798": {
            "path": "/api/mobile/uploads/:id/complete",
            "sources": MOBILE_UPLOAD_COMPLETE_RUNTIME_SOURCES,
            "source_count": 12,
            "manifest": "08cb691aed6602219569e8c37977fe21b02b3f66dadf561d166916d8e84dfa04",
            "local_status": "rust_exact_mobile_upload_complete_d1_r2_provider_intent_local_contract",
            "completion": {
                "decision": "retain_replace_with_provider_effect",
                "local_work": "complete",
                "protected_gates": ["provider_execution"],
                "retirement_decision": "not_proposed",
                "production_behavior": "fail_closed_unavailable",
            },
        },
        "cap-v1-62469fe03e030052": {
            "path": "/api/mobile/uploads/:id/progress",
            "sources": MOBILE_UPLOAD_PROGRESS_RUNTIME_SOURCES,
            "source_count": 12,
            "manifest": "29dd22432c3807153944b701e72c0aee34481ecd8ed1bf9c46577c88ca9699da",
            "local_status": "rust_exact_mobile_upload_progress_d1_local_contract",
            "completion": {
                "decision": "serve_frame_exact_business",
                "local_work": "complete",
                "protected_gates": [],
                "retirement_decision": "not_proposed",
                "production_behavior": "serve_exact_d1",
            },
        },
    }
    fixture_operations = {
        item.get("id"): item
        for item in fixture.get("operations", [])
        if isinstance(item, dict)
    }
    if (
        fixture.get("schema_version") != "frame.legacy-mobile-uploads.v1"
        or fixture.get("reference_commit") != REFERENCE_COMMIT
        or set(fixture_operations) != set(expected_operations)
    ):
        errors.append("mobile-upload fixture identity or reference drifted")

    report_by_id = {row.get("id"): row for row in report.get("entries", [])}
    handler_source = (
        "apps/web/app/api/mobile/[...route]/route.ts",
        "02df2ce92dc6e8ae11748b6e082c1304596ba9e4c370b35069867754218f5f79",
    )
    declaration_source = (
        "packages/web-domain/src/Mobile.ts",
        "331d76900372d62389d729f8682baca1344f3583e3f41f42ad6e3ef2be7a3d5b",
    )
    for operation_id, expected in expected_operations.items():
        item = fixture_operations.get(operation_id, {})
        row = report_by_id.get(operation_id, {})
        expected_source_pairs = {
            handler_source,
            declaration_source,
            *(
                (source["path"], source["sha256"])
                for source in expected["sources"]
            ),
        }
        report_source_pairs = {
            (source.get("path"), source.get("sha256"))
            for source in row.get("sources", [])
            if isinstance(source, dict)
        }
        if (
            item.get("method") != "POST"
            or item.get("path") != expected["path"]
            or item.get("source_count") != expected["source_count"]
            or item.get("source_manifest_sha256") != expected["manifest"]
            or expected["manifest"] not in application
            or row.get("kind") != "route"
            or row.get("method") != "POST"
            or row.get("legacy_path") != expected["path"]
            or row.get("clients") != ["mobile"]
            or row.get("auth") != "session_or_api_key"
            or row.get("policy") != "upload_storage.v1"
            or row.get("implementation", {}).get("local_status")
            != expected["local_status"]
            or row.get("security")
            != {
                "max_body_bytes": 8 * 1024 * 1024,
                "accepted_content_types": ["application/json"],
                "rate_limit_bucket": "upload_storage.v1",
                "idempotency": "forbidden",
                "tenant_non_disclosure": True,
            }
            or set(row.get("contract_evidence", {}).values()) != {"local_contract"}
            or row.get("completion") != expected["completion"]
            or len(row.get("sources", [])) != expected["source_count"]
            or report_source_pairs != expected_source_pairs
        ):
            errors.append(f"mobile-upload report or source evidence drifted: {operation_id}")

    semantic_text = json.dumps(fixture, sort_keys=True)
    for token in (
        "second segment of exactly 36 bytes",
        "Math.max(0, Math.trunc(value))",
        "must equal the exact minted rawFileKey",
        "HEAD must observe the exact nonempty object",
        "If-None-Match: *",
        "provider_execution",
        "does not create a fake media job or workflow receipt",
    ):
        if token not in semantic_text:
            errors.append(f"mobile-upload fixture lost semantic token: {token}")

    for token in (
        "LEGACY_MOBILE_UPLOAD_CREATE_SOURCES",
        "LEGACY_MOBILE_UPLOAD_PROGRESS_SOURCES",
        "LEGACY_MOBILE_UPLOAD_COMPLETE_SOURCES",
        "legacy_mobile_upload_raw_key",
        "legacy_mobile_upload_source_manifest",
        "LEGACY_MOBILE_UPLOADS_PROVIDER_GATES",
    ):
        if token not in application:
            errors.append(f"mobile-upload application proof lost token: {token}")
    if (
        "mod legacy_mobile_uploads;" not in application_lib
        or "pub use legacy_mobile_uploads::*;" not in application_lib
    ):
        errors.append("mobile-upload contract is not exported by frame-application")

    expected_queries = {
        "assertion_cleanup.sql",
        "complete_authority_assert.sql",
        "complete_intent_insert.sql",
        "complete_pending_assert.sql",
        "complete_record_pending.sql",
        "complete_snapshot.sql",
        "complete_upload_bytes.sql",
        "create_alias_insert.sql",
        "create_authority.sql",
        "create_authority_assert.sql",
        "create_postcondition_assert.sql",
        "create_record_insert.sql",
        "create_upload_insert.sql",
        "create_video_insert.sql",
        "operation_insert.sql",
        "progress_authority_assert.sql",
        "progress_postcondition_assert.sql",
        "progress_snapshot.sql",
        "progress_update.sql",
        "progress_upload_insert.sql",
    }
    if query_names != expected_queries:
        errors.append("mobile-upload checked SQL closure drifted")
    for token in (
        "provider_pending",
        "legacy_mobile_upload_records_v1",
        "legacy_mobile_upload_processing_intents_v1",
        "raw_file_key",
        "received_bytes",
        "expected_bytes",
    ):
        if token not in query_text:
            errors.append(f"mobile-upload checked SQL lost token: {token}")
    for token in (
        "legacy_mobile_upload_records_v1",
        "legacy_mobile_upload_operations_v1",
        "legacy_mobile_upload_processing_intents_v1",
        "frame_legacy_mobile_upload_processing_intent_transition_v1",
        "legacy_mobile_upload_assertions_guard_v1",
    ):
        if token not in migration:
            errors.append(f"mobile-upload migration proof lost token: {token}")
    for token in (
        "D1LegacyMobileUploadsV1",
        "sign_legacy_storage_put",
        "begin_completion",
        "LegacyMobileUploadsFailureV1::ProviderGated",
        "COMPLETE_INTENT_INSERT_SQL",
    ):
        if token not in runtime:
            errors.append(f"mobile-upload D1 runtime lost token: {token}")
    for token in (
        "required_actor",
        "CompatibilityRateLimitBucketV1::UploadStorage",
        'bucket("RECORDINGS")',
        ".head(&snapshot.raw_file_key)",
        "object.size()",
        '(503, "provider_execution")',
    ):
        if token not in ingress:
            errors.append(f"mobile-upload HTTP/R2 carrier lost token: {token}")
    for token in (
        "LEGACY_MOBILE_UPLOAD_CREATE_OPERATION_ID",
        "LEGACY_MOBILE_UPLOAD_COMPLETE_OPERATION_ID",
        "LEGACY_MOBILE_UPLOAD_PROGRESS_OPERATION_ID",
        "LegacyRegistrationSourcesV1::MobileUploads",
    ):
        if token not in registry:
            errors.append(f"mobile-upload central registry lost token: {token}")
    for token in (
        "LegacyMobileUploadCreate",
        "LegacyMobileUploadComplete",
        "LegacyMobileUploadProgress",
    ):
        if token not in routing or token not in control:
            errors.append(f"mobile-upload route wiring lost token: {token}")
    for token in (
        "prove_progress_and_non_disclosure",
        "prove_completion_is_durable_and_provider_gated",
        "SELECT COUNT(*) FROM media_jobs",
        'cap_progress["phase"] == "uploading"',
    ):
        if token not in conformance:
            errors.append(f"mobile-upload SQLite proof lost token: {token}")
    for token in (
        "legacy-mobile-uploads-sqlite-conformance.py",
        "mobile-uploads.json",
        "frame-application --lib legacy_mobile_uploads",
        "frame-control-plane --lib legacy_mobile_uploads",
    ):
        if token not in workflow:
            errors.append(f"mobile-upload workflow proof lost token: {token}")
    return errors


def validate_mobile_session_fixture(report: dict[str, Any]) -> list[str]:
    """Bind all four mobile-session routes to exact wire, D1, and provider-gate evidence."""
    try:
        fixture = json.loads(MOBILE_SESSION_FIXTURE.read_text(encoding="utf-8"))
        application = APPLICATION_MOBILE_SESSION.read_text(encoding="utf-8")
        application_lib = APPLICATION_LIB.read_text(encoding="utf-8")
        runtime = CONTROL_MOBILE_SESSION_RUNTIME.read_text(encoding="utf-8")
        ingress = CONTROL_MOBILE_SESSION_WEB_RUNTIME.read_text(encoding="utf-8")
        migration = MOBILE_SESSION_MIGRATION.read_text(encoding="utf-8")
        conformance = MOBILE_SESSION_CONFORMANCE.read_text(encoding="utf-8")
        control_lib = CONTROL_LIB.read_text(encoding="utf-8")
        routing = CONTROL_ROUTING.read_text(encoding="utf-8")
        registry = CONTROL_RUNTIME.read_text(encoding="utf-8")
        workflow = parity_workflow_contract()
        query_names = {path.name for path in MOBILE_SESSION_QUERY_ROOT.glob("*.sql")}
        query_text = "\n".join(
            path.read_text(encoding="utf-8")
            for path in sorted(MOBILE_SESSION_QUERY_ROOT.glob("*.sql"))
        )
    except (OSError, json.JSONDecodeError) as error:
        return [f"unable to load mobile-session fixture: {error}"]

    expected = {
        "cap-v1-e16563e40f697519": (
            "POST", "/api/mobile/session/email/request", 10,
            "2c92c8fa1625007f094af3279b2a84c40b6103a260a4fb5934863f7245efb2ba",
            "public_or_flow_token",
            "rust_exact_mobile_email_session_request_d1_outbox_local_contract",
            ["provider_execution"], "fail_closed_unavailable",
        ),
        "cap-v1-139a189f8a00b38c": (
            "POST", "/api/mobile/session/email/verify", 13,
            "52f971ccd6df308d8de5bd71209a8c73f2a97ce3b2e82cec6bae21f986cbec97",
            "public_or_flow_token",
            "rust_exact_mobile_email_session_verify_d1_local_contract_provider_gated_new_user",
            ["provider_execution"], "fail_closed_unavailable",
        ),
        "cap-v1-ea999fdc5829fbd1": (
            "GET", "/api/mobile/session/request", 8,
            "06c54e1ea054d18b3dd6aa45cdbb393f8df3d4b72a452082c027b836c9410739",
            "optional_session_or_share_capability",
            "rust_exact_mobile_session_request_d1_local_contract",
            [], "serve_exact_d1",
        ),
        "cap-v1-1eef72e518a37abd": (
            "POST", "/api/mobile/session/revoke", 8,
            "6b824b47d20686a7f05a480faf76a8444f20801d294dae3bd969c220eb14ecfd",
            "session_or_api_key",
            "rust_exact_mobile_session_revoke_d1_local_contract",
            [], "serve_exact_d1",
        ),
    }
    errors: list[str] = []
    if (
        fixture.get("schema_version") != "frame.legacy-mobile-session.v1"
        or fixture.get("reference_commit") != REFERENCE_COMMIT
        or fixture.get("wire_contract", {}).get("ttl_ms") != 600000
        or fixture.get("wire_contract", {}).get("token_hash")
        != "sha256(code || NEXTAUTH_SECRET)"
    ):
        errors.append("mobile-session fixture identity or hash/TTL contract drifted")
    fixture_text = json.dumps(fixture, sort_keys=True)
    for token in (
        "100000..999999", "delete the identifier", "createUser database state commits",
        "cap://auth", "exp+cap", "36-byte second authorization segment",
        "fixed-size AES-GCM", "Stripe", "400", "401", "403", "404", "500",
    ):
        if token not in fixture_text:
            errors.append(f"mobile-session semantic closure lost token: {token}")
    operations = fixture.get("operations", [])
    if not isinstance(operations, list) or {row.get("id") for row in operations} != set(expected):
        errors.append("mobile-session fixture operation set drifted")
        return errors
    report_by_id = {row.get("id"): row for row in report.get("entries", [])}
    for operation in operations:
        operation_id = operation["id"]
        method, path, source_count, manifest, auth, status, gates, behavior = expected[operation_id]
        row = report_by_id.get(operation_id, {})
        if (
            operation.get("method") != method
            or operation.get("legacy_identity") != path
            or operation.get("source_count") != source_count
            or operation.get("source_manifest_sha256") != manifest
            or operation.get("authentication") != auth
            or operation.get("idempotency") != "forbidden"
            or operation.get("protected_gates") != gates
            or operation.get("production_behavior") != behavior
        ):
            errors.append(f"mobile-session fixture contract drifted: {operation_id}")
        expected_body = 0 if method == "GET" or path.endswith("/revoke") else 256 * 1024
        expected_types = [] if expected_body == 0 else ["application/json"]
        if (
            row.get("kind") != "route"
            or row.get("method") != method
            or row.get("legacy_path") != path
            or row.get("auth") != auth
            or row.get("policy") != "auth_session.v1"
            or len(row.get("sources", [])) != source_count
            or canonical_json_sha256(row.get("sources", [])) != manifest
            or row.get("implementation", {}).get("local_status") != status
            or row.get("security", {}).get("max_body_bytes") != expected_body
            or row.get("security", {}).get("accepted_content_types") != expected_types
            or row.get("security", {}).get("idempotency") != "forbidden"
            or set(row.get("contract_evidence", {}).values()) != {"local_contract"}
            or row.get("completion", {}).get("local_work") != "complete"
            or row.get("completion", {}).get("protected_gates") != gates
            or row.get("completion", {}).get("production_behavior") != behavior
        ):
            errors.append(f"mobile-session report five-axis evidence drifted: {operation_id}")

    for token in (
        "LEGACY_MOBILE_EMAIL_REQUEST_SOURCES", "LEGACY_MOBILE_EMAIL_VERIFY_SOURCES",
        "LEGACY_MOBILE_SESSION_REQUEST_SOURCES", "LEGACY_MOBILE_SESSION_REVOKE_SOURCES",
        "normalize_legacy_mobile_email", "legacy_mobile_email_code_digest",
        "is_legacy_mobile_auth_redirect_uri", "legacy_mobile_middleware_api_key",
        "legacy_mobile_bearer_token", "legacy_mobile_signup_domain_allowed",
        "legacy_mobile_provisioned_user_name",
    ):
        if token not in application:
            errors.append(f"mobile-session application proof lost token: {token}")
    if (
        "mod legacy_mobile_session;" not in application_lib
        or "pub use legacy_mobile_session::*;" not in application_lib
    ):
        errors.append("mobile-session semantics are not exported by frame-application")
    for token in (
        "D1LegacyMobileSessionV1", "consume_email_challenge", "verify_email_user",
        "StripeEffectPending", "CHALLENGE_DELETE_MATCHING_SQL",
        "MOBILE_KEY_POSTCONDITION_ASSERT_SQL", ".batch(statements)",
    ):
        if token not in runtime:
            errors.append(f"mobile-session D1 runtime proof lost token: {token}")
    for token in (
        "WorkerDeliverySealer", "seal_mobile_email", "local_provider_test_ack",
        "authenticate_host_only_browser_session", "legacy_mobile_login_redirect",
        "legacy_mobile_authenticated_redirect", "stripe_available", "error_response",
    ):
        if token not in ingress:
            errors.append(f"mobile-session HTTP ingress lost token: {token}")
    expected_queries = {
        "actor_authority_assert.sql", "assertion_cleanup.sql", "audit_insert.sql", "challenge_delete_identifier.sql",
        "challenge_delete_matching.sql", "challenge_postcondition_assert.sql",
        "challenge_snapshot.sql", "challenge_upsert.sql", "email_user_exists.sql",
        "handoff_insert.sql", "member_alias_insert.sql", "member_insert.sql",
        "mobile_key_actor.sql", "mobile_key_count.sql", "mobile_key_insert.sql",
        "mobile_key_postcondition_assert.sql", "mobile_key_revoke.sql",
        "mobile_keys_delete.sql", "operation_insert.sql", "organization_alias_insert.sql",
        "organization_insert.sql", "pending_invite.sql", "postcondition_assert.sql",
        "receipt_insert.sql", "revoke_postcondition_assert.sql", "session_actor.sql",
        "stripe_effect_insert.sql", "stripe_effect_postcondition_assert.sql",
        "user_alias_insert.sql", "user_authority_assert.sql", "user_insert.sql",
        "user_organization_select.sql", "user_snapshot.sql",
        "user_verify_provisioned.sql", "user_verify_visible.sql",
    }
    if query_names != expected_queries:
        errors.append("mobile-session checked-in SQL closure drifted")
    for token in (
        "ON CONFLICT(identifier_digest) DO UPDATE", "token_digest = ?2",
        "legacy_source = 'mobile'", "identity_accounts", "has_pending_provisioned_invite",
        "auth_delivery_provider_handoffs_v1", "stripe_effects_v1",
    ):
        if token not in query_text:
            errors.append(f"mobile-session checked SQL proof lost token: {token}")
    for token in (
        "legacy_mobile_session_challenges_v1", "expires_at_ms = created_at_ms + 600000",
        "legacy_mobile_session_stripe_effects_v1", "stripe_effects_ready_v1",
        "legacy_mobile_session_assertion_guard_v1", "payload_immutable",
    ):
        if token not in migration:
            errors.append(f"mobile-session migration proof lost token: {token}")
    for token in (
        "LegacyRegistrationSourcesV1::MobileSession", "LEGACY_MOBILE_EMAIL_REQUEST_OPERATION_ID",
        "LEGACY_MOBILE_EMAIL_VERIFY_OPERATION_ID", "LEGACY_MOBILE_SESSION_REQUEST_OPERATION_ID",
        "LEGACY_MOBILE_SESSION_REVOKE_OPERATION_ID",
    ):
        if token not in registry:
            errors.append(f"mobile-session central registry proof lost token: {token}")
    for token in (
        "mod legacy_mobile_session_runtime;", "mod legacy_mobile_session_web_runtime;",
        "Route::LegacyMobileEmailSessionRequest", "Route::LegacyMobileEmailSessionVerify",
        "Route::LegacyMobileSessionRequest", "Route::LegacyMobileSessionRevoke",
    ):
        if token not in control_lib:
            errors.append(f"mobile-session route wiring lost token: {token}")
    for token in (
        "LegacyMobileEmailSessionRequest", "LegacyMobileEmailSessionVerify",
        "LegacyMobileSessionRequest", "LegacyMobileSessionRevoke",
        '"/api/mobile/session/email/request"', '"/api/mobile/session/email/verify"',
        '"/api/mobile/session/request"', '"/api/mobile/session/revoke"',
    ):
        if token not in routing:
            errors.append(f"mobile-session raw route classification lost token: {token}")
    for token in (
        "replacement", "one-use", "Stripe", "Cap-ID provisioning",
        "replace-all keys", "rollback", "foreign_key_check",
    ):
        if token not in conformance:
            errors.append(f"mobile-session SQLite proof lost token: {token}")
    for token in (
        "legacy-mobile-session-sqlite-conformance.py", "mobile-session.json",
        "frame-application --lib legacy_mobile_session",
        "frame-control-plane --lib legacy_mobile_session",
    ):
        if token not in workflow:
            errors.append(f"mobile-session workflow proof lost token: {token}")
    return errors


def validate_extension_auth_fixture(report: dict[str, Any]) -> list[str]:
    """Bind extension consent/key/bootstrap routes to exact local D1 evidence."""
    try:
        fixture = json.loads(EXTENSION_AUTH_FIXTURE.read_text(encoding="utf-8"))
        application = APPLICATION_EXTENSION_AUTH.read_text(encoding="utf-8")
        application_lib = APPLICATION_LIB.read_text(encoding="utf-8")
        runtime = CONTROL_EXTENSION_AUTH_RUNTIME.read_text(encoding="utf-8")
        ingress = CONTROL_EXTENSION_AUTH_WEB_RUNTIME.read_text(encoding="utf-8")
        migration = EXTENSION_AUTH_MIGRATION.read_text(encoding="utf-8")
        conformance = EXTENSION_AUTH_CONFORMANCE.read_text(encoding="utf-8")
        control_lib = CONTROL_LIB.read_text(encoding="utf-8")
        routing = CONTROL_ROUTING.read_text(encoding="utf-8")
        workflow = parity_workflow_contract()
        query_names = {path.name for path in EXTENSION_AUTH_QUERY_ROOT.glob("*.sql")}
        query_text = "\n".join(
            path.read_text(encoding="utf-8")
            for path in sorted(EXTENSION_AUTH_QUERY_ROOT.glob("*.sql"))
        )
    except (OSError, json.JSONDecodeError) as error:
        return [f"unable to load extension-auth fixture: {error}"]

    errors: list[str] = []
    expected = {
        "cap-v1-249fbd2f77ee7209": (
            "GET", "/api/extension/auth/start", "public_or_flow_token",
            "auth_session.v1", "rust_exact_extension_auth_start_local_contract",
        ),
        "cap-v1-96499b6c8e845b35": (
            "POST", "/api/extension/auth/approve", "public_or_flow_token",
            "auth_session.v1", "rust_exact_extension_auth_approve_d1_local_contract",
        ),
        "cap-v1-ed715d4d23e82181": (
            "POST", "/api/extension/auth/revoke", "session_or_api_key",
            "auth_session.v1", "rust_exact_extension_auth_revoke_d1_local_contract",
        ),
        "cap-v1-12159b1acbaeba7a": (
            "GET", "/api/extension/bootstrap", "session_or_api_key",
            "client_compatibility.v1", "rust_exact_extension_bootstrap_d1_local_contract",
        ),
    }
    if (
        fixture.get("schema_version") != "frame.legacy-extension-auth.v1"
        or fixture.get("reference_commit") != REFERENCE_COMMIT
        or fixture.get("source_count") != 16
        or fixture.get("source_manifest_sha256")
        != "6ec986a350a8f882e8f7150460a24f77f7a0fb5573f3b0b7f634d5578d79aca5"
        or fixture.get("protected_gates") != []
    ):
        errors.append("extension-auth fixture source closure drifted")
    routes = fixture.get("routes", [])
    if not isinstance(routes, list) or {route.get("id") for route in routes} != set(expected):
        errors.append("extension-auth fixture route set drifted")
        return errors
    report_by_id = {row.get("id"): row for row in report.get("entries", [])}
    required_source_paths = {
        "packages/web-domain/src/Extension.ts",
        *(source["path"] for source in EXTENSION_AUTH_RUNTIME_SOURCES),
    }
    for route in routes:
        operation_id = route["id"]
        method, path, auth, policy, status = expected[operation_id]
        if (
            route.get("method") != method
            or route.get("path") != path
            or any(
                not isinstance(route.get(axis), str) or not route.get(axis)
                for axis in (
                    "success", "validation", "authorization", "idempotency_retry", "failure"
                )
            )
        ):
            errors.append(f"extension-auth fixture five-axis contract drifted: {operation_id}")
        row = report_by_id.get(operation_id, {})
        report_source_paths = {
            source.get("path") for source in row.get("sources", []) if isinstance(source, dict)
        }
        if (
            row.get("kind") != "route"
            or row.get("method") != method
            or row.get("legacy_path") != path
            or row.get("auth") != auth
            or row.get("policy") != policy
            or row.get("implementation", {}).get("local_status") != status
            or set(row.get("contract_evidence", {}).values()) != {"local_contract"}
            or row.get("completion", {}).get("local_work") != "complete"
            or row.get("completion", {}).get("protected_gates") != []
            or row.get("completion", {}).get("production_behavior") != "serve_exact_d1"
            or not required_source_paths.issubset(report_source_paths)
        ):
            errors.append(f"extension-auth report evidence drifted: {operation_id}")

    fixture_text = json.dumps(fixture, sort_keys=True)
    for token in (
        "side-effect-free", "same-origin", "SHA-256", "active-owned",
        "NEXT_PUBLIC_IS_CAP=false", "300", "ten user keys", "actor-owned",
    ):
        if token not in fixture_text:
            errors.append(f"extension-auth semantic closure lost token: {token}")
    for token in (
        "LEGACY_EXTENSION_AUTH_SOURCES", "LEGACY_EXTENSION_AUTH_PROFILES",
        "validate_legacy_extension_redirect_uri", "render_legacy_extension_consent_page",
        "legacy_extension_user_is_pro", "!is_cap_hosted",
        "active organization is owned by the actor",
    ):
        if token not in application:
            errors.append(f"extension-auth application proof lost token: {token}")
    if (
        "mod legacy_extension_auth;" not in application_lib
        or "pub use legacy_extension_auth::*;" not in application_lib
    ):
        errors.append("extension-auth contract is not exported by frame-application")
    expected_queries = {
        "api_key_actor.sql", "assertion_delete.sql", "bootstrap_repair.sql",
        "bootstrap_repair_assert.sql", "bootstrap_resolve.sql", "mint_assert.sql",
        "mint_insert.sql", "mint_overflow_delete.sql", "mint_recent_count.sql",
        "revoke_owned.sql", "session_user.sql",
    }
    if query_names != expected_queries:
        errors.append("extension-auth checked-in SQL closure drifted")
    for token in (
        "created_at_ms > ?3 - 3600000", ") > 10", "key_digest = ?2",
        "0 AS priority", "1 AS priority", "2 AS priority",
        "organization_preference_revision = organization_preference_revision + 1",
    ):
        if token not in query_text:
            errors.append(f"extension-auth SQL proof lost token: {token}")
    for token in (
        "legacy_source", "legacy_extension_auth_assertions_v1",
        "frame_legacy_extension_auth_assertion_failed_v1",
    ):
        if token not in migration:
            errors.append(f"extension-auth migration lost token: {token}")
    for token in (
        "Uuid::new_v4", "sha256_hex", ".batch(statements)",
        "LegacyExtensionBootstrapPlanV1::from_pro", "is_cap_hosted",
    ):
        if token not in runtime:
            errors.append(f"extension-auth D1 runtime lost token: {token}")
    for token in (
        "sec_fetch_site != \"same-origin\"", "application/x-www-form-urlencoded",
        "legacy_extension_header_selects_api_key", "with_status(302)",
        "{ \"success\": false }", "CAP_CHROME_EXTENSION_ID",
    ):
        if token not in ingress and token not in control_lib:
            errors.append(f"extension-auth HTTP carrier lost token: {token}")
    for token in (
        "mod legacy_extension_auth_runtime;", "mod legacy_extension_auth_web_runtime;",
        "Route::LegacyExtensionAuthStart", "Route::LegacyExtensionAuthApprove",
        "Route::LegacyExtensionAuthRevoke", "Route::LegacyExtensionBootstrap",
        "NEXT_PUBLIC_IS_CAP",
    ):
        if token not in control_lib:
            errors.append(f"extension-auth control-plane wiring lost token: {token}")
    for token in (
        '"/api/extension/auth/start"', '"/api/extension/auth/approve"',
        '"/api/extension/auth/revoke"', '"/api/extension/bootstrap"',
    ):
        if token not in routing:
            errors.append(f"extension-auth raw route classification lost token: {token}")
    for token in (
        "eleventh hourly extension key", "strict `createdAt > now - one hour`",
        "active-pointer", "non-Cap unlimited branch", "foreign_key_check",
    ):
        if token not in conformance:
            errors.append(f"extension-auth SQLite proof lost token: {token}")
    for token in (
        "legacy-extension-auth-sqlite-conformance.py", "extension-auth.json",
        "frame-application --lib legacy_extension_auth",
        "frame-control-plane --lib legacy_extension_auth",
    ):
        if token not in workflow:
            errors.append(f"extension-auth workflow proof lost token: {token}")
    return errors


def validate_extension_instant_fixture(report: dict[str, Any]) -> list[str]:
    """Bind extension instant recording routes to exact local D1/R2 evidence."""
    try:
        fixture = json.loads(EXTENSION_INSTANT_FIXTURE.read_text(encoding="utf-8"))
        application = APPLICATION_EXTENSION_INSTANT.read_text(encoding="utf-8")
        application_lib = APPLICATION_LIB.read_text(encoding="utf-8")
        runtime = CONTROL_EXTENSION_INSTANT_RUNTIME.read_text(encoding="utf-8")
        ingress = CONTROL_EXTENSION_INSTANT_WEB_RUNTIME.read_text(encoding="utf-8")
        r2_signer = R2_DIRECT_UPLOAD_RUNTIME.read_text(encoding="utf-8")
        migration = EXTENSION_INSTANT_MIGRATION.read_text(encoding="utf-8")
        conformance = EXTENSION_INSTANT_CONFORMANCE.read_text(encoding="utf-8")
        control_lib = CONTROL_LIB.read_text(encoding="utf-8")
        routing = CONTROL_ROUTING.read_text(encoding="utf-8")
        workflow = parity_workflow_contract()
        query_names = {path.name for path in EXTENSION_INSTANT_QUERY_ROOT.glob("*.sql")}
        query_text = "\n".join(
            path.read_text(encoding="utf-8")
            for path in sorted(EXTENSION_INSTANT_QUERY_ROOT.glob("*.sql"))
        )
    except (OSError, json.JSONDecodeError) as error:
        return [f"unable to load extension-instant fixture: {error}"]

    errors: list[str] = []
    expected = {
        "cap-v1-00422c50f4d39053": (
            "POST", "/api/extension/instant-recordings", 256 * 1024,
            ["application/json"],
            "rust_exact_extension_instant_create_d1_r2_local_contract",
        ),
        "cap-v1-82dec55d0fbea3db": (
            "POST", "/api/extension/instant-recordings/progress", 256 * 1024,
            ["application/json"],
            "rust_exact_extension_instant_progress_d1_local_contract",
        ),
        "cap-v1-8fd4741d6e52465e": (
            "DELETE", "/api/extension/instant-recordings/:videoId", 0, [],
            "rust_exact_extension_instant_delete_d1_r2_local_contract",
        ),
    }
    if (
        fixture.get("schema_version")
        != "frame.legacy-extension-instant-recordings.v1"
        or fixture.get("reference_commit") != REFERENCE_COMMIT
        or fixture.get("source_count") != 25
        or fixture.get("source_manifest_sha256")
        != "9a36f7c2832868b8ae4c1b8de1a7e825b15210323243becec599b39dabe208a7"
        or fixture.get("protected_gates") != []
    ):
        errors.append("extension-instant fixture source closure drifted")
    routes = fixture.get("routes", [])
    if not isinstance(routes, list) or {route.get("id") for route in routes} != set(expected):
        errors.append("extension-instant fixture route set drifted")
        return errors
    report_by_id = {row.get("id"): row for row in report.get("entries", [])}
    required_source_paths = {
        "packages/web-domain/src/Extension.ts",
        *(source["path"] for source in EXTENSION_INSTANT_RUNTIME_SOURCES),
    }
    for route in routes:
        operation_id = route["id"]
        method, path, max_body, accepted_types, status = expected[operation_id]
        if (
            route.get("method") != method
            or route.get("path") != path
            or any(
                not isinstance(route.get(axis), str) or not route.get(axis)
                for axis in (
                    "success", "validation", "authorization", "idempotency_retry", "failure"
                )
            )
        ):
            errors.append(f"extension-instant fixture five-axis contract drifted: {operation_id}")
        row = report_by_id.get(operation_id, {})
        report_source_paths = {
            source.get("path") for source in row.get("sources", []) if isinstance(source, dict)
        }
        if (
            row.get("kind") != "route"
            or row.get("method") != method
            or row.get("legacy_path") != path
            or row.get("auth") != "session_or_api_key"
            or row.get("policy") != "video_media.v1"
            or row.get("security", {}).get("max_body_bytes") != max_body
            or row.get("security", {}).get("accepted_content_types") != accepted_types
            or row.get("security", {}).get("idempotency") != "forbidden"
            or row.get("implementation", {}).get("local_status") != status
            or set(row.get("contract_evidence", {}).values()) != {"local_contract"}
            or row.get("completion", {}).get("local_work") != "complete"
            or row.get("completion", {}).get("protected_gates") != []
            or row.get("completion", {}).get("production_behavior") != "serve_exact_d1"
            or len(row.get("sources", [])) != 25
            or not required_source_paths.issubset(report_source_paths)
        ):
            errors.append(f"extension-instant report evidence drifted: {operation_id}")

    fixture_text = json.dumps(fixture, sort_keys=True)
    for token in (
        "immutable 15-character", "UUIDv7", "R2", "If-None-Match",
        "900-second", "monotonic", "two-phase", "404", "CAP_VIDEOS_DEFAULT_PUBLIC",
    ):
        if token not in fixture_text:
            errors.append(f"extension-instant semantic closure lost token: {token}")
    for token in (
        "LEGACY_EXTENSION_INSTANT_SOURCES", "LEGACY_EXTENSION_INSTANT_PROFILES",
        "LEGACY_EXTENSION_INSTANT_SOURCE_MANIFEST_SHA256",
        "legacy_extension_instant_valid_wire_id", "clamped_uploaded",
        "legacy_extension_instant_share_url", "legacy_extension_instant_upload_headers",
    ):
        if token not in application:
            errors.append(f"extension-instant application proof lost token: {token}")
    if (
        "mod legacy_extension_instant_recordings;" not in application_lib
        or "pub use legacy_extension_instant_recordings::*;" not in application_lib
    ):
        errors.append("extension-instant contract is not exported by frame-application")
    expected_queries = {
        "assertion_cleanup.sql", "create_alias_insert.sql", "create_authority.sql",
        "create_authority_assert.sql", "create_operation_insert.sql",
        "create_postcondition_assert.sql", "create_recording_insert.sql",
        "create_upload_insert.sql", "create_video_insert.sql", "delete_authority_assert.sql",
        "delete_cleanup_assert.sql", "delete_finalize_operation.sql",
        "delete_finalize_recording.sql", "delete_mark.sql", "delete_operation_insert.sql",
        "delete_snapshot.sql", "delete_upload_abort.sql", "delete_video_tombstone.sql",
        "progress_authority_assert.sql", "progress_operation_insert.sql",
        "progress_recording_claim_upload.sql", "progress_snapshot.sql",
        "progress_update.sql", "progress_upload_insert.sql",
    }
    if query_names != expected_queries:
        errors.append("extension-instant checked-in SQL closure drifted")
    for token in (
        "actor.active_organization_id = organization.id", "storage.provider = 'r2'",
        "received_bytes <= ?4", "expected_bytes <= ?5", "updated_at_ms <= ?6",
        "changes() = 1", "storage_cleanup_state = 'pending'",
        "storage_cleanup_state = 'complete'",
    ):
        if token not in query_text:
            errors.append(f"extension-instant SQL proof lost token: {token}")
    for token in (
        "legacy_extension_instant_recordings_v1", "legacy_extension_instant_operations_v1",
        "legacy_extension_instant_progress_monotonic_v1",
        "frame_legacy_extension_instant_assertion_failed_v1",
    ):
        if token not in migration:
            errors.append(f"extension-instant migration lost token: {token}")
    for token in (
        "Uuid::now_v7", "random_cap_nanoid", ".batch(statements)",
        "delete_r2_prefix", ".delete_multiple(keys)", "MAX_R2_DELETE_PAGES",
    ):
        if token not in runtime:
            errors.append(f"extension-instant D1/R2 runtime lost token: {token}")
    for token in (
        "required_actor", "LEGACY_EXTENSION_INSTANT_MAX_BODY_BYTES", "Date::parse",
        "videos_default_public", 'env.bucket("RECORDINGS")',
        'json!({"success": true})', "VideoNotFoundError",
    ):
        if token not in ingress:
            errors.append(f"extension-instant HTTP carrier lost token: {token}")
    for token in (
        "sign_legacy_instant_put", "UNSIGNED-PAYLOAD", "if-none-match",
        "valid_legacy_instant_key", "canonical_legacy_headers",
    ):
        if token not in r2_signer:
            errors.append(f"extension-instant R2 signer lost token: {token}")
    for token in (
        "mod legacy_extension_instant_recordings_runtime;",
        "mod legacy_extension_instant_recordings_web_runtime;",
        "Route::LegacyExtensionInstantCreate", "Route::LegacyExtensionInstantProgress",
        "Route::LegacyExtensionInstantDelete", "CAP_VIDEOS_DEFAULT_PUBLIC",
    ):
        if token not in control_lib:
            errors.append(f"extension-instant control-plane wiring lost token: {token}")
    for token in (
        '"/api/extension/instant-recordings"',
        '"/api/extension/instant-recordings/progress"',
        '"instant-recordings", video_id',
    ):
        if token not in routing:
            errors.append(f"extension-instant raw route classification lost token: {token}")
    for token in (
        "durable NanoID alias", "tenant isolation", "equal retry convergence",
        "two-phase prefix cleanup", "preserved tombstone", "foreign_key_check",
    ):
        if token not in conformance:
            errors.append(f"extension-instant SQLite proof lost token: {token}")
    for token in (
        "legacy-extension-instant-recordings-sqlite-conformance.py",
        "extension-instant-recordings.json",
        "frame-application --lib legacy_extension_instant",
        "frame-control-plane --lib legacy_extension_instant",
        "frame-control-plane --lib r2_direct_upload",
    ):
        if token not in workflow:
            errors.append(f"extension-instant workflow proof lost token: {token}")
    return errors


def validate_video_domain_info_fixture(report: dict[str, Any]) -> list[str]:
    """Prove the anonymous timestamp-or-false video domain-info contract."""
    try:
        fixture = json.loads(VIDEO_DOMAIN_INFO_FIXTURE.read_text(encoding="utf-8"))
        application = APPLICATION_VIDEO_DOMAIN_INFO.read_text(encoding="utf-8")
        application_lib = APPLICATION_LIB.read_text(encoding="utf-8")
        runtime = CONTROL_VIDEO_DOMAIN_INFO_RUNTIME.read_text(encoding="utf-8")
        web_runtime = CONTROL_VIDEO_DOMAIN_INFO_WEB_RUNTIME.read_text(encoding="utf-8")
        control_lib = CONTROL_LIB.read_text(encoding="utf-8")
        routing = CONTROL_ROUTING.read_text(encoding="utf-8")
        compatibility_runtime = CONTROL_RUNTIME.read_text(encoding="utf-8")
        conformance = VIDEO_DOMAIN_INFO_CONFORMANCE.read_text(encoding="utf-8")
        workflow = parity_workflow_contract()
        query_names = {path.name for path in VIDEO_DOMAIN_INFO_QUERY_ROOT.glob("*.sql")}
        query_text = "\n".join(
            path.read_text(encoding="utf-8")
            for path in VIDEO_DOMAIN_INFO_QUERY_ROOT.glob("*.sql")
        )
    except (OSError, json.JSONDecodeError) as error:
        return [f"unable to load video domain-info evidence: {error}"]

    errors: list[str] = []
    expected_operation = {
        "id": "cap-v1-10e17d0e86b49830",
        "kind": "route",
        "method": "GET",
        "path": "/api/video/domain-info",
        "auth": "anonymous",
        "policy": "video_media.v1",
        "idempotency": "forbidden",
        "protected_gates": [],
        "production_behavior": "serve_exact_d1",
    }
    if (
        fixture.get("schema_version") != "frame.legacy-video-domain-info.v1"
        or fixture.get("reference_commit") != REFERENCE_COMMIT
        or fixture.get("operation") != expected_operation
    ):
        errors.append("video domain-info identity or completion drifted")

    fixture_sources = {
        source.get("path"): source.get("sha256")
        for source in fixture.get("sources", [])
        if isinstance(source, dict)
    }
    expected_source_paths = {
        "apps/web/app/api/video/domain-info/route.ts",
        "packages/database/schema.ts",
        "packages/web-domain/src/Video.ts",
        "packages/database/index.ts",
        "apps/web/proxy.ts",
        "apps/web/package.json",
        "pnpm-lock.yaml",
    }
    if set(fixture_sources) != expected_source_paths or any(
        not isinstance(digest, str) or len(digest) != 64
        for digest in fixture_sources.values()
    ):
        errors.append("video domain-info source closure drifted")

    row = next(
        (
            entry
            for entry in report.get("entries", [])
            if entry.get("id") == expected_operation["id"]
        ),
        {},
    )
    report_sources = {
        source.get("path"): source.get("sha256") for source in row.get("sources", [])
    }
    if (
        row.get("kind") != "route"
        or row.get("method") != "GET"
        or row.get("legacy_path") != "/api/video/domain-info"
        or row.get("clients") != ["web"]
        or row.get("auth") != "anonymous"
        or row.get("policy") != "video_media.v1"
        or report_sources != fixture_sources
        or row.get("implementation", {}).get("local_status")
        != "rust_exact_anonymous_video_domain_info_d1_local_contract"
        or set(row.get("contract_evidence", {}).values()) != {"local_contract"}
        or row.get("security")
        != {
            "max_body_bytes": 0,
            "accepted_content_types": [],
            "rate_limit_bucket": "video_media.v1",
            "idempotency": "forbidden",
            "tenant_non_disclosure": False,
        }
        or row.get("completion")
        != {
            "decision": "serve_frame_exact_business",
            "local_work": "complete",
            "protected_gates": [],
            "retirement_decision": "not_proposed",
            "production_behavior": "serve_exact_d1",
        }
    ):
        errors.append("video domain-info report evidence drifted")

    for token in (
        "anonymous_concrete_handler_no_session_lookup",
        "nullable_drizzle_timestamp_serializes_as_iso_string_else_false",
        "LEGACY_VIDEO_DOMAIN_INFO_SOURCE_MANIFEST_SHA256",
        "legacy_video_domain_info_select",
    ):
        if token not in application:
            errors.append(f"video domain-info application proof lost token: {token}")
    if (
        "mod legacy_video_domain_info;" not in application_lib
        or "pub use legacy_video_domain_info::*;" not in application_lib
    ):
        errors.append("video domain-info application contract is not exported")
    for token in (
        "VIDEO_AUTHORITY_SQL",
        "ORGANIZATION_DOMAIN_SQL",
        "OWNER_DOMAIN_SQL",
        "shared_organization_id",
        "valid_iso_timestamp",
    ):
        if token not in runtime:
            errors.append(f"video domain-info D1 runtime lost token: {token}")
    for token in (
        "first_video_id",
        '"Video ID is required"',
        '"Video not found"',
        '"Invalid video data"',
        '"Internal server error"',
        "Value::Bool(false)",
    ):
        if token not in web_runtime:
            errors.append(f"video domain-info HTTP carrier lost token: {token}")
    if query_names != {
        "video_authority.sql",
        "organization_domain.sql",
        "owner_domain.sql",
    }:
        errors.append("video domain-info SQL closure drifted")
    for token in (
        "shared.revoked_at_ms IS NULL",
        "shared.video_id = v.id",
        "organization.owner_id = ?1",
        "legacy_org_custom_domain_projection_v1",
        "LIMIT 1",
    ):
        if token not in query_text:
            errors.append(f"video domain-info query proof lost token: {token}")
    for token in (
        "mod legacy_video_domain_info_runtime;",
        "mod legacy_video_domain_info_web_runtime;",
        "Route::LegacyVideoDomainInfo",
    ):
        if token not in control_lib:
            errors.append(f"video domain-info control wiring lost token: {token}")
    for token in ("LegacyVideoDomainInfo", '"/api/video/domain-info"'):
        if token not in routing:
            errors.append(f"video domain-info routing lost token: {token}")
    for token in (
        "LegacyRegistrationSourcesV1::VideoDomainInfo",
        "LEGACY_VIDEO_DOMAIN_INFO_OPERATION_ID",
        "LEGACY_VIDEO_DOMAIN_INFO_SOURCES",
    ):
        if token not in compatibility_runtime:
            errors.append(f"video domain-info registry proof lost token: {token}")
    for token in (
        "shared-before-owner precedence",
        "ISO timestamp projection",
        "revoked share",
    ):
        if token not in conformance:
            errors.append(f"video domain-info SQLite proof lost token: {token}")
    for token in (
        "legacy-video-domain-info-sqlite-conformance.py",
        "video-domain-info.json",
        "frame-application --lib legacy_video_domain_info",
        "frame-control-plane --lib legacy_video_domain_info",
    ):
        if token not in workflow:
            errors.append(f"video domain-info workflow proof lost token: {token}")
    return errors


def validate_video_lifecycle_fixture(report: dict[str, Any]) -> list[str]:
    """Bind Effect-RPC video lifecycle carriers to D1/R2 and retry closure."""
    try:
        fixture = json.loads(VIDEO_LIFECYCLE_FIXTURE.read_text(encoding="utf-8"))
        application = APPLICATION_VIDEO_LIFECYCLE.read_text(encoding="utf-8")
        application_lib = APPLICATION_LIB.read_text(encoding="utf-8")
        runtime = CONTROL_VIDEO_LIFECYCLE_RUNTIME.read_text(encoding="utf-8")
        ingress = CONTROL_VIDEO_LIFECYCLE_WEB_RUNTIME.read_text(encoding="utf-8")
        migration = VIDEO_LIFECYCLE_MIGRATION.read_text(encoding="utf-8")
        conformance = VIDEO_LIFECYCLE_CONFORMANCE.read_text(encoding="utf-8")
        control_lib = CONTROL_LIB.read_text(encoding="utf-8")
        routing = CONTROL_ROUTING.read_text(encoding="utf-8")
        registry = CONTROL_RUNTIME.read_text(encoding="utf-8")
        workflow = parity_workflow_contract()
        query_names = {path.name for path in VIDEO_LIFECYCLE_QUERY_ROOT.glob("*.sql")}
    except (OSError, json.JSONDecodeError) as error:
        return [f"unable to load video lifecycle evidence: {error}"]

    errors: list[str] = []
    operations = fixture.get("operations")
    if not isinstance(operations, list):
        return ["video lifecycle fixture operations must be an array"]
    fixture_by_id = {
        operation.get("id"): operation
        for operation in operations
        if isinstance(operation, dict) and isinstance(operation.get("id"), str)
    }
    expected_ids = {
        adapter["id"] for adapter in VIDEO_LIFECYCLE_LOCAL_ENDPOINT_ADAPTERS.values()
    }
    if (
        fixture.get("schema_version") != "frame.legacy-video-lifecycle.v1"
        or fixture.get("reference_commit") != REFERENCE_COMMIT
        or set(fixture_by_id) != expected_ids
    ):
        errors.append("video lifecycle fixture identity set drifted")

    report_by_id = {row["id"]: row for row in report.get("entries", [])}
    for identity, adapter in VIDEO_LIFECYCLE_LOCAL_ENDPOINT_ADAPTERS.items():
        operation_id = adapter["id"]
        operation = fixture_by_id.get(operation_id, {})
        row = report_by_id.get(operation_id, {})
        kind, method, legacy_path = identity
        expected_body = 0 if method in {"GET", "DELETE"} else 256 * 1024
        expected_types = [] if expected_body == 0 else ["application/json"]
        expected_idempotency = (
            "forbidden" if method in {"GET", "DELETE"} else "required"
        )
        if (
            operation.get("kind") != kind
            or operation.get("method") != method
            or operation.get("legacy_identity") != legacy_path
            or operation.get("protected_gates") != []
            or operation.get("production_behavior")
            != adapter["completion"]["production_behavior"]
            or not re.fullmatch(
                r"[0-9a-f]{64}", operation.get("source_manifest_sha256", "")
            )
        ):
            errors.append(f"video lifecycle fixture contract drifted: {operation_id}")
        if (
            row.get("kind") != kind
            or row.get("method") != method
            or row.get("legacy_path") != legacy_path
            or row.get("implementation")
            != {
                "rust_authority": adapter["rust_authority"],
                "local_status": adapter["local_status"],
            }
            or set(row.get("contract_evidence", {}).values()) != {"local_contract"}
            or row.get("completion") != adapter["completion"]
            or row.get("security", {}).get("max_body_bytes") != expected_body
            or row.get("security", {}).get("accepted_content_types") != expected_types
            or row.get("security", {}).get("idempotency") != expected_idempotency
        ):
            errors.append(f"video lifecycle report evidence drifted: {operation_id}")

    required_queries = {
        "copy_receipt_exists.sql",
        "copy_receipt_insert.sql",
        "delete_postcondition_assert.sql",
        "delete_tombstone.sql",
        "duplicate_alias_insert.sql",
        "duplicate_media_update.sql",
        "duplicate_video_insert.sql",
        "og_snapshot.sql",
        "operation_by_key.sql",
        "operation_complete.sql",
        "operation_insert.sql",
        "operation_storage_pending.sql",
        "organization_admin_snapshot.sql",
        "organization_icon_update.sql",
        "video_owner_snapshot.sql",
    }
    if query_names != required_queries:
        errors.append("video lifecycle checked-in D1 query closure drifted")

    required_tokens = {
        "application": [
            "LEGACY_VIDEO_LIFECYCLE_PROFILES",
            "LEGACY_VIDEO_DELETE_ROUTE_SOURCES",
            "LEGACY_ORGANISATION_UPDATE_SOURCES",
            "LEGACY_VIDEO_RPC_SOURCES",
            "durable_destination_binding_and_per_object_copy_receipts",
            "LEGACY_VIDEO_LIFECYCLE_NO_PROTECTED_GATES",
        ],
        "runtime": [
            "begin_delete",
            "begin_duplicate",
            "delete_r2_prefix",
            "copy_r2_prefix",
            "copy_receipt_exists",
            "ResponseBody::Stream",
        ],
        "ingress": [
            '"OrganisationUpdate" | "VideoDelete" | "VideoDuplicate" | "VideoInstantCreate"',
            "effect_rpc_get_response",
            "delete_route_response",
            "og_response",
            "LegacyExtensionInstantLifecycleReceiptV1",
            "A concurrent retry can lose the unique receipt race",
        ],
        "migration": [
            "legacy_video_lifecycle_operations_v1",
            "legacy_video_lifecycle_copy_receipts_v1",
            "legacy_video_lifecycle_operation_transition_v1",
            "legacy_video_lifecycle_assertion_guard_v1",
        ],
        "conformance": [
            "tenant isolation",
            "D1 tombstone must precede R2 cleanup",
            "copy receipt accepted mutation",
            "same Effect request key accepted different bytes",
            "foreign_key_check",
        ],
        "control_lib": [
            "mod legacy_video_lifecycle_runtime;",
            "mod legacy_video_lifecycle_web_runtime;",
            "Route::LegacyVideoDelete",
            "Route::LegacyVideoOg",
            "effect_rpc_get_response",
        ],
        "routing": [
            "LegacyVideoDelete",
            "LegacyVideoOg",
            '"/api/video/delete"',
            '"/api/video/og"',
        ],
        "registry": [
            "LegacyRegistrationSourcesV1::VideoLifecycle",
            "LEGACY_ERPC_GET_OPERATION_ID",
            "LEGACY_VIDEO_INSTANT_CREATE_OPERATION_ID",
        ],
        "workflow": [
            "legacy-video-lifecycle-sqlite-conformance.py",
            "video-lifecycle.json",
            "frame-application --lib legacy_video_lifecycle",
            "frame-control-plane --lib legacy_video_lifecycle",
        ],
    }
    sources = {
        "application": application,
        "runtime": runtime,
        "ingress": ingress,
        "migration": migration,
        "conformance": conformance,
        "control_lib": control_lib,
        "routing": routing,
        "registry": registry,
        "workflow": workflow,
    }
    for label, tokens in required_tokens.items():
        for token in tokens:
            if token not in sources[label]:
                errors.append(f"video lifecycle {label} lost semantic token: {token}")
    if (
        "mod legacy_video_lifecycle;" not in application_lib
        or "pub use legacy_video_lifecycle::*;" not in application_lib
    ):
        errors.append("video lifecycle application contract is not exported")
    return errors


def validate_core_and_upload_storage_fixtures(report: dict[str, Any]) -> list[str]:
    """Bind retained storage routes, RPCs, actions, and workflow to D1/R2 proof."""
    try:
        core_fixture = json.loads(CORE_STORAGE_FIXTURE.read_text(encoding="utf-8"))
        upload_fixture = json.loads(UPLOAD_STORAGE_FIXTURE.read_text(encoding="utf-8"))
        application_core = APPLICATION_CORE_STORAGE.read_text(encoding="utf-8")
        application_upload = APPLICATION_UPLOAD_STORAGE.read_text(encoding="utf-8")
        application_lib = APPLICATION_LIB.read_text(encoding="utf-8")
        core_runtime = CONTROL_CORE_STORAGE_RUNTIME.read_text(encoding="utf-8")
        core_ingress = CONTROL_CORE_STORAGE_WEB_RUNTIME.read_text(encoding="utf-8")
        upload_runtime = CONTROL_UPLOAD_STORAGE_RUNTIME.read_text(encoding="utf-8")
        upload_ingress = CONTROL_UPLOAD_STORAGE_WEB_RUNTIME.read_text(encoding="utf-8")
        core_migration = CORE_STORAGE_MIGRATION.read_text(encoding="utf-8")
        upload_migration = UPLOAD_STORAGE_MIGRATION.read_text(encoding="utf-8")
        core_conformance = CORE_STORAGE_CONFORMANCE.read_text(encoding="utf-8")
        upload_conformance = UPLOAD_STORAGE_CONFORMANCE.read_text(encoding="utf-8")
        control = CONTROL_LIB.read_text(encoding="utf-8")
        registry = CONTROL_RUNTIME.read_text(encoding="utf-8")
        workflow = parity_workflow_contract()
        core_queries = list(CORE_STORAGE_QUERY_ROOT.glob("*.sql"))
        upload_queries = list(UPLOAD_STORAGE_QUERY_ROOT.glob("*.sql"))
    except (OSError, json.JSONDecodeError) as error:
        return [f"unable to load core/upload storage evidence: {error}"]

    errors: list[str] = []
    if (
        core_fixture.get("schema_version") != "frame.legacy-core-storage.v1"
        or core_fixture.get("reference_commit") != REFERENCE_COMMIT
        or upload_fixture.get("schema_version") != "frame.legacy-upload-storage.v1"
        or upload_fixture.get("reference_commit") != REFERENCE_COMMIT
    ):
        errors.append("core/upload storage fixture identity or reference drifted")

    core_fixture_by_id = {
        item.get("id"): item
        for item in core_fixture.get("operations", [])
        if isinstance(item, dict)
    }
    upload_fixture_by_id = {
        item.get("id"): item
        for item in upload_fixture.get("operations", [])
        if isinstance(item, dict)
    }
    expected_core_ids = {
        adapter["id"] for adapter in CORE_STORAGE_LOCAL_ENDPOINT_ADAPTERS.values()
    }
    expected_upload_ids = {
        adapter["id"] for adapter in UPLOAD_STORAGE_LOCAL_ENDPOINT_ADAPTERS.values()
    }
    if set(core_fixture_by_id) != expected_core_ids:
        errors.append("core-storage fixture operation closure drifted")
    if set(upload_fixture_by_id) != expected_upload_ids:
        errors.append("upload-storage fixture operation closure drifted")

    report_by_id = {row.get("id"): row for row in report.get("entries", [])}
    for identity, adapter in {
        **CORE_STORAGE_LOCAL_ENDPOINT_ADAPTERS,
        **UPLOAD_STORAGE_LOCAL_ENDPOINT_ADAPTERS,
    }.items():
        row = report_by_id.get(adapter["id"], {})
        fixture_item = core_fixture_by_id.get(adapter["id"]) or upload_fixture_by_id.get(
            adapter["id"], {}
        )
        source_pairs = {
            (source.get("path"), source.get("sha256"))
            for source in row.get("sources", [])
            if isinstance(source, dict)
        }
        if (
            (row.get("kind"), row.get("method"), row.get("legacy_path")) != identity
            or row.get("auth") != adapter["auth"]
            or row.get("policy") != adapter["policy"]
            or row.get("security", {}).get("rate_limit_bucket") != adapter["policy"]
            or row.get("implementation", {}).get("local_status")
            != adapter["local_status"]
            or set(row.get("contract_evidence", {}).values()) != {"local_contract"}
            or row.get("completion") != adapter["completion"]
            or (adapter["source_path"], adapter["source_sha256"]) not in source_pairs
            or fixture_item.get("protected_gates")
            != adapter["completion"]["protected_gates"]
            or fixture_item.get("production_behavior")
            != adapter["completion"]["production_behavior"]
        ):
            errors.append(f"core/upload storage report evidence drifted: {adapter['id']}")

    if len(core_queries) != 21 or len(upload_queries) != 24:
        errors.append("core/upload storage checked SQL closure drifted")
    token_groups = {
        "application_core": (
            application_core,
            (
                "LEGACY_CORE_STORAGE_PROFILES",
                "LEGACY_CORE_STORAGE_SOURCES",
                "LEGACY_CORE_STORAGE_PROVIDER_GATES",
                "legacy_core_storage_source_manifest",
            ),
        ),
        "application_upload": (
            application_upload,
            (
                "LEGACY_UPLOAD_STORAGE_OPERATION_IDS",
                "LEGACY_UPLOAD_STORAGE_ACTION_SCHEMA_V1",
                "LEGACY_GET_UPLOAD_PROGRESS_SOURCE_MANIFEST_SHA256",
                "LEGACY_RECONCILE_STALE_EDIT_UPLOAD_SOURCE_MANIFEST_SHA256",
            ),
        ),
        "core_runtime": (
            core_runtime,
            (
                "D1LegacyCoreStorageV1",
                "completion_pending",
                "LegacyCoreStorageFailureV1::ProviderGated",
                "FINALIZE_INTENT_INSERT_SQL",
            ),
        ),
        "core_ingress": (
            core_ingress,
            (
                "download_response",
                "playlist_response",
                "storage_object_response",
                '(503, "provider_execution")',
            ),
        ),
        "upload_runtime": (
            upload_runtime,
            (
                "D1LegacyUploadStorageV1",
                "read_authority",
                "progress_update",
                "reconcile_edit",
                "share_cap",
            ),
        ),
        "upload_ingress": (
            upload_ingress,
            (
                "effect_rpc_response_from_bytes",
                "decode_action_request",
                "existing_password_hashes",
                "download_authority",
            ),
        ),
        "core_migration": (
            core_migration,
            (
                "legacy_core_storage_operations_v1",
                "legacy_core_storage_finalize_intents_v1",
                "effect_pending",
                "completion_pending",
            ),
        ),
        "upload_migration": (
            upload_migration,
            (
                "legacy_upload_storage_operations_v1",
                "legacy_upload_storage_progress_started_at_insert_v1",
                "legacy_upload_storage_space_shares_v1",
            ),
        ),
        "core_conformance": (
            core_conformance,
            (
                "provider",
                "multipart",
                "foreign_key_check",
            ),
        ),
        "upload_conformance": (
            upload_conformance,
            (
                "progress",
                "reconcile",
                "share",
                "foreign_key_check",
            ),
        ),
        "control": (
            control,
            (
                "mod legacy_core_storage_runtime;",
                "mod legacy_core_storage_web_runtime;",
                "mod legacy_upload_storage_runtime;",
                "mod legacy_upload_storage_web_runtime;",
                "Route::LegacyMultipartComplete",
                "legacy_upload_storage_web_runtime::is_action",
            ),
        ),
        "registry": (
            registry,
            (
                "LegacyRegistrationSourcesV1::CoreStorage",
                "LegacyRegistrationSourcesV1::UploadStorage",
                "LEGACY_MULTIPART_COMPLETE_OPERATION_ID",
                "LEGACY_RECONCILE_STALE_EDIT_UPLOAD_OPERATION_ID",
            ),
        ),
        "workflow": (
            workflow,
            (
                "legacy-core-storage-sqlite-conformance.py",
                "legacy-upload-storage-sqlite-conformance.py",
                "core-storage.json",
                "upload-storage.json",
            ),
        ),
    }
    for label, (source, tokens) in token_groups.items():
        for token in tokens:
            if token not in source:
                errors.append(f"core/upload storage {label} lost semantic token: {token}")
    if (
        "mod legacy_core_storage;" not in application_lib
        or "pub use legacy_core_storage::*;" not in application_lib
        or "mod legacy_upload_storage;" not in application_lib
        or "pub use legacy_upload_storage::*;" not in application_lib
    ):
        errors.append("core/upload storage application contracts are not exported")
    return errors


def validate_analytics_fixture(report: dict[str, Any]) -> list[str]:
    """Bind all analytics carriers to exact D1 authority and honest provider gates."""
    try:
        fixture = json.loads(ANALYTICS_FIXTURE.read_text(encoding="utf-8"))
        application = APPLICATION_ANALYTICS.read_text(encoding="utf-8")
        application_lib = APPLICATION_LIB.read_text(encoding="utf-8")
        runtime = CONTROL_ANALYTICS_RUNTIME.read_text(encoding="utf-8")
        ingress = CONTROL_ANALYTICS_WEB_RUNTIME.read_text(encoding="utf-8")
        migration = ANALYTICS_MIGRATION.read_text(encoding="utf-8")
        conformance = ANALYTICS_CONFORMANCE.read_text(encoding="utf-8")
        control = CONTROL_LIB.read_text(encoding="utf-8")
        routing = CONTROL_ROUTING.read_text(encoding="utf-8")
        registry = CONTROL_RUNTIME.read_text(encoding="utf-8")
        workflow = parity_workflow_contract()
        query_names = {path.name for path in ANALYTICS_QUERY_ROOT.glob("*.sql")}
    except (OSError, json.JSONDecodeError) as error:
        return [f"unable to load analytics evidence: {error}"]

    errors: list[str] = []
    fixture_by_id = {
        item.get("id"): item
        for item in fixture.get("operations", [])
        if isinstance(item, dict)
    }
    expected_ids = {adapter["id"] for adapter in ANALYTICS_LOCAL_ENDPOINT_ADAPTERS.values()}
    provider_ids = {
        adapter["id"]
        for adapter in ANALYTICS_LOCAL_ENDPOINT_ADAPTERS.values()
        if adapter["completion"]["protected_gates"] == ["provider_execution"]
    }
    if (
        fixture.get("schema_version") != "frame.legacy-analytics.v1"
        or fixture.get("cap_commit") != REFERENCE_COMMIT
        or set(fixture_by_id) != expected_ids
        or set(
            fixture.get("completion", {})
            .get("production_behavior", {})
            .get("provider_execution", [])
        )
        != provider_ids
        or fixture.get("completion", {})
        .get("production_behavior", {})
        .get("serve_exact_d1")
        != ["cap-v1-dd88ded400188c1e"]
    ):
        errors.append("analytics fixture identity, reference, or gate closure drifted")

    report_by_id = {row.get("id"): row for row in report.get("entries", [])}
    for identity, adapter in ANALYTICS_LOCAL_ENDPOINT_ADAPTERS.items():
        row = report_by_id.get(adapter["id"], {})
        sources = {
            (source.get("path"), source.get("sha256"))
            for source in row.get("sources", [])
            if isinstance(source, dict)
        }
        if (
            (row.get("kind"), row.get("method"), row.get("legacy_path")) != identity
            or row.get("auth") != adapter["auth"]
            or row.get("policy") != "analytics_consent.v1"
            or row.get("implementation", {}).get("local_status")
            != adapter["local_status"]
            or set(row.get("contract_evidence", {}).values()) != {"local_contract"}
            or row.get("completion") != adapter["completion"]
            or (adapter["source_path"], adapter["source_sha256"]) not in sources
        ):
            errors.append(f"analytics report evidence drifted: {adapter['id']}")

    expected_queries = set(fixture.get("persistence", {}).get("queries", []))
    if query_names != expected_queries or len(query_names) != 13:
        errors.append("analytics checked SQL closure drifted")
    semantic_fixture = json.dumps(fixture, sort_keys=True)
    for token in (
        "never claim success from insertion",
        "including organization and space members",
        "per_item_not_found_exit_without_cross_item_disclosure",
        "absent_until_verified_executor_receipts_are_added",
        "Atomic conditional UPDATE with JSON_SET",
    ):
        if token not in semantic_fixture:
            errors.append(f"analytics fixture lost semantic token: {token}")

    token_groups = {
        "application": (
            application,
            (
                "LEGACY_ANALYTICS_VIDEO_COUNT_SOURCES",
                "LEGACY_ANALYTICS_VIDEO_RPC_SOURCES",
                "LEGACY_ANALYTICS_SIGNUP_SOURCES",
                "ProviderPending { operation_id: String }",
            ),
        ),
        "runtime": (
            runtime,
            (
                "D1LegacyAnalyticsPortV1",
                "QUERY_OUTBOX_INSERT_SQL",
                "EVENT_OUTBOX_INSERT_SQL",
                "NOTIFICATION_OUTBOX_INSERT_SQL",
                "ProviderPending",
            ),
        ),
        "ingress": (
            ingress,
            (
                "http_response",
                "effect_rpc_response_from_bytes",
                "is_action",
                "issue_password_grant",
                'json!({"error": "provider_execution"})',
            ),
        ),
        "migration": (
            migration,
            (
                "legacy_analytics_provider_operations_v1",
                "legacy_analytics_query_outbox_v1",
                "legacy_analytics_event_outbox_v1",
                "frame_legacy_analytics_outbox_immutable_v1",
            ),
        ),
        "conformance": (
            conformance,
            (
                "foreign_key_check",
                "prove_query_staging_and_replay",
                "prove_signup_cas",
            ),
        ),
        "control": (
            control,
            (
                "mod legacy_analytics_runtime;",
                "mod legacy_analytics_web_runtime;",
                "Route::LegacyAnalytics",
                "legacy_analytics_web_runtime::is_action",
            ),
        ),
        "routing": (
            routing,
            (
                "LegacyAnalytics",
                "LegacyAnalyticsTrack",
                "LegacyDashboardAnalytics",
                "LegacyVideoAnalytics",
            ),
        ),
        "registry": (
            registry,
            (
                "LegacyRegistrationSourcesV1::Analytics",
                "LEGACY_ANALYTICS_VIDEO_COUNT_OPERATION_ID",
                "LEGACY_ANALYTICS_SIGNUP_OPERATION_ID",
            ),
        ),
        "workflow": (
            workflow,
            (
                "legacy-analytics-sqlite-conformance.py",
                "analytics.json",
                "frame-application --lib legacy_analytics",
                "frame-control-plane --lib legacy_analytics",
            ),
        ),
    }
    for label, (source, tokens) in token_groups.items():
        for token in tokens:
            if token not in source:
                errors.append(f"analytics {label} lost semantic token: {token}")
    if (
        "mod legacy_analytics;" not in application_lib
        or "pub use legacy_analytics::*;" not in application_lib
    ):
        errors.append("analytics application contract is not exported")
    return errors


def validate_organization_library_fixture(report: dict[str, Any]) -> list[str]:
    """Bind all provider-free organization/library actions to D1/R2 evidence."""
    try:
        fixture = json.loads(ORGANIZATION_LIBRARY_FIXTURE.read_text(encoding="utf-8"))
        application = APPLICATION_ORGANIZATION_LIBRARY.read_text(encoding="utf-8")
        application_lib = APPLICATION_LIB.read_text(encoding="utf-8")
        runtime = CONTROL_ORGANIZATION_LIBRARY_RUNTIME.read_text(encoding="utf-8")
        ingress = CONTROL_ORGANIZATION_LIBRARY_WEB_RUNTIME.read_text(encoding="utf-8")
        migration = ORGANIZATION_LIBRARY_MIGRATION.read_text(encoding="utf-8")
        conformance = ORGANIZATION_LIBRARY_CONFORMANCE.read_text(encoding="utf-8")
        control = CONTROL_LIB.read_text(encoding="utf-8")
        registry = CONTROL_RUNTIME.read_text(encoding="utf-8")
        workflow = parity_workflow_contract()
        query_names = {path.name for path in ORGANIZATION_LIBRARY_QUERY_ROOT.glob("*.sql")}
    except (OSError, json.JSONDecodeError) as error:
        return [f"unable to load organization-library evidence: {error}"]

    errors: list[str] = []
    fixture_by_id = {
        item.get("id"): item
        for item in fixture.get("operations", [])
        if isinstance(item, dict)
    }
    if (
        fixture.get("schema_version") != "frame.legacy-organization-library.v1"
        or fixture.get("reference_commit") != REFERENCE_COMMIT
        or set(fixture_by_id) != ORGANIZATION_LIBRARY_EXPECTED_IDS
        or fixture.get("protected_gates") != []
        or fixture.get("production_behavior") != "serve_exact_d1_r2_action"
    ):
        errors.append("organization-library fixture identity or completion drifted")

    report_by_id = {row.get("id"): row for row in report.get("entries", [])}
    for identity, adapter in ORGANIZATION_LIBRARY_LOCAL_ENDPOINT_ADAPTERS.items():
        row = report_by_id.get(adapter["id"], {})
        item = fixture_by_id.get(adapter["id"], {})
        sources = {
            (source.get("path"), source.get("sha256"))
            for source in row.get("sources", [])
            if isinstance(source, dict)
        }
        anonymous = adapter["id"] == "cap-v1-61e089033a34d239"
        if (
            (row.get("kind"), row.get("method"), row.get("legacy_path")) != identity
            or row.get("auth") != ("anonymous" if anonymous else "session")
            or row.get("policy")
            != ("share_playback.v1" if anonymous else "organization_library.v1")
            or row.get("security")
            != {
                "max_body_bytes": 4 * 1024 * 1024,
                "accepted_content_types": ["application/json"],
                "rate_limit_bucket": (
                    "share_playback.v1" if anonymous else "organization_library.v1"
                ),
                "idempotency": "forbidden" if anonymous else "required",
                "tenant_non_disclosure": True,
            }
            or row.get("implementation", {}).get("local_status")
            != "rust_exact_organization_library_d1_r2_action_local_contract"
            or set(row.get("contract_evidence", {}).values()) != {"local_contract"}
            or row.get("completion") != adapter["completion"]
            or (item.get("source_path"), item.get("source_sha256")) not in sources
        ):
            errors.append(f"organization-library report evidence drifted: {adapter['id']}")

    if len(query_names) != 51:
        errors.append("organization-library checked SQL closure drifted")
    semantic_fixture = json.dumps(fixture, sort_keys=True)
    for token in (
        "PBKDF2-HMAC-SHA256",
        "plaintext_password_and_verification_outcome_are_never_persisted",
        "durable_pending_effect_rows_are_retried",
        "local_HMAC_signed_OAuth_state_and_URL_projection_only_no_Google_network_call",
        "save_remove_test_provider_actions_remain_excluded",
    ):
        if token not in semantic_fixture:
            errors.append(f"organization-library fixture lost semantic token: {token}")

    token_groups = {
        "application": (
            application,
            (
                "LEGACY_ORGANIZATION_LIBRARY_OPERATION_COUNT",
                "LegacyOrganizationLibraryProfileV1",
                "VerifyCollectionPassword",
                "ConnectOrganizationGoogleDrive",
                "protected_gates: &[]",
            ),
        ),
        "runtime": (
            runtime,
            (
                "D1R2LegacyOrganizationLibraryPortV1",
                "R2_EFFECT_INSERT_SQL",
                "MAX_R2_PREFIX_OBJECTS",
                "verify_password",
                "google_drive_authorization_url",
            ),
        ),
        "ingress": (
            ingress,
            (
                "decode_action_request",
                "action_response",
                "consume_session_grant",
                "CompatibilityRateLimitBucketV1::SharePlayback",
                "password_cookie",
            ),
        ),
        "migration": (
            migration,
            (
                "legacy_organization_library_operations_v1",
                "legacy_organization_library_r2_effects_v1",
                "legacy_organization_library_assertions_v1",
            ),
        ),
        "conformance": (
            conformance,
            (
                "prove_password_non_journaled",
                "prove_stale_authority_rolls_back",
                "prove_resumable_r2_delete",
                "prove_create_without_active_tenant",
            ),
        ),
        "control": (
            control,
            (
                "mod legacy_organization_library_runtime;",
                "mod legacy_organization_library_web_runtime;",
                "legacy_organization_library_web_runtime::is_action",
            ),
        ),
        "registry": (
            registry,
            (
                "LegacyRegistrationSourcesV1::OrganizationLibrary",
                "LEGACY_SET_COLLECTION_LOGO_OPERATION_ID",
                "LEGACY_CREATE_ORGANIZATION_OPERATION_ID",
            ),
        ),
        "workflow": (
            workflow,
            (
                "legacy-organization-library-sqlite-conformance.py",
                "organization-library.json",
                "frame-application --lib legacy_organization_library",
                "frame-control-plane --lib legacy_organization_library",
            ),
        ),
    }
    for label, (source, tokens) in token_groups.items():
        for token in tokens:
            if token not in source:
                errors.append(f"organization-library {label} lost semantic token: {token}")
    if (
        "mod legacy_organization_library;" not in application_lib
        or "pub use legacy_organization_library::*;" not in application_lib
    ):
        errors.append("organization-library application contract is not exported")
    return errors


def validate_protected_media_fixture(report: dict[str, Any]) -> list[str]:
    """Bind all 41 media/hardware contracts to immutable fail-closed staging."""
    try:
        fixture = json.loads(PROTECTED_MEDIA_FIXTURE.read_text(encoding="utf-8"))
        application = APPLICATION_PROTECTED_MEDIA.read_text(encoding="utf-8")
        application_lib = APPLICATION_LIB.read_text(encoding="utf-8")
        runtime = CONTROL_PROTECTED_MEDIA_RUNTIME.read_text(encoding="utf-8")
        ingress = CONTROL_PROTECTED_MEDIA_WEB_RUNTIME.read_text(encoding="utf-8")
        migration = PROTECTED_MEDIA_MIGRATION.read_text(encoding="utf-8")
        conformance = PROTECTED_MEDIA_CONFORMANCE.read_text(encoding="utf-8")
        control = CONTROL_LIB.read_text(encoding="utf-8")
        routing = CONTROL_ROUTING.read_text(encoding="utf-8")
        registry = CONTROL_RUNTIME.read_text(encoding="utf-8")
        workflow = parity_workflow_contract()
        query_names = {path.name for path in PROTECTED_MEDIA_QUERY_ROOT.glob("*.sql")}
    except (OSError, json.JSONDecodeError) as error:
        return [f"unable to load protected-media evidence: {error}"]

    errors: list[str] = []
    operations = fixture.get("operations", [])
    fixture_by_id = {
        item.get("id"): item for item in operations if isinstance(item, dict)
    }
    if (
        fixture.get("schema_version")
        != "frame.api-parity.legacy-protected-media.v1"
        or fixture.get("reference", {}).get("commit") != REFERENCE_COMMIT
        or fixture.get("summary")
        != {
            "hardware_and_provider": 25,
            "hardware_only": 16,
            "local_terminal_behavior": "fail_closed_unavailable",
            "operation_count": 41,
        }
        or len(fixture_by_id) != 41
        or fixture.get("durable_contract")
        != {
            "authority_model": "exact credential plus ordered live policy proofs and optional AI entitlement",
            "caller_idempotency_header": False,
            "execution_evidence": "independent executor lease plus provider evidence when required",
            "request_storage": "redacted v2 descriptor plus optional opaque sealed request reference",
            "terminal_storage": "typed opaque sealed terminal reference with 15-minute retention",
            "workflow_parent_model": "shared protected effect registry, exact parent authority digest, allowlisted target rule",
        }
    ):
        errors.append("protected-media fixture identity or summary drifted")

    report_by_id = {row.get("id"): row for row in report.get("entries", [])}
    for identity, adapter in PROTECTED_MEDIA_LOCAL_ENDPOINT_ADAPTERS.items():
        item = fixture_by_id.get(adapter["id"], {})
        row = report_by_id.get(adapter["id"], {})
        report_sources = {
            (source.get("path"), source.get("symbol"), source.get("sha256"))
            for source in row.get("sources", [])
            if isinstance(source, dict)
        }
        fixture_sources = {
            (source.get("path"), source.get("symbol"), source.get("sha256"))
            for source in item.get("source_manifest", [])
            if isinstance(source, dict)
        }
        expected_security = {
            "max_body_bytes": item.get("max_body_bytes"),
            "accepted_content_types": item.get("accepted_content_types"),
            "rate_limit_bucket": item.get("rate_limit_bucket"),
            "idempotency": item.get("idempotency"),
            "tenant_non_disclosure": True,
        }
        local_auth_overrides = {
            "cap-v1-c471cd8f8f990fcc": "session",
            "cap-v1-fbd3d44a0ca1786f": "public_edge_or_job_capability",
            "cap-v1-0bf20f7e9b1a474c": "public_edge_or_job_capability",
            "cap-v1-43bc9ae6aa4f44a8": "public_edge_or_job_capability",
            "cap-v1-986bf73a0b5cb676": "public_edge_or_job_capability",
            "cap-v1-aa2bd4c3be69ed42": "optional_session_or_share_capability",
        }
        expected_local_auth = (
            "parent_derived"
            if item.get("kind") == "workflow"
            else local_auth_overrides.get(adapter["id"], item.get("auth"))
        )
        if (
            (row.get("kind"), row.get("method"), row.get("legacy_path")) != identity
            or row.get("auth") != item.get("auth")
            or row.get("clients") != item.get("clients")
            or row.get("policy") != "video_media.v1"
            or row.get("security") != expected_security
            or row.get("implementation", {}).get("local_status")
            != "rust_exact_protected_media_execution_staging_local_contract"
            or set(row.get("contract_evidence", {}).values()) != {"local_contract"}
            or row.get("completion") != adapter["completion"]
            or item.get("local_auth") != expected_local_auth
            or not fixture_sources
            or not fixture_sources.issubset(report_sources)
        ):
            errors.append(f"protected-media report evidence drifted: {adapter['id']}")

    if query_names != {
        "ai_entitlement.sql",
        "generated_claim_upsert.sql",
        "generated_replay.sql",
        "job_capability.sql",
        "organization_access.sql",
        "outbox_insert.sql",
        "receipt_insert.sql",
        "receipt_replay.sql",
        "share_capability_by_hash.sql",
        "share_public_capability.sql",
        "video_policy_base.sql",
        "workflow_parent_read.sql",
    }:
        errors.append("protected-media checked SQL closure drifted")
    if sum(
        "provider_execution" in item.get("protected_gates", []) for item in operations
    ) != 25 or sum(
        item.get("protected_gates") == ["hardware_execution"] for item in operations
    ) != 16:
        errors.append("protected-media evidence-gate partition drifted")

    token_groups = {
        "application": (
            application,
            (
                "LEGACY_PROTECTED_MEDIA_OPERATION_COUNT",
                "LEGACY_PROTECTED_MEDIA_PROFILES",
                "additional_sources",
                "validate_legacy_protected_media_envelope",
                "legacy_protected_media_authority_binding_digest",
                "parent_authority_binding_digest",
            ),
        ),
        "runtime": (
            runtime,
            (
                "D1LegacyProtectedMediaRuntimeV1",
                "batch(statements)",
                "pending_execution_evidence",
                "ExecutionEvidenceRequired",
                "GENERATED_CLAIM_UPSERT_SQL",
            ),
        ),
        "ingress": (
            ingress,
            (
                "route_response",
                "effect_rpc_response_from_bytes",
                "server_action_response",
                "workflow_response",
                "EXECUTION_EVIDENCE_REQUIRED",
                "ProtectedMediaRequestVaultV1",
                "ProtectedMediaTerminalV1",
                "WORKFLOW_PARENT_READ_SQL",
                "ParentReceiptClaimV1",
                "LegacyProtectedMediaReplayOriginV1::Workflow",
            ),
        ),
        "migration": (
            migration,
            (
                "legacy_protected_media_receipts_v1",
                "legacy_protected_media_execution_outbox_v1",
                "legacy_protected_media_execution_evidence_v1",
                "legacy_protected_media_evidence_gate_v1",
                "legacy_protected_effect_parent_registry_v1",
                "legacy_protected_media_live_video_policy_v1",
            ),
        ),
        "conformance": (
            conformance,
            (
                "validate_fixture",
                "frame_protected_media_receipt_immutable_v1",
                "PRAGMA foreign_key_check",
            ),
        ),
        "control": (
            control,
            (
                "dispatch_legacy_protected_media_callable_v1",
                "dispatch_legacy_protected_media_workflow_v1",
                "legacy_protected_media_route_dispatch",
                "legacy_protected_media_web_runtime::is_server_action",
            ),
        ),
        "routing": (routing, ("LegacyProtectedMedia",)),
        "registry": (
            registry,
            (
                "LegacyRegistrationSourcesV1::ProtectedMedia",
                "protected_media_registration",
                "legacy_protected_media_profile",
            ),
        ),
        "workflow": (
            workflow,
            (
                "legacy-protected-media-sqlite-conformance.py",
                "protected-media-contracts.json",
                "frame-application --lib legacy_protected_media",
                "frame-control-plane --lib legacy_protected_media",
            ),
        ),
    }
    for label, (source, tokens) in token_groups.items():
        for token in tokens:
            if token not in source:
                errors.append(f"protected-media {label} lost semantic token: {token}")
    if (
        "mod legacy_protected_media;" not in application_lib
        or "pub use legacy_protected_media::*;" not in application_lib
    ):
        errors.append("protected-media application contract is not exported")
    return errors


def validate_protected_integrations_fixture(report: dict[str, Any]) -> list[str]:
    """Bind all 45 provider-only carriers to vault-bound immutable D1 staging."""
    try:
        fixture = json.loads(PROTECTED_INTEGRATIONS_FIXTURE.read_text(encoding="utf-8"))
        application = APPLICATION_PROTECTED_INTEGRATIONS.read_text(encoding="utf-8")
        application_lib = APPLICATION_LIB.read_text(encoding="utf-8")
        runtime = CONTROL_PROTECTED_INTEGRATIONS_RUNTIME.read_text(encoding="utf-8")
        ingress = CONTROL_PROTECTED_INTEGRATIONS_WEB_RUNTIME.read_text(encoding="utf-8")
        migration = PROTECTED_INTEGRATIONS_MIGRATION.read_text(encoding="utf-8")
        conformance = PROTECTED_INTEGRATIONS_CONFORMANCE.read_text(encoding="utf-8")
        control = CONTROL_LIB.read_text(encoding="utf-8")
        routing = CONTROL_ROUTING.read_text(encoding="utf-8")
        registry = CONTROL_RUNTIME.read_text(encoding="utf-8")
        workflow = parity_workflow_contract()
        query_names = {
            path.name for path in PROTECTED_INTEGRATIONS_QUERY_ROOT.glob("*.sql")
        }
    except (OSError, json.JSONDecodeError) as error:
        return [f"unable to load protected-integration evidence: {error}"]

    errors: list[str] = []
    operations = fixture.get("operations", [])
    fixture_by_id = {
        item.get("id"): item for item in operations if isinstance(item, dict)
    }
    profile_sources = protected_integration_profile_sources()
    if (
        fixture.get("schema_version") != "frame.legacy-protected-integrations.v1"
        or fixture.get("reference", {}).get("commit") != REFERENCE_COMMIT
        or fixture.get("operation_count") != 45
        or fixture.get("protected_gates") != ["provider_execution"]
        or len(fixture_by_id) != 45
        or set(fixture_by_id) != set(profile_sources)
    ):
        errors.append("protected-integration fixture identity or gate drifted")

    report_by_id = {row.get("id"): row for row in report.get("entries", [])}
    for identity, adapter in PROTECTED_INTEGRATION_LOCAL_ENDPOINT_ADAPTERS.items():
        item = fixture_by_id.get(adapter["id"], {})
        row = report_by_id.get(adapter["id"], {})
        source = profile_sources.get(adapter["id"], {})
        report_sources = {
            (entry.get("path"), entry.get("sha256"))
            for entry in row.get("sources", [])
            if isinstance(entry, dict)
        }
        if (
            (row.get("kind"), row.get("method"), row.get("legacy_path")) != identity
            or row.get("auth") != item.get("auth")
            or row.get("policy") != adapter["policy"]
            or row.get("security", {}).get("idempotency") != item.get("idempotency")
            or row.get("security", {}).get("tenant_non_disclosure")
            is not (item.get("authority") != "public")
            or row.get("implementation", {}).get("local_status")
            != "rust_exact_protected_integration_provider_staging_local_contract"
            or set(row.get("contract_evidence", {}).values()) != {"local_contract"}
            or row.get("completion") != adapter["completion"]
            or len(row.get("sources", [])) != item.get("source_count")
            or (source.get("path"), source.get("sha256")) not in report_sources
        ):
            errors.append(
                f"protected-integration report evidence drifted: {adapter['id']}"
            )

    declared_queries = set(fixture.get("persistence", {}).get("queries", []))
    required_queries = {
        "authority_read.sql",
        "generated_claim_upsert.sql",
        "generated_receipt_replay.sql",
        "outbox_insert.sql",
        "receipt_insert.sql",
        "receipt_replay.sql",
        "workflow_parent_read.sql",
    }
    if query_names != declared_queries or not required_queries <= query_names:
        errors.append("protected-integration checked SQL closure drifted")
    declared_tables = set(fixture.get("persistence", {}).get("tables", []))
    required_tables = {
        "legacy_protected_integration_receipts_v1",
        "legacy_protected_integration_generated_replay_claims_v1",
        "legacy_protected_integration_outbox_v1",
        "legacy_protected_integration_evidence_v1",
        "legacy_protected_effect_parent_registry_v1",
        "legacy_protected_effect_parent_edges_v1",
    }
    if not required_tables <= declared_tables:
        errors.append("protected-integration persistence inventory drifted")
    declared_adapter_sources = set(
        fixture.get("implementation", {}).get("local_adapter_sources", [])
    )
    required_adapter_sources = {
        "crates/application/src/legacy_protected_integrations.rs",
        "apps/control-plane/src/legacy_protected_integrations_runtime.rs",
        "apps/control-plane/src/legacy_protected_integrations_web_runtime.rs",
        "apps/control-plane/migrations/0062_legacy_protected_integrations_expand.sql",
        "fixtures/api-parity/v1/protected-integrations.json",
        "scripts/ci/legacy-protected-integrations-sqlite-conformance.py",
        *{
            f"apps/control-plane/queries/legacy_protected_integrations/{name}"
            for name in declared_queries
        },
    }
    if not required_adapter_sources <= declared_adapter_sources:
        errors.append("protected-integration local adapter source closure drifted")
    security_text = json.dumps(fixture.get("security", {}), sort_keys=True)
    for token in (
        "frame-pi-request-v1",
        "frame-pi-terminal-v1",
        "Idempotency-Key",
        "Plaintext credentials",
        "fail closed",
    ):
        if token not in security_text:
            errors.append(f"protected-integration fixture lost security token: {token}")
    conditional_authority = fixture.get("security", {}).get(
        "conditional_authority", {}
    )
    if conditional_authority.get("space_create_actor_pro") != [
        "passwordEnabled",
        "disableSummary",
        "disableChapters",
        "disableTranscript",
    ]:
        errors.append("protected-integration create-space Pro gates drifted")
    if "need not be that owner" not in conditional_authority.get(
        "space_publish_owner_pro", ""
    ):
        errors.append("protected-integration owner-plan publish gate drifted")
    if "preserves existing" not in conditional_authority.get(
        "space_update_non_pro_settings", ""
    ):
        errors.append("protected-integration update-space preservation drifted")
    if "hasProSeat is false" not in conditional_authority.get(
        "seat_capacity", ""
    ):
        errors.append("protected-integration seat-capacity projection drifted")

    workflow_authority = fixture.get("security", {}).get("workflow_authority", {})
    if not all(
        token in workflow_authority.get("loom_domain_identity", "")
        for token in (
            "cap.userId",
            "cap.orgId",
            "loom.orgId",
            "legacy_collaboration_user_aliases_v1",
            "legacy_user_account_organization_ids_v1",
        )
    ):
        errors.append("protected-integration Loom domain identity binding drifted")
    if not all(
        token in workflow_authority.get("loom_media_identity", "")
        for token in ("userId", "target video's native owner", "CSV", "active organization member")
    ):
        errors.append("protected-integration Loom media owner binding drifted")
    raw_file_key = workflow_authority.get("raw_file_key", {})
    if (
        "{legacy_user_id}/{legacy_video_id}/raw-upload.mp4"
        not in raw_file_key.get("initial", "")
        or "legacy_mobile_cap_uploads_v1.raw_file_key"
        not in raw_file_key.get("retry", "")
    ):
        errors.append("protected-integration Loom raw-file-key binding drifted")
    if (
        "may be empty" not in workflow_authority.get("empty_retry_url", "")
        or "parent family, receipt, and request digest"
        not in workflow_authority.get("parent_scoped_replay", "")
    ):
        errors.append("protected-integration Loom retry contract drifted")

    natural_replay = fixture.get("security", {}).get("natural_replay", {})
    if (
        "globally unique" not in natural_replay.get("scope", "")
        or "generated replay remains principal-bound"
        not in natural_replay.get("scope", "")
        or "key rotation" not in natural_replay.get("signed_webhook", "")
        or "session or API-key rotation"
        not in natural_replay.get("loom_domain_workflow", "")
    ):
        errors.append("protected-integration natural replay scope drifted")

    loom_egress = fixture.get("security", {}).get("loom_download_egress", {})
    if (
        loom_egress.get("payload_pointer") != "/loom/video/downloadUrl"
        or loom_egress.get("scheme") != "https"
        or not all(
            token in loom_egress.get("host_policy", "")
            for token in ("loom.com", "IP literals", "localhost", "user-info", "explicit ports")
        )
        or "every hop" not in loom_egress.get("redirect_policy", "")
        or not all(
            token in loom_egress.get("dns_policy", "")
            for token in ("loopback", "link-local", "private", "multicast", "unspecified")
        )
    ):
        errors.append("protected-integration Loom egress policy drifted")

    source_branches = fixture.get("security", {}).get("source_branch_contracts", {})
    create_space = source_branches.get("create_space", {})
    create_space_ids = ["cap-v1-0c233c1115838206", "cap-v1-5e7e4265d65c8365"]
    if (
        create_space.get("operation_ids") != create_space_ids
        or create_space.get("authority") != "organization_member"
        or "No organizationId or orgId" not in create_space.get("tenant_selector", "")
        or any(fixture_by_id.get(operation_id, {}).get("authority") != "organization_member"
               for operation_id in create_space_ids)
    ):
        errors.append("protected-integration selector-free create-space contract drifted")
    loom_parent = source_branches.get("loom_http_parent", {})
    if (
        loom_parent.get("operation_id") != "cap-v1-f0a00e93ab606a52"
        or loom_parent.get("required_paths")
        != [
            "/cap/orgId",
            "/loom/userId",
            "/loom/orgId",
            "/loom/video/id",
            "/loom/video/name",
            "/loom/video/downloadUrl",
        ]
    ):
        errors.append("protected-integration Loom HTTP parent schema drifted")
    video_create = source_branches.get("desktop_video_create", {})
    if (
        video_create.get("operation_id") != "cap-v1-60f863b2cb19353f"
        or not all(
            token in video_create.get("existing_video", "")
            for token in ("ignores orgId", "video.organization_id", "video_existing_owner")
        )
        or not all(
            token in video_create.get("new_video", "")
            for token in ("unknown", "no effective target", "video_new_organization_member")
        )
        or not all(
            token in video_create.get("tenant_fallback", "")
            for token in ("explicit accessible orgId", "default organization", "oldest active", "active_organization_id is not")
        )
        or not all(
            token in video_create.get("duration_gate", "")
            for token in ("strictly greater than 300", "fractional", "video_duration_pro")
        )
        or video_create.get("header_inputs")
        != [
            "X-Cap-Desktop-Features: googleDriveUpload",
            "X-Cap-Desktop-Version >= 0.3.68",
        ]
        or "sealed request and request digest"
        not in video_create.get("header_binding", "")
    ):
        errors.append("protected-integration desktop video-create branch contract drifted")

    source_semantics = {
        "application": (
            application,
            (
                "validate_external_download_url",
                '"/loom/userId"',
                '"/loom/video/name"',
                '"/loomDownloadUrl"',
                '"parent_receipt_id"',
                '"video_existing_owner"',
                '"video_new_organization_member"',
                '"video_duration_pro"',
            ),
        ),
        "runtime": (
            runtime,
            (
                "legacy_workflow_actor_id",
                "legacy_workflow_cap_tenant_id",
                "workflow_raw_file_key",
            ),
        ),
        "ingress": (
            ingress,
            ("x-cap-desktop-features", "x-cap-desktop-version"),
        ),
        "migration": (
            migration,
            (
                "workflow_raw_file_key",
                "legacy_mobile_cap_uploads_v1",
                "WHERE replay_origin='natural'",
                "json_extract(binding.value,'$.kind')='space_publish_owner_pro'",
                "json_extract(binding.value,'$.kind')='seat_capacity'",
            ),
        ),
    }
    for label, (source, tokens) in source_semantics.items():
        for token in tokens:
            if token not in source:
                errors.append(
                    f"protected-integration {label} lost frozen semantic token: {token}"
                )

    # These client inputs do not exist in the pinned Cap carriers. Admitting
    # any of them would silently replace source authentication/replay with a
    # Frame-only protocol.
    for invented_header in (
        "idempotency-key",
        "x-frame-sealed-payload-ref",
        "x-frame-flow-token",
    ):
        if invented_header in ingress.lower():
            errors.append(
                "protected-integration ingress invented released-client header: "
                f"{invented_header}"
            )

    token_groups = {
        "application": (
            application,
            (
                "LEGACY_PROTECTED_INTEGRATIONS_OPERATION_COUNT",
                "LEGACY_PROTECTED_INTEGRATION_PROFILES",
                "LegacyProtectedIntegrationCredentialKindV1",
                "LegacyProtectedIntegrationReplayOriginV1",
                "legacy_protected_integration_entitlement",
                "legacy_protected_integration_plaintext_request_digest",
                "legacy_protected_integration_authority_binding_digest",
                "validate_legacy_protected_integration_envelope",
                "validate_public_loom_url",
            ),
        ),
        "runtime": (
            runtime,
            (
                "D1LegacyProtectedIntegrationRuntimeV1",
                "AUTHORITY_READ_SQL",
                "GENERATED_RECEIPT_REPLAY_SQL",
                "GENERATED_CLAIM_UPSERT_SQL",
                "WORKFLOW_PARENT_READ_SQL",
                "OUTBOX_INSERT_SQL",
                "pro_space_settings_requested",
                "space_settings_pro",
                "owner_revision",
                "ProviderEvidenceRequired",
                "sealed_terminal_ref",
            ),
        ),
        "ingress": (
            ingress,
            (
                "route_response",
                "effect_rpc_response_from_bytes",
                "server_action_http_response",
                "workflow_response",
                "ProtectedIntegrationRequestVaultV1",
                "ProtectedIntegrationTerminalResolverV1",
                "google_callback_public_error_response",
                "compatibility_rate_limit::admit_principal",
                "PROVIDER_EXECUTION_REQUIRED",
            ),
        ),
        "migration": (
            migration,
            (
                "legacy_protected_integration_receipts_v1",
                "legacy_protected_integration_generated_replay_claims_v1",
                "legacy_protected_integration_outbox_v1",
                "legacy_protected_integration_evidence_v1",
                "legacy_protected_integration_evidence_gate_v1",
                "legacy_protected_integration_live_authority_v1",
                "credential_subject_id",
                "credential_key_version",
                "credential_expires_at_ms",
                "policy_proofs_json",
                "entitlement_revision",
                "entitlement_expires_at_ms",
                "conditional_bindings_json",
                "conditional_pro_settings_requested",
                "space_settings_pro",
                "space_publish_owner_pro",
                "seat_capacity",
                "authority_binding_digest",
                "parent_authority_binding_digest",
                "legacy_protected_effect_parent_registry_v1",
                "legacy_protected_effect_parent_edges_v1",
                "target_binding_rule",
                "child_derived",
                "frame-pi-request-v1:",
                "frame-pi-terminal-v1:",
            ),
        ),
        "conformance": (
            conformance,
            (
                "prove_inventory",
                "prove_conditional_authority",
                "space_settings_pro",
                "rust_exact_protected_integration_provider_staging_local_contract",
                "PRAGMA foreign_key_check",
            ),
        ),
        "control": (
            control,
            (
                "dispatch_legacy_protected_integration_workflow_v1",
                "Route::LegacyProtectedIntegration",
                "legacy_protected_integrations_web_runtime::is_server_action",
            ),
        ),
        "routing": (routing, ("LegacyProtectedIntegration",)),
        "registry": (
            registry,
            (
                "LegacyRegistrationSourcesV1::ProtectedIntegration",
                "protected_integration_registration",
                "legacy_protected_integration_profile",
            ),
        ),
        "workflow": (
            workflow,
            (
                "legacy-protected-integrations-sqlite-conformance.py",
                "protected-integrations.json",
                "frame-application --lib legacy_protected_integrations",
                "frame-control-plane --lib legacy_protected_integrations",
            ),
        ),
    }
    for label, (source, tokens) in token_groups.items():
        for token in tokens:
            if token not in source:
                errors.append(f"protected-integration {label} lost semantic token: {token}")
    if (
        "mod legacy_protected_integrations;" not in application_lib
        or "pub use legacy_protected_integrations::*;" not in application_lib
    ):
        errors.append("protected-integration application contract is not exported")
    return errors


def validate_protected_billing_auth_fixture(report: dict[str, Any]) -> list[str]:
    """Bind 16 auth/billing contracts to gates and prove the exact local preflight."""
    try:
        fixture = json.loads(PROTECTED_BILLING_AUTH_FIXTURE.read_text(encoding="utf-8"))
        application = APPLICATION_PROTECTED_BILLING_AUTH.read_text(encoding="utf-8")
        application_lib = APPLICATION_LIB.read_text(encoding="utf-8")
        runtime = CONTROL_PROTECTED_BILLING_AUTH_RUNTIME.read_text(encoding="utf-8")
        ingress = CONTROL_PROTECTED_BILLING_AUTH_WEB_RUNTIME.read_text(encoding="utf-8")
        migration = PROTECTED_BILLING_AUTH_MIGRATION.read_text(encoding="utf-8")
        conformance = PROTECTED_BILLING_AUTH_CONFORMANCE.read_text(encoding="utf-8")
        control = CONTROL_LIB.read_text(encoding="utf-8")
        routing = CONTROL_ROUTING.read_text(encoding="utf-8")
        registry = CONTROL_RUNTIME.read_text(encoding="utf-8")
        workflow = parity_workflow_contract()
        query_names = {
            path.name for path in PROTECTED_BILLING_AUTH_QUERY_ROOT.glob("*.sql")
        }
    except (OSError, json.JSONDecodeError) as error:
        return [f"unable to load protected billing/auth evidence: {error}"]

    errors: list[str] = []
    operations = fixture.get("operations", [])
    fixture_by_id = {
        item.get("id"): item for item in operations if isinstance(item, dict)
    }
    if (
        fixture.get("schema_version") != 1
        or fixture.get("reference", {}).get("commit") != REFERENCE_COMMIT
        or fixture.get("summary")
        != {
            "operation_count": 17,
            "human_and_provider": 14,
            "provider_only": 2,
            "local_exact": 1,
            "local_terminal_behavior": (
                "sixteen_fail_closed_plus_credentialed_cors_preflight"
            ),
        }
        or len(fixture_by_id) != 17
    ):
        errors.append("protected billing/auth fixture identity or summary drifted")

    report_by_id = {row.get("id"): row for row in report.get("entries", [])}
    for identity, adapter in PROTECTED_BILLING_AUTH_LOCAL_ENDPOINT_ADAPTERS.items():
        item = fixture_by_id.get(adapter["id"], {})
        row = report_by_id.get(adapter["id"], {})
        report_sources = {
            (source.get("path"), source.get("symbol"), source.get("sha256"))
            for source in row.get("sources", [])
            if isinstance(source, dict)
        }
        fixture_sources = {
            (source.get("path"), source.get("symbol"), source.get("sha256"))
            for source in item.get("source_manifest", [])
            if isinstance(source, dict)
        }
        expected_security = {
            "max_body_bytes": item.get("max_body_bytes"),
            "accepted_content_types": item.get("accepted_content_types"),
            "rate_limit_bucket": item.get("rate_limit_bucket"),
            "idempotency": item.get("idempotency"),
            "tenant_non_disclosure": item.get("authority") != "public_flow",
        }
        if (
            (row.get("kind"), row.get("method"), row.get("legacy_path")) != identity
            or row.get("auth") != item.get("auth")
            or row.get("policy") != item.get("rate_limit_bucket")
            or row.get("security") != expected_security
            or row.get("implementation", {}).get("local_status")
            != "rust_exact_protected_billing_auth_staging_local_contract"
            or set(row.get("contract_evidence", {}).values()) != {"local_contract"}
            or row.get("completion") != adapter["completion"]
            or not fixture_sources
            or not fixture_sources.issubset(report_sources)
        ):
            errors.append(f"protected billing/auth report evidence drifted: {adapter['id']}")

    if sum(
        item.get("protected_gates") == ["provider_execution"] for item in operations
    ) != 2 or sum(
        item.get("protected_gates") == ["human_approval", "provider_execution"]
        for item in operations
    ) != 14 or sum(
        item.get("protected_gates") == [] for item in operations
    ) != 1:
        errors.append("protected billing/auth evidence-gate partition drifted")
    if fixture.get("local_contract", {}).get("local_exact_operations") != [
        "cap-v1-572763e7b4977abd"
    ]:
        errors.append("protected billing/auth local preflight identity drifted")
    if query_names != {
        "authority_read.sql",
        "receipt_replay.sql",
        "receipt_replay_assert.sql",
        "receipt_insert.sql",
        "outbox_insert.sql",
        "approval_request_insert.sql",
        "delivery_audit_insert.sql",
        "generated_claim_upsert.sql",
        "generated_receipt_replay.sql",
        "workflow_parent_read.sql",
    }:
        errors.append("protected billing/auth checked SQL closure drifted")
    never_implies = fixture.get("local_contract", {}).get("never_implies", [])
    for token in (
        "checkout_success",
        "subscription_success",
        "payment_success",
        "cache_invalidation",
        "upload_capability",
        "video_reprocessing",
    ):
        if token not in never_implies:
            errors.append(f"protected billing/auth fixture lost terminal token: {token}")

    token_groups = {
        "application": (
            application,
            (
                "LEGACY_PROTECTED_BILLING_AUTH_OPERATION_COUNT",
                "LEGACY_PROTECTED_BILLING_AUTH_PROFILES",
                "canonical_stripe_event",
                "local_credentialed_cors_preflight",
                "redact_value",
            ),
        ),
        "runtime": (
            runtime,
            (
                "D1LegacyProtectedBillingAuthRuntimeV1",
                "APPROVAL_REQUEST_INSERT_SQL",
                "batch(statements)",
                "EvidenceRequired",
            ),
        ),
        "ingress": (
            ingress,
            (
                "route_response",
                "server_action_response",
                "workflow_response",
                "add_developer_checkout_cors",
                "STRIPE_WEBHOOK_SECRET",
                "PROTECTED_EXECUTION_EVIDENCE_REQUIRED",
            ),
        ),
        "migration": (
            migration,
            (
                "legacy_protected_billing_auth_receipts_v1",
                "legacy_protected_billing_auth_approval_requests_v1",
                "legacy_protected_billing_auth_human_evidence_v1",
                "legacy_protected_billing_auth_provider_evidence_v1",
            ),
        ),
        "conformance": (
            conformance,
            (
                "validate_fixture",
                "frame_protected_billing_auth_receipt_immutable_v1",
                "PRAGMA foreign_key_check",
            ),
        ),
        "control": (
            control,
            (
                "Route::LegacyProtectedBillingAuth",
                "legacy_protected_billing_auth_web_runtime::server_action_http_response",
                "legacy_protected_billing_auth_web_runtime::workflow_response",
            ),
        ),
        "routing": (routing, ("LegacyProtectedBillingAuth",)),
        "registry": (
            registry,
            (
                "LegacyRegistrationSourcesV1::ProtectedBillingAuth",
                "protected_billing_auth_registration",
                "legacy_protected_billing_auth_profile",
            ),
        ),
        "workflow": (
            workflow,
            (
                "legacy-protected-billing-auth-sqlite-conformance.py",
                "protected-billing-auth.json",
                "frame-application --lib legacy_protected_billing_auth",
                "frame-control-plane --lib legacy_protected_billing_auth",
            ),
        ),
    }
    for label, (source, tokens) in token_groups.items():
        for token in tokens:
            if token not in source:
                errors.append(f"protected billing/auth {label} lost semantic token: {token}")
    if (
        "mod legacy_protected_billing_auth;" not in application_lib
        or "pub use legacy_protected_billing_auth::*;" not in application_lib
    ):
        errors.append("protected billing/auth application contract is not exported")
    return errors


def validate_messenger_retirement_fixture(report: dict[str, Any]) -> list[str]:
    """Prove the local 410 shape without authorizing the protected retirement."""
    try:
        fixture = json.loads(MESSENGER_RETIREMENT_FIXTURE.read_text(encoding="utf-8"))
        application = REGISTRY.read_text(encoding="utf-8")
        business_data = (ROOT / "docs" / "architecture" / "business-data-v1.md").read_text(
            encoding="utf-8"
        )
    except (OSError, json.JSONDecodeError) as error:
        return [f"unable to load messenger-retirement evidence: {error}"]

    errors: list[str] = []
    expected_response = {
        "schema_version": "frame.legacy-retirement.v1",
        "http_status": 410,
        "code": "legacy_operation_retired",
        "message": "This legacy operation has been retired.",
        "migration_path": "privacy-safe export",
        "cache_control": "no-store, max-age=0",
        "retryable": False,
        "tenant_data": "none",
    }
    expected_completion = {
        "local_work": "complete",
        "protected_gates": ["human_approval"],
        "retirement_decision": "repository_owner_pending",
        "production_behavior": "fail_closed_unavailable",
    }
    expected_authority = {
        "application": "crates/application/src/legacy_compatibility.rs",
        "quarantine": "docs/architecture/business-data-v1.md",
        "product_read_write": "forbidden",
        "export": "privacy-safe and separately authorized",
    }
    operations = fixture.get("operations", [])
    fixture_identities = {
        operation.get("id"): operation.get("legacy_identity")
        for operation in operations
        if isinstance(operation, dict)
    }
    if (
        fixture.get("schema_version") != 1
        or fixture.get("family") != "messenger_support.v1"
        or fixture.get("response") != expected_response
        or fixture.get("completion") != expected_completion
        or fixture.get("authority") != expected_authority
        or fixture_identities != MESSENGER_RETIREMENT_IDENTITIES
        or len(operations) != len(MESSENGER_RETIREMENT_IDENTITIES)
    ):
        errors.append("messenger-retirement fixture contract drifted")

    required_application_tokens = (
        'LEGACY_RETIREMENT_SCHEMA_V1: &str = "frame.legacy-retirement.v1"',
        'LEGACY_RETIREMENT_CODE_V1: &str = "legacy_operation_retired"',
        "LegacyRetirementV1::deterministic",
        "http_status: 410",
        "retryable: false",
        "retirement_requires_explicit_approval_and_never_fabricates_frame_success",
    )
    for token in required_application_tokens:
        if token not in application:
            errors.append(f"messenger retirement application proof lost token: {token}")
    if (
        "messenger tables intentionally have no product read or write capability"
        not in business_data
        or "privacy-safe" not in json.dumps(fixture)
    ):
        errors.append("messenger retirement lost its quarantine/export authority")

    report_by_id = {row.get("id"): row for row in report.get("entries", [])}
    expected_report_completion = {
        "decision": "retirement_response_contract_ready",
        **expected_completion,
    }
    for operation_id, identity in MESSENGER_RETIREMENT_IDENTITIES.items():
        row = report_by_id.get(operation_id, {})
        if (
            row.get("kind") != "server_action"
            or row.get("method") != "ACTION"
            or row.get("legacy_path") != identity
            or row.get("policy") != "messenger_support.v1"
            or row.get("disposition") != "retire"
            or row.get("implementation", {}).get("local_status")
            != "rust_retirement_response_complete_owner_approval_pending"
            or row.get("contract_evidence", {}).get("success")
            != "retirement_contract_pending_approval"
            or row.get("completion") != expected_report_completion
            or row.get("deprecation", {}).get("approval") != "repository_owner_pending"
        ):
            errors.append(f"messenger retirement report evidence drifted: {operation_id}")
    return errors


def validate_registry_contract(report: dict[str, Any]) -> list[str]:
    errors: list[str] = []
    try:
        registry = REGISTRY.read_text(encoding="utf-8")
        application_lib = APPLICATION_LIB.read_text(encoding="utf-8")
        workflow = parity_workflow_contract()
    except OSError as error:
        return [f"unable to load central compatibility registry evidence: {error}"]
    required_registry_tokens = (
        "LegacyCompatibilityRegistryV1",
        '../../../fixtures/api-parity/v1/route-workflow-report.json',
        "pinned_registry_is_exhaustive_and_keeps_every_unproven_operation_on_fallback",
            "pinned_report_identifies_only_its_exact_local_contracts",
        "every_retained_row_passes_the_shared_admission_axes_when_endpoint_evidence_is_enabled",
        "every_retained_row_reaches_the_atomic_execution_and_audit_port_boundary",
        "every_inventory_identity_and_raw_http_pattern_resolves_without_decoding",
        "retirement_requires_explicit_approval_and_never_fabricates_frame_success",
        "assert_eq!(registry.len(), 288);",
        "assert_eq!(released_associations, 267);",
    )
    for token in required_registry_tokens:
        if token not in registry:
            errors.append(f"central compatibility registry lost required evidence token: {token}")
    if (
        "mod legacy_compatibility;" not in application_lib
        or "pub use legacy_compatibility::*;" not in application_lib
    ):
        errors.append("central compatibility registry is not exported by frame-application")
    if (
        "mod legacy_notification_preferences;" not in application_lib
        or "pub use legacy_notification_preferences::*;" not in application_lib
    ):
        errors.append("notification preference semantics are not exported by frame-application")
    if "cargo test --locked -p frame-application --lib legacy_compatibility" not in workflow:
        errors.append(
            "API parity workflow does not execute the central compatibility registry suite"
        )

    try:
        control_runtime = CONTROL_RUNTIME.read_text(encoding="utf-8")
        control_lib = CONTROL_LIB.read_text(encoding="utf-8")
        browser_runtime = CONTROL_BROWSER_RUNTIME.read_text(encoding="utf-8")
        control_web_action_runtime = CONTROL_WEB_ACTION_RUNTIME.read_text(encoding="utf-8")
        control_folder_web_runtime = CONTROL_FOLDER_WEB_RUNTIME.read_text(encoding="utf-8")
        control_folder_runtime = CONTROL_FOLDER_RUNTIME.read_text(encoding="utf-8")
        control_routing = CONTROL_ROUTING.read_text(encoding="utf-8")
        application_theme = APPLICATION_THEME.read_text(encoding="utf-8")
        application_folder_assignment = APPLICATION_FOLDER_ASSIGNMENT.read_text(
            encoding="utf-8"
        )
        web_browser_client = WEB_BROWSER_CLIENT.read_text(encoding="utf-8")
        web_hydration = WEB_HYDRATION.read_text(encoding="utf-8")
        organization_repository = ORGANIZATION_REPOSITORY.read_text(encoding="utf-8")
        organization_selection_conformance = ORGANIZATION_SELECTION_CONFORMANCE.read_text(
            encoding="utf-8"
        )
        notification_runtime = NOTIFICATION_RUNTIME.read_text(encoding="utf-8")
        notification_query = NOTIFICATION_QUERY.read_text(encoding="utf-8")
        notification_conformance = NOTIFICATION_CONFORMANCE.read_text(encoding="utf-8")
        execution_migration = EXECUTION_MIGRATION.read_text(encoding="utf-8")
        execution_queries = "\n".join(
            (EXECUTION_QUERY_ROOT / name).read_text(encoding="utf-8")
            for name in (
                "legacy_execution_claim.sql",
                "legacy_execution_intent.sql",
                "legacy_execution_complete.sql",
                "legacy_execution_audit.sql",
                "legacy_execution_load.sql",
            )
        )
        execution_conformance = EXECUTION_CONFORMANCE.read_text(encoding="utf-8")
        folder_assignment_migration = FOLDER_ASSIGNMENT_MIGRATION.read_text(
            encoding="utf-8"
        )
        folder_assignment_queries = "\n".join(
            path.read_text(encoding="utf-8")
            for path in sorted(FOLDER_ASSIGNMENT_QUERY_ROOT.glob("*.sql"))
        )
        folder_assignment_conformance = FOLDER_ASSIGNMENT_CONFORMANCE.read_text(
            encoding="utf-8"
        )
    except OSError as error:
        errors.append(f"unable to load production legacy execution evidence: {error}")
    else:
        required_runtime_tokens = (
            "D1LegacyOperationExecutionPortV1",
            "LegacyCompatibilityTransportV1",
            "const ENABLED_SEMANTIC_ADAPTERS: &[&str] = &[",
            "LEGACY_STATUS_SOURCE_SHA256",
            "LEGACY_MEDIA_SERVER_ROOT_SOURCE_SHA256",
            "LEGACY_CHANGELOG_STATUS_SOURCE_SHA256",
            "LEGACY_CHANGELOG_LATEST_SOURCE_SHA256",
            "LEGACY_CHANGELOG_FEED_SOURCE_SHA256",
            "LEGACY_CHANGELOG_CORS_SOURCE_SHA256",
            "LEGACY_CHANGELOG_FEED_SOURCE_MANIFEST_SHA256",
            "LEGACY_CHANGELOG_FEED_BODY_SHA256",
            "LEGACY_MOBILE_SESSION_CONFIG_ROUTE_SOURCE_SHA256",
            "LEGACY_MOBILE_SESSION_CONFIG_DOMAIN_SOURCE_SHA256",
            "LEGACY_NOTIFICATION_PREFERENCES_ROUTE_SOURCE_SHA256",
            "LEGACY_NOTIFICATION_PREFERENCES_SESSION_SOURCE_SHA256",
            "LEGACY_NOTIFICATION_PREFERENCES_SCHEMA_SOURCE_SHA256",
            "LegacySemanticAdapterV1::PublicStatusOk",
            "LegacySemanticAdapterV1::MediaServerRootMetadata",
            "LegacySemanticAdapterV1::ChangelogStatusGet",
            "LegacySemanticAdapterV1::ChangelogStatusOptions",
            "LegacySemanticAdapterV1::ChangelogFeedGet",
            "LegacySemanticAdapterV1::ChangelogFeedOptions",
            "LegacySemanticAdapterV1::MobileSessionConfigGet",
            "LegacySemanticAdapterV1::NotificationPreferencesGet",
            "ENABLED_EXACT_BUSINESS_ADAPTERS",
            "LOCALLY_PROVEN_EXACT_BUSINESS_ADAPTERS",
            "LEGACY_WEB_ACTIVE_ORGANIZATION_OPERATION_ID",
            "dispatch_web_active_organization_action",
            "LEGACY_WEB_THEME_OPERATION_ID",
            "dispatch_web_theme_action",
            "LEGACY_ADD_VIDEOS_TO_FOLDER_OPERATION_ID",
            "LEGACY_REMOVE_VIDEOS_FROM_FOLDER_OPERATION_ID",
            "LEGACY_MOVE_VIDEO_TO_FOLDER_OPERATION_ID",
            "dispatch_web_folder_assignment_action",
            "SetCookieThenResolveVoid",
            "InvalidateThenResolveVoid",
            "new_static_only",
            "new_static_from_worker_env",
            "dispatch_http_response",
            "static_transport_serves_all_seven_exact_source_pinned_success_contracts",
            "mobile_session_config_binds_all_four_env_states_to_exact_json_and_fingerprints",
            "mobile_session_config_ignores_unmodeled_query_and_origin_without_caller_control",
            "changelog_status_preserves_url_search_params_update_semantics",
            "changelog_feed_preserves_pinned_body_query_and_cors_semantics",
            "exact_static_validation_and_idempotency_retry_axes_are_local",
            "exact_static_authorization_and_failure_axes_fail_closed",
            "ENABLED_SEMANTIC_ADAPTERS.len() + ENABLED_EXACT_BUSINESS_ADAPTERS.len()",
            "assert_eq!(fail_closed, report.entries.len() - promoted);",
            "LegacyExecutionErrorV1::Unsupported",
            "database\n            .batch(statements)",
            "execution_outcome(",
        )
        for token in required_runtime_tokens:
            if token not in control_runtime:
                errors.append(f"legacy control-plane runtime lost required token: {token}")
        if "pub mod legacy_compatibility_runtime;" not in control_lib:
            errors.append("legacy compatibility runtime is not exported by the control plane")
        if "mod legacy_web_action_runtime;" not in control_lib:
            errors.append("legacy web ACTION ingress is not wired into the control plane")
        for token in (
            "mod legacy_folder_assignment_runtime;",
            "mod legacy_folder_web_runtime;",
            "legacy_folder_web_runtime::is_action",
            "legacy_folder_assignment_action_response",
        ):
            if token not in control_lib:
                errors.append(f"legacy folder ACTION control-plane wiring lost token: {token}")
        for token in (
            "AuthenticatedWebCompatibilityAction",
            "/api/v1/web/compatibility-actions/",
        ):
            if token not in control_routing:
                errors.append(f"legacy web ACTION routing lost required token: {token}")
        for token in (
            "authenticate_compatibility_mutation",
            "authenticate_read",
            "authenticate_mutation",
            "same_origin_fetch",
        ):
            if token not in browser_runtime:
                errors.append(f"legacy web ACTION auth boundary lost required token: {token}")
        for token in (
            "WEB_COMPATIBILITY_ACTION_REQUEST_SCHEMA_V1",
            "ACTIVE_ORGANIZATION_ACTION_ID",
            "THEME_ACTION_ID",
            "idempotency-key",
            "exact_action_envelope",
            "dispatch_web_theme_action",
            "dispatch_web_active_organization_action",
            "declared_body_length",
            "read_bounded_legacy_body(request, MAX_ACTION_BODY_BYTES)",
        ):
            if token not in control_web_action_runtime:
                errors.append(f"legacy web ACTION ingress lost required token: {token}")
        for token in (
            "WEB_FOLDER_ASSIGNMENT_REQUEST_SCHEMA_V1",
            "required_nullable_cap_nanoid",
            "header_key.as_deref() != Some(body.idempotency_key.as_str())",
            "trusted_active_organization_id",
            "D1LegacyFolderAssignmentAtomicPortV1::new",
            "dispatch_web_folder_assignment_action",
            "consume_session_grant",
            "read_bounded_legacy_body(request, MAX_ACTION_BODY_BYTES)",
        ):
            if token not in control_folder_web_runtime:
                errors.append(f"legacy folder ACTION ingress lost required token: {token}")
        for token in (
            "LegacyFolderAssignmentAtomicPortV1",
            "LegacyFolderAssignmentRequiredAuthorizationV1",
            "LegacyFolderAssignmentRequiredMutationV1",
            "LegacyFolderAssignmentBrowserFenceV1",
            "released_legacy_client_e2e",
            "SessionActorActiveTenantFolderScopeAndEveryActorOwnedVideo",
            "SessionActorActiveTenantManagerSelectedContextAndTenantVideo",
        ):
            if token not in application_folder_assignment:
                errors.append(f"folder-assignment application contract lost token: {token}")
        for token in (
            "D1LegacyFolderAssignmentAtomicPortV1",
            "PRODUCT_PRECONDITION_SQL",
            "PRODUCT_POSTCONDITION_SQL",
            "BROWSER_MUTATION_GRANT_ASSERT_SQL",
            "BROWSER_MUTATION_GRANT_DELETE_SQL",
            "AUDIT_ACTION",
            "consume_replay",
            "reconcile",
        ):
            if token not in control_folder_runtime:
                errors.append(f"folder-assignment D1 runtime lost token: {token}")
        for token in (
            "LEGACY_WEB_THEME_SOURCES",
            "apps/web/app/(org)/dashboard/Contexts.tsx",
            "LastWriteWinsWithoutClientIdempotency",
            "only_the_two_pinned_theme_values_produce_the_cookie_effect",
        ):
            if token not in application_theme:
                errors.append(f"theme application contract lost required token: {token}")
        for token in (
            "set_theme",
            "set_legacy_active_organization",
            "add_videos_to_folder",
            "remove_videos_from_folder",
            "move_video_to_folder",
            "WEB_FOLDER_ASSIGNMENT_REQUEST_SCHEMA_V1",
            "LEGACY_ADD_VIDEOS_TO_FOLDER_ACTION_ID",
            "LEGACY_REMOVE_VIDEOS_FROM_FOLDER_ACTION_ID",
            "LEGACY_MOVE_VIDEO_TO_FOLDER_ACTION_ID",
            "valid_compatibility_action_path",
            "idempotency_key: None",
            "csrf_protected: true",
        ):
            if token not in web_browser_client:
                errors.append(f"browser compatibility-action client lost required token: {token}")
        for token in (
            "BrowserFencedLegacySelectionRepository",
            "browser_proof_matches_actor",
            "grant_assertion_statement",
            "grant_delete_statement",
            '"grant_consumed"',
        ):
            if token not in organization_repository:
                errors.append(f"browser-fenced selection repository lost token: {token}")
        for token in (
            "test_browser_proof_is_consumed_atomically_with_selection_and_journals",
            "test_stale_or_missing_browser_proof_rolls_back_every_selection_effect",
            "test_denied_target_rolls_back_then_proof_is_consumed_without_mutation",
            '"active_organization_set"',
            "GRANT_ASSERT",
            "GRANT_DELETE",
            "GRANT_CHANGE_ASSERT",
        ):
            if token not in organization_selection_conformance:
                errors.append(f"browser-fenced selection SQLite proof lost token: {token}")
        for token in (
            "browser_theme_cookie",
            "apply_browser_theme",
            "client.set_theme(next)",
        ):
            if token not in web_hydration:
                errors.append(f"theme hydration consumer lost required token: {token}")
        for token in (
            "Route::LegacyApiStatus",
            "Route::LegacyMediaServerRoot",
            "Route::LegacyChangelog",
            "Route::LegacyChangelogStatus",
            "Route::LegacyMobileSessionConfig",
            "Route::LegacyNotificationPreferences",
            "legacy_api_status_response",
            "legacy_media_server_root_response",
            "legacy_changelog_response",
            "legacy_changelog_status_response",
            "legacy_mobile_session_config_response",
            "legacy_notification_preferences_response",
            "LegacyCompatibilityTransportV1::new_static_only",
            "LegacyCompatibilityTransportV1::new_static_from_worker_env",
        ):
            if token not in control_lib:
                errors.append(f"legacy status Worker route lost required token: {token}")
        for token in (
            "LegacyNotificationPreferencesCandidateV1",
            "D1LegacyNotificationPreferencesAuthorityV1",
            'r#"{"error":"Unauthorized"}"#',
            'r#"{"error":"Failed to fetch user preferences"}"#',
            "#[serde(default)]",
            "pause_anon_views",
            "decode_rows",
            "exact_json_body",
            "missing_null_and_schema_invalid_preferences_default_all_flags",
            "valid_shape_defaults_only_missing_anon_and_strips_unknown_fields",
        ):
            if token not in notification_runtime:
                errors.append(
                    f"notification-preferences runtime lost exact contract token: {token}"
                )
        for token in ("SELECT u.preferences_json", "WHERE u.id = ?1", "LIMIT 1"):
            if token not in notification_query:
                errors.append(
                    f"notification-preferences actor query lost required token: {token}"
                )
        for forbidden in (
            "organization_members",
            "active_organization_id",
            "u.status",
            "u.deleted_at_ms",
            "?2",
        ):
            if forbidden in notification_query:
                errors.append(
                    "notification-preferences query added unpinned authority filter: "
                    + forbidden
                )
        for token in (
            "authenticate_host_only_browser_session",
            "authenticate_session_credential",
            "AuthClientKind::Browser",
            "unique_cookie(request, SESSION_COOKIE_NAME)",
        ):
            if token not in browser_runtime:
                errors.append(
                    f"notification-preferences session boundary lost required token: {token}"
                )
        for token in (
            "LegacyNotificationPreferences",
            '"/api/notifications/preferences"',
            "notification preferences lookalike must fail closed",
        ):
            if token not in control_routing:
                errors.append(
                    f"notification-preferences raw route lost required token: {token}"
                )
        for token in (
            "Provider-free proof for the raw notification-preferences actor read.",
            "read_for_actor.sql",
            "valid_source",
            "partial_source",
            "json_valid(preferences_json)",
        ):
            if token not in notification_conformance:
                errors.append(
                    f"notification-preferences SQLite proof lost required token: {token}"
                )
        for token in (
            "legacy_api_execution_operations_v1",
            "legacy_api_execution_intents_v1",
            "legacy_api_execution_audit_v1",
            "legacy_api_execution_operations_v1_transition_guard",
        ):
            if token not in execution_migration:
                errors.append(f"legacy D1 execution migration lost required token: {token}")
        for token in (
            "INSERT OR IGNORE INTO legacy_api_execution_operations_v1",
            "RETURNING reservation_digest",
            "reservation_digest = ?4",
            "request_fingerprint = ?5",
            "LEFT JOIN legacy_api_execution_intents_v1",
            "LEFT JOIN legacy_api_execution_audit_v1",
        ):
            if token not in execution_queries:
                errors.append(f"legacy D1 execution queries lost required token: {token}")
        for token in (
            "legacy_direct_folder_insert_scope_guard_v1",
            "legacy_space_video_insert_scope_guard_v1",
            "legacy_shared_video_insert_scope_guard_v1",
            "legacy_shared_video_one_current_insert_guard_v1",
        ):
            if token not in folder_assignment_migration:
                errors.append(f"folder-assignment migration lost token: {token}")
        for token in (
            "authenticated_web_action_assertions_v1",
            "authenticated_web_action_effects_v1",
            "business_audit_events_v1",
            "json_each(?1)",
            "LIMIT 501",
            "authenticated_web_action_operations_v1",
        ):
            if token not in folder_assignment_queries:
                errors.append(f"folder-assignment SQL closure lost token: {token}")
        for token in (
            "test_full_migration_and_scope_triggers",
            "test_normalized_assignment_storage",
            "test_complete_batch_and_failure_atomicity",
            "test_all_or_nothing_list_authorization",
            "test_replay_conflict_and_race_winner",
            "test_dirty_shared_multiplicity_remains_auditable_and_fails_closed",
            "test_static_redaction_and_business_journal_guards",
        ):
            if token not in folder_assignment_conformance:
                errors.append(f"folder-assignment SQLite proof lost token: {token}")
        for token in (
            '"semantic_adapters_enabled": 8',
            "STATIC_SEMANTIC_ADAPTER_IDS = (",
            '"cap-v1-05b6ba3f76daac22"',
            '"cap-v1-ff19008f47194c43"',
            '"cap-v1-a1b180c5d123c870"',
            '"cap-v1-16668b858461f386"',
            '"cap-v1-0fa8384f3666825b"',
            '"cap-v1-237f41f3086a2d67"',
            '"cap-v1-4f21920a947c4c84"',
            '"cap-v1-d130c840f654bd72"',
            '"inventory_endpoint_success_proven_locally": 37',
            '"production_endpoint_success_enabled": 13',
            "sorted(race_outcomes) != expected_race",
            "conflicting key reuse mutated the durable journal",
        ):
            if token not in execution_conformance:
                errors.append(f"legacy D1 execution conformance lost required token: {token}")
        if (
            "legacy-api-execution-sqlite-conformance.py" not in workflow
            or "legacy-api-execution-sqlite-conformance.json" not in workflow
        ):
            errors.append("API parity workflow does not retain legacy D1 execution evidence")
        if (
            "cargo test --locked -p frame-control-plane --lib "
            "legacy_compatibility_runtime"
        ) not in workflow:
            errors.append("API parity workflow does not compile the legacy transport runtime")
        if (
            "legacy-organization-selection-sqlite-conformance.py" not in workflow
            or "legacy_organization_selection" not in workflow
            or "active-organization-selection.json" not in workflow
        ):
            errors.append(
                "API parity workflow does not retain the active-organization contract proof"
            )
        if (
            "theme-action.json" not in workflow
            or "frame-application --lib legacy_theme" not in workflow
            or "frame-control-plane --lib legacy_web_action_runtime" not in workflow
            or "frame-web --lib browser_authenticated" not in workflow
        ):
            errors.append("API parity workflow does not retain the exact theme ACTION proof")
        if (
            "legacy-folder-assignment-sqlite-conformance.py" not in workflow
            or "legacy-folder-assignment-sqlite-conformance.json" not in workflow
            or "folder-assignment-actions.json" not in workflow
            or "frame-application --lib legacy_folder_assignment" not in workflow
            or "frame-control-plane --lib legacy_folder" not in workflow
        ):
            errors.append(
                "API parity workflow does not retain the exact folder-assignment ACTION proof"
            )
        if (
            "legacy-library-placement-sqlite-conformance.py" not in workflow
            or "legacy-library-placement-sqlite-conformance.json" not in workflow
            or "library-placement-actions.json" not in workflow
            or "frame-application --lib legacy_library_placement" not in workflow
            or "frame-control-plane --lib legacy_library" not in workflow
        ):
            errors.append(
                "API parity workflow does not retain the exact library-placement ACTION proof"
            )
        if (
            "legacy-notification-actions-sqlite-conformance.py" not in workflow
            or "legacy-notification-actions-sqlite-conformance.json" not in workflow
            or "notification-actions.json" not in workflow
            or "frame-application --lib legacy_notification_actions" not in workflow
            or "frame-control-plane --lib legacy_notification" not in workflow
        ):
            errors.append(
                "API parity workflow does not retain the exact notification ACTION proof"
            )
        if (
            "legacy-developer-actions-sqlite-conformance.py" not in workflow
            or "legacy-developer-actions-sqlite-conformance.json" not in workflow
            or "developer-actions.json" not in workflow
            or "frame-application --lib legacy_developer_actions" not in workflow
            or "frame-control-plane --lib legacy_developer" not in workflow
        ):
            errors.append(
                "API parity workflow does not retain the exact developer ACTION proof"
            )
        if (
            "legacy-membership-actions-sqlite-conformance.py" not in workflow
            or "legacy-membership-actions-sqlite-conformance.json" not in workflow
            or "membership-actions.json" not in workflow
            or "frame-application --lib legacy_membership_actions" not in workflow
            or "frame-control-plane --lib legacy_membership" not in workflow
        ):
            errors.append(
                "API parity workflow does not retain the exact membership ACTION proof"
            )
        if "messenger-retirement.json" not in workflow:
            errors.append(
                "API parity workflow does not retain the deterministic messenger retirement proof"
            )
        if (
            "legacy-notification-preferences-sqlite-conformance.py" not in workflow
            or "legacy_notification_preferences" not in workflow
        ):
            errors.append(
                "API parity workflow does not execute the notification-preferences contract proof"
            )
    entries = report.get("entries", [])
    released_clients = {"web", "desktop", "mobile", "extension", "developer"}
    associations = sum(
        client in released_clients for row in entries for client in row.get("clients", [])
    )
    if len(entries) != 288 or associations != 267:
        errors.append("central compatibility registry coverage constants drifted from the report")
    promoted_ids = {
        row.get("id")
        for row in entries
        if row.get("contract_evidence", {}).get("success") == "local_contract"
    }
    expected_promoted_ids = {adapter["id"] for adapter in LOCAL_ENDPOINT_ADAPTERS.values()}
    if promoted_ids != expected_promoted_ids:
        errors.append("endpoint success evidence differs from the explicit semantic adapters")
    return errors


def build_summary(entries: list[dict[str, Any]]) -> dict[str, Any]:
    by_kind = Counter(row["kind"] for row in entries)
    by_disposition = Counter(row["disposition"] for row in entries)
    by_status = Counter(row["implementation"]["local_status"] for row in entries)
    return {
        "total": len(entries),
        "by_kind": dict(sorted(by_kind.items())),
        "by_disposition": dict(sorted(by_disposition.items())),
        "by_local_status": dict(sorted(by_status.items())),
        "endpoint_success_proven": sum(
            row["contract_evidence"]["success"] == "local_contract" for row in entries
        ),
        "endpoint_success_pending": sum(
            row["contract_evidence"]["success"] != "local_contract" for row in entries
        ),
    }


def reference_commit() -> str | None:
    if not (CAP / ".git").exists():
        return None
    result = subprocess.run(
        ["git", "-C", str(CAP), "rev-parse", "HEAD"],
        check=False,
        capture_output=True,
        text=True,
    )
    return result.stdout.strip() if result.returncode == 0 else None


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--generate", action="store_true")
    parser.add_argument("--require-reference", action="store_true")
    args = parser.parse_args()

    commit = reference_commit()
    if (args.generate or args.require_reference) and commit != REFERENCE_COMMIT:
        print(
            f"expected pinned Cap checkout {REFERENCE_COMMIT}, found {commit or 'none'}",
            file=sys.stderr,
        )
        return 1
    compare_reference = commit == REFERENCE_COMMIT

    if args.generate:
        report = build_report()
        operation_contract_catalog = build_operation_contract_catalog(report)
        changelog_feed, _, _ = build_changelog_feed()
        REPORT.parent.mkdir(parents=True, exist_ok=True)
        DOC.parent.mkdir(parents=True, exist_ok=True)
        REPORT.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
        OPERATION_CONTRACT_CATALOG.write_text(
            json.dumps(operation_contract_catalog, indent=2) + "\n", encoding="utf-8"
        )
        CHANGELOG_FEED.write_text(changelog_feed, encoding="utf-8")
        DOC.write_text(render_doc(report), encoding="utf-8")

    try:
        report = json.loads(REPORT.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        print(f"unable to load API parity report: {error}", file=sys.stderr)
        return 1

    errors = validate_report(report, compare_reference=compare_reference)
    try:
        operation_contract_catalog = json.loads(
            OPERATION_CONTRACT_CATALOG.read_text(encoding="utf-8")
        )
    except (OSError, json.JSONDecodeError) as error:
        errors.append(f"unable to load operation contract catalog: {error}")
    else:
        errors.extend(
            validate_operation_contract_catalog(operation_contract_catalog, report)
        )
    errors.extend(validate_changelog_feed(compare_reference=compare_reference))
    errors.extend(validate_theme_action_fixture(report))
    errors.extend(validate_folder_assignment_fixture(report))
    errors.extend(validate_library_placement_fixture(report))
    errors.extend(validate_notification_actions_fixture(report))
    errors.extend(validate_developer_actions_fixture(report))
    errors.extend(validate_membership_actions_fixture(report))
    errors.extend(validate_folder_crud_fixture(report))
    errors.extend(validate_user_account_fixture(report))
    errors.extend(validate_collaboration_fixture(report))
    errors.extend(validate_video_properties_fixture(report))
    errors.extend(validate_library_id_reads_fixture(report))
    errors.extend(validate_library_detail_reads_fixture(report))
    errors.extend(validate_space_authorization_fixture(report))
    errors.extend(validate_invite_lifecycle_fixture(report))
    errors.extend(validate_mobile_session_fixture(report))
    errors.extend(validate_mobile_bootstrap_caps_fixture(report))
    errors.extend(validate_mobile_uploads_fixture(report))
    errors.extend(validate_transcripts_fixture(report))
    errors.extend(validate_developer_api_fixture(report))
    errors.extend(validate_extension_auth_fixture(report))
    errors.extend(validate_extension_instant_fixture(report))
    errors.extend(validate_notification_read_fixture(report))
    errors.extend(validate_declaration_only_dispositions_fixture(report))
    errors.extend(validate_desktop_compatibility_fixture(report))
    errors.extend(validate_desktop_session_fixture(report))
    errors.extend(validate_org_custom_domain_fixture(report))
    errors.extend(validate_video_domain_info_fixture(report))
    errors.extend(validate_video_lifecycle_fixture(report))
    errors.extend(validate_core_and_upload_storage_fixtures(report))
    errors.extend(validate_analytics_fixture(report))
    errors.extend(validate_organization_library_fixture(report))
    errors.extend(validate_protected_media_fixture(report))
    errors.extend(validate_protected_integrations_fixture(report))
    errors.extend(validate_protected_billing_auth_fixture(report))
    errors.extend(validate_messenger_retirement_fixture(report))
    errors.extend(validate_registry_contract(report))
    expected_doc = render_doc(report)
    try:
        actual_doc = DOC.read_text(encoding="utf-8")
    except OSError as error:
        errors.append(f"unable to load generated API documentation: {error}")
    else:
        if actual_doc != expected_doc:
            errors.append("generated API documentation differs from the report")

    if errors:
        print("API/workflow parity validation failed:", file=sys.stderr)
        for error in errors:
            print(f"- {error}", file=sys.stderr)
        return 1

    mode = "with pinned-reference drift comparison" if compare_reference else "offline"
    print(
        f"API/workflow parity validation passed {mode}: "
        f"{report['summary']['total']} entries; "
        f"{report['summary']['endpoint_success_pending']} endpoint/retirement gates remain explicit."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
