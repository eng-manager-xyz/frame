use axum::http::StatusCode;
use frame_client::{
    InstantUiErrorCodeV1, InstantUiPhaseV1, InstantUiProgressV1, PublicShareSummary,
    ShareAvailability,
};
use frame_ui::{
    Alert, AlertVariant, AspectRatio, Badge, BadgeVariant, Button, ButtonGroup, ButtonLink,
    ButtonSize, ButtonVariant, Card, EmptyState, FeatureCard, Input, Label, NavigationMenu,
    Progress, STYLESHEET, Select,
};
use leptos::prelude::*;

use crate::authenticated::{RecordingFilter, RouteViewQuery};
use crate::config::{Deployment, RuntimeConfig};
use crate::hydration::{
    AUTHENTICATED_ROOT_ID, HydrationBoundary, PLAYER_HELP_ROOT_ID, PUBLIC_COLLABORATION_ROOT_ID,
    PlayerKeyboardHelp, PublicCollaborationPanel, ROOT_ID,
};
use crate::product::{
    AuthenticatedRoute, AuthenticatedState, RecordingState, ShareView, WorkspaceRole, WorkspaceView,
};

pub const NO_STORE: &str = "no-store";

pub struct Page {
    pub status: StatusCode,
    pub body: String,
    pub cache_control: &'static str,
    pub robots: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignInState {
    Ready,
    Invalid,
    Failed,
}

pub fn landing(config: &RuntimeConfig) -> Page {
    let canonical = format!("{}/", config.public_origin().as_str());
    let body = view! {
        <main id="main" tabindex="-1">
            <NavigationMenu attr:aria-label="Primary">
                <a class="brand" href="/" aria-label="Frame home">
                    <span class="mark" aria-hidden="true">"F"</span>
                    <span>"Frame"</span>
                </a>
                <div class="nav-links">
                    <a href="/login">"Sign in"</a>
                    <a href="https://github.com/eng-manager-xyz/frame" rel="noopener noreferrer">
                        "Source"
                    </a>
                </div>
            </NavigationMenu>
            <section class="hero" aria-labelledby="page-title">
                <p class="eyebrow">"Private recording, built in Rust"</p>
                <h1 id="page-title">"Record locally. Share deliberately."</h1>
                <p class="lede">
                    "Frame is building an accessible recording workflow with a privacy-safe web boundary and native media processing."
                </p>
                <ButtonGroup class="actions">
                    <ButtonLink href="/login">"Open Frame"</ButtonLink>
                    <ButtonLink variant=ButtonVariant::Secondary href="/health/live">"Service health"</ButtonLink>
                </ButtonGroup>
            </section>
            <section class="grid" aria-label="Frame architecture">
                <FeatureCard>
                    <p class="card-label">"Capture"</p>
                    <h2>"Native by default"</h2>
                    <p>"Recording and advanced media work stay in least-privilege native processes."</p>
                </FeatureCard>
                <FeatureCard>
                    <p class="card-label">"Sharing"</p>
                    <h2>"Privacy before metadata"</h2>
                    <p>"Unavailable recordings never disclose titles, thumbnails, storage keys, or signed URLs."</p>
                </FeatureCard>
                <FeatureCard>
                    <p class="card-label">"Access"</p>
                    <h2>"Keyboard-ready shells"</h2>
                    <p>"Every route starts with semantic structure, visible focus, and reduced-motion support."</p>
                </FeatureCard>
            </section>
        </main>
    }
    .to_html();

    Page {
        status: StatusCode::OK,
        body: document(
            "Frame · Private recording, built in Rust",
            "Record locally and share deliberately with Frame.",
            &canonical,
            "index,follow",
            body,
        ),
        // This HTML names one exact hydration-asset closure. Keep it out of
        // intermediary caches until deploys retain old hashed assets or purge
        // cached documents atomically.
        cache_control: NO_STORE,
        robots: if config.deployment() == Deployment::Production {
            "index,follow"
        } else {
            "noindex,nofollow"
        },
    }
}

pub fn login(config: &RuntimeConfig, state: SignInState) -> Page {
    let canonical = format!("{}/login", config.public_origin().as_str());
    let body = view! {
        <main id="main" class="narrow" tabindex="-1">
            <a class="back" href="/">"← Frame home"</a>
            <Card attr:aria-labelledby="page-title">
                <p class="eyebrow">"Authentication boundary"</p>
                <h1 id="page-title">"Sign in on Frame"</h1>
                <p id="signin-help">
                    "Use your workspace email. Frame never accepts session tokens in URLs or hands credentials to another origin."
                </p>
                {match state {
                    SignInState::Ready => view! {
                        <form class="stack" method="post" action="/api/v1/web/auth/login" aria-describedby="signin-help">
                            <Label attr:r#for="email">"Email address"</Label>
                            <Input
                                attr:id="email"
                                attr:name="email"
                                attr:r#type="email"
                                attr:inputmode="email"
                                attr:autocomplete="email"
                                attr:maxlength="254"
                                attr:required=true
                            />
                            <Button  attr:r#type="submit">"Continue securely"</Button>
                        </form>
                    }.into_any(),
                    SignInState::Invalid => view! {
                        <Alert attr:id="signin-error" variant=AlertVariant::Destructive attr:role="alert" attr:tabindex="-1">
                            "Enter a valid email address. Nothing was submitted."
                        </Alert>
                        <form class="stack" method="post" action="/api/v1/web/auth/login" aria-describedby="signin-help signin-error">
                            <Label attr:r#for="email">"Email address"</Label>
                            <Input
                                attr:id="email"
                                attr:name="email"
                                attr:r#type="email"
                                attr:inputmode="email"
                                attr:autocomplete="email"
                                attr:maxlength="254"
                                attr:aria-invalid="true"
                                attr:required=true
                            />
                            <Button  attr:r#type="submit">"Continue securely"</Button>
                        </form>
                    }.into_any(),
                    SignInState::Failed => view! {
                        <Alert variant=AlertVariant::Destructive attr:role="alert" attr:tabindex="-1">
                            "Sign-in is temporarily unavailable. No session was created. Try again later."
                        </Alert>
                        <ButtonLink variant=ButtonVariant::Secondary href="/login">"Try again"</ButtonLink>
                    }.into_any(),
                }}
                <p class="form-help">
                    "New to Frame? " <a href="/signup">"Create an account"</a>
                    " · " <a href="/recovery">"Recover access"</a>
                </p>
            </Card>
        </main>
    }
    .to_html();

    Page {
        status: StatusCode::OK,
        body: document(
            "Sign in · Frame",
            "Sign in to Frame.",
            &canonical,
            "noindex,nofollow",
            body,
        ),
        cache_control: NO_STORE,
        robots: "noindex,nofollow",
    }
}

pub fn recovery(config: &RuntimeConfig, state: SignInState) -> Page {
    let canonical = format!("{}/recovery", config.public_origin().as_str());
    let body = view! {
        <main id="main" class="narrow" tabindex="-1">
            <a class="back" href="/login">"← Return to sign in"</a>
            <Card attr:aria-labelledby="page-title">
                <p class="eyebrow">"Authentication boundary"</p>
                <h1 id="page-title">"Recover Frame access"</h1>
                <p id="recovery-help">
                    "Enter your workspace email. Frame gives the same response whether or not an account exists."
                </p>
                {match state {
                    SignInState::Ready => view! {
                        <form class="stack" method="post" action="/api/v1/web/auth/recovery" aria-describedby="recovery-help">
                            <Label attr:r#for="recovery-email">"Email address"</Label>
                            <Input
                                attr:id="recovery-email"
                                attr:name="email"
                                attr:r#type="email"
                                attr:inputmode="email"
                                attr:autocomplete="email"
                                attr:maxlength="254"
                                attr:required=true
                            />
                            <Button  attr:r#type="submit">"Continue securely"</Button>
                        </form>
                    }.into_any(),
                    SignInState::Invalid => view! {
                        <Alert attr:id="recovery-error" variant=AlertVariant::Destructive attr:role="alert" attr:tabindex="-1">
                            "Enter a valid email address. Nothing was submitted."
                        </Alert>
                        <form class="stack" method="post" action="/api/v1/web/auth/recovery" aria-describedby="recovery-help recovery-error">
                            <Label attr:r#for="recovery-email">"Email address"</Label>
                            <Input
                                attr:id="recovery-email"
                                attr:name="email"
                                attr:r#type="email"
                                attr:inputmode="email"
                                attr:autocomplete="email"
                                attr:maxlength="254"
                                attr:aria-invalid="true"
                                attr:required=true
                            />
                            <Button  attr:r#type="submit">"Continue securely"</Button>
                        </form>
                    }.into_any(),
                    SignInState::Failed => view! {
                        <Alert variant=AlertVariant::Destructive attr:role="alert" attr:tabindex="-1">
                            "Recovery is temporarily unavailable. No account details were disclosed."
                        </Alert>
                        <ButtonLink variant=ButtonVariant::Secondary href="/recovery">"Try again later"</ButtonLink>
                    }.into_any(),
                }}
            </Card>
        </main>
    }
    .to_html();
    Page {
        status: StatusCode::OK,
        body: document(
            "Recover access · Frame",
            "Recover access to Frame.",
            &canonical,
            "noindex,nofollow",
            body,
        ),
        cache_control: NO_STORE,
        robots: "noindex,nofollow",
    }
}

pub fn signup(config: &RuntimeConfig, state: SignInState) -> Page {
    let canonical = format!("{}/signup", config.public_origin().as_str());
    let body = view! {
        <main id="main" class="narrow" tabindex="-1">
            <a class="back" href="/login">"← Sign in instead"</a>
            <Card attr:aria-labelledby="page-title">
                <p class="eyebrow">"Authentication boundary"</p>
                <h1 id="page-title">"Create your Frame account"</h1>
                <p id="signup-help">
                    "Account creation is same-origin, rate-limited, and enumeration-safe. Verification material never appears in a URL."
                </p>
                {match state {
                    SignInState::Ready => view! {
                        <form class="stack" method="post" action="/api/v1/web/auth/signup" aria-describedby="signup-help">
                            <Label attr:r#for="signup-name">"Display name"</Label>
                            <Input attr:id="signup-name" attr:name="display_name" attr:maxlength="120" attr:autocomplete="name" attr:required=true/>
                            <Label attr:r#for="signup-email">"Email address"</Label>
                            <Input attr:id="signup-email" attr:name="email" attr:r#type="email" attr:inputmode="email" attr:autocomplete="email" attr:maxlength="254" attr:required=true/>
                            <Button  attr:r#type="submit">"Create account securely"</Button>
                        </form>
                    }.into_any(),
                    SignInState::Invalid => view! {
                        <Alert variant=AlertVariant::Destructive attr:role="alert" attr:tabindex="-1">
                            "Check the highlighted account details. Nothing was submitted."
                        </Alert>
                        <ButtonLink variant=ButtonVariant::Secondary href="/signup">"Try again"</ButtonLink>
                    }.into_any(),
                    SignInState::Failed => view! {
                        <Alert variant=AlertVariant::Destructive attr:role="alert" attr:tabindex="-1">
                            "Account creation is temporarily unavailable. No partial account is shown."
                        </Alert>
                        <ButtonLink variant=ButtonVariant::Secondary href="/signup">"Try again later"</ButtonLink>
                    }.into_any(),
                }}
            </Card>
        </main>
    }
    .to_html();
    Page {
        status: StatusCode::OK,
        body: document(
            "Create account · Frame",
            "Create a Frame account.",
            &canonical,
            "noindex,nofollow",
            body,
        ),
        cache_control: NO_STORE,
        robots: "noindex,nofollow",
    }
}

pub fn verify(config: &RuntimeConfig, state: SignInState) -> Page {
    let canonical = format!("{}/verify", config.public_origin().as_str());
    let body = view! {
        <main id="main" class="narrow" tabindex="-1">
            <a class="back" href="/login">"← Start sign in again"</a>
            <Card attr:aria-labelledby="page-title">
                <p class="eyebrow">"Verification boundary"</p>
                <h1 id="page-title">"Enter your one-time code"</h1>
                <p id="verify-help">
                    "Use the code from your verification message. The pending challenge is held in a secure same-origin cookie, not this page or URL."
                </p>
                {match state {
                    SignInState::Ready => view! {
                        <form class="stack" method="post" action="/api/v1/web/auth/verify" aria-describedby="verify-help">
                            <Label attr:r#for="otp">"Six-digit code"</Label>
                            <Input
                                attr:id="otp"
                                attr:name="otp"
                                attr:inputmode="numeric"
                                attr:autocomplete="one-time-code"
                                attr:minlength="6"
                                attr:maxlength="6"
                                attr:pattern="[0-9]{6}"
                                attr:required=true
                            />
                            <Button  attr:r#type="submit">"Verify securely"</Button>
                        </form>
                    }.into_any(),
                    SignInState::Invalid => view! {
                        <Alert variant=AlertVariant::Destructive attr:role="alert" attr:tabindex="-1">
                            "The code must contain exactly six digits. No verification was attempted."
                        </Alert>
                        <ButtonLink variant=ButtonVariant::Secondary href="/verify">"Try the code again"</ButtonLink>
                    }.into_any(),
                    SignInState::Failed => view! {
                        <Alert variant=AlertVariant::Destructive attr:role="alert" attr:tabindex="-1">
                            "Verification is unavailable or the code cannot be accepted. Start sign in again."
                        </Alert>
                        <ButtonLink variant=ButtonVariant::Secondary href="/login">"Restart sign in"</ButtonLink>
                    }.into_any(),
                }}
            </Card>
        </main>
    }
    .to_html();
    Page {
        status: StatusCode::OK,
        body: document(
            "Verify sign in · Frame",
            "Verify your Frame sign in.",
            &canonical,
            "noindex,nofollow",
            body,
        ),
        cache_control: NO_STORE,
        robots: "noindex,nofollow",
    }
}

#[cfg(test)]
pub fn authenticated(
    config: &RuntimeConfig,
    route: AuthenticatedRoute,
    state: AuthenticatedState,
) -> Page {
    authenticated_at(
        config,
        route,
        state,
        route.path(),
        &RouteViewQuery::default(),
    )
}

pub fn authenticated_at(
    config: &RuntimeConfig,
    route: AuthenticatedRoute,
    state: AuthenticatedState,
    canonical_path: &str,
    query: &RouteViewQuery,
) -> Page {
    let canonical = format!("{}{canonical_path}", config.public_origin().as_str());
    let browser_loader = config.deployment() != Deployment::Local;
    let state = match state {
        AuthenticatedState::Ready(workspace) if !route.permitted_for(workspace.role) => {
            AuthenticatedState::Denied
        }
        state => state,
    };
    let (status, content) = match state {
        AuthenticatedState::Loading => (
            StatusCode::ACCEPTED,
            private_status_shell(
                "Loading workspace",
                "Private workspace data remains hidden while the same-origin session is checked.",
                "status",
            ),
        ),
        AuthenticatedState::Unauthenticated => (
            StatusCode::UNAUTHORIZED,
            private_status_shell(
                "Sign in required",
                "Your private workspace remains hidden until same-origin authentication succeeds.",
                "alert",
            ),
        ),
        AuthenticatedState::Denied => (
            StatusCode::FORBIDDEN,
            private_status_shell(
                "Access denied",
                "Your workspace role does not allow this action. No resource details are available.",
                "alert",
            ),
        ),
        AuthenticatedState::Failed => (
            StatusCode::SERVICE_UNAVAILABLE,
            private_status_shell(
                "Workspace unavailable",
                "Frame could not load the workspace. Retry without resubmitting any change.",
                "alert",
            ),
        ),
        AuthenticatedState::Ready(workspace) => {
            (StatusCode::OK, workspace_shell(route, &workspace, query))
        }
    };
    let body = view! {
        <main
            id="main"
            class="workspace-page"
            tabindex="-1"
        >
            <div
                id=AUTHENTICATED_ROOT_ID
                data-frame-authenticated-surface=route.name()
                data-frame-browser-loader=browser_loader.then_some("true")
            >
                {content}
            </div>
        </main>
    }
    .to_html();

    Page {
        status,
        body: themed_private_document(
            &format!("{} · Frame", route.label()),
            "Private Frame workspace.",
            &canonical,
            "noindex,nofollow",
            body,
            query.theme().as_str(),
        ),
        cache_control: NO_STORE,
        robots: "noindex,nofollow",
    }
}

pub fn share(config: &RuntimeConfig, video_id: &str, view: ShareView) -> Page {
    let view = if view.matches_route(video_id) {
        view
    } else {
        ShareView::Unavailable
    };
    let fallback_canonical = format!(
        "{}/s/{}",
        config.public_origin().as_str(),
        if view.availability() == ShareAvailability::Unavailable {
            "unavailable"
        } else {
            safe_id(video_id)
        }
    );
    let (status, title, description, canonical, cache, robots, content, open_graph) = match view {
        ShareView::Validated(summary) if summary.availability == ShareAvailability::Public => {
            let title = summary
                .title
                .clone()
                .unwrap_or_else(|| "Shared recording".into());
            let description = summary
                .description
                .clone()
                .unwrap_or_else(|| "A public recording shared with Frame.".into());
            let canonical = summary
                .canonical_url
                .clone()
                .unwrap_or_else(|| fallback_canonical.clone());
            let content = public_player_shell(&summary, None);
            (
                StatusCode::OK,
                format!("{title} · Frame"),
                description,
                canonical,
                NO_STORE,
                "index,follow",
                content,
                true,
            )
        }
        ShareView::Validated(summary) if summary.availability == ShareAvailability::Processing => {
            let content = processing_status_shell(summary.processing_status.as_ref());
            (
                StatusCode::ACCEPTED,
                "Recording processing · Frame".into(),
                "This recording is still processing.".into(),
                summary
                    .canonical_url
                    .unwrap_or_else(|| fallback_canonical.clone()),
                NO_STORE,
                "noindex,nofollow",
                content,
                false,
            )
        }
        _ => {
            let (status, title, description, cache, robots, content) = unavailable_share();
            (
                status,
                title.into(),
                description.into(),
                fallback_canonical.clone(),
                cache,
                robots,
                content,
                false,
            )
        }
    };

    let body = view! {
        <main id="main" class="player-page" tabindex="-1">
            <NavigationMenu attr:aria-label="Share navigation">
                <a class="brand" href="/" aria-label="Frame home">
                    <span class="mark" aria-hidden="true">"F"</span>
                    <span>"Frame"</span>
                </a>
            </NavigationMenu>
            {content}
        </main>
    }
    .to_html();

    Page {
        status,
        body: if open_graph {
            public_document(&title, &description, &canonical, robots, body)
        } else {
            document(&title, &description, &canonical, robots, body)
        },
        cache_control: cache,
        robots,
    }
}

pub fn embed(config: &RuntimeConfig, video_id: &str, share: ShareView) -> Page {
    let share = if share.matches_route(video_id) {
        share
    } else {
        ShareView::Unavailable
    };
    let public = matches!(
        &share,
        ShareView::Validated(summary) if summary.availability == ShareAvailability::Public
    );
    let canonical_id = if !config.embed_policy().enabled() || !public {
        "unavailable"
    } else {
        safe_id(video_id)
    };
    let unavailable_canonical = format!("{}/embed/{canonical_id}", config.public_origin().as_str());
    if !config.embed_policy().enabled() || !public {
        return unavailable_embed(&unavailable_canonical);
    }

    let ShareView::Validated(summary) = share else {
        return unavailable_embed(&unavailable_canonical);
    };
    let canonical = summary
        .canonical_url
        .clone()
        .unwrap_or_else(|| format!("{}/s/{canonical_id}", config.public_origin().as_str()));
    let embed_origins = config
        .embed_policy()
        .ancestors()
        .iter()
        .map(|origin| origin.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    let body = view! {
        <main id="main" class="embed-page" tabindex="-1">
            {public_player_shell(
                &summary,
                Some((canonical_id.to_owned(), embed_origins)),
            )}
        </main>
    }
    .to_html();
    Page {
        status: StatusCode::OK,
        body: document(
            "Shared recording · Frame",
            "An embedded public Frame recording.",
            &canonical,
            "noindex,follow",
            body,
        ),
        cache_control: NO_STORE,
        robots: "noindex,follow",
    }
}

fn unavailable_embed(canonical: &str) -> Page {
    let body = view! {
        <main id="main" class="embed-page" tabindex="-1">
            <Card attr:aria-labelledby="page-title">
                <h1 id="page-title">"Embedded playback unavailable"</h1>
                <p>"No recording metadata or storage location is available in this response."</p>
            </Card>
        </main>
    }
    .to_html();
    Page {
        status: StatusCode::NOT_FOUND,
        body: document(
            "Playback unavailable · Frame",
            "Playback is unavailable.",
            canonical,
            "noindex,nofollow",
            body,
        ),
        cache_control: NO_STORE,
        robots: "noindex,nofollow",
    }
}

pub fn not_found(config: &RuntimeConfig) -> Page {
    let canonical = format!("{}/", config.public_origin().as_str());
    let body = view! {
        <main id="main" class="narrow" tabindex="-1">
            <Card attr:aria-labelledby="page-title">
                <p class="eyebrow">"404"</p>
                <h1 id="page-title">"Page not found"</h1>
                <p>"The requested Frame page is unavailable."</p>
                <ButtonLink href="/">"Frame home"</ButtonLink>
            </Card>
        </main>
    }
    .to_html();
    Page {
        status: StatusCode::NOT_FOUND,
        body: document(
            "Page not found · Frame",
            "The requested page is unavailable.",
            &canonical,
            "noindex,nofollow",
            body,
        ),
        cache_control: NO_STORE,
        robots: "noindex,nofollow",
    }
}

fn unavailable_share() -> (
    StatusCode,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    AnyView,
) {
    (
        StatusCode::NOT_FOUND,
        "Recording unavailable · Frame",
        "This recording is unavailable.",
        NO_STORE,
        "noindex,nofollow",
        status_shell(
            "Recording unavailable",
            "This recording cannot be viewed. No additional details are available.",
        ),
    )
}

fn private_status_shell(label: &'static str, message: &'static str, role: &'static str) -> AnyView {
    view! {
        <div class="narrow private-boundary">
            <a class="back" href="/">"← Frame home"</a>
            <Card attr:aria-labelledby="page-title">
                <p class="eyebrow">"Private workspace"</p>
                <h1 id="page-title">{label}</h1>
                <p>
                    "No account, tenant, recording, developer, admin, or billing data is rendered into this response."
                </p>
                <Alert attr:role=role>{message}</Alert>
                <ButtonLink href="/login">"Go to sign in"</ButtonLink>
            </Card>
        </div>
    }
    .into_any()
}

fn workspace_shell(
    route: AuthenticatedRoute,
    workspace: &WorkspaceView,
    query: &RouteViewQuery,
) -> AnyView {
    let navigation = AuthenticatedRoute::NAVIGATION
        .into_iter()
        .filter(|candidate| candidate.permitted_for(workspace.role))
        .map(|candidate| {
            let current = candidate == route.navigation_parent();
            view! {
                <li>
                    <a
                        href=candidate.path()
                        aria-current=current.then_some("page")
                    >
                        {candidate.label()}
                    </a>
                </li>
            }
        })
        .collect_view();
    let content = surface_content(route, workspace, query);
    let member_label = workspace.member_label.clone();
    let organization_name = workspace.organization_name.clone();
    let role_label = workspace.role.label();

    view! {
        <header class="workspace-header">
            <a class="brand" href="/dashboard" aria-label="Frame dashboard">
                <span class="mark" aria-hidden="true">"F"</span>
                <span>"Frame"</span>
            </a>
            <div class="session-summary">
                <span>{member_label}</span>
                <Badge variant=BadgeVariant::Outline class="role-badge">{role_label}</Badge>
            </div>
        </header>
        <div class="workspace-layout">
            <NavigationMenu class="workspace-nav" attr:aria-label="Workspace">
                <p class="workspace-name">{organization_name}</p>
                <ul>{navigation}</ul>
                <a href="/login">"Go to sign in"</a>
            </NavigationMenu>
            <section class="workspace-content" aria-labelledby="page-title">
                <p class="eyebrow">"Private workspace"</p>
                <h1 id="page-title">{route.label()}</h1>
                {content}
            </section>
        </div>
    }
    .into_any()
}

fn surface_content(
    route: AuthenticatedRoute,
    workspace: &WorkspaceView,
    query: &RouteViewQuery,
) -> AnyView {
    match route {
        AuthenticatedRoute::Dashboard | AuthenticatedRoute::Library => {
            recording_library(workspace, query)
        }
        AuthenticatedRoute::Imports => import_surface(workspace),
        AuthenticatedRoute::Spaces | AuthenticatedRoute::Folders => {
            collection_surface(route, workspace.role)
        }
        AuthenticatedRoute::Space | AuthenticatedRoute::Folder => detail_surface(route),
        AuthenticatedRoute::Onboarding => form_surface(
            "onboarding-title",
            "Complete onboarding",
            "Workspace details are validated before the server creates or selects an organization.",
            "/onboarding",
            "Workspace name",
            "workspace_name",
            true,
        ),
        AuthenticatedRoute::Settings => settings_index(workspace),
        AuthenticatedRoute::AccountSettings => form_surface(
            "account-title",
            "Account profile",
            "Unsaved changes are announced before navigation. Session revocation requires a separate fresh authorization.",
            "/settings/account",
            "Display name",
            "display_name",
            false,
        ),
        AuthenticatedRoute::OrganizationSettings => form_surface(
            "organization-title",
            "Organization settings",
            "Organization policy changes use an expected revision and a server-side role decision.",
            "/settings/organization",
            "Organization name",
            "organization_name",
            false,
        ),
        AuthenticatedRoute::MemberSettings => restricted_surface(
            "Members and invites",
            "Member rows, invitations, seat state, and role changes remain server-authorized. Unknown and forbidden identities are indistinguishable.",
        ),
        AuthenticatedRoute::StorageSettings => restricted_surface(
            "Storage integrations",
            "Provider credentials never enter rendered HTML. Connection, verification, rollback, and deletion are separate audited actions.",
        ),
        AuthenticatedRoute::Analytics => {
            let organization_name = workspace.organization_name.clone();
            let role_label = workspace.role.label();
            view! {
                <Card attr:aria-labelledby="settings-title">
                    <h2 id="settings-title">"Usage analytics"</h2>
                    <dl class="detail-list">
                        <div><dt>"Workspace"</dt><dd>{organization_name}</dd></div>
                        <div><dt>"Your role"</dt><dd>{role_label}</dd></div>
                    </dl>
                    <p>"Product telemetry remains off until a recorded consent decision exists. Operational measurements contain no private titles or identities."</p>
                </Card>
            }
            .into_any()
        }
        AuthenticatedRoute::Developer => restricted_surface(
            "Developer access",
            "API keys are never rendered in this SSR fixture. New secrets must be shown once, after a CSRF-protected action.",
        ),
        AuthenticatedRoute::Billing => restricted_surface(
            "Billing",
            "Billing details remain server-authorized and are never inferred from client-visible role labels.",
        ),
        AuthenticatedRoute::Admin => restricted_surface(
            "Administration",
            "Administrative controls require a fresh server-side authorization decision for every action.",
        ),
    }
}

fn collection_surface(route: AuthenticatedRoute, role: WorkspaceRole) -> AnyView {
    let singular = if route == AuthenticatedRoute::Spaces {
        "space"
    } else {
        "folder"
    };
    let can_create = matches!(role, WorkspaceRole::Owner | WorkspaceRole::Admin);
    view! {
        <EmptyState class="empty-state" attr:aria-labelledby="collection-title">
            <h2 id="collection-title">{format!("No {singular}s yet")}</h2>
            <p>
                "No optimistic resource is shown before the server accepts a tenant-scoped create command."
            </p>
            {can_create.then(|| view! {
                <Button
                    variant=ButtonVariant::Secondary
                    attr:r#type="button"
                    attr:disabled=true
                    attr:aria-describedby="collection-action-status"
                >
                    {format!("Create {singular}")}
                </Button>
            })}
            <p id="collection-action-status" class="form-help">
                "Creation remains disabled until the CSRF-protected typed action adapter is available."
            </p>
        </EmptyState>
    }
    .into_any()
}

fn detail_surface(route: AuthenticatedRoute) -> AnyView {
    let label = route.label().to_lowercase();
    view! {
        <Card attr:aria-labelledby="detail-title">
            <h2 id="detail-title">{format!("{label} details")}</h2>
            <Alert attr:role="status">
                "The local fixture proves the routed, authorized detail boundary. Production identifiers and resources load only through the typed tenant-scoped API."
            </Alert>
        </Card>
    }
    .into_any()
}

fn settings_index(workspace: &WorkspaceView) -> AnyView {
    let organization_name = workspace.organization_name.clone();
    let role_label = workspace.role.label();
    let show_organization = AuthenticatedRoute::OrganizationSettings.permitted_for(workspace.role);
    let show_members = AuthenticatedRoute::MemberSettings.permitted_for(workspace.role);
    let show_storage = AuthenticatedRoute::StorageSettings.permitted_for(workspace.role);
    view! {
        <Card attr:aria-labelledby="settings-title">
            <h2 id="settings-title">"Settings surfaces"</h2>
            <dl class="detail-list">
                <div><dt>"Workspace"</dt><dd>{organization_name}</dd></div>
                <div><dt>"Your role"</dt><dd>{role_label}</dd></div>
            </dl>
            <ul class="settings-links">
                <li><a href="/settings/account">"Account"</a></li>
                {show_organization.then(|| view! {
                    <li><a href="/settings/organization">"Organization"</a></li>
                })}
                {show_members.then(|| view! {
                    <li><a href="/settings/members">"Members"</a></li>
                })}
                {show_storage.then(|| view! {
                    <li><a href="/settings/storage">"Storage"</a></li>
                })}
            </ul>
        </Card>
    }
    .into_any()
}

#[allow(clippy::too_many_arguments)]
fn form_surface(
    title_id: &'static str,
    title: &'static str,
    description: &'static str,
    action: &'static str,
    field_label: &'static str,
    field_name: &'static str,
    required: bool,
) -> AnyView {
    view! {
        <Card attr:aria-labelledby=title_id>
            <h2 id=title_id>{title}</h2>
            <p id="form-contract-help">{description}</p>
            <form
                class="stack"
                method="post"
                action=action
                data-form-contract="revision-fenced-v1"
                data-unsaved-guard="required"
                aria-describedby="form-contract-help form-authority-status"
            >
                <Label attr:r#for=field_name>{field_label}</Label>
                <Input
                    attr:id=field_name
                    attr:name=field_name
                    attr:maxlength="120"
                    attr:autocomplete="off"
                    attr:required=required
                    attr:disabled=true
                />
                <Button  attr:r#type="submit" attr:disabled=true>"Save changes"</Button>
            </form>
            <Alert attr:id="form-authority-status" attr:role="status">
                "The form contract covers validation, pending state, duplicate suppression, retry, stale completion, and unsaved changes. Submission stays disabled until the server action adapter is connected."
            </Alert>
        </Card>
    }
    .into_any()
}

fn restricted_surface(title: &'static str, message: &'static str) -> AnyView {
    view! {
        <Card attr:aria-labelledby="restricted-title">
            <h2 id="restricted-title">{title}</h2>
            <Alert attr:role="status">{message}</Alert>
        </Card>
    }
    .into_any()
}

fn recording_library(workspace: &WorkspaceView, query: &RouteViewQuery) -> AnyView {
    let search_value = query.search().unwrap_or_default().to_owned();
    let active_filter = query.filter();
    let filtered = workspace
        .recordings
        .iter()
        .filter(|recording| {
            query.search().is_none_or(|search| {
                recording
                    .title
                    .to_lowercase()
                    .contains(&search.to_lowercase())
            })
        })
        .filter(|recording| match query.filter() {
            RecordingFilter::All => true,
            RecordingFilter::Ready => recording.state == RecordingState::Ready,
            RecordingFilter::Processing => recording.state == RecordingState::Processing,
            RecordingFilter::Failed => recording.state == RecordingState::Failed,
        })
        .collect::<Vec<_>>();
    let empty = filtered.is_empty();
    let recordings = filtered
        .into_iter()
        .map(|recording| {
            let identifier = safe_id(&recording.public_id).to_owned();
            let ready = recording.state == RecordingState::Ready && identifier != "unavailable";
            let state = recording.state;
            let title = recording.title.clone();
            let duration_label = recording.duration_label.clone();
            let state_class = match state {
                RecordingState::Ready => "state ready",
                RecordingState::Processing => "state processing",
                RecordingState::Failed => "state failed",
            };
            let state_variant = match state {
                RecordingState::Ready => BadgeVariant::Success,
                RecordingState::Processing => BadgeVariant::Secondary,
                RecordingState::Failed => BadgeVariant::Destructive,
            };
            view! {
                <li class="recording-row">
                    <div>
                        <h3>{title}</h3>
                        <p>
                            <Badge variant=state_variant class=state_class>{state.label()}</Badge>
                            {duration_label.map(|duration| {
                                view! { <span class="duration">{duration}</span> }
                            })}
                        </p>
                    </div>
                    {ready.then(|| view! {
                        <ButtonLink variant=ButtonVariant::Secondary size=ButtonSize::Small href=format!("/s/{identifier}")>
                            "Open share"
                        </ButtonLink>
                    })}
                </li>
            }
        })
        .collect_view();

    view! {
        <form class="search-form" method="get" action="/library" role="search">
            <Label attr:r#for="recording-search">"Search recordings"</Label>
            <div>
                <Input
                    attr:id="recording-search"
                    attr:name="q"
                    attr:r#type="search"
                    attr:maxlength="120"
                    attr:autocomplete="off"
                    attr:value=search_value
                />
                <Label class="visually-hidden" attr:r#for="recording-filter">"Filter by status"</Label>
                <Select attr:id="recording-filter" attr:name="filter">
                    <option value="all" selected={active_filter == RecordingFilter::All}>"All statuses"</option>
                    <option value="ready" selected={active_filter == RecordingFilter::Ready}>"Ready"</option>
                    <option value="processing" selected={active_filter == RecordingFilter::Processing}>"Processing"</option>
                    <option value="failed" selected={active_filter == RecordingFilter::Failed}>"Needs attention"</option>
                </Select>
                <Button  attr:r#type="submit">"Search"</Button>
            </div>
            <Input attr:r#type="hidden" attr:name="page" attr:value="1"/>
        </form>
        {if empty {
            view! {
                <EmptyState class="empty-state" attr:aria-labelledby="empty-title">
                    <h2 id="empty-title">"No recordings match"</h2>
                    <p>"Clear search and filters, record in the desktop app, or begin an authorized import."</p>
                </EmptyState>
            }.into_any()
        } else {
            view! {
                <section aria-labelledby="recordings-title">
                    <h2 id="recordings-title">"Recent recordings"</h2>
                    <ul class="recording-list">{recordings}</ul>
                </section>
            }.into_any()
        }}
    }
    .into_any()
}

fn import_surface(workspace: &WorkspaceView) -> AnyView {
    let Some(import) = workspace.import.as_ref() else {
        return view! {
            <EmptyState class="empty-state" attr:aria-labelledby="imports-title">
                <h2 id="imports-title">"No import in progress"</h2>
                <p>"Completed and quarantined imports will appear only after a server-authorized load."</p>
            </EmptyState>
        }
        .into_any();
    };
    let percent = import.percent();
    let label = import.label.clone();
    let completed = import.completed.min(import.total);
    let total = import.total;
    view! {
        <Card attr:aria-labelledby="imports-title">
            <h2 id="imports-title">{label}</h2>
            <p id="import-progress-label">
                {format!("{completed} of {total} objects verified ({percent}%)")}
            </p>
            <Progress
                attr:max="100"
                attr:value=percent
                attr:aria-labelledby="import-progress-label"
            >{format!("{percent}%")}</Progress>
            <p>"Refresh is safe: progress is read from a durable checkpoint, not inferred in the browser."</p>
        </Card>
    }
    .into_any()
}

fn public_player_shell(summary: &PublicShareSummary, embed: Option<(String, String)>) -> AnyView {
    let title = summary
        .title
        .clone()
        .unwrap_or_else(|| "Shared recording".into());
    let description = summary.description.clone();
    let duration = summary.duration_ms.map(format_duration);
    let Some(playback) = summary.playback.clone() else {
        return status_shell("Recording unavailable", "Playback is unavailable.");
    };
    let caption_tracks = playback
        .captions
        .iter()
        .map(|caption| {
            view! {
                <track
                    kind="captions"
                    src=caption.path.clone()
                    srclang=caption.language.clone()
                    label=caption.label.clone()
                    default=caption.default
                />
            }
        })
        .collect_view();
    let caption_labels = playback
        .captions
        .iter()
        .map(|caption| view! { <li>{caption.label.clone()}</li> })
        .collect_view();
    let transcript_links = playback
        .captions
        .iter()
        .map(|caption| {
            view! {
                <li>
                    <a href=caption.path.clone()>
                        {format!("{} transcript (WebVTT)", caption.label)}
                    </a>
                </li>
            }
        })
        .collect_view();
    let (embed_share, embed_origins) = embed
        .map(|(share, origins)| (Some(share), Some(origins)))
        .unwrap_or((None, None));
    let collaboration_share = summary
        .canonical_url
        .as_deref()
        .and_then(|value| value.strip_prefix("/s/"))
        .filter(|value| !value.is_empty() && !value.contains('/'))
        .map(str::to_owned);

    view! {
        <FeatureCard class="player-shell" attr:aria-labelledby="page-title">
            <p class="eyebrow">"Shared recording"</p>
            <h1 id="page-title">{title.clone()}</h1>
            {description.map(|description| view! { <p class="lede compact-lede">{description}</p> })}
            {duration.map(|duration| view! { <p class="duration-summary">{duration}</p> })}
            <AspectRatio class="video-frame">
                <video
                    id="frame-public-player"
                    controls
                    playsinline
                    preload="metadata"
                    controlslist="nodownload noremoteplayback"
                    disableremoteplayback
                    data-allow-fullscreen="true"
                    data-allow-picture-in-picture="true"
                    aria-describedby="player-privacy-description"
                    aria-label=format!("Video: {title}")
                >
                    <source src=playback.path.clone() type=playback.content_type.clone()/>
                    {caption_tracks}
                    "Your browser does not support HTML video."
                </video>
            </AspectRatio>
            <div class="player-grid">
                <section aria-labelledby="captions-title">
                    <h2 id="captions-title">"Captions"</h2>
                    {if playback.captions.is_empty() {
                        view! { <p>"No caption track is available."</p> }.into_any()
                    } else {
                        view! { <ul>{caption_labels}</ul> }.into_any()
                    }}
                </section>
                <section aria-labelledby="privacy-title">
                    <h2 id="privacy-title">"Privacy"</h2>
                    <p id="player-privacy-description">"Analytics stay off unless a separate, same-share consent flow records a choice. This page does not fingerprint the browser or infer consent."</p>
                </section>
                <section aria-labelledby="transcript-title">
                    <h2 id="transcript-title">"Transcript"</h2>
                    {if playback.captions.is_empty() {
                        view! { <p>"No transcript is available."</p> }.into_any()
                    } else {
                        view! { <ul>{transcript_links}</ul> }.into_any()
                    }}
                </section>
                <section aria-labelledby="comments-title">
                    <h2 id="comments-title">"Comments"</h2>
                    <p>"Comments appear only after the same-origin collaboration service authorizes this exact share. No comment mutation is attempted by the server-rendered fallback."</p>
                </section>
            </div>
            <p class="player-help">
                "Playback and caption paths come from a validated provider-neutral public descriptor. Storage keys and signed provider URLs are never rendered."
            </p>
            <div
                id=PUBLIC_COLLABORATION_ROOT_ID
                data-frame-hydration-scope="interaction-island"
                data-frame-public-share=collaboration_share
            >
                <PublicCollaborationPanel/>
            </div>
            <div
                id=PLAYER_HELP_ROOT_ID
                data-frame-hydration-scope="interaction-island"
                data-frame-embed-share=embed_share
                data-frame-embed-origins=embed_origins
            >
                <PlayerKeyboardHelp/>
            </div>
        </FeatureCard>
    }
    .into_any()
}

fn format_duration(duration_ms: u64) -> String {
    let seconds = duration_ms / 1_000;
    let minutes = seconds / 60;
    let remainder = seconds % 60;
    format!("{minutes} minutes, {remainder} seconds")
}

fn status_shell(label: &'static str, message: &'static str) -> AnyView {
    view! {
        <Card attr:aria-labelledby="page-title">
            <h1 id="page-title">{label}</h1>
            <Alert attr:role="status">{message}</Alert>
            <ButtonLink variant=ButtonVariant::Secondary href="/">"Frame home"</ButtonLink>
        </Card>
    }
    .into_any()
}

fn processing_status_shell(status: Option<&InstantUiProgressV1>) -> AnyView {
    let phase = match status.map(|status| status.phase) {
        Some(InstantUiPhaseV1::Uploading) => "Uploading verified recording parts.",
        Some(InstantUiPhaseV1::Finalizing) => "Finalizing the playable recording.",
        _ => "Processing the recording.",
    };
    let message = match status.and_then(|status| status.error) {
        Some(InstantUiErrorCodeV1::UploadDelayed) => {
            "The upload is delayed. Frame is retrying from its durable checkpoint."
        }
        Some(InstantUiErrorCodeV1::FinalizeDelayed) => {
            "Finalization is delayed. Frame is retrying without requesting media in this page."
        }
        _ => "The recording is not available yet. No media request has been made.",
    };
    let progress = match status.and_then(|status| status.progress_basis_points) {
        Some(basis_points) => view! {
            <Progress
                attr:max="10000"
                attr:value=basis_points
                attr:aria-labelledby="processing-progress-label"
            >{format!("{} percent", basis_points / 100)}</Progress>
        }
        .into_any(),
        None => view! {
            <Progress attr:max="10000" attr:aria-labelledby="processing-progress-label">
                "Processing"
            </Progress>
        }
        .into_any(),
    };

    view! {
        <Card attr:aria-labelledby="page-title">
            <p class="eyebrow">"Instant recording"</p>
            <h1 id="page-title">"Recording processing"</h1>
            <p id="processing-progress-label">{phase}</p>
            {progress}
            <Alert attr:role="status">{message}</Alert>
            <ButtonLink variant=ButtonVariant::Secondary href="/">"Frame home"</ButtonLink>
        </Card>
    }
    .into_any()
}

fn safe_id(value: &str) -> &str {
    if !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        value
    } else {
        "unavailable"
    }
}

fn document(title: &str, description: &str, canonical: &str, robots: &str, app: String) -> String {
    document_with_head(title, description, canonical, robots, app, false, None)
}

fn themed_private_document(
    title: &str,
    description: &str,
    canonical: &str,
    robots: &str,
    app: String,
    theme: &str,
) -> String {
    document_with_head(
        title,
        description,
        canonical,
        robots,
        app,
        false,
        Some(theme),
    )
}

fn public_document(
    title: &str,
    description: &str,
    canonical: &str,
    robots: &str,
    app: String,
) -> String {
    document_with_head(title, description, canonical, robots, app, true, None)
}

fn document_with_head(
    title: &str,
    description: &str,
    canonical: &str,
    robots: &str,
    app: String,
    public_open_graph: bool,
    body_theme: Option<&str>,
) -> String {
    let head = view! {
        <meta charset="utf-8"/>
        <meta name="viewport" content="width=device-width,initial-scale=1"/>
        <meta name="description" content=description.to_owned()/>
        <meta name="robots" content=robots.to_owned()/>
        <link
            rel="icon"
            href="data:image/svg+xml,%3Csvg%20xmlns=%22http://www.w3.org/2000/svg%22%20viewBox=%220%200%2032%2032%22%3E%3Crect%20width=%2232%22%20height=%2232%22%20rx=%228%22%20fill=%22%23a7f3d0%22/%3E%3Cpath%20d=%22M9%208h15v5H15v4h8v5h-8v6H9z%22%20fill=%22%23081014%22/%3E%3C/svg%3E"
        />
        <link rel="canonical" href=canonical.to_owned()/>
        {public_open_graph.then(|| (
            leptos::html::meta().attr("property", "og:type").content("video.other"),
            leptos::html::meta().attr("property", "og:title").content(title.to_owned()),
            leptos::html::meta().attr("property", "og:description").content(description.to_owned()),
            leptos::html::meta().attr("property", "og:url").content(canonical.to_owned()),
        ))}
        <title>{title.to_owned()}</title>
        <style data-frame-ui="shadcn-tailwind">{STYLESHEET}</style>
    }
    .to_html();
    let hydration = view! {
        <div id=ROOT_ID data-frame-hydration-scope="interaction-island">
            <HydrationBoundary/>
        </div>
    }
    .to_html();
    let body_theme = match body_theme {
        Some(theme @ ("system" | "dark" | "light")) => format!(" data-theme=\"{theme}\""),
        _ => String::new(),
    };
    format!(
        "<!doctype html><html lang=\"en\"><head>{head}<!--FRAME_HYDRATION_HEAD--></head><body data-frame-surface=\"web\"{body_theme}><a class=\"skip-link\" href=\"#main\">Skip to content</a>{app}{hydration}<!--FRAME_HYDRATION_SCRIPT--></body></html>"
    )
}

#[cfg(test)]
mod tests {
    use frame_client::{
        ApiVersion, InstantUiErrorCodeV1, InstantUiPhaseV1, InstantUiProgressV1,
        PlaybackDescriptor, PublicShareSummary, ShareAvailability,
    };

