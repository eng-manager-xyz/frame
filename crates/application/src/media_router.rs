use frame_ports::{MediaTransformRequest, MediaTransformResult, MediaTransformer, PortError};

use crate::ApplicationError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MediaRoutingPolicy {
    pub managed_enabled: bool,
    pub allow_native_fallback: bool,
}

impl Default for MediaRoutingPolicy {
    fn default() -> Self {
        Self {
            managed_enabled: true,
            allow_native_fallback: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaRoute {
    Managed,
    NativeByCapability,
    NativeAfterManagedFailure,
    NativeByPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutedTransform {
    pub route: MediaRoute,
    pub result: MediaTransformResult,
}

/// Routes a stable transform contract without exposing executor choice to API callers.
pub struct MediaRouter<'a> {
    managed: &'a dyn MediaTransformer,
    native: &'a dyn MediaTransformer,
}

impl<'a> MediaRouter<'a> {
    #[must_use]
    pub const fn new(managed: &'a dyn MediaTransformer, native: &'a dyn MediaTransformer) -> Self {
        Self { managed, native }
    }

    pub async fn transform(
        &self,
        request: &MediaTransformRequest,
        policy: MediaRoutingPolicy,
    ) -> Result<RoutedTransform, ApplicationError> {
        validate_request(request)?;
        if !policy.managed_enabled {
            return self.run_native(request, MediaRoute::NativeByPolicy).await;
        }

        if !self.managed.supports(request.kind) {
            return self
                .run_native(request, MediaRoute::NativeByCapability)
                .await;
        }

        match self.managed.transform(request).await {
            Ok(result) => Ok(RoutedTransform {
                route: MediaRoute::Managed,
                result,
            }),
            Err(PortError::Adapter(_) | PortError::Unsupported(_))
                if policy.allow_native_fallback =>
            {
                self.run_native(request, MediaRoute::NativeAfterManagedFailure)
                    .await
            }
            Err(error) => Err(error.into()),
        }
    }

    async fn run_native(
        &self,
        request: &MediaTransformRequest,
        route: MediaRoute,
    ) -> Result<RoutedTransform, ApplicationError> {
        if !self.native.supports(request.kind) {
            return Err(ApplicationError::Unsupported);
        }
        let result = self.native.transform(request).await?;
        Ok(RoutedTransform { route, result })
    }
}

fn validate_request(request: &MediaTransformRequest) -> Result<(), ApplicationError> {
    if request.source == request.output
        || request.profile_version == 0
        || request.content_type.trim().is_empty()
    {
        return Err(ApplicationError::Invalid);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;
    use frame_domain::ObjectKey;
    use frame_ports::{
        DerivativeKind, MediaExecutor, MediaTransformRequest, MediaTransformResult,
        MediaTransformer, MemoryMediaTransformer, PortError,
    };

    use super::*;

    struct FailingManaged;

    #[async_trait]
    impl MediaTransformer for FailingManaged {
        fn executor(&self) -> MediaExecutor {
            MediaExecutor::CloudflareMedia
        }

        fn supports(&self, _kind: DerivativeKind) -> bool {
            true
        }

        async fn transform(
            &self,
            _request: &MediaTransformRequest,
        ) -> Result<MediaTransformResult, PortError> {
            Err(PortError::Adapter(
                "provider URL and credentials must be redacted".into(),
            ))
        }
    }

    fn request(kind: DerivativeKind) -> MediaTransformRequest {
        MediaTransformRequest {
            source: ObjectKey::parse("videos/v1/source.mp4").expect("source"),
            output: ObjectKey::parse("videos/v1/derivatives/output.mp4").expect("output"),
            kind,
            profile_version: 1,
            content_type: "video/mp4".into(),
        }
    }

    #[tokio::test]
    async fn selects_managed_then_replays_same_logical_artifact() {
        let managed =
            MemoryMediaTransformer::new(MediaExecutor::CloudflareMedia, [DerivativeKind::Frame]);
        let native =
            MemoryMediaTransformer::new(MediaExecutor::NativeGstreamer, [DerivativeKind::Frame]);
        let router = MediaRouter::new(&managed, &native);
        let first = router
            .transform(
                &request(DerivativeKind::Frame),
                MediaRoutingPolicy::default(),
            )
            .await
            .expect("first transform");
        let replay = router
            .transform(
                &request(DerivativeKind::Frame),
                MediaRoutingPolicy::default(),
            )
            .await
            .expect("replay");
        assert_eq!(first, replay);
        assert_eq!(first.route, MediaRoute::Managed);
        assert_eq!(first.result.executor, MediaExecutor::CloudflareMedia);
    }

    #[tokio::test]
    async fn falls_back_for_capability_and_provider_failure() {
        let limited_managed =
            MemoryMediaTransformer::new(MediaExecutor::CloudflareMedia, [DerivativeKind::Frame]);
        let native = MemoryMediaTransformer::new(
            MediaExecutor::NativeGstreamer,
            [DerivativeKind::OptimizedVideo, DerivativeKind::Frame],
        );
        let capability = MediaRouter::new(&limited_managed, &native)
            .transform(
                &request(DerivativeKind::OptimizedVideo),
                MediaRoutingPolicy::default(),
            )
            .await
            .expect("capability fallback");
        assert_eq!(capability.route, MediaRoute::NativeByCapability);

        let fallback_native =
            MemoryMediaTransformer::new(MediaExecutor::NativeGstreamer, [DerivativeKind::Frame]);
        let failure = MediaRouter::new(&FailingManaged, &fallback_native)
            .transform(
                &request(DerivativeKind::Frame),
                MediaRoutingPolicy::default(),
            )
            .await
            .expect("provider fallback");
        assert_eq!(failure.route, MediaRoute::NativeAfterManagedFailure);
        assert_eq!(failure.result.executor, MediaExecutor::NativeGstreamer);
    }

    #[tokio::test]
    async fn kill_switch_and_fallback_policy_fail_closed() {
        let managed = FailingManaged;
        let native =
            MemoryMediaTransformer::new(MediaExecutor::NativeGstreamer, [DerivativeKind::Frame]);
        let router = MediaRouter::new(&managed, &native);
        let disabled = router
            .transform(
                &request(DerivativeKind::Frame),
                MediaRoutingPolicy {
                    managed_enabled: false,
                    allow_native_fallback: true,
                },
            )
            .await
            .expect("kill switch route");
        assert_eq!(disabled.route, MediaRoute::NativeByPolicy);
        assert_eq!(
            router
                .transform(
                    &request(DerivativeKind::Frame),
                    MediaRoutingPolicy {
                        managed_enabled: true,
                        allow_native_fallback: false,
                    },
                )
                .await,
            Err(ApplicationError::Unavailable)
        );
    }
}
