//! Runtime composition boundary for business-data HTTP handlers.
//!
//! The main Worker router is intentionally not changed here while its owner is
//! integrating the shared route surface. Route handlers can construct this service
//! from the request-scoped D1 binding without exposing `worker` values through
//! the application or domain layers.

use frame_application::BusinessDataService;
use worker::D1Database;

use super::D1BusinessRepository;

pub type D1BusinessDataService<'database> = BusinessDataService<D1BusinessRepository<'database>>;

#[must_use]
pub const fn service(database: &D1Database) -> D1BusinessDataService<'_> {
    BusinessDataService::new(D1BusinessRepository::new(database))
}

/// Route groups which must delegate through [`service`] during router wiring.
/// Keeping this inventory next to the factory makes omissions reviewable while
/// the route enum and Worker dispatch are owned by a separate integration.
pub const DEFERRED_ROUTE_GROUPS: [&str; 8] = [
    "videos-and-edits",
    "shares-comments-notifications",
    "uploads-imports",
    "storage-and-derivatives",
    "developer-platform",
    "usage-and-credit-ledgers",
    "retention-export-deletion",
    "legal-holds",
];

#[cfg(test)]
mod tests {
    use super::DEFERRED_ROUTE_GROUPS;

    #[test]
    fn route_inventory_is_closed_and_unique() {
        let mut groups = DEFERRED_ROUTE_GROUPS;
        groups.sort_unstable();
        assert!(groups.windows(2).all(|pair| pair[0] != pair[1]));
    }
}
