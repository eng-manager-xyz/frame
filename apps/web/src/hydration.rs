use leptos::prelude::*;

pub const ROOT_ID: &str = "frame-hydration-root";
pub const PLAYER_HELP_ROOT_ID: &str = "frame-player-help-root";

/// A deliberately data-free hydration boundary shared by the native SSR
/// renderer and browser Wasm. Private state continues to come only from a
/// same-origin, server-authorized bootstrap; hydration never infers a session.
#[component]
pub fn HydrationBoundary() -> impl IntoView {
    let ready = RwSignal::new(false);
    Effect::new(move |_| ready.set(true));

    view! {
        <span
            id="frame-hydration-state"
            hidden
            aria-hidden="true"
            data-frame-island-version="1"
            data-frame-hydrated=move || ready.get().then_some("true")
        >
            {move || if ready.get() {
                "Interactive enhancements ready."
            } else {
                "Server-rendered content ready."
            }}
        </span>
    }
}

/// Progressive player help is the only custom state in the initial web Wasm
/// slice. Native video controls and static keyboard help remain available when
/// JavaScript or its bundle fails; hydration replaces that help with a compact
/// disclosure.
#[component]
pub fn PlayerKeyboardHelp() -> impl IntoView {
    let open = RwSignal::new(false);
    let hydrated = RwSignal::new(false);
    Effect::new(move |_| hydrated.set(true));

    view! {
        <section
            class="player-keyboard-help"
            data-frame-island-version="1"
            aria-labelledby="player-keyboard-help-title"
            data-frame-enhanced=move || hydrated.get().then_some("true")
        >
            <h2 id="player-keyboard-help-title">"Keyboard playback help"</h2>
            <button
                class="button secondary compact hydration-only"
                type="button"
                aria-controls="player-keyboard-help-panel"
                aria-expanded=move || open.get().to_string()
                on:click=move |_| open.update(|value| *value = !*value)
            >
                {move || if open.get() { "Hide shortcuts" } else { "Show shortcuts" }}
            </button>
            <div
                id="player-keyboard-help-panel"
                class="hydration-only player-keyboard-help-panel"
                hidden=move || !open.get()
            >
                <p>
                    "Focus the native player controls. Use Space to play or pause, arrow keys to seek or adjust volume, and the captions control to select an available track."
                </p>
            </div>
            <p class="player-keyboard-help-fallback">
                "Focus the native player controls. Use Space to play or pause, arrow keys to seek or adjust volume, and the captions control to select an available track."
            </p>
        </section>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hydration_boundary_has_stable_server_markup() {
        let html = view! { <HydrationBoundary/> }.to_html();
        assert!(html.contains("frame-hydration-state"));
        assert!(html.contains("data-frame-island-version=\"1\""));
        assert!(html.contains("Server-rendered content ready."));
        assert!(html.contains("aria-hidden=\"true\""));
        assert!(!html.contains("aria-live="));
        assert!(!html.contains("role=\"status\""));
        assert!(!html.contains("Interactive enhancements ready."));
        assert!(!html.contains("data-frame-hydrated=\"true\""));
    }

    #[test]
    fn player_help_keeps_a_static_degraded_mode_fallback() {
        let html = view! { <PlayerKeyboardHelp/> }.to_html();
        assert!(html.contains("player-keyboard-help-fallback"));
        assert!(!html.contains("<noscript>"));
        assert!(html.contains("Keyboard playback help"));
        assert!(html.contains("data-frame-island-version=\"1\""));
        assert!(html.contains("aria-controls=\"player-keyboard-help-panel\""));
        assert!(html.contains("aria-expanded=\"false\""));
        assert!(html.contains(" hidden"));
        assert!(!html.contains("onclick="));
    }
}
