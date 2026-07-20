use leptos::prelude::*;

use crate::class_contract::{BUTTON_GROUP, CARD, merge};

const CARD_BASE: &str = "rounded-xl border border-border bg-card text-card-foreground shadow-sm";

/// Sectioning card for application panels.
#[component]
pub fn Card(#[prop(optional, into)] class: String, children: Children) -> impl IntoView {
    let class = merge(&[CARD, &class]);
    view! { <section class=class>{children()}</section> }
}

/// Article card for self-contained marketing or list content.
#[component]
pub fn FeatureCard(#[prop(optional, into)] class: String, children: Children) -> impl IntoView {
    let class = merge(&[CARD, &class]);
    view! { <article class=class>{children()}</article> }
}

/// Non-sectioning card frame for content whose semantic element is owned by a
/// parent component.
#[component]
pub fn CardFrame(#[prop(optional, into)] class: String, children: Children) -> impl IntoView {
    let class = merge(&[CARD_BASE, &class]);
    view! { <div class=class>{children()}</div> }
}

/// Empty state is a card variant with centered, bounded copy.
#[component]
pub fn EmptyState(#[prop(optional, into)] class: String, children: Children) -> impl IntoView {
    let class = merge(&[
        CARD_BASE,
        "grid min-h-40 place-items-center gap-3 p-8 text-center [&>p]:max-w-xl [&>p]:text-muted-foreground",
        &class,
    ]);
    view! { <section class=class>{children()}</section> }
}

/// Navigation-menu primitive. Product-specific link collections stay with the
/// owning route so permission filtering cannot drift into the design system.
#[component]
pub fn NavigationMenu(#[prop(optional, into)] class: String, children: Children) -> impl IntoView {
    let class = merge(&["flex items-center gap-3", &class]);
    view! { <nav class=class>{children()}</nav> }
}

/// Visual separator retaining an explicit native separator role.
#[component]
pub fn Separator(#[prop(optional, into)] class: String) -> impl IntoView {
    let class = merge(&["shrink-0 bg-border h-px w-full", &class]);
    view! { <div class=class role="separator"></div> }
}

/// Toggle-group container. Each child button owns its `aria-pressed` value.
#[component]
pub fn ToggleGroup(#[prop(optional, into)] class: String, children: Children) -> impl IntoView {
    let class = merge(&[BUTTON_GROUP, &class]);
    view! { <div class=class role="group">{children()}</div> }
}

/// Shadcn-style button group for related actions that do not represent a
/// single pressed-value selection.
#[component]
pub fn ButtonGroup(#[prop(optional, into)] class: String, children: Children) -> impl IntoView {
    let class = merge(&[BUTTON_GROUP, &class]);
    view! { <div class=class>{children()}</div> }
}

/// Aspect-ratio frame for public media.
#[component]
pub fn AspectRatio(#[prop(optional, into)] class: String, children: Children) -> impl IntoView {
    let class = merge(&[
        "relative aspect-video w-full overflow-hidden rounded-lg bg-black [&>video]:size-full",
        &class,
    ]);
    view! { <div class=class>{children()}</div> }
}
