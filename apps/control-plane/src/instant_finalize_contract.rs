//! First-party wire contract wrapper for the authenticated Instant finalize boundary.

pub(crate) use frame_authenticated_client::{
    INSTANT_FINALIZE_SCHEMA_VERSION, InstantFinalizeReceiptV1, InstantFinalizeRequestV1,
    InstantFinalizeStateV1,
};
