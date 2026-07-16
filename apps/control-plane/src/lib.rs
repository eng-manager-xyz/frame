use serde::{Deserialize, Serialize};
use worker::*;

#[derive(Debug, Deserialize)]
struct ReadyRow {
    ready: i32,
}

#[derive(Debug, Serialize)]
struct Health {
    service: &'static str,
    status: &'static str,
    d1: bool,
    r2: bool,
    media_transformations: bool,
}

fn has_binding(env: &Env, name: &str) -> bool {
    js_sys::Reflect::get(env, &wasm_bindgen::JsValue::from_str(name))
        .is_ok_and(|binding| !binding.is_undefined())
}

#[event(fetch, respond_with_errors)]
pub async fn main(request: Request, env: Env, _context: Context) -> Result<Response> {
    Router::new()
        .get_async("/health", |_request, context| async move {
            let database = context.env.d1("DB")?;
            let ready = database
                .prepare("SELECT 1 AS ready")
                .first::<ReadyRow>(None)
                .await?
                .is_some_and(|row| row.ready == 1);
            let _recordings = context.env.bucket("RECORDINGS")?;
            let media_transformations = has_binding(&context.env, "MEDIA");

            Response::from_json(&Health {
                service: "frame-control-plane",
                status: if ready && media_transformations {
                    "ok"
                } else {
                    "degraded"
                },
                d1: ready,
                r2: true,
                media_transformations,
            })
        })
        .get("/", |_request, _context| {
            Response::ok("Frame control plane. See /health.")
        })
        .run(request, env)
        .await
}
