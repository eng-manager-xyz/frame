use leptos::prelude::*;

use crate::class_contract::{INPUT, merge};

const CONTROL_BASE: &str = "flex min-h-11 w-full rounded-md border border-input bg-background px-3 py-2 text-sm text-foreground shadow-xs transition-colors placeholder:text-muted-foreground focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring disabled:cursor-not-allowed disabled:opacity-50";

/// Shadcn-style label retaining the native label/for browser contract.
#[component]
pub fn Label(#[prop(optional, into)] class: String, children: Children) -> impl IntoView {
    let class = merge(&[
        "text-sm font-medium leading-none peer-disabled:cursor-not-allowed peer-disabled:opacity-70",
        &class,
    ]);
    view! { <label class=class>{children()}</label> }
}

/// Shadcn-style native input. DOM attributes and `prop:value` are forwarded.
#[component]
pub fn Input(#[prop(optional, into)] class: String) -> impl IntoView {
    let class = merge(&[INPUT, &class]);
    view! { <input class=class/> }
}

/// Shadcn-style native select with intentionally visible platform affordances.
#[component]
pub fn Select(#[prop(optional, into)] class: String, children: Children) -> impl IntoView {
    let class = merge(&[CONTROL_BASE, "appearance-auto pr-9", &class]);
    view! { <select class=class>{children()}</select> }
}

/// Shadcn-style native textarea.
#[component]
pub fn Textarea(#[prop(optional, into)] class: String) -> impl IntoView {
    let class = merge(&[CONTROL_BASE, "min-h-24 resize-y", &class]);
    view! { <textarea class=class></textarea> }
}

/// Native fieldset with shadcn card treatment. The legend is supplied by the
/// caller so the browser retains its fieldset/legend accessibility contract.
#[component]
pub fn FieldGroup(#[prop(optional, into)] class: String, children: Children) -> impl IntoView {
    let class = merge(&[
        "my-4 rounded-lg border border-border bg-muted/35 p-4 [&>legend]:px-1 [&>legend]:text-sm [&>legend]:font-semibold",
        &class,
    ]);
    view! { <fieldset class=class>{children()}</fieldset> }
}
