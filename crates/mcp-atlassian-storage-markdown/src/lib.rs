//! Confluence storage format (XHTML) <-> Markdown conversion (DECISIONS.md D10).
//!
//! Reading: `htmd` converts storage XHTML to Markdown. Confluence's `code`
//! macro is rewritten to `<pre><code>` first so it becomes a fenced block
//! with its language; other macros degrade to their text content, which is
//! acceptable for LLM consumption. Writing: `comrak` renders CommonMark
//! (+ tables, strikethrough, task lists) to HTML, which Confluence accepts as
//! storage representation; fenced code blocks become `code` macros, and raw
//! HTML — a macro the model wrote by hand — passes through.
//!
//! Also here: section replacement on the storage document itself (D36), so
//! editing one section never round-trips the rest of the page through
//! Markdown and loses its macros.
//!
//! No Atlassian dependencies in this crate.

use std::time::Duration;

use comrak::{markdown_to_html, Options};
use similar::TextDiff;

/// Lines of unchanged context kept around each change.
const DIFF_CONTEXT_LINES: usize = 3;

/// Upper bound on diff refinement; past it `similar` returns a coarser diff
/// rather than continuing to search.
const DIFF_TIMEOUT: Duration = Duration::from_secs(1);

/// Converts Confluence storage format (XHTML) to Markdown.
///
/// The common macros are rewritten to plain HTML first (D43): `code` to a
/// fenced block, panels to a labelled blockquote, page and attachment
/// links to their titles, images to `![name]`, task lists to `- [ ]`,
/// `expand` to its body under its title, `toc`/`status`/`jira` to a short
/// marker. Anything else degrades to its text content.
pub fn storage_to_markdown(storage: &str) -> String {
    let html = macros_to_html(storage);
    htmd::convert(&html).unwrap_or_else(|_| storage.to_string())
}

/// Converts Markdown to HTML accepted as Confluence storage representation.
///
/// Raw HTML is passed through rather than stripped: the input comes from the
/// model on the user's behalf, for the user's own instance, and stripping it
/// would make every hand-written `<ac:structured-macro>` vanish silently.
pub fn markdown_to_storage(markdown: &str) -> String {
    let mut options = Options::default();
    options.extension.table = true;
    options.extension.strikethrough = true;
    options.extension.tasklist = true;
    options.render.r#unsafe = true;
    let html = markdown_to_html(&hide_namespaces(markdown), &options);
    code_blocks_to_macros(&unwrap_sole_macros(&restore_namespaces(&html)))
}

/// CommonMark's raw-HTML grammar allows no `:` in a tag name, so `<ac:…>`
/// and `<ri:…>` would be escaped to text by comrak. They are spelled with a
/// `-` while comrak looks at them and restored afterwards.
fn hide_namespaces(markdown: &str) -> String {
    markdown
        .replace("<ac:", "<ac-")
        .replace("</ac:", "</ac-")
        .replace("<ri:", "<ri-")
        .replace("</ri:", "</ri-")
}

fn restore_namespaces(html: &str) -> String {
    html.replace("<ac-", "<ac:")
        .replace("</ac-", "</ac:")
        .replace("<ri-", "<ri:")
        .replace("</ri-", "</ri:")
}