    use crate::config::ConfigValues;
    use crate::product::{WorkspaceRole, local_authenticated_fixture, local_share_fixture};

    use super::*;

    fn config() -> RuntimeConfig {
        RuntimeConfig::from_values(ConfigValues::default()).expect("local config")
    }

    fn embed_config() -> RuntimeConfig {
        RuntimeConfig::from_values(ConfigValues {
            public_embed_enabled: Some("true".into()),
            embed_ancestors: Some("https://engmanager.xyz".into()),
            ..ConfigValues::default()
        })
        .expect("local embed config")
    }

    #[test]
    fn every_page_has_accessible_document_landmarks() {
        for page in [
            landing(&config()),
            login(&config(), SignInState::Ready),
            authenticated(
                &config(),
                AuthenticatedRoute::Dashboard,
                AuthenticatedState::Unauthenticated,
            ),
        ] {
            assert!(page.body.starts_with("<!doctype html>"));
            assert!(page.body.contains("Skip to content"));
            assert!(page.body.contains("id=\"main\""));
            assert!(page.body.contains("rel=\"canonical\""));
            assert!(page.body.contains("name=\"robots\""));
            assert!(
                page.body
                    .contains("data-frame-hydration-scope=\"interaction-island\"")
            );
            assert!(page.body.contains("Server-rendered content ready."));
        }
    }

