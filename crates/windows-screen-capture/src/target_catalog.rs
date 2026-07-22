use std::collections::BTreeMap;

use frame_media::{
    DisplayGeometryTransform, LogicalRect, MAX_SCREEN_TARGETS, ScreenSourceInstanceId,
    ScreenTargetBinding, ScreenTargetDescriptor, ScreenTargetEpoch, ScreenTargetId,
    ScreenTargetKind, ScreenTargetSnapshot,
};
use ring::hmac;

use crate::{WindowsCaptureError, WindowsRegionSelection};

const TARGET_TOKEN_DOMAIN: &[u8] = b"frame/windows-capture-target/v1\0";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct NativeDisplayRecord {
    native_id: u64,
    transform: DisplayGeometryTransform,
}

impl NativeDisplayRecord {
    pub(crate) const fn new(native_id: u64, transform: DisplayGeometryTransform) -> Self {
        Self {
            native_id,
            transform,
        }
    }

    pub(crate) const fn native_id(self) -> u64 {
        self.native_id
    }

    pub(crate) const fn transform(self) -> DisplayGeometryTransform {
        self.transform
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct NativeWindowRecord {
    native_id: u64,
    logical_bounds: LogicalRect,
}

impl NativeWindowRecord {
    pub(crate) const fn new(native_id: u64, logical_bounds: LogicalRect) -> Self {
        Self {
            native_id,
            logical_bounds,
        }
    }

    pub(crate) const fn native_id(self) -> u64 {
        self.native_id
    }

    pub(crate) const fn logical_bounds(self) -> LogicalRect {
        self.logical_bounds
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NativeTargetRecord {
    Display(NativeDisplayRecord),
    Window(NativeWindowRecord),
    Region {
        display: NativeDisplayRecord,
        logical_bounds: LogicalRect,
    },
}

impl NativeTargetRecord {
    pub(crate) fn expected_dimensions(self) -> Result<(u32, u32), WindowsCaptureError> {
        match self {
            Self::Display(display) => {
                let bounds = display.transform().physical_bounds();
                Ok((bounds.width(), bounds.height()))
            }
            Self::Window(window) => {
                let bounds = window.logical_bounds();
                Ok((bounds.width(), bounds.height()))
            }
            Self::Region {
                display,
                logical_bounds,
            } => {
                let bounds = display
                    .transform()
                    .logical_rect_to_physical(logical_bounds)
                    .map_err(|_| WindowsCaptureError::InvalidRegionGeometry)?;
                Ok((bounds.width(), bounds.height()))
            }
        }
    }

    pub(crate) fn validate_output(
        self,
        width: u32,
        height: u32,
    ) -> Result<(), WindowsCaptureError> {
        if self.expected_dimensions()? != (width, height) {
            return Err(WindowsCaptureError::OutputDimensionsDoNotMatchTarget);
        }
        Ok(())
    }
}

pub(crate) struct BuiltTargetCatalog {
    snapshot: ScreenTargetSnapshot,
    target_map: BTreeMap<ScreenTargetBinding, NativeTargetRecord>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ResolvedRegionSelection {
    display: NativeDisplayRecord,
    logical_bounds: LogicalRect,
}

pub(crate) fn resolve_region_selections(
    target_map: &BTreeMap<ScreenTargetBinding, NativeTargetRecord>,
    regions: &[WindowsRegionSelection],
) -> Result<Vec<ResolvedRegionSelection>, WindowsCaptureError> {
    if regions.len() > MAX_SCREEN_TARGETS {
        return Err(WindowsCaptureError::TargetCatalogLimitExceeded);
    }
    regions
        .iter()
        .map(|selection| {
            let NativeTargetRecord::Display(display) = target_map
                .get(&selection.display())
                .copied()
                .ok_or(WindowsCaptureError::StaleOrForeignRegionDisplay)?
            else {
                return Err(WindowsCaptureError::StaleOrForeignRegionDisplay);
            };
            Ok(ResolvedRegionSelection {
                display,
                logical_bounds: selection.logical_bounds(),
            })
        })
        .collect()
}

pub(crate) fn assemble_records(
    mut displays: Vec<NativeDisplayRecord>,
    mut windows: Vec<NativeWindowRecord>,
    regions: &[ResolvedRegionSelection],
) -> Result<Vec<NativeTargetRecord>, WindowsCaptureError> {
    let count = displays
        .len()
        .checked_add(windows.len())
        .and_then(|count| count.checked_add(regions.len()))
        .ok_or(WindowsCaptureError::TargetCatalogLimitExceeded)?;
    if count > MAX_SCREEN_TARGETS {
        return Err(WindowsCaptureError::TargetCatalogLimitExceeded);
    }
    displays.sort_unstable_by_key(|display| display.native_id());
    windows.sort_unstable_by_key(|window| window.native_id());
    if displays
        .windows(2)
        .any(|pair| pair[0].native_id() == pair[1].native_id())
        || windows
            .windows(2)
            .any(|pair| pair[0].native_id() == pair[1].native_id())
    {
        return Err(WindowsCaptureError::DuplicateNativeTarget);
    }

    let mut records = Vec::with_capacity(count);
    records.extend(displays.iter().copied().map(NativeTargetRecord::Display));
    records.extend(windows.into_iter().map(NativeTargetRecord::Window));

    let mut sorted_regions = regions.to_vec();
    sorted_regions.sort_unstable_by_key(|region| {
        let bounds = region.logical_bounds;
        (
            region.display.native_id(),
            bounds.x(),
            bounds.y(),
            bounds.width(),
            bounds.height(),
        )
    });
    if sorted_regions.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(WindowsCaptureError::DuplicateNativeTarget);
    }
    for region in sorted_regions {
        let display = displays
            .iter()
            .copied()
            .find(|display| display.native_id() == region.display.native_id())
            .ok_or(WindowsCaptureError::TargetNoLongerAvailable)?;
        if display != region.display {
            return Err(WindowsCaptureError::StaleTargetTopology);
        }
        if !display
            .transform()
            .logical_bounds()
            .contains_rect(region.logical_bounds)
        {
            return Err(WindowsCaptureError::InvalidRegionGeometry);
        }
        records.push(NativeTargetRecord::Region {
            display,
            logical_bounds: region.logical_bounds,
        });
    }
    Ok(records)
}

pub(crate) fn build_catalog(
    session_secret: &[u8; 32],
    source_instance: ScreenSourceInstanceId,
    generation: u64,
    records: &[NativeTargetRecord],
) -> Result<BuiltTargetCatalog, WindowsCaptureError> {
    let epoch = ScreenTargetEpoch::new(generation)
        .map_err(|_| WindowsCaptureError::MediaCatalogRejected)?;
    let mut target_map = BTreeMap::new();
    let mut display_bindings = BTreeMap::new();
    let mut targets = Vec::with_capacity(records.len());
    for record in records {
        let target_id = derive_target_id(session_secret, *record)?;
        let binding = ScreenTargetBinding::new(source_instance, generation, epoch, target_id)
            .map_err(|_| WindowsCaptureError::MediaCatalogRejected)?;
        if target_map.insert(binding, *record).is_some() {
            return Err(WindowsCaptureError::IdentityUnavailable);
        }
        let descriptor = match *record {
            NativeTargetRecord::Display(display) => {
                display_bindings.insert(display.native_id(), binding);
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
                    .get(&display.native_id())
                    .copied()
                    .ok_or(WindowsCaptureError::MediaCatalogRejected)?;
                ScreenTargetDescriptor::region(
                    binding,
                    containing_display,
                    logical_bounds,
                    display.transform(),
                )
            }
        }
        .map_err(|_| WindowsCaptureError::MediaCatalogRejected)?;
        targets.push(descriptor);
    }
    let snapshot = ScreenTargetSnapshot::new(source_instance, generation, targets)
        .map_err(|_| WindowsCaptureError::MediaCatalogRejected)?;
    Ok(BuiltTargetCatalog {
        snapshot,
        target_map,
    })
}

fn derive_target_id(
    session_secret: &[u8; 32],
    record: NativeTargetRecord,
) -> Result<ScreenTargetId, WindowsCaptureError> {
    let key = hmac::Key::new(hmac::HMAC_SHA256, session_secret);
    let mut context = hmac::Context::with_key(&key);
    context.update(TARGET_TOKEN_DOMAIN);
    let kind = match record {
        NativeTargetRecord::Display(display) => {
            context.update(&[0]);
            context.update(&display.native_id().to_be_bytes());
            ScreenTargetKind::Display
        }
        NativeTargetRecord::Window(window) => {
            context.update(&[1]);
            context.update(&window.native_id().to_be_bytes());
            ScreenTargetKind::Window
        }
        NativeTargetRecord::Region {
            display,
            logical_bounds,
        } => {
            context.update(&[2]);
            context.update(&display.native_id().to_be_bytes());
            context.update(&logical_bounds.x().to_be_bytes());
            context.update(&logical_bounds.y().to_be_bytes());
            context.update(&logical_bounds.width().to_be_bytes());
            context.update(&logical_bounds.height().to_be_bytes());
            ScreenTargetKind::Region
        }
    };
    let tag = context.sign();
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&tag.as_ref()[..16]);
    ScreenTargetId::new(kind, bytes).map_err(|_| WindowsCaptureError::IdentityUnavailable)
}

#[cfg(test)]
mod tests {
    use frame_media::{
        DisplayGeometryTransform, DpiScale, PhysicalRect, Rotation, ScreenSourceInstanceId,
    };

    use super::*;

    fn display(native_id: u64, x: i32) -> NativeDisplayRecord {
        NativeDisplayRecord::new(
            native_id,
            DisplayGeometryTransform::new(
                LogicalRect::new(x, 0, 800, 600).expect("logical"),
                PhysicalRect::new(x * 2, 0, 1_600, 1_200).expect("physical"),
                DpiScale::new(2, 1).expect("scale"),
                Rotation::Degrees0,
            )
            .expect("transform"),
        )
    }

    #[test]
    fn catalog_sorts_native_records_and_uses_opaque_kind_safe_ids() {
        let window = NativeWindowRecord::new(
            5,
            LogicalRect::new(50, 50, 640, 480).expect("window bounds"),
        );
        let records = assemble_records(vec![display(9, 800), display(2, 0)], vec![window], &[])
            .expect("records");
        let source = ScreenSourceInstanceId::new([3; 16]).expect("source");
        let built = build_catalog(&[7; 32], source, 1, &records).expect("catalog");
        let (snapshot, map) = built.into_parts();
        assert_eq!(snapshot.targets().len(), 3);
        assert_eq!(map.len(), 3);
        assert!(
            snapshot
                .targets()
                .iter()
                .take(2)
                .all(|target| target.kind() == ScreenTargetKind::Display)
        );
        assert_ne!(snapshot.targets()[0].id(), snapshot.targets()[1].id());
    }

    #[test]
    fn region_must_be_inside_the_exact_catalog_display() {
        let display = display(2, 0);
        let records = assemble_records(vec![display], vec![], &[]).expect("records");
        let source = ScreenSourceInstanceId::new([3; 16]).expect("source");
        let (snapshot, map) = build_catalog(&[7; 32], source, 1, &records)
            .expect("catalog")
            .into_parts();
        let selection = WindowsRegionSelection::new(
            snapshot.targets()[0].binding(),
            LogicalRect::new(700, 500, 200, 200).expect("region"),
        )
        .expect("selection");
        let resolved = resolve_region_selections(&map, &[selection]).expect("resolved");
        assert_eq!(
            assemble_records(vec![display], vec![], &resolved),
            Err(WindowsCaptureError::InvalidRegionGeometry)
        );
    }

    #[test]
    fn output_must_match_native_pixels_without_an_unclaimed_scaler() {
        let record = NativeTargetRecord::Display(display(2, 0));
        assert!(record.validate_output(1_600, 1_200).is_ok());
        assert_eq!(
            record.validate_output(800, 600),
            Err(WindowsCaptureError::OutputDimensionsDoNotMatchTarget)
        );
    }
}
