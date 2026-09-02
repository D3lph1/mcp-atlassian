//! Building JQL and CQL from values that arrive as tool arguments.
//!
//! Both languages quote string literals with `"` and escape with `\\`. A
//! project key or a search term interpolated raw into `project = "{key}"`
//! breaks the query on the first `"` — and turns the rest of the value into
//! query syntax. The fix is the usual one: there is one function that knows
//! how to quote, and every `format!` that builds a query goes through it.

/// Renders `value` as a double-quoted JQL/CQL string literal, escaping the
/// backslash and the quote.
pub fn quote(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::quote;

    #[test]
    fn plain_values_are_quoted_verbatim() {
        assert_eq!(quote("PROJ"), "\"PROJ\"");
        assert_eq!(quote("Jane Doe"), "\"Jane Doe\"");
    }

    #[test]
    fn quotes_and_backslashes_cannot_escape_the_literal() {
        // `PROJ" OR project = "OTHER` must stay one string, not two clauses.
        assert_eq!(
            quote("PROJ\" OR project = \"OTHER"),
            "\"PROJ\\\" OR project = \\\"OTHER\""
        );
        assert_eq!(quote("a\\b"), "\"a\\\\b\"");
    }
}
