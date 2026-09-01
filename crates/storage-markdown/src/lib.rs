//! Confluence storage format (XHTML) <-> Markdown conversion (DECISIONS.md D10).
//!
//! Reading: `htmd` converts storage XHTML to Markdown; unknown Confluence
//! macro tags degrade to their text content, which is acceptable for LLM
//! consumption. Writing: `comrak` renders CommonMark (+ tables and
//! strikethrough) to HTML, which Confluence accepts as storage representation.
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
pub fn storage_to_markdown(storage: &str) -> String {
    htmd::convert(storage).unwrap_or_else(|_| storage.to_string())
}

/// Converts Markdown to HTML accepted as Confluence storage representation.
pub fn markdown_to_storage(markdown: &str) -> String {
    let mut options = Options::default();
    options.extension.table = true;
    options.extension.strikethrough = true;
    markdown_to_html(markdown, &options)
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
}
