//! The startup banner (D29).
//!
//! **Always stderr, never stdout.** In stdio mode stdout carries the MCP
//! protocol, and one box-drawing character on it desynchronizes the client's
//! JSON reader. This is the same reason the tracing subscriber writes to
//! stderr (`main.rs`).
//!
//! Printed unconditionally, not only on a terminal: the common deployment is a
//! container, where stderr is a pipe and `docker logs` is the only place anyone
//! looks. Colour is what depends on the terminal — ANSI escapes in a captured
//! log file are noise, so they are emitted only for a real TTY and suppressed
//! by `NO_COLOR`. `NO_BANNER` turns the whole thing off in favour of the
//! structured startup line.

use std::io::{IsTerminal, Write};

use mcp_atlassian_client::Config;

/// Inner width of the box; the frame adds two columns.
const WIDTH: usize = 78;

/// `MCP ATLASSIAN` in half-block glyphs, 50 columns wide.
const ART: [&str; 2] = [
    "█▀▄▀█ █▀▀ █▀█   ▄▀█ ▀█▀ █   ▄▀█ █▀▀ █▀▀ █ ▄▀█ █▄ █",
    "█ ▀ █ █▄▄ █▀▀   █▀█  █  █▄▄ █▀█ ▄▄█ ▄▄█ █ █▀█ █ ▀█",
];

const RESET: &str = "\x1b[0m";
const FRAME: &str = "\x1b[38;5;24m";
const GLYPH: &str = "\x1b[1;38;5;39m";
const TITLE: &str = "\x1b[1;38;5;75m";
const MUTED: &str = "\x1b[2m";
/// Underlined, so a terminal that linkifies URLs still looks deliberate and
/// one that does not still marks the line as an address.
const LINK: &str = "\x1b[2;4;38;5;110m";
const LABEL: &str = "\x1b[38;5;110m";
const ON: &str = "\x1b[32m";
const OFF: &str = "\x1b[2;31m";
const FLAG: &str = "\x1b[33m";
const PLAIN: &str = "";

