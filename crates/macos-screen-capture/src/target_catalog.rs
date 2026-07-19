use std::{collections::BTreeMap, fmt};

use frame_media::{
    DisplayGeometryTransform, LogicalRect, MAX_SCREEN_TARGETS, ScreenSourceInstanceId,
    ScreenTargetBinding, ScreenTargetDescriptor, ScreenTargetEpoch, ScreenTargetId,
    ScreenTargetKind, ScreenTargetSnapshot, VideoFrameSpec,
};
use ring::hmac;

use crate::MacOsCaptureError;

const TARGET_TOKEN_DOMAIN: &[u8] = b"frame/macos-capture-target/v2\0";

/// One user-selected region, bound to an opaque display identity from a prior
/// catalog. Native display handles never cross the adapter boundary.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct MacOsRegionSelection {
    display: ScreenTargetBinding,
    logical_bounds: LogicalRect,
}

impl MacOsRegionSelection {
    pub fn new(
        display: ScreenTargetBinding,
        logical_bounds: LogicalRect,
    ) -> Result<Self, MacOsCaptureError> {
        if display.id().kind() != ScreenTargetKind::Display {
            return Err(MacOsCaptureError::RegionRequiresDisplayTarget);
        }
        Ok(Self {
            display,
            logical_bounds,
        })
    }

    #[must_use]
    pub const fn display(self) -> ScreenTargetBinding {
        self.display
    }

    #[must_use]
    pub const fn logical_bounds(self) -> LogicalRect {
        self.logical_bounds
    }
}

