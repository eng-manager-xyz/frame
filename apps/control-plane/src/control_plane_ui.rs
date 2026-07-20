//! Shared document shell for the control plane's small server-rendered UI
//! surfaces.

/// Wraps server-rendered body markup in the same minified theme used by the
/// full Leptos web and desktop applications.
pub(crate) fn utility_document(title: &str, body: &str) -> String {
    let title = escape_html(title);
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><meta name=\"robots\" content=\"noindex\"><title>{title}</title><style data-frame-ui=\"shadcn-tailwind\">{}</style></head><body data-frame-surface=\"utility\" class=\"grid min-h-screen place-items-center bg-background p-4 text-foreground\">{body}</body></html>",
        frame_ui::STYLESHEET,
    )
}

pub(crate) fn escape_html(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            other => escaped.push(other),
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::utility_document;

    #[test]
    fn document_embeds_the_shared_minified_theme_and_escapes_titles() {
        let document = utility_document("A < B", "<main>Ready</main>");
        assert!(document.starts_with("<!doctype html>"));
        assert!(document.contains("<title>A &lt; B</title>"));
        assert!(document.contains("data-frame-ui=\"shadcn-tailwind\""));
        assert!(document.contains(frame_ui::STYLESHEET));
        assert!(document.ends_with("</body></html>"));
    }
}
