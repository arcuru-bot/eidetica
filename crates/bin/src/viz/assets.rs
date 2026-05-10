//! Static frontend assets, embedded in the binary at compile time.
//!
//! Keeping them embedded means the visualizer ships fully self-contained with
//! the `eidetica` binary — no separate static directory to deploy and no
//! runtime asset path to misconfigure.

/// Raw HTML template. Tokens `__DATABASE_ID__` and `__DATABASE_NAME__` are
/// substituted server-side per request before the page is sent to the
/// browser; the rest of the document is identical for every database.
pub const VIZ_HTML: &str = include_str!("static_assets/viz.html");

/// Stylesheet served at `/static/viz.css`.
pub const VIZ_CSS: &str = include_str!("static_assets/viz.css");

/// Frontend logic served at `/static/viz.js`.
pub const VIZ_JS: &str = include_str!("static_assets/viz.js");

/// Render the visualizer HTML for a specific database.
pub fn render_html(database_id: &str, database_name: &str) -> String {
    VIZ_HTML
        .replace("__DATABASE_ID__", &escape_attr(database_id))
        .replace("__DATABASE_NAME__", &escape_attr(database_name))
}

/// Minimal HTML attribute escaping. Database IDs are CIDs (safe), but names
/// come from user-controlled settings so they must be sanitized before being
/// inlined into a `data-*` attribute and the page title.
fn escape_attr(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_substitutes_tokens() {
        let html = render_html("abc123", "My DB");
        assert!(html.contains(r#"data-database-id="abc123""#));
        assert!(html.contains(r#"data-database-name="My DB""#));
    }

    #[test]
    fn html_escapes_attribute_unsafe_chars() {
        let html = render_html("ok", r#"<script>"&'"#);
        assert!(!html.contains("<script>"));
        assert!(html.contains("&lt;script&gt;"));
        assert!(html.contains("&quot;"));
        assert!(html.contains("&amp;"));
        assert!(html.contains("&#39;"));
    }
}