impl fmt::Debug for MacOsRegionSelection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MacOsRegionSelection")
            .field("display", &self.display)
            .field("logical_bounds", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct NativeDisplayRecord {
    display_id: u32,
    transform: DisplayGeometryTransform,
}

impl NativeDisplayRecord {
    pub(crate) const fn new(display_id: u32, transform: DisplayGeometryTransform) -> Self {
        Self {
            display_id,
            transform,
        }
    }

    pub(crate) const fn display_id(self) -> u32 {
        self.display_id
    }

    pub(crate) const fn transform(self) -> DisplayGeometryTransform {
        self.transform
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct NativeWindowRecord {
    window_id: u32,
    owner_pid: i32,
    logical_bounds: LogicalRect,
}

impl NativeWindowRecord {
    pub(crate) const fn new(window_id: u32, owner_pid: i32, logical_bounds: LogicalRect) -> Self {
        Self {
            window_id,
            owner_pid,
            logical_bounds,
        }
    }

    pub(crate) const fn window_id(self) -> u32 {
        self.window_id
    }

    pub(crate) const fn owner_pid(self) -> i32 {
        self.owner_pid
    }

    pub(crate) const fn logical_bounds(self) -> LogicalRect {
        self.logical_bounds
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum NativeTargetRecord {
    Display(NativeDisplayRecord),
    Window(NativeWindowRecord),
    Region {
        display: NativeDisplayRecord,
        logical_bounds: LogicalRect,
    },
}

impl NativeTargetRecord {
    pub(crate) fn validate_output(self, output: VideoFrameSpec) -> Result<(), MacOsCaptureError> {
        let expected = match self {
            Self::Display(display) => Some(display.transform().physical_bounds()),
            Self::Region {
                display,
                logical_bounds,
            } => Some(
                display
                    .transform()
                    .logical_rect_to_physical(logical_bounds)
                    .map_err(|_| MacOsCaptureError::InvalidRegionGeometry)?,
            ),
            Self::Window(_) => None,
        };
        if let Some(expected) = expected
            && !dimensions_preserve_aspect(
                expected.width(),
                expected.height(),
                output.width,
                output.height,
            )
        {
            return Err(MacOsCaptureError::OutputAspectRatioDoesNotMatchTarget);
        }
        Ok(())
    }
}

fn dimensions_preserve_aspect(
    source_width: u32,
    source_height: u32,
    output_width: u32,
    output_height: u32,
) -> bool {
    let left = u64::from(source_width) * u64::from(output_height);
    let right = u64::from(source_height) * u64::from(output_width);
    // Integer downscaling rounds one output axis. Permit at most one output
    // pixel of that rounding, while rejecting a materially different canvas.
    left.abs_diff(right) <= u64::from(source_width.max(source_height))
}

pub(crate) struct BuiltTargetCatalog {
    snapshot: ScreenTargetSnapshot,
    target_map: BTreeMap<ScreenTargetBinding, NativeTargetRecord>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct ResolvedRegionSelection {
    display: NativeDisplayRecord,
    logical_bounds: LogicalRect,
}

pub(crate) fn resolve_region_selections(
    target_map: &BTreeMap<ScreenTargetBinding, NativeTargetRecord>,
    regions: &[MacOsRegionSelection],
) -> Result<Vec<ResolvedRegionSelection>, MacOsCaptureError> {
    if regions.len() > MAX_SCREEN_TARGETS {
        return Err(MacOsCaptureError::TargetCatalogLimitExceeded);
    }
    regions
        .iter()
        .map(|selection| {
            let NativeTargetRecord::Display(display) = target_map
                .get(&selection.display())
                .copied()
                .ok_or(MacOsCaptureError::StaleOrForeignRegionDisplay)?
            else {
                return Err(MacOsCaptureError::StaleOrForeignRegionDisplay);
            };
            Ok(ResolvedRegionSelection {
                display,
                logical_bounds: selection.logical_bounds(),
            })
        })
        .collect()
}

impl BuiltTargetCatalog {
    pub(crate) fn into_parts(
        self,
    ) -> (
        ScreenTargetSnapshot,
        BTreeMap<ScreenTargetBinding, NativeTargetRecord>,
    ) {
        (self.snapshot, self.target_map)
    }
}

pub(crate) fn assemble_records(
    mut displays: Vec<NativeDisplayRecord>,
    mut windows: Vec<NativeWindowRecord>,
    regions: &[ResolvedRegionSelection],
) -> Result<Vec<NativeTargetRecord>, MacOsCaptureError> {
    let record_count = displays
        .len()
        .checked_add(windows.len())
        .and_then(|count| count.checked_add(regions.len()))
        .ok_or(MacOsCaptureError::TargetCatalogLimitExceeded)?;
    if record_count > MAX_SCREEN_TARGETS {
        return Err(MacOsCaptureError::TargetCatalogLimitExceeded);
    }
    displays.sort_unstable_by_key(|display| display.display_id());
    windows.sort_unstable_by_key(|window| window.window_id());
    if displays
        .windows(2)
        .any(|pair| pair[0].display_id() == pair[1].display_id())
        || windows
            .windows(2)
            .any(|pair| pair[0].window_id() == pair[1].window_id())
    {
        return Err(MacOsCaptureError::DuplicateNativeTarget);
    }

    let mut records = Vec::with_capacity(record_count);
    records.extend(displays.iter().copied().map(NativeTargetRecord::Display));
    records.extend(windows.into_iter().map(NativeTargetRecord::Window));

    let mut sorted_regions = regions.to_vec();
    sorted_regions.sort_unstable_by_key(|selection| {
        let bounds = selection.logical_bounds;
        (
            selection.display.display_id(),
            bounds.x(),
            bounds.y(),
            bounds.width(),
            bounds.height(),
        )
    });
    if sorted_regions.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(MacOsCaptureError::DuplicateNativeTarget);
    }
    for selection in sorted_regions {
        let display = displays
            .iter()
            .copied()
            .find(|display| display.display_id() == selection.display.display_id())
            .ok_or(MacOsCaptureError::TargetNoLongerAvailable)?;
        if display != selection.display {
            return Err(MacOsCaptureError::StaleTargetTopology);
        }
        if !display
            .transform()
            .logical_bounds()
            .contains_rect(selection.logical_bounds)
        {
            return Err(MacOsCaptureError::InvalidRegionGeometry);
        }
        records.push(NativeTargetRecord::Region {
            display,
            logical_bounds: selection.logical_bounds,
        });
    }
    Ok(records)
}

pub(crate) fn exclude_current_process_windows(
    windows: impl IntoIterator<Item = NativeWindowRecord>,
    current_pid: i32,
) -> Vec<NativeWindowRecord> {
    windows
        .into_iter()
        .filter(|window| window.owner_pid() != current_pid)
        .take(MAX_SCREEN_TARGETS.saturating_add(1))
        .collect()
}

pub(crate) fn build_catalog(
    session_secret: &[u8; 32],
    source_instance: ScreenSourceInstanceId,
    generation: u64,
    records: &[NativeTargetRecord],
) -> Result<BuiltTargetCatalog, MacOsCaptureError> {
    let epoch =
        ScreenTargetEpoch::new(generation).map_err(|_| MacOsCaptureError::MediaCatalogRejected)?;
    let mut target_map = BTreeMap::new();
    let mut display_bindings = BTreeMap::new();
    let mut targets = Vec::with_capacity(records.len());

    for record in records {
        let identity = NativeTargetIdentity::from_record(*record);
        let target_id = derive_target_id(session_secret, identity)?;
        let binding = ScreenTargetBinding::new(source_instance, generation, epoch, target_id)
            .map_err(|_| MacOsCaptureError::MediaCatalogRejected)?;
        if target_map.insert(binding, *record).is_some() {
            return Err(MacOsCaptureError::TargetTokenCollision);
        }
        let descriptor = match *record {
            NativeTargetRecord::Display(display) => {
                display_bindings.insert(display.display_id(), binding);
                ScreenTargetDescriptor::display(binding, display.transform())
            }
            NativeTargetRecord::Window(window) => {
                ScreenTargetDescriptor::window(binding, window.logical_bounds())
            }
            NativeTargetRecord::Region {
                display,
                logical_bounds,
            } => {
                let containing_display = display_bindings
                    .get(&display.display_id())
                    .copied()
                    .ok_or(MacOsCaptureError::MediaCatalogRejected)?;
                ScreenTargetDescriptor::region(
                    binding,
                    containing_display,
                    logical_bounds,
                    display.transform(),
                )
            }
        }
        .map_err(|_| MacOsCaptureError::MediaCatalogRejected)?;
        targets.push(descriptor);
    }
    let snapshot = ScreenTargetSnapshot::new(source_instance, generation, targets)
        .map_err(|_| MacOsCaptureError::MediaCatalogRejected)?;
    Ok(BuiltTargetCatalog {
        snapshot,
        target_map,
    })
}

#[derive(Clone, Copy)]
enum NativeTargetIdentity {
    Display(u32),
    Window(u32),
    Region {
        display_id: u32,
        logical_bounds: LogicalRect,
    },
}

impl NativeTargetIdentity {
    const fn from_record(record: NativeTargetRecord) -> Self {
        match record {
            NativeTargetRecord::Display(display) => Self::Display(display.display_id()),
            NativeTargetRecord::Window(window) => Self::Window(window.window_id()),
            NativeTargetRecord::Region {
                display,
                logical_bounds,
            } => Self::Region {
                display_id: display.display_id(),
                logical_bounds,
            },
        }
    }

    const fn kind(self) -> ScreenTargetKind {
        match self {
            Self::Display(_) => ScreenTargetKind::Display,
            Self::Window(_) => ScreenTargetKind::Window,
            Self::Region { .. } => ScreenTargetKind::Region,
        }
    }
}

fn derive_target_id(
    session_secret: &[u8; 32],
    identity: NativeTargetIdentity,
) -> Result<ScreenTargetId, MacOsCaptureError> {
    let key = hmac::Key::new(hmac::HMAC_SHA256, session_secret);
    let mut context = hmac::Context::with_key(&key);
    context.update(TARGET_TOKEN_DOMAIN);
    match identity {
        NativeTargetIdentity::Display(display_id) => {
            context.update(&[0]);
            context.update(&display_id.to_be_bytes());
        }
        NativeTargetIdentity::Window(window_id) => {
            context.update(&[1]);
            context.update(&window_id.to_be_bytes());
        }
        NativeTargetIdentity::Region {
            display_id,
            logical_bounds,
        } => {
            context.update(&[2]);
            context.update(&display_id.to_be_bytes());
            context.update(&logical_bounds.x().to_be_bytes());
            context.update(&logical_bounds.y().to_be_bytes());
            context.update(&logical_bounds.width().to_be_bytes());
            context.update(&logical_bounds.height().to_be_bytes());
        }
    }
    let tag = context.sign();
    let mut opaque = [0_u8; 16];
    opaque.copy_from_slice(&tag.as_ref()[..16]);
    ScreenTargetId::new(identity.kind(), opaque)
        .map_err(|_| MacOsCaptureError::TargetTokenCollision)
}

#[cfg(test)]
mod tests {
    use frame_media::{DpiScale, PhysicalRect, Rotation};

    use super::*;

    fn source() -> ScreenSourceInstanceId {
        ScreenSourceInstanceId::new([9; 16]).expect("source")
    }

    fn display(display_id: u32, x: i32) -> NativeDisplayRecord {
        let logical = LogicalRect::new(x, 0, 1_920, 1_080).expect("logical");
        let transform = DisplayGeometryTransform::new(
            logical,
            PhysicalRect::new(0, 0, 3_840, 2_160).expect("physical"),
            DpiScale::new(2, 1).expect("scale"),
            Rotation::Degrees0,
        )
        .expect("transform");
        NativeDisplayRecord::new(display_id, transform)
    }

    fn records(secret: &[u8; 32]) -> Vec<NativeTargetRecord> {
        let display = display(7, 0);
        let display_records =
            assemble_records(vec![display], vec![], &[]).expect("display records");
        let (display_catalog, target_map) = build_catalog(secret, source(), 1, &display_records)
            .expect("display catalog")
            .into_parts();
        let region = MacOsRegionSelection::new(
            display_catalog.targets()[0].binding(),
            LogicalRect::new(10, 20, 100, 80).expect("region"),
        )
        .expect("selection");
        let regions = resolve_region_selections(&target_map, &[region]).expect("resolved region");
        assemble_records(
            vec![display],
            vec![NativeWindowRecord::new(
                11,
                42,
                LogicalRect::new(50, 60, 640, 480).expect("window"),
            )],
            &regions,
        )
        .expect("records")
    }

    fn assert_records_error(
        result: Result<Vec<NativeTargetRecord>, MacOsCaptureError>,
        expected: MacOsCaptureError,
    ) {
        match result {
            Err(error) => assert_eq!(error, expected),
            Ok(_) => panic!("expected target catalog construction to fail"),
        }
    }

    #[test]
    fn opaque_ids_are_session_bound_kind_separated_and_geometry_sensitive() {
        let first = records(&[1; 32]);
        let first_catalog = build_catalog(&[1; 32], source(), 1, &first).expect("catalog");
        let (first_snapshot, _) = first_catalog.into_parts();
        let repeat = build_catalog(&[1; 32], source(), 1, &first).expect("repeat");
        let (repeat_snapshot, _) = repeat.into_parts();
        let second_records = records(&[2; 32]);
        let second = build_catalog(&[2; 32], source(), 1, &second_records).expect("second");
        let (second_snapshot, _) = second.into_parts();

        assert_eq!(
            first_snapshot
                .targets()
                .iter()
                .map(ScreenTargetDescriptor::id)
                .collect::<Vec<_>>(),
            repeat_snapshot
                .targets()
                .iter()
                .map(ScreenTargetDescriptor::id)
                .collect::<Vec<_>>()
        );
        assert_ne!(
            first_snapshot.targets()[0].id(),
            second_snapshot.targets()[0].id()
        );
        assert_eq!(
            first_snapshot.targets()[0].kind(),
            ScreenTargetKind::Display
        );
        assert_eq!(first_snapshot.targets()[1].kind(), ScreenTargetKind::Window);
        assert_eq!(first_snapshot.targets()[2].kind(), ScreenTargetKind::Region);

        let first_region = derive_target_id(
            &[1; 32],
            NativeTargetIdentity::Region {
                display_id: 7,
                logical_bounds: LogicalRect::new(10, 20, 100, 80).expect("region"),
            },
        )
        .expect("first region");
        let moved_region = derive_target_id(
            &[1; 32],
            NativeTargetIdentity::Region {
                display_id: 7,
                logical_bounds: LogicalRect::new(11, 20, 100, 80).expect("moved region"),
            },
        )
        .expect("moved region");
        assert_ne!(first_region, moved_region);
    }

    #[test]
    fn region_must_use_a_current_display_and_stay_inside_its_bounds() {
        let secret = [3; 32];
        let records = assemble_records(vec![display(7, 0)], vec![], &[]).expect("records");
        let (catalog, target_map) = build_catalog(&secret, source(), 1, &records)
            .expect("catalog")
            .into_parts();
        let outside = MacOsRegionSelection::new(
            catalog.targets()[0].binding(),
            LogicalRect::new(1_900, 0, 100, 100).expect("outside"),
        )
        .expect("selection shape");
        let outside = resolve_region_selections(&target_map, &[outside]).expect("resolved outside");
        assert_records_error(
            assemble_records(vec![display(7, 0)], vec![], &outside),
            MacOsCaptureError::InvalidRegionGeometry,
        );
        let (foreign_catalog, _) = build_catalog(&[4; 32], source(), 1, &records)
            .expect("foreign catalog")
            .into_parts();
        let stale = MacOsRegionSelection::new(
            foreign_catalog.targets()[0].binding(),
            LogicalRect::new(0, 0, 100, 100).expect("inside"),
        )
        .expect("selection shape");
        assert_eq!(
            resolve_region_selections(&target_map, &[stale]).map(drop),
            Err(MacOsCaptureError::StaleOrForeignRegionDisplay)
        );

        let inside = MacOsRegionSelection::new(
            catalog.targets()[0].binding(),
            LogicalRect::new(0, 0, 100, 100).expect("inside"),
        )
        .expect("selection shape");
        let inside = resolve_region_selections(&target_map, &[inside]).expect("resolved inside");
        assert_records_error(
            assemble_records(vec![display(7, 100)], vec![], &inside),
            MacOsCaptureError::StaleTargetTopology,
        );
    }

    #[test]
    fn snapshot_rejects_duplicate_native_targets_and_generation_rebinds_every_target() {
        let secret = [5; 32];
        assert_records_error(
            assemble_records(vec![display(7, 0), display(7, 0)], vec![], &[]),
            MacOsCaptureError::DuplicateNativeTarget,
        );
        let records = records(&secret);
        let (first, _) = build_catalog(&secret, source(), 1, &records)
            .expect("first")
            .into_parts();
        let (second, _) = build_catalog(&secret, source(), 2, &records)
            .expect("second")
            .into_parts();
        assert!(
            first
                .targets()
                .iter()
                .zip(second.targets())
                .all(|(left, right)| left.id() == right.id()
                    && left.binding() != right.binding()
                    && left.target_epoch() != right.target_epoch())
        );
    }

    #[test]
    fn catalog_limit_and_region_target_kind_fail_closed() {
        let displays = (1..=u32::try_from(MAX_SCREEN_TARGETS + 1).expect("bounded test size"))
            .map(|display_id| display(display_id, 0))
            .collect();
        assert_records_error(
            assemble_records(displays, vec![], &[]),
            MacOsCaptureError::TargetCatalogLimitExceeded,
        );

        let window = ScreenTargetBinding::new(
            source(),
            1,
            ScreenTargetEpoch::new(1).expect("epoch"),
            ScreenTargetId::new(ScreenTargetKind::Window, [8; 16]).expect("window"),
        )
        .expect("binding");
        assert_eq!(
            MacOsRegionSelection::new(window, LogicalRect::new(0, 0, 100, 100).expect("bounds")),
            Err(MacOsCaptureError::RegionRequiresDisplayTarget)
        );

        let too_many_regions = vec![
            MacOsRegionSelection {
                display: window,
                logical_bounds: LogicalRect::new(0, 0, 100, 100).expect("bounds"),
            };
            MAX_SCREEN_TARGETS + 1
        ];
        assert_eq!(
            resolve_region_selections(&BTreeMap::new(), &too_many_regions).map(drop),
            Err(MacOsCaptureError::TargetCatalogLimitExceeded)
        );
    }

    #[test]
    fn duplicate_regions_are_rejected_before_token_derivation() {
        let region = ResolvedRegionSelection {
            display: display(7, 0),
            logical_bounds: LogicalRect::new(10, 20, 100, 80).expect("region"),
        };
        assert_records_error(
            assemble_records(vec![display(7, 0)], vec![], &[region, region]),
            MacOsCaptureError::DuplicateNativeTarget,
        );
    }

    #[test]
    fn display_and_region_outputs_allow_only_aspect_preserving_scaling() {
        let display = display(7, 0);
        let region = NativeTargetRecord::Region {
            display,
            logical_bounds: LogicalRect::new(10, 20, 100, 80).expect("region"),
        };
        let mut output = VideoFrameSpec {
            width: 200,
            height: 160,
            pixel_format: frame_media::PixelFormat::Bgra8,
            color_space: frame_media::ColorSpace::Srgb,
            nominal_frame_duration_ns: 33_333_333,
            memory: frame_media::FrameMemory::Cpu,
        };
        assert_eq!(region.validate_output(output), Ok(()));
        output.width = 100;
        output.height = 80;
        assert_eq!(region.validate_output(output), Ok(()));
        output.width = 103;
        assert_eq!(
            region.validate_output(output),
            Err(MacOsCaptureError::OutputAspectRatioDoesNotMatchTarget)
        );
    }

    #[test]
    fn window_catalog_excludes_every_current_process_window_without_titles_or_handles() {
        let bounds = LogicalRect::new(0, 0, 640, 480).expect("bounds");
        let windows = vec![
            NativeWindowRecord::new(1, 77, bounds),
            NativeWindowRecord::new(2, 88, bounds),
            NativeWindowRecord::new(3, 77, bounds),
            NativeWindowRecord::new(4, 99, bounds),
        ];
        let filtered = exclude_current_process_windows(windows, 77);
        assert_eq!(filtered.len(), 2);
        assert!(filtered.iter().all(|window| window.owner_pid() != 77));
        assert_eq!(
            filtered
                .iter()
                .map(|window| window.window_id())
                .collect::<Vec<_>>(),
            vec![2, 4]
        );
    }

    #[test]
    fn public_debug_output_redacts_region_geometry_and_native_values_stay_internal() {
        let records = assemble_records(vec![display(7, 0)], vec![], &[]).expect("records");
        let (catalog, _) = build_catalog(&[7; 32], source(), 1, &records)
            .expect("catalog")
            .into_parts();
        let selection = MacOsRegionSelection::new(
            catalog.targets()[0].binding(),
            LogicalRect::new(-123, 456, 789, 321).expect("region"),
        )
        .expect("selection");
        let rendered = format!("{selection:?}");
        assert!(!rendered.contains("-123"));
        assert!(!rendered.contains("456"));
        assert!(!rendered.contains("789"));
        assert!(rendered.contains("<redacted>"));
    }
}
