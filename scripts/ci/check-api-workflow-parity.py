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
DOC = ROOT / "docs" / "generated" / "api-workflow-parity-v1.md"
REGISTRY = ROOT / "crates" / "application" / "src" / "legacy_compatibility.rs"
APPLICATION_LIB = ROOT / "crates" / "application" / "src" / "lib.rs"
CONTROL_RUNTIME = ROOT / "apps" / "control-plane" / "src" / "legacy_compatibility_runtime.rs"
CONTROL_LIB = ROOT / "apps" / "control-plane" / "src" / "lib.rs"
CONTROL_BROWSER_RUNTIME = ROOT / "apps" / "control-plane" / "src" / "browser_web_runtime.rs"
CONTROL_ROUTING = ROOT / "apps" / "control-plane" / "src" / "routing.rs"
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
LICENSING_DECLARATION_ONLY_ID = "cap-v1-261c3cb23ca88bf9"

# Operation-level transport facts that cannot be inferred from a path or family.
# Every override is tied to the stable identity and the implementation graph below,
# so a broad family rule cannot silently rewrite released-client behavior.
OPERATION_CONTRACT_OVERRIDES: dict[str, dict[str, Any]] = {
    "cap-v1-a3b4c805d409bc7c": {
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
    "cap-v1-9e125712cee9ce5a": {
        "idempotency": "optional",
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
}

OPERATION_COMPLETION_OVERRIDES: dict[str, dict[str, Any]] = {
    "cap-v1-05776c542380771e": {
        "decision": "retain_replace_with_provider_effect",
        "local_work": "exact_image_signing_and_nullable_root_folder_bootstrap_projection_required",
        "protected_gates": ["provider_execution"],
        "retirement_decision": "not_proposed",
        "production_behavior": "fail_closed_unavailable",
    },
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
}

# Endpoint promotion is deliberately identity- and source-pinned. A family
# authority or similar route name is never enough to enter this map.
LOCAL_ENDPOINT_ADAPTERS: dict[tuple[str, str, str], dict[str, Any]] = {
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
        "local_status": "rust_exact_active_organization_action_local_contract_ingress_pending",
        "rust_authority": "frame-application legacy organization selection + control-plane D1 active-only adapter",
        "auth": "session",
        "policy": "organization_library.v1",
        "completion": {
            "decision": "serve_frame_exact_business",
            "local_work": "leptos_server_action_ingress_required",
            "protected_gates": [],
            "retirement_decision": "not_proposed",
            "production_behavior": "fail_closed_unavailable",
        },
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

    if operation_id == LICENSING_DECLARATION_ONLY_ID:
        decision = "dependency_pending"
        local_work = "concrete_licensing_authority_and_exact_adapter_required"
        protected_gates = []
        retirement_decision = "not_proposed"
    elif operation_id in PROVIDER_BACKED_IDS:
        decision = "retain_replace_with_provider_effect"
        local_work = "exact_adapter_and_provider_effect_orchestration_required"
        protected_gates = ["provider_execution"]
        retirement_decision = "not_proposed"
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
            effective_prefix = "" if endpoint.startswith("/commercial/") else prefix
            rows.append(
                raw_row(
                    "route",
                    method,
                    join_path(effective_prefix, endpoint),
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
            prefix = "" if endpoint.startswith("/commercial/") else "/api"
            rows.append(
                raw_row(
                    "route",
                    method,
                    join_path(prefix, endpoint),
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
        local_status = "contract_declarations_only_licensing_authority_pending"
        authority = "no concrete Frame commercial licensing authority"
        evidence_default = "dependency_pending"
        retry_evidence = "dependency_pending"
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
            "tenant_non_disclosure": family not in {"service_misc", "analytics_consent"},
        },
        "contract_evidence": {
            "success": endpoint_success,
            "validation": evidence_default,
            "authorization": evidence_default,
            "idempotency_retry": retry_evidence,
            "failure": evidence_default,
        },
        "completion": (
            OPERATION_COMPLETION_OVERRIDES[operation_id]
            if operation_id in OPERATION_COMPLETION_OVERRIDES
            else adapter["completion"]
            if adapter and "completion" in adapter
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
        effect_cardinality = "at_most_once_per_effect_key"
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
                "safe_transport_retry_without_business_mutation"
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
            "in_flight": "conflict_or_indeterminate_without_resubmission",
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
                    "promotion_authorized": locally_tested and complete,
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
            "endpoint_or_retirement_pending": total - locally_tested,
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
            f"work; {protected_overlap} of those additionally name genuine protected gates, while "
            f"{local_only_pending} are local-only. Protected evidence never converts unfinished "
            "local work into a completed route.",
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
            f"`{LICENSING_DECLARATION_ONLY_ID}` remains provider-free but dependency-pending: its "
            "Cap contract declarations do not establish a concrete Frame commercial licensing "
            "authority.",
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
            "synthesizes an HTTP path. The contract is proven locally, while production remains "
            "fail-closed until a Leptos server-action ingress consumes that effect. The mobile "
            "`PATCH /api/mobile/user/active-organization` row remains unpromoted and provider-gated: "
            "its exact fresh bootstrap still requires provider image signing and nullable-space root "
            "folder semantics. Production fallback availability stays false, so every unpromoted "
            "operation returns a closed unavailable error rather than manufacturing a business "
            "success or a legacy fallback.",
            "",
            "The registry exercises current and previous release decisions for all 267 "
            "release-managed client associations and rejects older releases. This is local "
            "registry evidence, not a released client binary/build. Endpoint success is therefore "
            f"limited to {summary['endpoint_success_proven']} exact contracts (seven static, one "
            "D1 preference read, and one D1 business action); the remaining "
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
        elif "/api/webhooks/media-server/" in row["legacy_path"] and row[
            "implementation"
        ]["local_status"] != (
            "rust_authority_present_issue_28_adapter_or_protected_evidence_pending"
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
            OPERATION_COMPLETION_OVERRIDES[row["id"]]
            if row["id"] in OPERATION_COMPLETION_OVERRIDES
            else adapter["completion"]
            if adapter and "completion" in adapter
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
        if row["id"] == ORGANISATION_SOFT_DELETE_ID and row["completion"] != {
            "decision": "retain_replace_with_provider_effect",
            "local_work": "exact_adapter_and_provider_effect_orchestration_required",
            "protected_gates": ["provider_execution"],
            "retirement_decision": "not_proposed",
            "production_behavior": "fail_closed_unavailable",
        }:
            errors.append(
                f"{label}: organisation soft-delete lost its S3/Tinybird provider gate"
            )
        if (
            row["id"] in PROVIDER_BACKED_IDS
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
                "local_status": "contract_declarations_only_licensing_authority_pending",
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
                "decision": "dependency_pending",
                "local_work": "concrete_licensing_authority_and_exact_adapter_required",
                "protected_gates": [],
                "retirement_decision": "not_proposed",
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


def validate_registry_contract(report: dict[str, Any]) -> list[str]:
    errors: list[str] = []
    try:
        registry = REGISTRY.read_text(encoding="utf-8")
        application_lib = APPLICATION_LIB.read_text(encoding="utf-8")
        workflow = WORKFLOW.read_text(encoding="utf-8")
    except OSError as error:
        return [f"unable to load central compatibility registry evidence: {error}"]
    required_registry_tokens = (
        "LegacyCompatibilityRegistryV1",
        '../../../fixtures/api-parity/v1/route-workflow-report.json',
        "pinned_registry_is_exhaustive_and_keeps_every_unproven_operation_on_fallback",
        "pinned_report_promotes_only_its_exact_contracts",
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
        control_routing = CONTROL_ROUTING.read_text(encoding="utf-8")
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
            "LEGACY_WEB_ACTIVE_ORGANIZATION_OPERATION_ID",
            "dispatch_web_active_organization_action",
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
            "assert_eq!(fail_closed, 279);",
            "LegacyExecutionErrorV1::Unsupported",
            "database\n            .batch(statements)",
            "execution_outcome(",
        )
        for token in required_runtime_tokens:
            if token not in control_runtime:
                errors.append(f"legacy control-plane runtime lost required token: {token}")
        if "pub mod legacy_compatibility_runtime;" not in control_lib:
            errors.append("legacy compatibility runtime is not exported by the control plane")
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
            '"inventory_endpoint_success_promoted": 9',
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
