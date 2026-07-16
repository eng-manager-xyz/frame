//! Compile-and-contract test for the cutover runtime shared by the Worker.

#[path = "../src/cutover_authority.rs"]
mod cutover_authority;
pub use cutover_authority::*;

#[path = "../src/cutover_authority_runtime.rs"]
pub mod cutover_authority_runtime;

#[test]
fn integrated_control_groups_remain_explicit() {
    assert_eq!(
        cutover_authority_runtime::INTEGRATED_ROUTE_GROUPS,
        [
            "cutover-authority-status",
            "cutover-transition-pause-resume",
            "cutover-shadow-signal-ingest",
            "scoped-writer-fence-for-every-d1-mutation",
        ]
    );
}

#[test]
fn worker_mutation_and_route_integration_stays_closed() {
    let worker = include_str!("../src/lib.rs");
    let routing = include_str!("../src/routing.rs");
    let managed_media = include_str!("../src/media_service_runtime.rs");

    for suffix in [
        "transition",
        "replay/pause",
        "replay/resume",
        "signals",
        "shadow-observations",
    ] {
        assert!(routing.contains(suffix), "missing cutover route {suffix}");
    }
    assert!(worker.contains("RequiredAccess::Admin"));
    assert!(worker.contains("authorized_cutover_scope"));
    assert!(worker.contains("with_cutover_fence"));

    // Request code may enter D1 directly only through the local branch of the
    // central helper. Production uses the scoped assertion branch beside it.
    assert_eq!(
        worker.matches("database.batch(statements).await").count(),
        1
    );
    assert!(!worker.contains(".run()"));
    assert!(!managed_media.contains(".batch("));
    assert!(!managed_media.contains(".run()"));
    assert!(!managed_media.contains("RETURNING job_id"));
}
