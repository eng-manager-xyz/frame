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
    object_storage: bool,
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

            Response::from_json(&Health {
                service: "frame-control-plane",
                status: if ready { "ok" } else { "degraded" },
                d1: ready,
                object_storage: true,
            })
        })
        .get("/", |_request, _context| {
            Response::ok("Frame control plane. See /health.")
        })
        .run(request, env)
        .await
}
