//! The `ENABLED_TOOLS` allowlist (D27).
//!
//! A comma-separated list of tool names, where `*` stands for any run of
//! characters and may appear anywhere in a pattern. A pattern without `*` is an
//! exact name, so a list written before wildcards existed keeps its meaning.
//!
//! Deliberately not a glob library: no `?`, no `[a-z]`, no `{a,b}`. Tool names
//! are a flat set of lowercase `product_verb_noun` identifiers, and `*` covers
//! every way anyone has wanted to slice them — a whole product (`jira_*`), a
//! verb across products (`*_get_*`), one noun (`*_attachment*`).

/// An allowlist of tool-name patterns. An absent filter (`None` at the call
/// site) means every tool is enabled; an empty one cannot be built.
#[derive(Debug, Clone)]
pub struct ToolFilter {
    patterns: Vec<String>,
}

impl ToolFilter {
    /// Parses the comma-separated form. Blank entries are dropped, and a list
    /// that holds nothing but blanks is `None` — "set to empty" reads as "do
    /// not filter", the same as leaving the variable unset.
    pub fn parse(raw: &str) -> Option<Self> {
        let patterns: Vec<String> = raw
            .split(',')
            .map(|pattern| pattern.trim().to_string())
            .filter(|pattern| !pattern.is_empty())
            .collect();
        (!patterns.is_empty()).then_some(Self { patterns })
    }

    /// Whether any pattern accepts this tool name.
    pub fn matches(&self, name: &str) -> bool {
        self.patterns
            .iter()
            .any(|pattern| matches_pattern(pattern, name))
    }

    /// Patterns that accept none of `tools`. A typo in a wildcard silently
    /// enables nothing, so the caller warns about these at startup.
    pub fn unmatched<'a>(&'a self, tools: &[String]) -> Vec<&'a str> {
        self.patterns
            .iter()
            .filter(|pattern| !tools.iter().any(|tool| matches_pattern(pattern, tool)))
            .map(String::as_str)
            .collect()
    }
}

/// `*` matches any run of characters, including none; everything else is
/// literal. Segments between wildcards are matched greedily from the left,
/// which is exact for this grammar: with no backtracking constructs, the
/// leftmost occurrence of a segment never rules out a match the rightmost one
/// would have allowed.
fn matches_pattern(pattern: &str, name: &str) -> bool {
    let segments: Vec<&str> = pattern.split('*').collect();
    let [prefix, middles @ .., suffix] = segments.as_slice() else {
        // No `*` at all: the pattern is a plain name.
        return pattern == name;
    };
    let Some(mut rest) = name.strip_prefix(prefix) else {
        return false;
    };
    for middle in middles {
        let Some(at) = rest.find(middle) else {
            return false;
        };
        rest = &rest[at + middle.len()..];
    }
    // The suffix may not reuse characters an earlier segment already consumed.
    rest.ends_with(suffix)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tools() -> Vec<String> {
        [
            "jira_search",
            "jira_get_issue",
            "jira_create_issue",
            "jira_download_attachment",
            "confluence_search",
            "confluence_get_page",
            "confluence_download_attachment",
        ]
        .map(String::from)
        .to_vec()
    }

    fn enabled(raw: &str) -> Vec<String> {
        let filter = ToolFilter::parse(raw).expect("empty filter");
        tools().into_iter().filter(|t| filter.matches(t)).collect()
    }

    #[test]
    fn a_pattern_without_a_wildcard_is_an_exact_name() {
        assert_eq!(enabled("jira_search"), ["jira_search"]);
        assert!(!ToolFilter::parse("jira_sea")
            .unwrap()
            .matches("jira_search"));
    }

    #[test]
    fn a_trailing_wildcard_selects_a_product() {
        assert_eq!(
            enabled("confluence_*"),
            [
                "confluence_search",
                "confluence_get_page",
                "confluence_download_attachment"
            ]
        );
    }

    #[test]
    fn a_leading_wildcard_selects_a_suffix() {
        assert_eq!(
            enabled("*_attachment"),
            ["jira_download_attachment", "confluence_download_attachment"]
        );
    }

    #[test]
    fn a_wildcard_in_the_middle_selects_a_verb_across_products() {
        assert_eq!(
            enabled("*_get_*"),
            ["jira_get_issue", "confluence_get_page"]
        );
    }

    #[test]
    fn several_wildcards_compose() {
        assert_eq!(
            enabled("*_*_attachment"),
            ["jira_download_attachment", "confluence_download_attachment"]
        );
        assert_eq!(
            enabled("jira_*_issue"),
            ["jira_get_issue", "jira_create_issue"]
        );
    }

    #[test]
    fn a_wildcard_also_matches_nothing() {
        // `*` is any run of characters *including empty*, so the pattern below
        // must still accept the bare name.
        assert!(ToolFilter::parse("jira_search*")
            .unwrap()
            .matches("jira_search"));
        assert!(ToolFilter::parse("*jira_search")
            .unwrap()
            .matches("jira_search"));
        assert!(ToolFilter::parse("jira*_search")
            .unwrap()
            .matches("jira_search"));
    }

    #[test]
    fn a_lone_wildcard_enables_everything() {
        assert_eq!(enabled("*"), tools());
    }

    #[test]
    fn segments_may_not_overlap() {
        // "jira" is consumed by the prefix, so the suffix has nothing left to
        // match — a naive `starts_with` + `ends_with` check would accept this.
        assert!(!ToolFilter::parse("jira*jira").unwrap().matches("jira"));
        assert!(ToolFilter::parse("jira*jira").unwrap().matches("jira_jira"));
    }

    #[test]
    fn patterns_combine_as_a_union() {
        assert_eq!(
            enabled("jira_search, confluence_*_page"),
            ["jira_search", "confluence_get_page"]
        );
    }

    #[test]
    fn blank_entries_are_dropped_and_an_all_blank_list_is_no_filter() {
        assert_eq!(enabled("jira_search,, ,"), ["jira_search"]);
        for raw in ["", "   ", ",", " , , "] {
            assert!(ToolFilter::parse(raw).is_none(), "{raw:?}");
        }
    }

    #[test]
    fn a_pattern_matching_no_tool_is_reported() {
        let filter = ToolFilter::parse("jira_*,jira_serch,bitbucket_*").unwrap();
        assert_eq!(filter.unmatched(&tools()), ["jira_serch", "bitbucket_*"]);
        assert!(ToolFilter::parse("*")
            .unwrap()
            .unmatched(&tools())
            .is_empty());
    }
}
