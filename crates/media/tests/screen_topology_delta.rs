use frame_media::{
    DisplayGeometryTransform, DpiScale, LogicalRect, PhysicalRect, Rotation,
    ScreenSourceInstanceId, ScreenTargetBinding, ScreenTargetDelta, ScreenTargetDescriptor,
    ScreenTargetEpoch, ScreenTargetId, ScreenTargetKind, ScreenTargetSnapshot,
};

fn source(byte: u8) -> ScreenSourceInstanceId {
    ScreenSourceInstanceId::new([byte; 16]).expect("source")
}

fn target_id(kind: ScreenTargetKind, byte: u8) -> ScreenTargetId {
    ScreenTargetId::new(kind, [byte; 16]).expect("target identity")
}

fn binding(
    source: ScreenSourceInstanceId,
    generation: u64,
    kind: ScreenTargetKind,
    byte: u8,
) -> ScreenTargetBinding {
    ScreenTargetBinding::new(
        source,
        generation,
        ScreenTargetEpoch::new(generation).expect("target epoch"),
        target_id(kind, byte),
    )
    .expect("target binding")
}

fn display(
    source: ScreenSourceInstanceId,
    generation: u64,
    byte: u8,
    width: u32,
) -> ScreenTargetDescriptor {
    let logical = LogicalRect::new(0, 0, width, 180).expect("logical");
    let physical = PhysicalRect::new(0, 0, width, 180).expect("physical");
    let transform = DisplayGeometryTransform::new(
        logical,
        physical,
        DpiScale::new(1, 1).expect("scale"),
        Rotation::Degrees0,
    )
    .expect("transform");
    ScreenTargetDescriptor::display(
        binding(source, generation, ScreenTargetKind::Display, byte),
        transform,
    )
    .expect("display")
}

fn window(source: ScreenSourceInstanceId, generation: u64, byte: u8) -> ScreenTargetDescriptor {
    ScreenTargetDescriptor::window(
        binding(source, generation, ScreenTargetKind::Window, byte),
        LogicalRect::new(10, 20, 300, 200).expect("window bounds"),
    )
    .expect("window")
}

fn snapshot(
    source: ScreenSourceInstanceId,
    generation: u64,
    targets: Vec<ScreenTargetDescriptor>,
) -> ScreenTargetSnapshot {
    ScreenTargetSnapshot::new(source, generation, targets).expect("snapshot")
}

#[test]
fn selected_removal_wins_over_an_unrelated_addition() {
    let source = source(1);
    let selected = window(source, 1, 2);
    let previous = snapshot(
        source,
        1,
        vec![display(source, 1, 1, 320), selected.clone()],
    );
    let current = snapshot(
        source,
        2,
        vec![display(source, 2, 1, 320), window(source, 2, 3)],
    );

    assert_eq!(
        current.first_delta_from(&previous, selected.id()),
        Ok(Some(ScreenTargetDelta::Removed(selected)))
    );
}

#[test]
fn generation_rebinding_without_semantic_change_uses_the_real_hotplug() {
    let source = source(1);
    let selected = display(source, 1, 1, 320);
    let previous = snapshot(source, 1, vec![selected.clone()]);
    let added = window(source, 2, 2);
    let current = snapshot(source, 2, vec![display(source, 2, 1, 320), added.clone()]);

    assert_eq!(
        current.first_delta_from(&previous, selected.id()),
        Ok(Some(ScreenTargetDelta::Added(added)))
    );
}

#[test]
fn selected_geometry_change_is_reconfigured() {
    let source = source(1);
    let selected = display(source, 1, 1, 320);
    let previous = snapshot(source, 1, vec![selected.clone()]);
    let reconfigured = display(source, 2, 1, 640);
    let current = snapshot(source, 2, vec![reconfigured.clone()]);

    assert_eq!(
        current.first_delta_from(&previous, selected.id()),
        Ok(Some(ScreenTargetDelta::Reconfigured(reconfigured)))
    );
}

#[test]
fn unchanged_or_cross_source_snapshots_are_rejected_precisely() {
    let local_source = source(1);
    let previous = snapshot(local_source, 1, vec![display(local_source, 1, 1, 320)]);
    assert_eq!(
        previous.first_delta_from(&previous, target_id(ScreenTargetKind::Display, 1)),
        Ok(None)
    );

    let foreign = source(9);
    let current = snapshot(foreign, 2, vec![display(foreign, 2, 1, 320)]);
    assert_eq!(
        current.first_delta_from(&previous, target_id(ScreenTargetKind::Display, 1)),
        Err(frame_media::ScreenCaptureError::CrossSourceEvent)
    );
}
