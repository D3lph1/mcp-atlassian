# mcp-atlassian-storage-markdown

Confluence storage format (XHTML) ↔ Markdown, for the
[`mcp-atlassian`](https://crates.io/crates/mcp-atlassian) server. No Atlassian
dependencies — it only knows about markup.

Part of the `mcp-atlassian` workspace. It is published because the crates that
depend on it are, not as a general-purpose library — the API moves whenever the
server needs it to.

## What it does

```rust
use mcp_atlassian_storage_markdown as storage;

let md = storage::storage_to_markdown("<h1>Title</h1><p>Body</p>");
let xhtml = storage::markdown_to_storage("# Title\n\nBody");

// A unified diff between two versions of a page.
let diff = storage::diff_pages("<p>old</p>", "<p>new</p>");

// Replace one section, leaving the rest of the document untouched.
// `None` when the document has no such heading.
let updated: Option<String> = storage::replace_section(&xhtml, "Title", "<p>New body</p>");
```

Conversion is [`htmd`](https://crates.io/crates/htmd) one way and
[`comrak`](https://crates.io/crates/comrak) the other; diffs come from
[`similar`](https://crates.io/crates/similar) and are emitted as `@@` hunks
rather than as the whole document.

## License

MIT.
