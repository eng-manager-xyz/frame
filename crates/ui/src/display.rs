use leptos::prelude::*;

use crate::class_contract::{ALERT_BASE, ALERT_DEFAULT, ALERT_DESTRUCTIVE, merge};

/// Visual intent for compact status badges.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum BadgeVariant {
    #[default]
    Default,
    Secondary,
    Outline,
    Destructive,
    Success,
}

impl BadgeVariant {
    const fn classes(self) -> &'static str {
        match self {
            Self::Default => "border-transparent bg-primary text-primary-foreground",
            Self::Secondary => "border-transparent bg-secondary text-secondary-foreground",
            Self::Outline => "border-border text-foreground",
            Self::Destructive => {
                "border-transparent bg-destructive-surface text-destructive-surface-foreground"
            }
            Self::Success => {
                "border-emerald-600/50 bg-emerald-500/10 text-emerald-300 [[data-theme=light]_&]:text-emerald-800"
            }
        }
    }
}

#[component]
pub fn Badge(
    #[prop(optional)] variant: BadgeVariant,
    #[prop(optional, into)] class: String,
    children: Children,
) -> impl IntoView {
    let class = merge(&[
        "inline-flex items-center rounded-full border px-2.5 py-0.5 text-xs font-semibold transition-colors",
        variant.classes(),
        &class,
    ]);
    view! { <span class=class>{children()}</span> }
}

/// Visual intent for inline alerts and status announcements.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AlertVariant {
    #[default]
    Default,
    Destructive,
    Success,
}

impl AlertVariant {
    const fn classes(self) -> &'static str {
        match self {
            Self::Default => ALERT_DEFAULT,
            Self::Destructive => ALERT_DESTRUCTIVE,
            Self::Success => "border-emerald-600/60 text-foreground",
        }
    }
}

#[component]
pub fn Alert(
    #[prop(optional)] variant: AlertVariant,
    #[prop(optional, into)] class: String,
    children: Children,
) -> impl IntoView {
    let class = merge(&[ALERT_BASE, variant.classes(), &class]);
    view! { <div class=class>{children()}</div> }
}

/// Styled native progress element. Values remain native attributes so assistive
/// technology receives the browser's platform semantics.
#[component]
pub fn Progress(#[prop(optional, into)] class: String, children: Children) -> impl IntoView {
    let class = merge(&[
        "h-3 w-full overflow-hidden rounded-full bg-secondary accent-primary",
        &class,
    ]);
    view! { <progress class=class>{children()}</progress> }
}

/// Styled native meter for live audio and device levels.
#[component]
pub fn Meter(#[prop(optional, into)] class: String, children: Children) -> impl IntoView {
    let class = merge(&["h-4 w-full accent-primary", &class]);
    view! { <meter class=class>{children()}</meter> }
}

/// Fixed, modal backdrop. Focus ownership remains explicit in the consuming
/// product state because web and Tauri have different lifecycle authorities.
#[component]
pub fn DialogOverlay(#[prop(optional, into)] class: String, children: Children) -> impl IntoView {
    let class = merge(&[
        "fixed inset-0 z-50 grid place-items-center bg-black/75 p-4",
        &class,
    ]);
    view! { <div class=class>{children()}</div> }
}

/// Accessible dialog panel used inside [`DialogOverlay`].
#[component]
pub fn DialogContent(#[prop(optional, into)] class: String, children: Children) -> impl IntoView {
    let class = merge(&[
        "w-full max-w-lg rounded-lg border border-border bg-card p-6 text-card-foreground shadow-2xl",
        &class,
    ]);
    view! { <section class=class>{children()}</section> }
}