    #[test]
    fn dashboard_shell_contains_no_private_fixture_data() {
        let page = authenticated(
            &config(),
            AuthenticatedRoute::Dashboard,
            AuthenticatedState::Unauthenticated,
        );
        assert_eq!(page.status, StatusCode::UNAUTHORIZED);
        assert_eq!(page.cache_control, NO_STORE);
        for forbidden in [
            "Local Frame workspace",
            "Product walkthrough",
            "owner@example.com",
            "tenant-",
            "signed=",
            "object_key",
        ] {
            assert!(!page.body.contains(forbidden));
        }
    }

    #[test]
    fn unavailable_share_is_generic_and_non_cacheable() {
        let private = share(&config(), "private-id", ShareView::Unavailable);
        let deleted = share(&config(), "deleted-id", ShareView::Unavailable);
        assert_eq!(private.status, StatusCode::NOT_FOUND);
        assert_eq!(private.cache_control, NO_STORE);
        assert!(private.body.contains("Recording unavailable"));
        assert!(!private.body.contains("private-id"));
        assert!(!deleted.body.contains("deleted-id"));
    }

    #[test]
    fn embed_fails_closed_by_default() {
        let config = config();
        let page = embed(
            &config,
            "fixture-public",
            local_share_fixture(&config, "fixture-public"),
        );
        assert_eq!(page.status, StatusCode::NOT_FOUND);
        assert_eq!(page.cache_control, NO_STORE);
        assert!(page.body.contains("Embedded playback unavailable"));
    }

