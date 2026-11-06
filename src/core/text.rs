//! Text and JSON helpers. Line numbers shown to the extraction model and the
//! lines copied back from the screen must be split by the same rules.

use serde_json::Value;

/// Python `str.isspace`: Unicode whitespace plus the ASCII separators 0x1c-0x1f.
pub fn is_space(c: char) -> bool {
    c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c)
}

pub fn strip(text: &str) -> &str {
    text.trim_matches(is_space)
}

pub fn is_blank(text: &str) -> bool {
    text.chars().all(is_space)
}

/// Python `str.splitlines`: every line boundary, no trailing empty line.
pub fn splitlines(text: &str) -> Vec<&str> {
    let mut lines = Vec::new();
    let mut start = 0;
    let mut chars = text.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        let end = match c {
            '\r' if matches!(chars.peek(), Some((_, '\n'))) => {
                chars.next();
                i + 2
            }
            '\r' | '\n' | '\u{b}' | '\u{c}' | '\u{1c}' | '\u{1d}' | '\u{1e}' | '\u{85}' | '\u{2028}' | '\u{2029}' => {
                i + c.len_utf8()
            }
            _ => continue,
        };
        lines.push(&text[start..i]);
        start = end;
    }
    if start < text.len() {
        lines.push(&text[start..]);
    }
    lines
}

/// Removes all whitespace, for layout-insensitive comparison of wrapped text.
pub fn compact(text: &str) -> String {
    text.chars().filter(|c| !is_space(*c)).collect()
}

/// Python truthiness of a JSON value.
pub fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|n| n != 0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

/// Display text for a JSON value: strings as-is, everything else as JSON.
pub fn value_text(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// The first truthy value of a Python `a or b or c` chain, as display text.
pub fn first_text<'a>(values: impl IntoIterator<Item = Option<&'a Value>>) -> Option<String> {
    values.into_iter().flatten().find(|value| truthy(value)).map(value_text)
}

/// The last `limit` characters, for bounded error messages.
pub fn tail(text: &str, limit: usize) -> &str {
    let count = text.chars().count();
    if count <= limit {
        return text;
    }
    let (index, _) = text.char_indices().nth(count - limit).unwrap();
    &text[index..]
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn splitlines_matches_python() {
        assert_eq!(splitlines(""), Vec::<&str>::new());
        assert_eq!(splitlines("\n"), vec![""]);
        assert_eq!(splitlines("a\n"), vec!["a"]);
        assert_eq!(splitlines("a\n\nb"), vec!["a", "", "b"]);
        assert_eq!(splitlines("a\r\nb\rc\u{2028}d"), vec!["a", "b", "c", "d"]);
    }

    #[test]
    fn compact_and_truthiness() {
        assert_eq!(compact(" a \n b\u{3000}c\u{1f}"), "abc");
        assert!(!truthy(&json!(null)) && !truthy(&json!("")) && !truthy(&json!({})));
        assert!(truthy(&json!("x")) && truthy(&json!(1)) && truthy(&json!([0])));
        assert_eq!(first_text([None, Some(&json!("")), Some(&json!("b"))]), Some("b".into()));
        assert_eq!(tail("一二三四", 2), "三四");
    }
}