/// A run of text with one style. A row is a list of these, so a single line can
/// mix colours and still be measured for padding.
type Span = (String, &'static str);

fn span(text: impl Into<String>, style: &'static str) -> Span {
    (text.into(), style)
}

/// Writes the banner to stderr. Never touches stdout — see the module docs.
pub fn print(config: &Config, transport: &str, tools: usize) {
    let colors = std::io::stderr().is_terminal() && std::env::var_os("NO_COLOR").is_none();
    let banner = render(config, transport, tools, colors);
    // One write, so a concurrent log line cannot land inside the box.
    let mut stderr = std::io::stderr().lock();
    let _ = stderr.write_all(banner.as_bytes());
    let _ = stderr.flush();
}

/// The banner as a string. Split out from [`print`] so the layout is testable
/// without capturing a file descriptor.
pub fn render(config: &Config, transport: &str, tools: usize, colors: bool) -> String {
    let mut facts: Vec<Vec<Span>> = vec![
        vec![
            span(label("Transport"), LABEL),
            span(transport.to_string(), PLAIN),
        ],
        vec![
            span(label("Services"), LABEL),
            service("Jira", config.jira.is_some()),
            span("  ", PLAIN),
            service("Confluence", config.confluence.is_some()),
        ],
        vec![
            span(label("Tools"), LABEL),
            span(tools.to_string(), if tools == 0 { OFF } else { PLAIN }),
            span(" registered", MUTED),
        ],
        vec![span(label("Mode"), LABEL), mode(config)],
    ];
    if let Some(path) = &config.audit_log {
        facts.push(vec![
            span(label("Audit"), LABEL),
            span(elide(&path.display().to_string(), 48), PLAIN),
        ]);
    }
    if let Some(ttl) = config.cache_ttl {
        facts.push(vec![
            span(label("Cache"), LABEL),
            span(format!("{}s", ttl.as_secs()), PLAIN),
        ]);
    }

    // The facts are left-aligned as a block, and the block as a whole is
    // centred — so the labels line up instead of drifting per row.
    let block = facts.iter().map(width).max().unwrap_or(0);
    let block_indent = WIDTH.saturating_sub(block) / 2;

    let mut out = String::new();
    out.push('\n');
    out.push_str(&frame('╭', '╮', colors));
    out.push_str(&row(&[], 0, colors));
    for glyph in ART {
        out.push_str(&row(
            &[span(glyph, GLYPH)],
            centered(glyph.chars().count()),
            colors,
        ));
    }
    out.push_str(&row(&[], 0, colors));
    let title = format!("mcp-atlassian {}", env!("CARGO_PKG_VERSION"));
    out.push_str(&row(
        &[span(&title, TITLE)],
        centered(title.chars().count()),
        colors,
    ));
    let tagline = "Jira and Confluence over MCP";
    out.push_str(&row(
        &[span(tagline, MUTED)],
        centered(tagline.chars().count()),
        colors,
    ));
    // From the manifest rather than a literal: the banner cannot then outlive
    // a move of the repository, and there is one place that says where this
    // came from.
    let source = env!("CARGO_PKG_REPOSITORY");
    out.push_str(&row(
        &[span(source, LINK)],
        centered(source.chars().count()),
        colors,
    ));
    out.push_str(&row(&[], 0, colors));
    for fact in &facts {
        out.push_str(&row(fact, block_indent, colors));
    }
    out.push_str(&row(&[], 0, colors));
    out.push_str(&frame('╰', '╯', colors));
    out.push('\n');
    out
}

fn centered(width: usize) -> usize {
    WIDTH.saturating_sub(width) / 2
}

fn label(text: &str) -> String {
    format!("{text:<11}")
}

fn service(name: &str, configured: bool) -> Span {
    if configured {
        span(format!("{name} ✓"), ON)
    } else {
        span(format!("{name} ✗"), OFF)
    }
}

/// The switches that change what the server will do, or an explicit statement
/// that none are set — silence here would read as "unknown", not "off".
fn mode(config: &Config) -> Span {
    let mut flags = Vec::new();
    if config.read_only {
        flags.push("read-only");
    }
    if config.dry_run {
        flags.push("dry-run");
    }
    if config.confirm_destructive {
        flags.push("confirm-destructive");
    }
    if flags.is_empty() {
        span("read-write", PLAIN)
    } else {
        span(flags.join(", "), FLAG)
    }
}

fn width(spans: &Vec<Span>) -> usize {
    spans.iter().map(|(text, _)| text.chars().count()).sum()
}

/// One framed line: indent, the spans, then padding out to the frame. Padding
/// is computed from visible characters, so colour codes never shift the box.
fn row(spans: &[Span], indent: usize, colors: bool) -> String {
    let used: usize = spans.iter().map(|(text, _)| text.chars().count()).sum();
    let indent = indent.min(WIDTH.saturating_sub(used));
    let mut line = String::new();
    line.push_str(&paint("│", FRAME, colors));
    line.push_str(&" ".repeat(indent));
    for (text, style) in spans {
        line.push_str(&paint(text, style, colors));
    }
    line.push_str(&" ".repeat(WIDTH - indent - used.min(WIDTH)));
    line.push_str(&paint("│", FRAME, colors));
    line.push('\n');
    line
}

fn frame(left: char, right: char, colors: bool) -> String {
    let bar = format!("{left}{}{right}", "─".repeat(WIDTH));
    format!("{}\n", paint(&bar, FRAME, colors))
}

fn paint(text: &str, style: &str, colors: bool) -> String {
    if !colors || style.is_empty() {
        return text.to_string();
    }
    format!("{style}{text}{RESET}")
}

/// Keeps the tail of an over-long value — for a path that is the file name,
/// which is the part worth reading.
fn elide(text: &str, max: usize) -> String {
    let count = text.chars().count();
    if count <= max {
        return text.to_string();
    }
    let tail: String = text.chars().skip(count - max + 1).collect();
    format!("…{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use mcp_atlassian_client::{Auth, ServiceConfig};
    use std::path::PathBuf;
    use std::time::Duration;

    fn config() -> Config {
        Config {
            jira: Some(ServiceConfig {
                base_url: "https://example.atlassian.net".into(),
                auth: Auth::Pat { token: "t".into() },
                deployment: None,
            }),
            confluence: None,
            ..Config::default()
        }
    }

    /// Visible width of a rendered line, with any colour codes removed.
    fn visible(line: &str) -> usize {
        let mut plain = String::new();
        let mut chars = line.chars();
        while let Some(c) = chars.next() {
            if c == '\x1b' {
                for c in chars.by_ref() {
                    if c == 'm' {
                        break;
                    }
                }
            } else {
                plain.push(c);
            }
        }
        plain.chars().count()
    }

    #[test]
    fn every_line_is_the_same_width_with_and_without_colour() {
        let mut config = config();
        config.confluence = config.jira.clone();
        config.read_only = true;
        config.dry_run = true;
        config.audit_log = Some(PathBuf::from("/var/log/mcp-atlassian/audit.jsonl"));
        config.cache_ttl = Some(Duration::from_secs(300));
        for colors in [false, true] {
            for line in render(&config, "streamable-http", 70, colors).lines() {
                if line.is_empty() {
                    continue;
                }
                assert_eq!(visible(line), WIDTH + 2, "{colors}: {line:?}");
            }
        }
    }

    #[test]
    fn a_long_audit_path_does_not_burst_the_box() {
        let mut config = config();
        config.audit_log = Some(PathBuf::from(format!("/{}/audit.jsonl", "x".repeat(300))));
        for line in render(&config, "stdio", 40, false).lines() {
            if !line.is_empty() {
                assert_eq!(visible(line), WIDTH + 2, "{line:?}");
            }
        }
    }

    #[test]
    fn without_colour_there_are_no_escape_sequences() {
        // What a captured stderr — a container log — must contain.
        let plain = render(&config(), "stdio", 40, false);
        assert!(!plain.contains('\x1b'), "{plain}");
        assert!(render(&config(), "stdio", 40, true).contains('\x1b'));
    }

    #[test]
    fn the_banner_states_what_the_server_will_do() {
        let mut config = config();
        config.read_only = true;
        let banner = render(&config, "stdio", 23, false);
        assert!(banner.contains("mcp-atlassian"), "{banner}");
        assert!(banner.contains("stdio"), "{banner}");
        assert!(banner.contains("Jira ✓"), "{banner}");
        assert!(banner.contains("Confluence ✗"), "{banner}");
        assert!(banner.contains("23 registered"), "{banner}");
        assert!(banner.contains("read-only"), "{banner}");
    }

    /// Where to report a problem, taken from the manifest rather than typed
    /// into the banner — the assertion is that the two agree.
    #[test]
    fn the_banner_says_where_the_server_came_from() {
        let banner = render(&config(), "stdio", 1, false);
        assert!(
            banner.contains(env!("CARGO_PKG_REPOSITORY")),
            "the repository from Cargo.toml should be on the banner: {banner}"
        );
    }

    #[test]
    fn an_unrestricted_server_says_so_rather_than_staying_silent() {
        assert!(render(&config(), "stdio", 70, false).contains("read-write"));
    }
}