    #[test]
    fn authenticated_role_navigation_is_server_filtered() {
        let config = config();
        let member = authenticated(
            &config,
            AuthenticatedRoute::Dashboard,
            local_authenticated_fixture(&config, Some("member")),
        );
        assert_eq!(member.status, StatusCode::OK);
        assert!(member.body.contains("Product walkthrough"));
        assert!(!member.body.contains("href=\"/billing\""));
        assert!(!member.body.contains("href=\"/admin\""));

        let denied = authenticated(
            &config,
            AuthenticatedRoute::Billing,
            local_authenticated_fixture(&config, Some("admin")),
        );
        assert_eq!(denied.status, StatusCode::FORBIDDEN);
        assert!(!denied.body.contains("Local Frame workspace"));
        assert!(!denied.body.contains("Product walkthrough"));
    }

    #[test]
    fn public_player_renders_only_validated_provider_neutral_paths() {
        let config = config();
        let page = share(
            &config,
            "fixture-public",
            local_share_fixture(&config, "fixture-public"),
        );
        assert_eq!(page.status, StatusCode::OK);
        assert_eq!(page.cache_control, NO_STORE);
        assert!(page.body.contains("<video"));
        assert!(page.body.contains("kind=\"captions\""));
        assert!(page.body.contains("id=\"frame-public-player\""));
        assert!(
            page.body
                .contains("controlslist=\"nodownload noremoteplayback\"")
        );
        assert!(page.body.contains("Play or pause"));
        assert!(page.body.contains("Back 10 seconds"));
        assert!(page.body.contains("Forward 10 seconds"));
        assert!(page.body.contains("Picture in picture"));
        assert!(page.body.contains("Retry playback"));
        assert!(page.body.contains("transcript (WebVTT)"));
        assert!(page.body.contains("property=\"og:title\""));
        assert!(
            page.body
                .contains("/api/v1/public/shares/fixture-public/media")
        );
        for forbidden in ["object_key", "x-amz", "X-Amz", "signed="] {
            assert!(!page.body.contains(forbidden));
        }
    }

