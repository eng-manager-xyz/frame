#[cfg(all(target_arch = "wasm32", feature = "hydrate"))]
fn main() {
    use frame_web::hydration::{
        AUTHENTICATED_ROOT_ID, AuthenticatedWorkspacePanel, HydrationBoundary, PLAYER_HELP_ROOT_ID,
        PUBLIC_COLLABORATION_ROOT_ID, PlayerKeyboardHelp, PublicCollaborationPanel, ROOT_ID,
    };
    use wasm_bindgen::JsCast;

    let document = leptos::tachys::dom::document();
    if let Some(root) = document.get_element_by_id(ROOT_ID) {
        leptos::mount::hydrate_from(root.unchecked_into(), HydrationBoundary).forget();
    }
    if let Some(root) = document.get_element_by_id(AUTHENTICATED_ROOT_ID)
        && root.get_attribute("data-frame-browser-loader").as_deref() == Some("true")
    {
        // Render emitted only a generic no-store shell. The browser owns this
        // island and sends its cookie directly to the same-origin Worker.
        root.set_inner_html("");
        leptos::mount::mount_to(root.unchecked_into(), AuthenticatedWorkspacePanel).forget();
    }
    if let Some(root) = document.get_element_by_id(PUBLIC_COLLABORATION_ROOT_ID) {
        // This island owns asynchronous lists whose client shape changes as
        // soon as the same-origin DTOs arrive. Replace only its data-free SSR
        // fallback so Leptos does not share a hydration cursor with the
        // independent player-help island.
        root.set_inner_html("");
        leptos::mount::mount_to(root.unchecked_into(), PublicCollaborationPanel).forget();
    }
    if let Some(root) = document.get_element_by_id(PLAYER_HELP_ROOT_ID) {
        leptos::mount::hydrate_from(root.unchecked_into(), PlayerKeyboardHelp).forget();
    }
}

#[cfg(not(all(target_arch = "wasm32", feature = "hydrate")))]
fn main() {}
