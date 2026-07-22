//! Accessible numeric region selection for the native capture adapter.

use frame_desktop_core::{
    CaptureTargetKind, CaptureTargetSummary, DesktopRuntimeSnapshot, IpcCommand, WindowRole,
};
use frame_ui::{Button, ButtonGroup, ButtonVariant, FieldGroup, Input, Label, ToggleGroup};
use leptos::prelude::*;

use super::{DesktopClient, submit};

#[component]
pub(super) fn RegionPicker(
    client: RwSignal<Option<DesktopClient>>,
    snapshot: RwSignal<Option<DesktopRuntimeSnapshot>>,
    status: RwSignal<String>,
    error: RwSignal<Option<String>>,
    busy: RwSignal<bool>,
) -> impl IntoView {
    let display_token = RwSignal::new(None::<String>);
    let x = RwSignal::new(0_u32);
    let y = RwSignal::new(0_u32);
    let width = RwSignal::new(640_u32);
    let height = RwSignal::new(480_u32);

    view! {
        <FieldGroup class="region-picker">
            <legend>"Screen region"</legend>
            <p id="region-help">
                "Choose a display, then enter a rectangle in logical pixels from its top-left corner. The native backend verifies the region against the current display topology."
            </p>
            <ToggleGroup class="button-row" attr:aria-label="Region display">
                <For
                    each=move || display_targets(snapshot.get())
                    key=|target| target.token.clone()
                    children=move |target| {
                        let pressed_token = target.token.clone();
                        let selected_token = target.token.clone();
                        let (logical_width, logical_height) = display_logical_size(&target);
                        let label = format!(
                            "Use display {} ({} by {} logical pixels)",
                            target.ordinal, logical_width, logical_height
                        );
                        view! {
                            <Button
                                variant=ButtonVariant::Outline
                                attr:r#type="button"
                                attr:aria-pressed=move || display_token
                                    .get()
                                    .as_deref()
                                    == Some(pressed_token.as_str())
                                attr:disabled=move || busy.get()
                                on:click=move |_| {
                                    display_token.set(Some(selected_token.clone()));
                                    x.update(|value| *value = (*value).min(logical_width.saturating_sub(1)));
                                    y.update(|value| *value = (*value).min(logical_height.saturating_sub(1)));
                                    width.update(|value| {
                                        *value = (*value).min(logical_width.saturating_sub(x.get_untracked())).max(1);
                                    });
                                    height.update(|value| {
                                        *value = (*value).min(logical_height.saturating_sub(y.get_untracked())).max(1);
                                    });
                                }
                            >{label}</Button>
                        }
                    }
                />
            </ToggleGroup>

            <Label attr:r#for="region-x">"Left offset, logical pixels"</Label>
            <Input
                attr:id="region-x"
                attr:r#type="number"
                attr:min="0"
                attr:max="65534"
                attr:step="1"
                prop:value=move || x.get().to_string()
                on:input=move |event| set_bounded_u32(x, event_target_value(&event))
                attr:aria-describedby="region-help"
            />
            <Label attr:r#for="region-y">"Top offset, logical pixels"</Label>
            <Input
                attr:id="region-y"
                attr:r#type="number"
                attr:min="0"
                attr:max="65534"
                attr:step="1"
                prop:value=move || y.get().to_string()
                on:input=move |event| set_bounded_u32(y, event_target_value(&event))
                attr:aria-describedby="region-help"
            />
            <Label attr:r#for="region-width">"Width, logical pixels"</Label>
            <Input
                attr:id="region-width"
                attr:r#type="number"
                attr:min="1"
                attr:max="65535"
                attr:step="1"
                prop:value=move || width.get().to_string()
                on:input=move |event| set_bounded_u32(width, event_target_value(&event))
                attr:aria-describedby="region-help"
            />
            <Label attr:r#for="region-height">"Height, logical pixels"</Label>
            <Input
                attr:id="region-height"
                attr:r#type="number"
                attr:min="1"
                attr:max="65535"
                attr:step="1"
                prop:value=move || height.get().to_string()
                on:input=move |event| set_bounded_u32(height, event_target_value(&event))
                attr:aria-describedby="region-help"
            />
            <ButtonGroup class="button-row">
                <Button
                    variant=ButtonVariant::Outline
                    attr:r#type="button"
                    attr:disabled=move || busy.get() || !valid_region(
                        snapshot.get(),
                        display_token.get(),
                        x.get(),
                        y.get(),
                        width.get(),
                        height.get(),
                    )
                    on:click=move |_| {
                        let Some(display_token) = display_token.get_untracked() else {
                            return;
                        };
                        submit(
                            client,
                            snapshot,
                            status,
                            error,
                            busy,
                            WindowRole::Recorder,
                            IpcCommand::CaptureRegionDefine {
                                display_token,
                                x: x.get_untracked(),
                                y: y.get_untracked(),
                                width: width.get_untracked(),
                                height: height.get_untracked(),
                            },
                        );
                    }
                >"Define and select region"</Button>
            </ButtonGroup>
        </FieldGroup>
    }
}

fn display_targets(snapshot: Option<DesktopRuntimeSnapshot>) -> Vec<CaptureTargetSummary> {
    snapshot
        .map(|state| {
            state
                .capture_targets
                .targets
                .into_iter()
                .filter(|target| target.kind == CaptureTargetKind::Display)
                .collect()
        })
        .unwrap_or_default()
}

fn display_logical_size(target: &CaptureTargetSummary) -> (u32, u32) {
    let (unrotated_width, unrotated_height) = match target.rotation_degrees {
        90 | 270 => (target.height_pixels, target.width_pixels),
        0 | 180 => (target.width_pixels, target.height_pixels),
        _ => return (0, 0),
    };
    let denominator = u64::from(target.scale_denominator);
    let numerator = u64::from(target.scale_numerator.max(1));
    let width = u64::from(unrotated_width)
        .saturating_mul(denominator)
        .checked_div(numerator)
        .and_then(|value| u32::try_from(value).ok())
        .unwrap_or(0);
    let height = u64::from(unrotated_height)
        .saturating_mul(denominator)
        .checked_div(numerator)
        .and_then(|value| u32::try_from(value).ok())
        .unwrap_or(0);
    (width, height)
}

fn valid_region(
    snapshot: Option<DesktopRuntimeSnapshot>,
    display_token: Option<String>,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
) -> bool {
    let Some(display_token) = display_token else {
        return false;
    };
    let Some(display) = display_targets(snapshot)
        .into_iter()
        .find(|target| target.token == display_token)
    else {
        return false;
    };
    let (display_width, display_height) = display_logical_size(&display);
    width > 0
        && height > 0
        && x.checked_add(width)
            .is_some_and(|right| right <= display_width)
        && y.checked_add(height)
            .is_some_and(|bottom| bottom <= display_height)
}

fn set_bounded_u32(signal: RwSignal<u32>, value: String) {
    if let Ok(value) = value.parse::<u32>()
        && value <= u32::from(u16::MAX)
    {
        signal.set(value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn display(rotation_degrees: u16) -> CaptureTargetSummary {
        CaptureTargetSummary {
            token: "display-token".into(),
            kind: CaptureTargetKind::Display,
            ordinal: 1,
            width_pixels: 3_840,
            height_pixels: 2_160,
            scale_numerator: 2,
            scale_denominator: 1,
            rotation_degrees,
        }
    }

    #[test]
    fn logical_size_accounts_for_scale_and_rotation() {
        assert_eq!(display_logical_size(&display(0)), (1_920, 1_080));
        assert_eq!(display_logical_size(&display(90)), (1_080, 1_920));
    }
}