    #[test]
    fn route_scope_confusion_collapses_to_the_generic_unavailable_page() {
        let config = config();
        let page = share(
            &config,
            "another-share",
            local_share_fixture(&config, "fixture-public"),
        );
        assert_eq!(page.status, StatusCode::NOT_FOUND);
        assert!(!page.body.contains("Local public recording"));
        assert!(!page.body.contains("fixture-public"));
        assert!(!page.body.contains("another-share"));
        assert!(!page.body.contains("property=\"og:title\""));
    }

    #[test]
    fn enabled_embed_is_noindex_exact_origin_scoped_and_uses_share_canonical() {
        let config = embed_config();
        let page = embed(
            &config,
            "fixture-public",
            local_share_fixture(&config, "fixture-public"),
        );
        assert_eq!(page.status, StatusCode::OK);
        assert_eq!(page.cache_control, NO_STORE);
        assert_eq!(page.robots, "noindex,follow");
        assert!(
            page.body
                .contains("data-frame-embed-share=\"fixture-public\"")
        );
        assert!(
            page.body
                .contains("data-frame-embed-origins=\"https://engmanager.xyz\"")
        );
        assert!(
            page.body
                .contains("rel=\"canonical\" href=\"http://127.0.0.1:3000/s/fixture-public\"")
        );
        assert!(!page.body.contains("property=\"og:title\""));
        assert!(!page.body.contains("object_key"));
    }

