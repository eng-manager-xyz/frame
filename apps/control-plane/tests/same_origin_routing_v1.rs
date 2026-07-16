#[path = "../src/routing.rs"]
mod routing;

use routing::{
    Deployment, HostPolicy, HostRejection, Route, classify_raw_path, parse_raw_request_target,
    validate_host,
};
use serde::Deserialize;

const MATRIX: &str =
    include_str!("../../../fixtures/same-origin-routing/v1/route-owner-matrix.json");
const CONTROL_PLANE: &str = include_str!("../src/lib.rs");

#[derive(Debug, Deserialize)]
struct Matrix {
    schema: String,
    canonical_host: String,
    worker_route_pattern: String,
    lookalike_policy: String,
    cases: Vec<RouteCase>,
    host_cases: Vec<HostCase>,
    transport_cases: Vec<TransportCase>,
}

#[derive(Debug, Deserialize)]
struct RouteCase {
    id: String,
    url: String,
    raw_path: String,
    edge_owner: String,
    worker_class: String,
}

#[derive(Debug, Deserialize)]
struct HostCase {
    id: String,
    url: String,
    host_header: Option<String>,
    expected: String,
}

#[derive(Debug, Deserialize)]
struct TransportCase {
    kind: String,
    contract: String,
}

fn matrix() -> Matrix {
    serde_json::from_str(MATRIX).expect("same-origin matrix must be valid JSON")
}

fn route_class(route: &Route) -> &'static str {
    match route {
        Route::LegacyRoot => "legacy_root",
        Route::LegacyHealth => "legacy_health",
        Route::Discovery => "discovery",
        Route::Capabilities => "capabilities",
        Route::ApiHealth => "api_health",
        Route::InvalidApiPath => "invalid_api_path",
        Route::UnknownApi => "unknown_api",
        Route::NotApi => "not_api",
        _ => "versioned_api",
    }
}

fn literal_broad_route_matches(url: &str, host: &str) -> bool {
    let Some(suffix) = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .and_then(|url| url.strip_prefix(host))
    else {
        return false;
    };
    suffix.starts_with("/api")
}

#[test]
fn exhaustive_owner_matrix_matches_the_literal_edge_pattern_and_raw_router() {
    let matrix = matrix();
    assert_eq!(matrix.schema, "frame.same-origin-route-owner-matrix.v1");
    assert_eq!(matrix.canonical_host, "frame.engmanager.xyz");
    assert_eq!(matrix.worker_route_pattern, "frame.engmanager.xyz/api*");
    assert_eq!(matrix.lookalike_policy, "non_cacheable_404");
    assert!(matrix.cases.len() >= 25);

    for case in matrix.cases {
        let target = parse_raw_request_target(&case.url)
            .unwrap_or_else(|error| panic!("{} did not parse: {error:?}", case.id));
        assert_eq!(target.path, case.raw_path, "{} path drifted", case.id);
        assert_eq!(
            route_class(&classify_raw_path(&target.path)),
            case.worker_class,
            "{} Worker classification drifted",
            case.id
        );

        let intercepted = literal_broad_route_matches(&case.url, &matrix.canonical_host);
        match case.edge_owner.as_str() {
            "render" => assert!(!intercepted, "{} unexpectedly entered Worker", case.id),
            "worker_api" | "worker_reject" => {
                assert!(
                    intercepted,
                    "{} unexpectedly fell through to Render",
                    case.id
                );
            }
            // Cloudflare URL-normalization settings can turn this raw spelling
            // into `/api` before route evaluation. The protected trace must
            // prove either Render pass-through or an edge rejection; the raw
            // Worker classifier remains closed in both cases.
            "render_or_edge_reject" => {
                assert!(!intercepted, "{} literal route assumption drifted", case.id);
                assert_eq!(case.worker_class, "not_api");
            }
            owner => panic!("{} has unsupported edge owner {owner}", case.id),
        }
    }
}

#[test]
fn production_host_matrix_is_exact_https_and_header_bound() {
    let matrix = matrix();
    let policy = HostPolicy::new(Deployment::Production, &matrix.canonical_host)
        .expect("canonical production policy");
    assert!(matrix.host_cases.len() >= 8);

    for case in matrix.host_cases {
        let parsed = parse_raw_request_target(&case.url);
        if case.expected == "malformed_target" {
            assert!(parsed.is_err(), "{} unexpectedly parsed", case.id);
            continue;
        }
        let target = parsed.unwrap_or_else(|error| panic!("{} did not parse: {error:?}", case.id));
        let actual = validate_host(&target, case.host_header.as_deref(), &policy);
        let expected = match case.expected.as_str() {
            "accepted" => Ok(()),
            "insecure_scheme" => Err(HostRejection::InsecureScheme),
            "unexpected_host" => Err(HostRejection::UnexpectedHost),
            "host_header_mismatch" => Err(HostRejection::HostHeaderMismatch),
            value => panic!("{} has unsupported host result {value}", case.id),
        };
        assert_eq!(actual, expected, "{} host policy drifted", case.id);
    }
}

#[test]
fn direct_worker_transport_has_no_gateway_rewrite_or_open_upgrade_surface() {
    let matrix = matrix();
    let kinds = matrix
        .transport_cases
        .iter()
        .map(|case| case.kind.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        kinds,
        [
            "error",
            "methods",
            "query",
            "range",
            "redirect",
            "request_body",
            "response_stream",
            "upgrade",
        ]
        .into_iter()
        .collect()
    );
    assert!(
        matrix
            .transport_cases
            .iter()
            .all(|case| !case.contract.trim().is_empty())
    );

    // The direct control-plane Route dispatches the original Request. These
    // production markers guard the method/body/query/range/streaming behavior
    // against silently growing a lossy edge proxy.
    for marker in [
        "let method = request.method();",
        "request.stream()?",
        ".response_body()?",
        "parse_range_header(request.headers().get(\"range\")?.as_deref(),",
        "\"content-range\"",
        "headers.set(\"x-request-id\", request_id)?;",
        "headers.set(\"cache-control\", \"no-store, max-age=0\")?;",
    ] {
        assert!(
            CONTROL_PLANE.contains(marker),
            "transport marker missing: {marker}"
        );
    }
    assert!(!CONTROL_PLANE.contains("WebSocketPair"));
    assert!(!CONTROL_PLANE.contains("with_status(101)"));
}