/// A macro written on a line of its own comes out of comrak as inline HTML
/// inside a paragraph; Confluence wants block macros at block level.
fn unwrap_sole_macros(html: &str) -> String {
    const OPEN: &str = "<p><ac:structured-macro";
    const CLOSE: &str = "</ac:structured-macro></p>";
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(at) = rest.find(OPEN) {
        out.push_str(&rest[..at]);
        let candidate = &rest[at + 3..];
        match candidate.find(CLOSE) {
            Some(close_at) => {
                let element_end = close_at + CLOSE.len() - 4;
                out.push_str(&candidate[..element_end]);
                rest = &candidate[element_end + 4..];
            }
            None => {
                out.push_str(&rest[at..at + 3]);
                rest = candidate;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Unified line diff between two Markdown documents.
///
/// Output is standard unified-diff format — `@@` hunk headers, `-`/`+` lines
/// and a few lines of context — so only the changed regions are returned. On
/// a long page that keeps the answer small instead of echoing the whole
/// document back.
///
/// Backed by `similar` (Myers with heuristics) rather than a hand-rolled LCS:
/// a naive LCS table costs O(n·m) memory, which on a 5000-line page is
/// ~250 MB — two orders of magnitude over this server's entire footprint.
/// The `timeout` caps pathological inputs by letting the algorithm fall back
/// to a coarser (still correct) diff instead of running unbounded.
pub fn diff_pages(before: &str, after: &str) -> String {
    let diff = TextDiff::configure()
        .timeout(DIFF_TIMEOUT)
        .diff_lines(before, after);
    diff.unified_diff()
        .context_radius(DIFF_CONTEXT_LINES)
        .header("before", "after")
        .to_string()
}

/// Replaces the section introduced by the heading whose text is `heading`
/// — from the end of that heading element up to the next heading of the
/// same or a higher level — with `replacement` (already in storage format).
/// Everything outside the section is kept byte for byte, macros included.
///
/// Heading text is compared after tags are stripped, entities decoded and
/// whitespace collapsed, so `<h2>Roll<em>back</em></h2>` matches `Rollback`.
/// Returns `None` when no heading matches. Headings inside macro bodies are
/// treated like any other heading; a page that puts headings inside expand
/// or panel macros is not something this function can edit safely.
pub fn replace_section(storage: &str, heading: &str, replacement: &str) -> Option<String> {
    let wanted = collapse_whitespace(heading);
    let headings = headings(storage);
    let (index, start) = headings
        .iter()
        .enumerate()
        .find(|(_, h)| h.text == wanted)?;
    let end = headings[index + 1..]
        .iter()
        .find(|h| h.level <= start.level)
        .map(|h| h.open_at)
        .unwrap_or(storage.len());
    let mut out = String::with_capacity(storage.len() + replacement.len());
    out.push_str(&storage[..start.close_end]);
    out.push_str(replacement);
    out.push_str(&storage[end..]);
    Some(out)
}

struct Heading {
    level: u8,
    /// Byte offset of `<hN`.
    open_at: usize,
    /// Byte offset just past `</hN>`.
    close_end: usize,
    /// Visible text, normalized.
    text: String,
}

/// Every `<h1>`…`<h6>` element in document order.
fn headings(storage: &str) -> Vec<Heading> {
    let mut found = Vec::new();
    let mut from = 0;
    while let Some(at) = storage[from..].find("<h") {
        let open_at = from + at;
        from = open_at + 2;
        let rest = &storage[open_at + 2..];
        let Some(level) = rest.chars().next().and_then(|c| c.to_digit(10)) else {
            continue;
        };
        if !(1..=6).contains(&level)
            || !rest[1..].starts_with(|c: char| c == '>' || c.is_whitespace())
        {
            continue;
        }
        let Some(open_end) = rest.find('>').map(|i| open_at + 2 + i + 1) else {
            continue;
        };
        let close_tag = format!("</h{level}>");
        let Some(close_at) = storage[open_end..].find(&close_tag).map(|i| open_end + i) else {
            continue;
        };
        found.push(Heading {
            level: level as u8,
            open_at,
            close_end: close_at + close_tag.len(),
            text: collapse_whitespace(&decode_entities(&strip_tags(&storage[open_end..close_at]))),
        });
        from = close_at + close_tag.len();
    }
    found
}

fn strip_tags(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    out
}

fn collapse_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The entities Confluence emits in text; anything else is left alone.
fn decode_entities(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
}

fn encode_entities(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

// ---- code macro <-> <pre><code> ------------------------------------------

const MACRO_OPEN: &str = "<ac:structured-macro";
const MACRO_CLOSE: &str = "</ac:structured-macro>";

/// One `<ac:structured-macro>` element, taken apart.
struct Macro<'a> {
    name: &'a str,
    /// `<ac:parameter ac:name="…">` values, by name.
    params: Vec<(&'a str, &'a str)>,
    /// Inside `<ac:rich-text-body>`, still XHTML.
    rich_body: Option<&'a str>,
    /// Inside `<ac:plain-text-body>`, CDATA unwrapped.
    plain_body: Option<String>,
}

impl<'a> Macro<'a> {
    fn param(&self, name: &str) -> Option<&'a str> {
        self.params
            .iter()
            .find(|(k, _)| *k == name)
            .map(|(_, v)| v.trim())
    }
}

fn parse_macro<'a>(open_tag: &'a str, inner: &'a str) -> Macro<'a> {
    let name = between(open_tag, "ac:name=\"", "\"").unwrap_or("");
    let mut params = Vec::new();
    let mut rest = inner;
    while let Some(at) = rest.find("<ac:parameter") {
        let candidate = &rest[at..];
        let Some(open_end) = candidate.find('>') else {
            break;
        };
        let Some(close_at) = candidate.find("</ac:parameter>") else {
            break;
        };
        let key = between(&candidate[..open_end], "ac:name=\"", "\"").unwrap_or("");
        params.push((key, &candidate[open_end + 1..close_at]));
        rest = &candidate[close_at..];
    }
    Macro {
        name,
        params,
        rich_body: between(inner, "<ac:rich-text-body>", "</ac:rich-text-body>"),
        plain_body: between(inner, "<ac:plain-text-body>", "</ac:plain-text-body>")
            .map(cdata_content),
    }
}

/// Rewrites the macros `htmd` cannot read into HTML it can (D43). Nested
/// macros are handled by rewriting the inner body first.
fn macros_to_html(storage: &str) -> String {
    let mut out = String::with_capacity(storage.len());
    let mut rest = storage;
    while let Some(at) = rest.find(MACRO_OPEN) {
        out.push_str(&links_and_images_to_html(&rest[..at]));
        let candidate = &rest[at..];
        let Some(open_end) = candidate.find('>') else {
            out.push_str(candidate);
            return out;
        };
        // `<ac:structured-macro ac:name="toc"/>` has no body and no close.
        let (inner, element_end) = if candidate[..open_end].ends_with('/') {
            ("", open_end + 1)
        } else {
            let Some(close_at) = find_matching_close(candidate, open_end + 1) else {
                out.push_str(candidate);
                return out;
            };
            (
                &candidate[open_end + 1..close_at],
                close_at + MACRO_CLOSE.len(),
            )
        };
        let m = parse_macro(&candidate[..open_end], inner);
        out.push_str(&render_macro(&m));
        rest = &candidate[element_end..];
    }
    out.push_str(&links_and_images_to_html(rest));
    out
}

/// The byte offset of the `</ac:structured-macro>` that closes the element
/// whose open tag ends at `from`, counting nested macros.
fn find_matching_close(text: &str, from: usize) -> Option<usize> {
    let mut depth = 1;
    let mut at = from;
    loop {
        let next_open = text[at..].find(MACRO_OPEN).map(|i| at + i);
        let next_close = text[at..].find(MACRO_CLOSE).map(|i| at + i)?;
        match next_open {
            Some(open) if open < next_close => {
                depth += 1;
                at = open + MACRO_OPEN.len();
            }
            _ => {
                depth -= 1;
                if depth == 0 {
                    return Some(next_close);
                }
                at = next_close + MACRO_CLOSE.len();
            }
        }
    }
}

fn render_macro(m: &Macro<'_>) -> String {
    // Bodies may hold macros of their own (a code block in a panel).
    let body = || m.rich_body.map(macros_to_html).unwrap_or_default();
    match m.name {
        "code" => {
            let mut html = String::from("<pre><code");
            if let Some(language) = m.param("language") {
                html.push_str(&format!(" class=\"language-{language}\""));
            }
            html.push('>');
            html.push_str(&encode_entities(m.plain_body.as_deref().unwrap_or("")));
            html.push_str("</code></pre>");
            html
        }
        "info" | "note" | "warning" | "tip" | "panel" => {
            let label = m
                .param("title")
                .map(str::to_string)
                .unwrap_or_else(|| capitalize(m.name));
            format!(
                "<blockquote><p><strong>{label}</strong></p>{}</blockquote>",
                body()
            )
        }
        "expand" => {
            let title = m.param("title").unwrap_or("Details");
            format!("<p><strong>{title}</strong></p>{}", body())
        }
        "toc" => "<p><em>(table of contents)</em></p>".into(),
        "status" => format!(
            "<strong>[{}]</strong>",
            m.param("title").unwrap_or("STATUS")
        ),
        "jira" => {
            let key = m.param("key").unwrap_or("");
            format!("<strong>{key}</strong>")
        }
        // Everything else: its body, or nothing — the same as `htmd` alone
        // would have shown, minus the parameter noise.
        _ => body(),
    }
}

fn capitalize(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// `<ac:link>` with a `ri:page` / `ri:attachment` / `ri:url` inside becomes
/// an `<a>` or plain text; `<ac:image>` with `ri:attachment` becomes `<img>`;
/// `<ac:task-list>` becomes a list of `[ ]`/`[x]` items.
fn links_and_images_to_html(html: &str) -> String {
    let mut out = replace_elements(html, "<ac:link", "</ac:link>", |open, inner| {
        let text = between(
            inner,
            "<ac:plain-text-link-body>",
            "</ac:plain-text-link-body>",
        )
        .map(cdata_content)
        .or_else(|| between(inner, "<ac:link-body>", "</ac:link-body>").map(strip_tags));
        if let Some(title) = between(inner, "ri:content-title=\"", "\"") {
            let text = text.unwrap_or_else(|| decode_entities(title));
            return format!("<a href=\"confluence:page:{title}\">{text}</a>");
        }
        if let Some(name) = between(inner, "ri:filename=\"", "\"") {
            return format!(
                "<a href=\"attachment:{name}\">{}</a>",
                text.unwrap_or_else(|| name.to_string())
            );
        }
        if let Some(url) = between(inner, "ri:value=\"", "\"") {
            return format!(
                "<a href=\"{url}\">{}</a>",
                text.unwrap_or_else(|| url.to_string())
            );
        }
        let _ = open;
        text.unwrap_or_default()
    });
    out = replace_elements(&out, "<ac:image", "</ac:image>", |open, inner| {
        let name = between(inner, "ri:filename=\"", "\"")
            .or_else(|| between(inner, "ri:value=\"", "\""))
            .unwrap_or("image");
        let alt = between(open, "ac:alt=\"", "\"").unwrap_or(name);
        format!("<img src=\"{name}\" alt=\"{alt}\">")
    });
    out = replace_elements(&out, "<ac:task-list", "</ac:task-list>", |_, inner| {
        let items = replace_elements(inner, "<ac:task", "</ac:task>", |_, task| {
            let done = between(task, "<ac:task-status>", "</ac:task-status>")
                .is_some_and(|s| s.trim() == "complete");
            let body = between(task, "<ac:task-body>", "</ac:task-body>").unwrap_or("");
            // `[x]` would come out of htmd escaped as `\[x\]`; the glyphs
            // read the same to a model and survive untouched.
            format!(
                "<li>{} {}</li>",
                if done { "☑" } else { "☐" },
                strip_tags(body).trim()
            )
        });
        format!("<ul>{items}</ul>")
    });
    out
}

/// Replaces each `open…close` element with `render(open_tag, inner)`.
fn replace_elements(
    html: &str,
    open: &str,
    close: &str,
    render: impl Fn(&str, &str) -> String,
) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(at) = rest.find(open) {
        out.push_str(&rest[..at]);
        let candidate = &rest[at..];
        // A self-closing form (`<ac:image … />` never is, but be safe).
        let (Some(open_end), Some(close_at)) = (candidate.find('>'), candidate.find(close)) else {
            out.push_str(candidate);
            return out;
        };
        if close_at < open_end {
            out.push_str(&candidate[..open_end + 1]);
            rest = &candidate[open_end + 1..];
            continue;
        }
        out.push_str(&render(
            &candidate[..open_end],
            &candidate[open_end + 1..close_at],
        ));
        rest = &candidate[close_at + close.len()..];
    }
    out.push_str(rest);
    out
}

/// Rewrites comrak's `<pre><code class="language-X">` into Confluence's
/// `code` macro, so a fenced block renders as a code block rather than as
/// preformatted text.
fn code_blocks_to_macros(html: &str) -> String {
    const OPEN: &str = "<pre><code";
    const CLOSE: &str = "</code></pre>";
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(at) = rest.find(OPEN) {
        out.push_str(&rest[..at]);
        let candidate = &rest[at..];
        let (Some(open_end), Some(close_at)) = (
            candidate[OPEN.len()..].find('>').map(|i| OPEN.len() + i),
            candidate.find(CLOSE),
        ) else {
            out.push_str(candidate);
            return out;
        };
        let attributes = &candidate[OPEN.len()..open_end];
        let language = between(attributes, "class=\"language-", "\"");
        let body = decode_entities(&candidate[open_end + 1..close_at]);
        out.push_str("<ac:structured-macro ac:name=\"code\">");
        if let Some(language) = language {
            out.push_str(&format!(
                "<ac:parameter ac:name=\"language\">{language}</ac:parameter>"
            ));
        }
        out.push_str("<ac:plain-text-body><![CDATA[");
        // A `]]>` inside the code would end the section early; the standard
        // trick splits it across two sections.
        out.push_str(
            &body
                .trim_end_matches('\n')
                .replace("]]>", "]]]]><![CDATA[>"),
        );
        out.push_str("]]></ac:plain-text-body></ac:structured-macro>");
        rest = &candidate[close_at + CLOSE.len()..];
    }
    out.push_str(rest);
    out
}

/// The text between the first `open` and the following `close`.
fn between<'a>(text: &'a str, open: &str, close: &str) -> Option<&'a str> {
    let start = text.find(open)? + open.len();
    let end = text[start..].find(close)? + start;
    Some(&text[start..end])
}

/// Unwraps `<![CDATA[...]]>` (rejoining a split `]]>`); leaves plain text
/// as is.
fn cdata_content(body: &str) -> String {
    let body = body.trim();
    match body
        .strip_prefix("<![CDATA[")
        .and_then(|b| b.strip_suffix("]]>"))
    {
        Some(inner) => inner.replace("]]]]><![CDATA[>", "]]>"),
        None => decode_entities(body),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn storage_headings_lists_and_links_become_markdown() {
        let storage = r#"<h2>Overview</h2><p>See <a href="https://example.com">docs</a>.</p><ul><li>one</li><li>two</li></ul>"#;
        let md = storage_to_markdown(storage);
        assert!(md.contains("## Overview"), "got: {md}");
        assert!(md.contains("[docs](https://example.com)"), "got: {md}");
        // htmd renders list items as `*   item`
        assert!(md.contains("*   one"), "got: {md}");
    }

    #[test]
    fn confluence_macro_degrades_to_text() {
        let storage = r#"<p>before</p><ac:structured-macro ac:name="info"><ac:rich-text-body><p>note text</p></ac:rich-text-body></ac:structured-macro>"#;
        let md = storage_to_markdown(storage);
        assert!(md.contains("before"), "got: {md}");
        assert!(md.contains("note text"), "got: {md}");
    }

    #[test]
    fn a_code_macro_becomes_a_fenced_block_with_its_language() {
        let storage = r#"<p>run</p><ac:structured-macro ac:name="code" ac:schema-version="1"><ac:parameter ac:name="language">bash</ac:parameter><ac:plain-text-body><![CDATA[echo "<hi>" && ls]]></ac:plain-text-body></ac:structured-macro><p>done</p>"#;
        let md = storage_to_markdown(storage);
        assert!(md.contains("```bash"), "got: {md}");
        assert!(md.contains("echo \"<hi>\" && ls"), "got: {md}");
        assert!(md.contains("done"), "got: {md}");
    }

    #[test]
    fn panels_links_images_and_tasks_are_readable() {
        let storage = concat!(
            "<ac:structured-macro ac:name=\"info\"><ac:parameter ac:name=\"title\">Heads up</ac:parameter>",
            "<ac:rich-text-body><p>read <ac:link><ri:page ri:content-title=\"Runbook\" /></ac:link> first</p>",
            "<ac:structured-macro ac:name=\"code\"><ac:plain-text-body><![CDATA[ls -la]]></ac:plain-text-body></ac:structured-macro>",
            "</ac:rich-text-body></ac:structured-macro>",
            "<p><ac:image ac:alt=\"diagram\"><ri:attachment ri:filename=\"arch.png\" /></ac:image></p>",
            "<ac:task-list><ac:task><ac:task-status>complete</ac:task-status><ac:task-body>done thing</ac:task-body></ac:task>",
            "<ac:task><ac:task-status>incomplete</ac:task-status><ac:task-body>open thing</ac:task-body></ac:task></ac:task-list>",
            "<ac:structured-macro ac:name=\"toc\"/>",
            "<p>see <ac:structured-macro ac:name=\"jira\"><ac:parameter ac:name=\"key\">PROJ-7</ac:parameter></ac:structured-macro></p>",
            "<ac:structured-macro ac:name=\"expand\"><ac:parameter ac:name=\"title\">More</ac:parameter><ac:rich-text-body><p>hidden</p></ac:rich-text-body></ac:structured-macro>",
        );
        let md = storage_to_markdown(storage);
        for expected in [
            "**Heads up**",
            "> ",
            "[Runbook](confluence:page:Runbook)",
            "```",
            "ls -la",
            "![diagram](arch.png)",
            "☑ done thing",
            "☐ open thing",
            "*(table of contents)*",
            "**PROJ-7**",
            "**More**",
            "hidden",
        ] {
            assert!(md.contains(expected), "missing {expected:?} in:\n{md}");
        }
    }

    #[test]
    fn a_fenced_block_becomes_a_code_macro() {
        let html = markdown_to_storage("```rust\nlet x = a < b && c;\n```\n");
        assert!(
            html.contains(r#"<ac:structured-macro ac:name="code">"#),
            "got: {html}"
        );
        assert!(
            html.contains(r#"<ac:parameter ac:name="language">rust</ac:parameter>"#),
            "got: {html}"
        );
        // Verbatim inside CDATA, not entity-encoded.
        assert!(
            html.contains("<![CDATA[let x = a < b && c;]]>"),
            "got: {html}"
        );
        assert!(!html.contains("<pre>"), "got: {html}");
        // Without a language the parameter is simply absent.
        let html = markdown_to_storage("```\nplain\n```\n");
        assert!(!html.contains("ac:name=\"language\""), "got: {html}");
        assert!(html.contains("<![CDATA[plain]]>"), "got: {html}");
    }

    #[test]
    fn code_survives_a_round_trip() {
        let markdown = "# T\n\n```sql\nSELECT 1; -- ]]> tricky\n```\n";
        let back = storage_to_markdown(&markdown_to_storage(markdown));
        assert!(back.contains("```sql"), "got: {back}");
        assert!(back.contains("SELECT 1; -- ]]> tricky"), "got: {back}");
    }

    #[test]
    fn raw_html_in_markdown_passes_through() {
        // A macro the model wrote by hand must reach Confluence, not become
        // `<!-- raw HTML omitted -->`.
        let html = markdown_to_storage(
            "text\n\n<ac:structured-macro ac:name=\"toc\"></ac:structured-macro>\n",
        );
        assert!(
            html.contains("<ac:structured-macro ac:name=\"toc\"></ac:structured-macro>"),
            "got: {html}"
        );
        // …and at block level, not wrapped in a paragraph.
        assert!(!html.contains("<p><ac:"), "got: {html}");
        // A page link survives too.
        let html = markdown_to_storage(
            "See <ac:link><ri:page ri:content-title=\"Runbook\" /></ac:link> now.\n",
        );
        assert!(
            html.contains("<ri:page ri:content-title=\"Runbook\" />"),
            "got: {html}"
        );
    }

    #[test]
    fn diff_marks_added_removed_and_context() {
        let diff = diff_pages("keep\nold line\ntail", "keep\nnew line\ntail");
        assert!(diff.contains("@@"), "hunk header missing: {diff}");
        assert!(diff.contains("-old line"), "removal missing: {diff}");
        assert!(diff.contains("+new line"), "addition missing: {diff}");
        assert!(diff.contains(" keep"), "context missing: {diff}");
    }

    #[test]
    fn diff_of_identical_documents_is_empty() {
        assert_eq!(diff_pages("a\nb", "a\nb"), "");
    }

    #[test]
    fn diff_returns_only_changed_regions_of_a_long_page() {
        // One edit in a long document must not echo the whole document back.
        let before: String = (0..1000).map(|i| format!("line {i}\n")).collect();
        let after = before.replace("line 500\n", "line five hundred\n");
        let diff = diff_pages(&before, &after);
        assert!(diff.contains("-line 500"), "{diff}");
        assert!(diff.contains("+line five hundred"), "{diff}");
        assert!(
            diff.lines().count() < 20,
            "diff should be a small hunk, got {} lines",
            diff.lines().count()
        );
    }

    #[test]
    fn markdown_becomes_html_storage() {
        let md = "# Title\n\nSome **bold** text.\n\n| a | b |\n|---|---|\n| 1 | 2 |";
        let html = markdown_to_storage(md);
        assert!(html.contains("<h1>Title</h1>"), "got: {html}");
        assert!(html.contains("<strong>bold</strong>"), "got: {html}");
        assert!(html.contains("<table>"), "got: {html}");
    }

    const PAGE: &str = "<h1>Runbook</h1><p>intro</p>\
        <h2>Deploy</h2><p>old deploy</p><h3>Step 1</h3><p>sub</p>\
        <h2>Rollback</h2><ac:structured-macro ac:name=\"code\"><ac:plain-text-body><![CDATA[git revert]]></ac:plain-text-body></ac:structured-macro>\
        <h2>Q&amp;A</h2><p>faq</p>";

    #[test]
    fn a_section_is_replaced_up_to_the_next_heading_of_its_level() {
        let out = replace_section(PAGE, "Deploy", "<p>new deploy</p>").unwrap();
        assert!(
            out.contains("<h2>Deploy</h2><p>new deploy</p><h2>Rollback</h2>"),
            "{out}"
        );
        // The subsection belonged to the replaced section and is gone.
        assert!(!out.contains("Step 1"), "{out}");
        // Everything else — the macro included — is byte for byte the same.
        assert!(out.contains("<![CDATA[git revert]]>"), "{out}");
        assert!(out.starts_with("<h1>Runbook</h1><p>intro</p>"), "{out}");
        assert!(out.ends_with("<h2>Q&amp;A</h2><p>faq</p>"), "{out}");
    }

    #[test]
    fn the_last_section_runs_to_the_end_and_entities_match_decoded_text() {
        let out = replace_section(PAGE, "Q&A", "<p>answers</p>").unwrap();
        assert!(out.ends_with("<h2>Q&amp;A</h2><p>answers</p>"), "{out}");
    }

    #[test]
    fn a_heading_with_inline_markup_or_attributes_still_matches() {
        let page = "<h2 id=\"x\">Roll<em>back</em>  plan</h2><p>a</p><h2>Next</h2>";
        let out = replace_section(page, "Rollback plan", "<p>b</p>").unwrap();
        assert_eq!(
            out,
            "<h2 id=\"x\">Roll<em>back</em>  plan</h2><p>b</p><h2>Next</h2>"
        );
    }

    #[test]
    fn an_unknown_heading_is_none_and_an_hr_is_not_a_heading() {
        assert!(replace_section(PAGE, "Nope", "x").is_none());
        // `<hr/>` starts with `<h` too.
        assert!(replace_section("<hr/><p>x</p>", "x", "y").is_none());
    }
}
