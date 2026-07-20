//! Stable utility-class contract for runtimes that cannot link Leptos.
//!
//! The Cloudflare Worker uses these constants for three tiny server-generated
//! documents because `workers-rs` and Leptos currently link incompatible
//! `wasm-streams` ABIs. Full web and desktop surfaces use the Leptos components.

pub const BUTTON_BASE: &str = "inline-flex shrink-0 items-center justify-center gap-2 whitespace-nowrap text-sm font-medium transition-colors focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring disabled:pointer-events-none disabled:opacity-50";
pub const BUTTON_PRIMARY: &str = "bg-primary text-primary-foreground shadow-xs hover:bg-primary/90";
pub const BUTTON_OUTLINE: &str = "border border-input bg-background text-foreground shadow-xs hover:bg-accent hover:text-accent-foreground";
pub const BUTTON_DEFAULT_SIZE: &str = "h-11 rounded-md px-4 py-2";
pub const BUTTON_LINK: &str = "no-underline";
pub const BUTTON_GROUP: &str = "flex flex-wrap items-center gap-2";
pub const CARD: &str = "rounded-xl border border-border bg-card p-6 text-card-foreground shadow-sm";
pub const ALERT_BASE: &str = "relative my-4 w-full rounded-lg border bg-card px-4 py-3 text-sm shadow-xs [&>p]:leading-relaxed";
pub const ALERT_DEFAULT: &str = "border-border text-card-foreground";
pub const ALERT_DESTRUCTIVE: &str = "border-destructive/60 text-destructive";
pub const INPUT: &str = "flex min-h-11 w-full rounded-md border border-input bg-background px-3 py-2 text-sm text-foreground shadow-xs transition-colors placeholder:text-muted-foreground focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring disabled:cursor-not-allowed disabled:opacity-50 file:border-0 file:bg-transparent file:text-sm file:font-medium";

#[cfg(any(feature = "ssr", feature = "hydrate", feature = "csr"))]
pub(crate) fn merge(classes: &[&str]) -> String {
    classes
        .iter()
        .map(|class| class.trim())
        .filter(|class| !class.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}
