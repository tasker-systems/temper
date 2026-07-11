//! Branded browser pages for the CLI OAuth2 callback.
//!
//! When the localhost callback server (see [`crate::login`]) receives the
//! OAuth redirect, it writes one of these pages into the user's browser tab.
//! They follow Temper's "Quiet Instrument" editorial aesthetic — obsidian
//! ground, parchment serif, a single steel-blue accent, the threaded-t brand
//! mark — matching the design system (`design-system/README.md`,
//! `design-system/colors_and_type.css`).
//!
//! The pages are fully self-contained: inline CSS, an inline brand mark, and
//! web fonts that degrade to system serif/mono. Nothing here fetches over the
//! network to render, so the tab looks right even offline.

/// The threaded-t brand mark + wordmark, inlined so the page shows the brand
/// with no network fetch. Geometry lifted from `design-system/assets/brand-mark.svg`.
const BRAND_MARK: &str = r##"<svg width="118" height="24" viewBox="0 0 200 40" xmlns="http://www.w3.org/2000/svg" aria-hidden="true">
  <path d="M 12 6 L 12 34" stroke="#7eb8da" stroke-width="3.5" stroke-linecap="round" fill="none"/>
  <path d="M 4 16 L 20 16 Q 27 16 30 21 Q 33 26 31 32" stroke="#7eb8da" stroke-width="2.6" stroke-linecap="round" fill="none"/>
  <text x="46" y="27" font-family="'JetBrains Mono','Fira Code',monospace" font-size="18" fill="#7eb8da" letter-spacing="0.12em">temper</text>
</svg>"##;

/// Page skeleton. `{{EYEBROW}}`, `{{HEADING}}`, and `{{BODY}}` are filled by
/// [`render`]; `{{MARK}}` carries [`BRAND_MARK`]. Placeholders (not `format!`)
/// keep the CSS braces intact.
const TEMPLATE: &str = r##"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>temper</title>
<link rel="preconnect" href="https://fonts.googleapis.com">
<link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
<link href="https://fonts.googleapis.com/css2?family=JetBrains+Mono:wght@400;500&family=Source+Serif+4:ital,opsz,wght@0,8..60,300;0,8..60,400;1,8..60,400&display=swap" rel="stylesheet">
<style>
  :root {
    --obsidian: #0a0a0f;
    --parchment: #e8e4df;
    --chalk: rgba(255, 255, 255, 0.65);
    --graphite: rgba(255, 255, 255, 0.45);
    --temper-blue: #7eb8da;
    --rule: rgba(255, 255, 255, 0.06);
    --serif: "Source Serif 4", "Source Serif Pro", Georgia, "Times New Roman", serif;
    --mono: "JetBrains Mono", "Fira Code", ui-monospace, monospace;
  }
  * { box-sizing: border-box; }
  html, body { height: 100%; }
  body {
    margin: 0;
    background: var(--obsidian);
    color: var(--parchment);
    font-family: var(--serif);
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 2rem;
    -webkit-font-smoothing: antialiased;
    text-rendering: optimizeLegibility;
  }
  .card {
    width: 100%;
    max-width: 30rem;
    border-left: 2px solid rgba(126, 184, 218, 0.25);
    padding-left: 1.9rem;
  }
  .mark { margin-bottom: 1.7rem; line-height: 0; }
  .eyebrow {
    font-family: var(--mono);
    font-size: 0.65rem;
    letter-spacing: 0.2em;
    text-transform: uppercase;
    color: var(--temper-blue);
    margin-bottom: 0.9rem;
  }
  .heading {
    font-family: var(--serif);
    font-weight: 300;
    font-size: clamp(1.7rem, 5vw, 2rem);
    line-height: 1.25;
    margin: 0 0 1.05rem 0;
    color: var(--parchment);
  }
  .heading em { font-style: italic; color: var(--temper-blue); }
  .body {
    font-family: var(--serif);
    font-size: 1rem;
    line-height: 1.75;
    color: var(--chalk);
    margin: 0;
  }
  .body code {
    font-family: var(--mono);
    font-size: 0.85em;
    color: var(--parchment);
  }
  .detail {
    font-family: var(--mono);
    font-size: 0.72rem;
    letter-spacing: 0.02em;
    line-height: 1.65;
    color: var(--graphite);
    margin: 1.2rem 0 0 0;
    padding-top: 1rem;
    border-top: 1px solid var(--rule);
    word-break: break-word;
  }
</style>
</head>
<body>
  <main class="card">
    <div class="mark">{{MARK}}</div>
    <div class="eyebrow">{{EYEBROW}}</div>
    <h1 class="heading">{{HEADING}}</h1>
    {{BODY}}
  </main>
</body>
</html>"##;

/// Escape the five HTML-significant characters so provider-supplied strings
/// (the OAuth `error` / `error_description`) can't break out of the page.
fn escape_html(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// Assemble a callback page from its editorial parts.
///
/// `heading_html` carries its own single `<em>` accent (the one italicised
/// blue word) and is trusted static markup; `body_html` is likewise assembled
/// by the callers below, which escape any provider-supplied text first.
fn render(eyebrow: &str, heading_html: &str, body_html: &str) -> String {
    TEMPLATE
        .replace("{{MARK}}", BRAND_MARK)
        .replace("{{EYEBROW}}", eyebrow)
        .replace("{{HEADING}}", heading_html)
        .replace("{{BODY}}", body_html)
}

/// The page shown once authentication has completed — the browser tab can be
/// closed and the terminal has the session.
pub fn success() -> String {
    render(
        "Temper CLI",
        "Authentication <em>complete</em>.",
        "<p class=\"body\">You can close this tab and return to the terminal — your session is ready to hand.</p>",
    )
}

/// The page shown when the provider returns an OAuth error on the callback.
///
/// `error` and `description` come straight from the provider's query string,
/// so they are HTML-escaped before being placed in the detail line.
pub fn failure(error: &str, description: &str) -> String {
    let detail = if description.is_empty() {
        escape_html(error)
    } else {
        format!("{} — {}", escape_html(error), escape_html(description))
    };
    let body = format!(
        "<p class=\"body\">Something interrupted the sign-in. Return to your terminal and run \
         <code>temper auth login</code> to try again.</p>\
         <p class=\"detail\">{detail}</p>"
    );
    render("Temper CLI", "Authentication <em>interrupted</em>.", &body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn success_carries_brand_and_copy() {
        let html = success();
        assert!(html.contains("Authentication <em>complete</em>."));
        assert!(html.contains("ready to hand"));
        // The threaded-t mark and the editorial ground are both present.
        assert!(html.contains("temper"));
        assert!(html.contains("#0a0a0f"));
        // No emoji, per the brand voice — the copy stays in the BMP.
        assert!(!html.chars().any(|c| c as u32 >= 0x1_F000));
    }

    #[test]
    fn failure_shows_escaped_provider_detail() {
        let html = failure("access_denied", "user said <no>");
        assert!(html.contains("Authentication <em>interrupted</em>."));
        assert!(html.contains("access_denied — user said &lt;no&gt;"));
        // The raw angle brackets from the provider must not survive.
        assert!(!html.contains("<no>"));
    }

    #[test]
    fn failure_without_description_omits_separator() {
        let html = failure("timeout", "");
        assert!(html.contains(">timeout<"));
        assert!(!html.contains("timeout —"));
    }
}
