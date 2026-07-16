use axum::http::StatusCode;
use leptos::prelude::*;

use crate::config::{Deployment, RuntimeConfig};

pub const NO_STORE: &str = "no-store";
const PUBLIC_HTML_CACHE: &str = "public, max-age=60, s-maxage=300, stale-while-revalidate=60";

pub struct Page {
    pub status: StatusCode,
    pub body: String,
    pub cache_control: &'static str,
    pub robots: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShareState {
    Public,
    Processing,
    Password,
    Unavailable,
}

pub fn landing(config: &RuntimeConfig) -> Page {
    let canonical = format!("{}/", config.public_origin().as_str());
    let body = view! {
        <main id="main" tabindex="-1">
            <nav aria-label="Primary">
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
            </nav>
            <section class="hero" aria-labelledby="page-title">
                <p class="eyebrow">"Private recording, built in Rust"</p>
                <h1 id="page-title">"Record locally. Share deliberately."</h1>
                <p class="lede">
                    "Frame is building an accessible recording workflow with a privacy-safe web boundary and native media processing."
                </p>
                <div class="actions">
                    <a class="button" href="/login">"Open Frame"</a>
                    <a class="button secondary" href="/health/live">"Service health"</a>
                </div>
            </section>
            <section class="grid" aria-label="Frame architecture">
                <article>
                    <p class="card-label">"Capture"</p>
                    <h2>"Native by default"</h2>
                    <p>"Recording and advanced media work stay in least-privilege native processes."</p>
                </article>
                <article>
                    <p class="card-label">"Sharing"</p>
                    <h2>"Privacy before metadata"</h2>
                    <p>"Unavailable recordings never disclose titles, thumbnails, storage keys, or signed URLs."</p>
                </article>
                <article>
                    <p class="card-label">"Access"</p>
                    <h2>"Keyboard-ready shells"</h2>
                    <p>"Every route starts with semantic structure, visible focus, and reduced-motion support."</p>
                </article>
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
        cache_control: if config.deployment() == Deployment::Production {
            PUBLIC_HTML_CACHE
        } else {
            NO_STORE
        },
        robots: if config.deployment() == Deployment::Production {
            "index,follow"
        } else {
            "noindex,nofollow"
        },
    }
}

pub fn login(config: &RuntimeConfig) -> Page {
    let canonical = format!("{}/login", config.public_origin().as_str());
    let body = view! {
        <main id="main" class="narrow" tabindex="-1">
            <a class="back" href="/">"← Frame home"</a>
            <section class="panel" aria-labelledby="page-title">
                <p class="eyebrow">"Authentication boundary"</p>
                <h1 id="page-title">"Sign in on Frame"</h1>
                <p>
                    "Shared portfolio login and token handoff are disabled. Frame authentication will use a host-only session and same-origin API."
                </p>
                <div class="notice" role="status">
                    "Sign-in is not enabled in this migration build."
                </div>
                <a class="button" href="/">"Return home"</a>
            </section>
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

pub fn dashboard(config: &RuntimeConfig) -> Page {
    let canonical = format!("{}/dashboard", config.public_origin().as_str());
    let body = view! {
        <main id="main" class="narrow" tabindex="-1">
            <a class="back" href="/">"← Frame home"</a>
            <section class="panel" aria-labelledby="page-title">
                <p class="eyebrow">"Private workspace"</p>
                <h1 id="page-title">"Sign in required"</h1>
                <p>
                    "No account, tenant, recording, or billing data is rendered into this unauthenticated response."
                </p>
                <div class="notice" role="alert">
                    "Your private dashboard remains hidden until same-origin authentication succeeds."
                </div>
                <a class="button" href="/login">"Go to sign in"</a>
            </section>
        </main>
    }
    .to_html();

    Page {
        status: StatusCode::UNAUTHORIZED,
        body: document(
            "Dashboard · Frame",
            "Authentication is required.",
            &canonical,
            "noindex,nofollow",
            body,
        ),
        cache_control: NO_STORE,
        robots: "noindex,nofollow",
    }
}

pub fn share(config: &RuntimeConfig, video_id: &str, state: ShareState) -> Page {
    let canonical_id = if state == ShareState::Unavailable {
        "unavailable"
    } else {
        safe_id(video_id)
    };
    let canonical = format!("{}/s/{}", config.public_origin().as_str(), canonical_id);
    let (status, title, description, cache, robots, content) = match state {
        ShareState::Public => (
            StatusCode::OK,
            "Shared recording · Frame",
            "A public recording shared with Frame.",
            PUBLIC_HTML_CACHE,
            "index,follow",
            player_shell(
                "Shared recording",
                "Playback is ready on the public media route.",
            ),
        ),
        ShareState::Processing => (
            StatusCode::ACCEPTED,
            "Recording processing · Frame",
            "This recording is still processing.",
            NO_STORE,
            "noindex,nofollow",
            status_shell(
                "Recording processing",
                "The recording is not available yet.",
            ),
        ),
        ShareState::Password => (
            StatusCode::UNAUTHORIZED,
            "Protected recording · Frame",
            "This recording requires authorization.",
            NO_STORE,
            "noindex,nofollow",
            status_shell(
                "Protected recording",
                "Password access is not enabled in this migration build.",
            ),
        ),
        ShareState::Unavailable => unavailable_share(),
    };

    let body = view! {
        <main id="main" class="player-page" tabindex="-1">
            <nav aria-label="Share navigation">
                <a class="brand" href="/" aria-label="Frame home">
                    <span class="mark" aria-hidden="true">"F"</span>
                    <span>"Frame"</span>
                </a>
            </nav>
            {content}
        </main>
    }
    .to_html();

    Page {
        status,
        body: document(title, description, &canonical, robots, body),
        cache_control: cache,
        robots,
    }
}

pub fn embed(config: &RuntimeConfig, video_id: &str, state: ShareState) -> Page {
    let canonical_id = if !config.embed_policy().enabled() || state != ShareState::Public {
        "unavailable"
    } else {
        safe_id(video_id)
    };
    let canonical = format!("{}/embed/{}", config.public_origin().as_str(), canonical_id);
    if !config.embed_policy().enabled() || state != ShareState::Public {
        let body = view! {
            <main id="main" class="embed-page" tabindex="-1">
                <section class="panel" aria-labelledby="page-title">
                    <h1 id="page-title">"Embedded playback unavailable"</h1>
                    <p>"No recording metadata or storage location is available in this response."</p>
                </section>
            </main>
        }
        .to_html();
        return Page {
            status: StatusCode::NOT_FOUND,
            body: document(
                "Playback unavailable · Frame",
                "Playback is unavailable.",
                &canonical,
                "noindex,nofollow",
                body,
            ),
            cache_control: NO_STORE,
            robots: "noindex,nofollow",
        };
    }

    let body = view! {
        <main id="main" class="embed-page" tabindex="-1">
            {player_shell("Shared recording", "Embedded playback is ready.")}
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
        cache_control: PUBLIC_HTML_CACHE,
        robots: "noindex,follow",
    }
}

pub fn not_found(config: &RuntimeConfig) -> Page {
    let canonical = format!("{}/", config.public_origin().as_str());
    let body = view! {
        <main id="main" class="narrow" tabindex="-1">
            <section class="panel" aria-labelledby="page-title">
                <p class="eyebrow">"404"</p>
                <h1 id="page-title">"Page not found"</h1>
                <p>"The requested Frame page is unavailable."</p>
                <a class="button" href="/">"Frame home"</a>
            </section>
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

fn player_shell(label: &'static str, status: &'static str) -> AnyView {
    view! {
        <section class="player-shell" aria-labelledby="page-title">
            <h1 id="page-title">{label}</h1>
            <div class="player-placeholder" role="region" aria-label="Video player">
                <span aria-hidden="true">"▶"</span>
                <p role="status">{status}</p>
            </div>
            <p class="player-help">
                "Player controls, captions, and media URLs are supplied only by an authorized public playback descriptor."
            </p>
        </section>
    }
    .into_any()
}

fn status_shell(label: &'static str, message: &'static str) -> AnyView {
    view! {
        <section class="panel" aria-labelledby="page-title">
            <h1 id="page-title">{label}</h1>
            <div class="notice" role="status">{message}</div>
            <a class="button secondary" href="/">"Frame home"</a>
        </section>
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
    let head = view! {
        <meta charset="utf-8"/>
        <meta name="viewport" content="width=device-width,initial-scale=1"/>
        <meta name="description" content=description.to_owned()/>
        <meta name="robots" content=robots.to_owned()/>
        <link rel="canonical" href=canonical.to_owned()/>
        <title>{title.to_owned()}</title>
        <style>{STYLE}</style>
    }
    .to_html();
    format!(
        "<!doctype html><html lang=\"en\"><head>{head}</head><body><a class=\"skip-link\" href=\"#main\">Skip to content</a>{app}</body></html>"
    )
}

const STYLE: &str = r#"
:root { color-scheme: dark; font-family: Inter, ui-sans-serif, system-ui, sans-serif; background: #090b10; color: #f3f5f7; }
* { box-sizing: border-box; }
html { scroll-behavior: smooth; }
body { margin: 0; min-height: 100vh; background: radial-gradient(circle at 75% 15%, #253150 0, transparent 30rem), #090b10; }
a { color: inherit; }
a:focus-visible, button:focus-visible, [tabindex]:focus-visible { outline: 3px solid #fbbf24; outline-offset: 4px; }
.skip-link { position: fixed; z-index: 10; top: 12px; left: 12px; padding: 10px 14px; background: #f3f5f7; color: #090b10; transform: translateY(-180%); }
.skip-link:focus { transform: translateY(0); }
main { width: min(1120px, calc(100% - 40px)); margin: auto; }
nav { min-height: 80px; display: flex; align-items: center; justify-content: space-between; gap: 16px; }
.brand { display: inline-flex; align-items: center; gap: 10px; font-weight: 800; text-decoration: none; }
.mark { display: grid; place-items: center; width: 34px; height: 34px; color: #081014; background: #a7f3d0; border-radius: 10px; }
.nav-links, .actions { display: flex; align-items: center; gap: 14px; }
.hero { padding: 92px 0 70px; max-width: 900px; }
.eyebrow, .card-label { color: #a7f3d0; font-size: 13px; font-weight: 800; letter-spacing: .12em; text-transform: uppercase; }
h1 { margin: 18px 0; font-size: clamp(44px, 8vw, 88px); line-height: 1; letter-spacing: -.05em; }
h2 { margin: 24px 0 8px; }
.lede { max-width: 720px; color: #c0c7d2; font-size: 20px; line-height: 1.6; }
.button { display: inline-block; margin-top: 18px; padding: 13px 17px; border-radius: 10px; background: #a7f3d0; color: #081014; font-weight: 800; text-decoration: none; }
.button.secondary { background: #171c25; color: #e6e9ee; border: 1px solid #394354; }
.grid { display: grid; grid-template-columns: repeat(3, 1fr); gap: 14px; padding-bottom: 60px; }
article, .panel, .player-shell { padding: 24px; background: rgba(18, 22, 30, .9); border: 1px solid #394354; border-radius: 16px; }
article { min-height: 210px; }
article p:last-child, .panel p, .player-help { color: #b2bbc8; line-height: 1.6; }
.narrow, .player-page { padding: 40px 0 80px; max-width: 760px; }
.narrow h1, .player-page h1, .embed-page h1 { font-size: clamp(36px, 7vw, 60px); }
.back { display: inline-block; margin-bottom: 28px; }
.notice { margin: 24px 0; padding: 16px; border-left: 4px solid #a7f3d0; background: #111827; line-height: 1.5; }
.player-placeholder { display: grid; min-height: 320px; place-items: center; align-content: center; gap: 12px; padding: 24px; background: #05070a; border: 1px solid #394354; border-radius: 12px; text-align: center; }
.player-placeholder > span { font-size: 44px; }
.embed-page { display: grid; min-height: 100vh; place-items: center; padding: 16px; }
@media (max-width: 760px) { .grid { grid-template-columns: 1fr; } .hero { padding-top: 52px; } }
@media (max-width: 520px) { .actions { align-items: stretch; flex-direction: column; } .nav-links { gap: 10px; font-size: 14px; } }
@media (prefers-reduced-motion: reduce) { *, *::before, *::after { scroll-behavior: auto !important; transition-duration: .01ms !important; animation-duration: .01ms !important; animation-iteration-count: 1 !important; } }
"#;

#[cfg(test)]
mod tests {
    use crate::config::ConfigValues;

    use super::*;

    fn config() -> RuntimeConfig {
        RuntimeConfig::from_values(ConfigValues::default()).expect("local config")
    }

    #[test]
    fn every_page_has_accessible_document_landmarks() {
        for page in [landing(&config()), login(&config()), dashboard(&config())] {
            assert!(page.body.starts_with("<!doctype html>"));
            assert!(page.body.contains("Skip to content"));
            assert!(page.body.contains("id=\"main\""));
            assert!(page.body.contains("rel=\"canonical\""));
            assert!(page.body.contains("name=\"robots\""));
        }
    }

    #[test]
    fn dashboard_shell_contains_no_private_fixture_data() {
        let page = dashboard(&config());
        assert_eq!(page.status, StatusCode::UNAUTHORIZED);
        assert_eq!(page.cache_control, NO_STORE);
        for forbidden in ["owner@example.com", "tenant-", "signed=", "object_key"] {
            assert!(!page.body.contains(forbidden));
        }
    }

    #[test]
    fn unavailable_share_is_generic_and_non_cacheable() {
        let private = share(&config(), "private-id", ShareState::Unavailable);
        let deleted = share(&config(), "deleted-id", ShareState::Unavailable);
        assert_eq!(private.status, StatusCode::NOT_FOUND);
        assert_eq!(private.cache_control, NO_STORE);
        assert!(private.body.contains("Recording unavailable"));
        assert!(!private.body.contains("private-id"));
        assert!(!deleted.body.contains("deleted-id"));
    }

    #[test]
    fn embed_fails_closed_by_default() {
        let page = embed(&config(), "public", ShareState::Public);
        assert_eq!(page.status, StatusCode::NOT_FOUND);
        assert_eq!(page.cache_control, NO_STORE);
        assert!(page.body.contains("Embedded playback unavailable"));
    }
}
