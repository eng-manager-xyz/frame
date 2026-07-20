use leptos::prelude::*;

use crate::class_contract::{
    BUTTON_BASE, BUTTON_DEFAULT_SIZE, BUTTON_LINK, BUTTON_OUTLINE, BUTTON_PRIMARY, merge,
};

/// Visual intent for buttons and button-shaped links.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ButtonVariant {
    #[default]
    Primary,
    Secondary,
    Outline,
    Ghost,
    Destructive,
}

impl ButtonVariant {
    const fn classes(self) -> &'static str {
        match self {
            Self::Primary => BUTTON_PRIMARY,
            Self::Secondary => {
                "border border-border bg-secondary text-secondary-foreground shadow-xs hover:bg-secondary/80"
            }
            Self::Outline => BUTTON_OUTLINE,
            Self::Ghost => "text-foreground hover:bg-accent hover:text-accent-foreground",
            Self::Destructive => {
                "bg-destructive-surface text-destructive-surface-foreground shadow-xs hover:bg-destructive-surface/90"
            }
        }
    }
}

/// Density for buttons and button-shaped links.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ButtonSize {
    Small,
    #[default]
    Default,
    Large,
    Icon,
}

impl ButtonSize {
    const fn classes(self) -> &'static str {
        match self {
            Self::Small => "h-11 rounded-md px-3 text-xs",
            Self::Default => BUTTON_DEFAULT_SIZE,
            Self::Large => "h-12 rounded-md px-8",
            Self::Icon => "size-11 rounded-md",
        }
    }
}

/// A shadcn-style native button. Use `attr:*`, `prop:*`, and `on:*` at the call
/// site for DOM-specific behavior; Leptos forwards those attributes to the
/// root button.
#[component]
pub fn Button(
    #[prop(optional)] variant: ButtonVariant,
    #[prop(optional)] size: ButtonSize,
    #[prop(optional, into)] class: String,
    children: Children,
) -> impl IntoView {
    let class = merge(&[BUTTON_BASE, variant.classes(), size.classes(), &class]);
    view! { <button class=class>{children()}</button> }
}

/// The shadcn `Button asChild` equivalent for navigation links.
#[component]
pub fn ButtonLink(
    #[prop(into)] href: String,
    #[prop(optional)] variant: ButtonVariant,
    #[prop(optional)] size: ButtonSize,
    #[prop(optional, into)] class: String,
    children: Children,
) -> impl IntoView {
    let class = merge(&[
        BUTTON_BASE,
        variant.classes(),
        size.classes(),
        BUTTON_LINK,
        &class,
    ]);
    view! { <a class=class href=href>{children()}</a> }
}

#[cfg(test)]
mod tests {
    use crate::class_contract::merge;

    use super::{ButtonSize, ButtonVariant};

    #[test]
    fn class_merge_discards_empty_segments_without_reordering_variants() {
        assert_eq!(merge(&["base", "", " extra "]), "base extra");
        assert!(ButtonVariant::Destructive.classes().contains("destructive"));
        assert!(ButtonSize::Icon.classes().contains("size-11"));
    }
}
