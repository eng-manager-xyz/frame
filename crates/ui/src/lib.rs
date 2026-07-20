//! Accessible, shadcn-inspired Leptos primitives shared by Frame's web and
//! desktop frontends.
//!
//! The crate owns visual variants and semantic defaults. Product-specific
//! state stays in the consuming application, and arbitrary DOM attributes and
//! event handlers are forwarded to each component's root element by Leptos.

#[cfg(any(feature = "ssr", feature = "hydrate", feature = "csr"))]
mod button;
pub mod class_contract;
#[cfg(any(feature = "ssr", feature = "hydrate", feature = "csr"))]
mod display;
#[cfg(any(feature = "ssr", feature = "hydrate", feature = "csr"))]
mod form;
#[cfg(any(feature = "ssr", feature = "hydrate", feature = "csr"))]
mod layout;

#[cfg(any(feature = "ssr", feature = "hydrate", feature = "csr"))]
pub use button::{Button, ButtonLink, ButtonSize, ButtonVariant};
#[cfg(any(feature = "ssr", feature = "hydrate", feature = "csr"))]
pub use display::{
    Alert, AlertVariant, Badge, BadgeVariant, DialogContent, DialogOverlay, Meter, Progress,
};
#[cfg(any(feature = "ssr", feature = "hydrate", feature = "csr"))]
pub use form::{FieldGroup, Input, Label, Select, Textarea};
#[cfg(any(feature = "ssr", feature = "hydrate", feature = "csr"))]
pub use layout::{
    AspectRatio, ButtonGroup, Card, CardFrame, EmptyState, FeatureCard, NavigationMenu, Separator,
    ToggleGroup,
};

/// The minified stylesheet generated from `styles/tailwind.css` by the pinned
/// Tailwind CLI. Keeping it in the crate gives SSR and CSR one exact theme.
pub const STYLESHEET: &str = include_str!("../styles/tailwind.generated.css");

/// Emits the shared stylesheet for a standalone Leptos CSR document.
#[cfg(any(feature = "ssr", feature = "hydrate", feature = "csr"))]
#[leptos::component]
pub fn UiStyles() -> impl leptos::IntoView {
    use leptos::prelude::*;

    view! { <style data-frame-ui="shadcn-tailwind">{STYLESHEET}</style> }
}

#[cfg(all(test, any(feature = "ssr", feature = "hydrate", feature = "csr")))]
mod tests {
    use leptos::prelude::*;

    use super::{Alert, Badge, Button, ButtonLink, Card, Input, STYLESHEET, Select};

    #[test]
    fn primitives_render_semantic_native_controls() {
        let html = view! {
            <Card attr:aria-labelledby="title">
                <h2 id="title">"Title"</h2>
                <Alert attr:role="status">"Saved"</Alert>
                <Badge>"Owner"</Badge>
                <Input attr:id="name" attr:name="name"/>
                <Select attr:id="state"><option value="ready">"Ready"</option></Select>
                <Button attr:r#type="submit">"Save"</Button>
                <ButtonLink href="/settings">"Settings"</ButtonLink>
            </Card>
        }
        .to_html();

        assert!(html.contains("<section"));
        assert!(html.contains("<input"));
        assert!(html.contains("<select"));
        assert!(html.contains("<button"));
        assert!(html.contains("href=\"/settings\""));
    }

    #[test]
    fn stylesheet_is_minified_and_contains_design_tokens() {
        assert!(STYLESHEET.contains("--color-background"));
        assert!(STYLESHEET.contains(".bg-primary"));
        assert!(!STYLESHEET.contains("@import"));
        assert!(STYLESHEET.len() < 96_000);
    }
}