    #[test]
    fn rejected_descriptor_cannot_leak_public_metadata() {
        let config = config();
        let summary = PublicShareSummary {
            api_version: ApiVersion::current(),
            availability: ShareAvailability::Public,
            title: Some("Confidential migration plan".into()),
            description: Some("Never render this".into()),
            canonical_url: Some("http://127.0.0.1:3000/s/secret".into()),
            duration_ms: Some(1_000),
            playback: Some(PlaybackDescriptor {
                path: "/api/v1/public/shares/secret/object-key".into(),
                content_type: "video/mp4".into(),
                supports_range: true,
                captions: Vec::new(),
            }),
            processing_status: None,
        };
        let page = share(&config, "secret", ShareView::from_summary(&config, summary));
        assert_eq!(page.status, StatusCode::NOT_FOUND);
        assert_eq!(page.cache_control, NO_STORE);
        assert!(!page.body.contains("Confidential migration plan"));
        assert!(!page.body.contains("Never render this"));
        assert!(!page.body.contains("secret"));
        assert!(!page.body.contains("property=\"og:title\""));
    }

    #[test]
    fn processing_state_never_renders_player_or_private_metadata() {
        let config = config();
        let page = share(
            &config,
            "fixture-processing",
            local_share_fixture(&config, "fixture-processing"),
        );
        assert_eq!(page.status, StatusCode::ACCEPTED);
        assert_eq!(page.cache_control, NO_STORE);
        assert!(page.body.contains("Recording processing"));
        assert!(page.body.contains("Finalizing the playable recording."));
        assert!(page.body.contains("<progress max=\"10000\""));
        assert!(!page.body.contains("value=\""));
        assert!(!page.body.contains("<video"));
        assert!(!page.body.contains("property=\"og:title\""));
    }

