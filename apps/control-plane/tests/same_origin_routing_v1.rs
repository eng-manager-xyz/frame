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
const PROTECTED_MEDIA_CHILD_ROUTES: [(&str, &str, &str, &str); 16] = [
    (
        "cap-v1-105318e146fceb4c",
        "POST",
        "/media-server/audio/check",
        "/media-server/audio/check",
    ),
    (
        "cap-v1-77fe8c9a4b418f53",
        "POST",
        "/media-server/audio/convert",
        "/media-server/audio/convert",
    ),
    (
        "cap-v1-a2814dde3550e586",
        "POST",
        "/media-server/audio/extract",
        "/media-server/audio/extract",
    ),
    (
        "cap-v1-fbd3d44a0ca1786f",
        "GET",
        "/media-server/audio/status",
        "/media-server/audio/status",
    ),
    (
        "cap-v1-0bf20f7e9b1a474c",
        "GET",
        "/media-server/health",
        "/media-server/health",
    ),
    (
        "cap-v1-ee9797dd352c4e11",
        "POST",
        "/media-server/video/cleanup",
        "/media-server/video/cleanup",
    ),
    (
        "cap-v1-9ed2e7b3f858eaaa",
        "POST",
        "/media-server/video/convert",
        "/media-server/video/convert",
    ),
    (
        "cap-v1-2b48f7704d996758",
        "POST",
        "/media-server/video/edit",
        "/media-server/video/edit",
    ),
    (
        "cap-v1-aa975a14fd384a5c",
        "POST",
        "/media-server/video/force-cleanup",
        "/media-server/video/force-cleanup",
    ),
    (
        "cap-v1-bf2eb9302de590a1",
        "POST",
        "/media-server/video/mux-segments",
        "/media-server/video/mux-segments",
    ),
    (
        "cap-v1-ba986b8c5b07cfd6",
        "POST",
        "/media-server/video/probe",
        "/media-server/video/probe",
    ),
    (
        "cap-v1-320876fa0aec77cb",
        "POST",
        "/media-server/video/process",
        "/media-server/video/process",
    ),
    (
        "cap-v1-fc2e2bd0d28ffbf3",
        "POST",
        "/media-server/video/process/:jobId/cancel",
        "/media-server/video/process/job-42/cancel",
    ),
    (
        "cap-v1-43bc9ae6aa4f44a8",
        "GET",
        "/media-server/video/process/:jobId/status",
        "/media-server/video/process/job-42/status",
    ),
    (
        "cap-v1-986bf73a0b5cb676",
        "GET",
        "/media-server/video/status",
        "/media-server/video/status",
    ),
    (
        "cap-v1-4165632f8266ae06",
        "POST",
        "/media-server/video/thumbnail",
        "/media-server/video/thumbnail",
    ),
];

#[derive(Debug, Deserialize)]
struct Matrix {
    schema: String,
    canonical_host: String,
    worker_route_pattern: String,
    compatibility_worker_route_pattern: String,
    lookalike_policy: String,
    protected_media_child_routes: Vec<ProtectedMediaChildRoute>,
    cases: Vec<RouteCase>,
    host_cases: Vec<HostCase>,
    transport_cases: Vec<TransportCase>,
}

#[derive(Debug, Deserialize)]
struct RouteCase {
    id: String,
    url: String,
    raw_path: String,
    method: Option<String>,
    source_operation_id: Option<String>,
    edge_owner: String,
    worker_class: String,
}

#[derive(Debug, Deserialize)]
struct ProtectedMediaChildRoute {
    operation_id: String,
    method: String,
    path: String,
    example_path: String,
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
        Route::LegacyMediaServerRoot => "legacy_media_server_root",
        Route::LegacyProtectedMedia => "legacy_protected_media",
        Route::Discovery => "discovery",
        Route::Capabilities => "capabilities",
        Route::ApiHealth => "api_health",
        Route::InvalidApiPath => "invalid_api_path",
        Route::UnknownApi => "unknown_api",
        Route::NotApi => "not_api",
        _ => "versioned_api",
    }
}

fn literal_worker_route(url: &str, host: &str) -> Option<&'static str> {
    let suffix = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .and_then(|url| url.strip_prefix(host))?;
    if suffix.starts_with("/api") {
        Some("api")
    } else if suffix.starts_with("/media-server") {
        Some("media-server")
    } else {
        None
    }
}

#[test]
fn exhaustive_owner_matrix_matches_the_literal_edge_pattern_and_raw_router() {
    let matrix = matrix();
    assert_eq!(matrix.schema, "frame.same-origin-route-owner-matrix.v1");
    assert_eq!(matrix.canonical_host, "frame.engmanager.xyz");
    assert_eq!(matrix.worker_route_pattern, "frame.engmanager.xyz/api*");
    assert_eq!(
        matrix.compatibility_worker_route_pattern,
        "frame.engmanager.xyz/media-server*"
    );
    assert_eq!(matrix.lookalike_policy, "non_cacheable_404");
    assert!(matrix.cases.len() >= 52);

    let declared_protected_routes = matrix
        .protected_media_child_routes
        .iter()
        .map(|route| {
            (
                route.operation_id.as_str(),
                route.method.as_str(),
                route.path.as_str(),
                route.example_path.as_str(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(declared_protected_routes, PROTECTED_MEDIA_CHILD_ROUTES);

    assert_eq!(
        matrix
            .cases
            .iter()
            .filter(|case| case.source_operation_id.is_some())
            .count(),
        PROTECTED_MEDIA_CHILD_ROUTES.len()
    );
    let protected_case_contracts = matrix
        .cases
        .iter()
        .filter_map(|case| {
            case.source_operation_id.as_deref().map(|operation_id| {
                (
                    operation_id,
                    case.method.as_deref(),
                    case.raw_path.as_str(),
                    case.edge_owner.as_str(),
                    case.worker_class.as_str(),
                )
            })
        })
        .collect::<std::collections::BTreeSet<_>>();
    let expected_case_contracts = PROTECTED_MEDIA_CHILD_ROUTES
        .iter()
        .map(|(operation_id, method, _path, example_path)| {
            (
                *operation_id,
                Some(*method),
                *example_path,
                "worker_compat",
                "legacy_protected_media",
            )
        })
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(protected_case_contracts, expected_case_contracts);

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

        let intercepted = literal_worker_route(&case.url, &matrix.canonical_host);
        match case.edge_owner.as_str() {
            "render" => assert!(
                intercepted.is_none(),
                "{} unexpectedly entered Worker",
                case.id
            ),
            "worker_api" => {
                assert_eq!(
                    intercepted,
                    Some("api"),
                    "{} entered the wrong Worker route",
                    case.id
                );
            }
            "worker_compat" => {
                assert_eq!(
                    intercepted,
                    Some("media-server"),
                    "{} entered the wrong Worker route",
                    case.id
                );
            }
            "worker_reject" => {
                assert!(
                    intercepted.is_some(),
                    "{} unexpectedly fell through to Render",
                    case.id
                );
            }
            // Cloudflare URL-normalization settings can turn this raw spelling
            // into `/api` before route evaluation. The protected trace must
            // prove either Render pass-through or an edge rejection; the raw
            // Worker classifier remains closed in both cases.
            "render_or_edge_reject" => {
                assert!(
                    intercepted.is_none(),
                    "{} literal route assumption drifted",
                    case.id
                );
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
