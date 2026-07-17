//! Provider-neutral public contract for Frame consumers.
//!
//! The default feature set contains only origin validation, public DTOs, and
//! a transport abstraction suitable for native and wasm consumers. The
//! optional native `client` feature supplies a bounded Reqwest transport. No
//! authenticated dashboard, storage key, Cloudflare, media-runtime, Axum, or
//! Leptos type belongs in this crate.

mod dto;
mod error;
#[cfg(all(feature = "client", not(target_arch = "wasm32")))]
mod native;
mod origin;
mod transport;

pub use dto::{
    ApiError, ApiVersion, Capabilities, CaptionTrack, Health, PlaybackDescriptor,
    PublicShareSummary, RetryAdvice, ServiceStatus, ShareAvailability,
};
pub use error::{ClientError, ClientErrorCode};
#[cfg(all(feature = "client", not(target_arch = "wasm32")))]
pub use native::NativeTransport;
pub use origin::FrameOrigin;
pub use transport::{
    BoxFuture, FrameClient, HttpMethod, Transport, TransportPolicy, TransportRequest,
    TransportResponse,
};