    #[test]
    fn processing_page_consumes_only_public_safe_progress_and_delayed_errors() {
        let config = config();
        let summary = |status| PublicShareSummary {
            api_version: ApiVersion::current(),
            availability: ShareAvailability::Processing,
            title: None,
            description: None,
            canonical_url: Some("http://127.0.0.1:3000/s/progress-demo".into()),
            duration_ms: None,
            playback: None,
            processing_status: Some(status),
        };

        let uploading = share(
            &config,
            "progress-demo",
            ShareView::from_summary(
                &config,
                summary(
                    InstantUiProgressV1::new(InstantUiPhaseV1::Uploading, Some(4_200), false, None)
                        .expect("uploading status"),
                ),
            ),
        );
        assert_eq!(uploading.status, StatusCode::ACCEPTED);
        assert!(
            uploading
                .body
                .contains("Uploading verified recording parts.")
        );
        assert!(uploading.body.contains("value=\"4200\""));
        assert!(uploading.body.contains("42 percent"));
        assert!(!uploading.body.contains("<video"));

        let delayed = share(
            &config,
            "progress-demo",
            ShareView::from_summary(
                &config,
                summary(
                    InstantUiProgressV1::new(
                        InstantUiPhaseV1::Finalizing,
                        None,
                        true,
                        Some(InstantUiErrorCodeV1::FinalizeDelayed),
                    )
                    .expect("delayed status"),
                ),
            ),
        );
        assert_eq!(delayed.status, StatusCode::ACCEPTED);
        assert!(delayed.body.contains("Finalization is delayed."));
        for private in ["LocalStorage", "NetworkOffline", "object_key", "tenant_id"] {
            assert!(!delayed.body.contains(private));
        }

        let private_status = summary(
            InstantUiProgressV1::new(
                InstantUiPhaseV1::RecoveryRequired,
                None,
                false,
                Some(InstantUiErrorCodeV1::LocalStorageFull),
            )
            .expect("desktop-only status"),
        );
        let rejected = share(
            &config,
            "progress-demo",
            ShareView::from_summary(&config, private_status),
        );
        assert_eq!(rejected.status, StatusCode::NOT_FOUND);
        assert!(!rejected.body.contains("storage"));
    }

