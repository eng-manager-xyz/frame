use std::{env, net::SocketAddr};

use anyhow::{Context, Result};
use axum::{Json, Router, response::Html, routing::get};
use leptos::prelude::*;
use serde::Serialize;
use tracing::info;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let address: SocketAddr = env::var("FRAME_ADDR")
        .unwrap_or_else(|_| "127.0.0.1:3000".into())
        .parse()
        .context("FRAME_ADDR must be a socket address")?;
    let app = Router::new()
        .route("/", get(index))
        .route("/health", get(health));
    let listener = tokio::net::TcpListener::bind(address)
        .await
        .context("bind web listener")?;

    info!(%address, "Frame Leptos shell listening");
    axum::serve(listener, app).await.context("serve web app")
}

async fn index() -> Html<String> {
    Html(render_page())
}

async fn health() -> Json<Health> {
    Json(Health {
        service: "frame-web",
        status: "ok",
    })
}

#[derive(Serialize)]
struct Health {
    service: &'static str,
    status: &'static str,
}

fn render_page() -> String {
    let app = view! { <App/> }.to_html();
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>Frame · Rust recording platform</title><style>{STYLE}</style></head><body>{app}</body></html>"
    )
}

#[component]
fn App() -> impl IntoView {
    let systems = [
        (
            "Capture & media",
            "Native Rust + GStreamer",
            "Capture, editing, advanced jobs, fallback",
        ),
        (
            "Control plane",
            "Rust/Wasm + D1",
            "Worker boundary scaffolded",
        ),
        ("Interface", "Leptos", "Server-rendered shell online"),
        ("Recordings", "Cloudflare R2", "Canonical object storage"),
        (
            "Edge derivatives",
            "Cloudflare Media",
            "R2-native clips, frames, sprites, and audio",
        ),
    ];

    view! {
        <main>
            <nav>
                <span class="mark">"F"</span>
                <span>"Frame"</span>
                <span class="status">"migration scaffold"</span>
            </nav>
            <section class="hero">
                <p class="eyebrow">"A Rust port of the Cap recording platform"</p>
                <h1>"Record locally. Process natively. Share from the edge."</h1>
                <p class="lede">
                    "Frame combines Cloudflare Media for supported edge derivatives with native GStreamer for capture and complex processing, joined by explicit contracts that can be tested, migrated, and rolled back."
                </p>
                <div class="actions">
                    <a href="/health">"Check web health"</a>
                    <a class="secondary" href="https://github.com/eng-manager-xyz/frame">
                        "View repository"
                    </a>
                </div>
            </section>
            <section class="grid">
                {systems
                    .into_iter()
                    .map(|(title, technology, state)| {
                        view! {
                            <article>
                                <p class="card-title">{title}</p>
                                <h2>{technology}</h2>
                                <p>{state}</p>
                            </article>
                        }
                    })
                    .collect_view()}
            </section>
            <footer>"Walking slice: create → upload → process → share"</footer>
        </main>
    }
}

const STYLE: &str = r#"
:root { color-scheme: dark; font-family: Inter, ui-sans-serif, system-ui, sans-serif; background: #090b10; color: #f3f5f7; }
* { box-sizing: border-box; }
body { margin: 0; min-height: 100vh; background: radial-gradient(circle at 75% 15%, #253150 0, transparent 30rem), #090b10; }
main { width: min(1120px, calc(100% - 40px)); margin: auto; }
nav { height: 80px; display: flex; align-items: center; gap: 12px; font-weight: 700; }
.mark { display: grid; place-items: center; width: 34px; height: 34px; color: #081014; background: #a7f3d0; border-radius: 10px; }
.status { margin-left: auto; padding: 8px 12px; color: #9ca3af; border: 1px solid #28303e; border-radius: 999px; font-size: 12px; font-weight: 600; }
.hero { padding: 96px 0 72px; max-width: 900px; }
.eyebrow { color: #a7f3d0; font-size: 13px; font-weight: 800; letter-spacing: .12em; text-transform: uppercase; }
h1 { margin: 18px 0; font-size: clamp(48px, 8vw, 92px); line-height: .98; letter-spacing: -.055em; }
.lede { max-width: 720px; color: #aeb5c2; font-size: 20px; line-height: 1.6; }
.actions { display: flex; gap: 12px; margin-top: 34px; }
a { padding: 13px 17px; border-radius: 10px; background: #a7f3d0; color: #081014; font-weight: 750; text-decoration: none; }
a.secondary { background: #171c25; color: #e6e9ee; border: 1px solid #29313f; }
.grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(190px, 1fr)); gap: 14px; padding-bottom: 60px; }
article { min-height: 190px; padding: 22px; background: rgba(18, 22, 30, .82); border: 1px solid #29313e; border-radius: 16px; }
article .card-title { color: #a7f3d0; font-size: 12px; font-weight: 800; text-transform: uppercase; letter-spacing: .08em; }
article h2 { margin: 28px 0 8px; font-size: 20px; }
article p:last-child, footer { color: #818b9b; }
footer { padding: 24px 0 50px; border-top: 1px solid #222a36; font-size: 13px; }
@media (max-width: 800px) { .hero { padding-top: 60px; } .grid { grid-template-columns: 1fr 1fr; } }
@media (max-width: 520px) { .grid { grid-template-columns: 1fr; } .actions { align-items: stretch; flex-direction: column; text-align: center; } }
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_is_server_rendered_by_leptos() {
        let page = render_page();
        assert!(page.starts_with("<!doctype html>"));
        assert!(page.contains("Native Rust + GStreamer"));
        assert!(page.contains("Rust/Wasm + D1"));
        assert!(page.contains("Cloudflare R2"));
        assert!(page.contains("Cloudflare Media"));
    }
}
