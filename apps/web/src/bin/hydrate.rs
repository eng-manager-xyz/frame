#[cfg(all(target_arch = "wasm32", feature = "hydrate"))]
fn main() {
    use frame_web::hydration::{
        HydrationBoundary, PLAYER_HELP_ROOT_ID, PlayerKeyboardHelp, ROOT_ID,
    };
    use wasm_bindgen::JsCast;

    let document = leptos::tachys::dom::document();
    if let Some(root) = document.get_element_by_id(ROOT_ID) {
        leptos::mount::hydrate_from(root.unchecked_into(), HydrationBoundary).forget();
    }
    if let Some(root) = document.get_element_by_id(PLAYER_HELP_ROOT_ID) {
        leptos::mount::hydrate_from(root.unchecked_into(), PlayerKeyboardHelp).forget();
    }
}

#[cfg(not(all(target_arch = "wasm32", feature = "hydrate")))]
fn main() {}