    #[test]
    fn private_values_are_html_escaped_in_authenticated_fixture() {
        let workspace = WorkspaceView {
            organization_name: "<script>tenant()</script>".into(),
            member_label: "Member & owner".into(),
            role: WorkspaceRole::Owner,
            revision: 1,
            recordings: vec![],
            spaces: vec![],
            folders: vec![],
            import: None,
        };
        let page = authenticated(
            &config(),
            AuthenticatedRoute::Dashboard,
            AuthenticatedState::Ready(workspace),
        );
        assert!(!page.body.contains("<script>tenant()</script>"));
        assert!(page.body.contains("&lt;script&gt;tenant()&lt;/script&gt;"));
    }

    #[test]
    fn sign_in_form_never_places_identity_in_url() {
        let page = login(&config(), SignInState::Ready);
        assert!(page.body.contains("method=\"post\""));
        assert!(page.body.contains("autocomplete=\"email\""));
        assert!(!page.body.contains("token="));
        assert_eq!(page.cache_control, NO_STORE);
    }

    #[test]
    fn every_authenticated_route_enforces_every_role_before_rendering() {
        let config = config();
        for route in AuthenticatedRoute::ALL {
            for (role, fixture) in [
                (WorkspaceRole::Owner, "owner"),
                (WorkspaceRole::Admin, "admin"),
                (WorkspaceRole::Member, "member"),
            ] {
                let page = authenticated_at(
                    &config,
                    route,
                    local_authenticated_fixture(&config, Some(fixture)),
                    route.path(),
                    &RouteViewQuery::default(),
                );
                assert_eq!(page.cache_control, NO_STORE, "{} cache", route.name());
                assert!(page.body.contains("noindex,nofollow"));
                assert!(page.body.contains("id=\"page-title\""));
                if route.permitted_for(role) {
                    assert_eq!(page.status, StatusCode::OK, "{} {fixture}", route.name());
                    assert!(page.body.contains("Local Frame workspace"));
                    assert!(page.body.contains(route.label()));
                } else {
                    assert_eq!(
                        page.status,
                        StatusCode::FORBIDDEN,
                        "{} {fixture}",
                        route.name()
                    );
                    assert!(page.body.contains("Access denied"));
                    assert!(!page.body.contains("Local Frame workspace"));
                    assert!(!page.body.contains("Product walkthrough"));
                }
            }
        }
    }

    #[test]
    fn auth_forms_are_post_only_bounded_and_non_reflective() {
        for page in [
            login(&config(), SignInState::Ready),
            signup(&config(), SignInState::Ready),
            recovery(&config(), SignInState::Ready),
            verify(&config(), SignInState::Ready),
        ] {
            assert_eq!(page.status, StatusCode::OK);
            assert_eq!(page.cache_control, NO_STORE);
            assert!(page.body.contains("method=\"post\""));
            assert!(page.body.contains("required"));
            assert!(page.body.contains("noindex,nofollow"));
            assert!(!page.body.contains("token="));
            assert!(!page.body.contains("otp="));
        }
        let failed = verify(&config(), SignInState::Failed);
        assert!(failed.body.contains("role=\"alert\""));
        assert!(!failed.body.contains("123456"));
    }

    #[test]
    fn library_query_filters_server_rendered_fixture_and_preserves_theme() {
        let config = config();
        let query = RouteViewQuery::parse(Some("Product"), Some("ready"), Some("1"), Some("light"))
            .expect("valid view query");
        let page = authenticated_at(
            &config,
            AuthenticatedRoute::Library,
            local_authenticated_fixture(&config, Some("owner")),
            "/library",
            &query,
        );
        assert_eq!(page.status, StatusCode::OK);
        assert!(page.body.contains("Product walkthrough"));
        assert!(!page.body.contains("Weekly update"));
        assert!(!page.body.contains("Interrupted import"));
        assert!(page.body.contains("data-theme=\"light\""));
        assert!(page.body.contains("value=\"Product\""));
    }
}
