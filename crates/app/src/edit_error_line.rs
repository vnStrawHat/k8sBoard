//! Where an Edit YAML error points in the text (UX fix H4): a syntax error carries its own line, and
//! a field path (a server 422, a placeholder that has no value) is looked up in the document. Pure;
//! the editor marks the line and the error rows jump to it.

use cluster::EditError;

/// The 1-based line the local `error` points at, `None` when it names no line or no field.
pub(crate) fn local_error_line(error: &EditError, text: &str) -> Option<usize> {
    match error {
        EditError::Syntax { line, .. } => usize::try_from(*line).ok().filter(|line| *line > 0),
        EditError::LeadingZero { line } => Some(*line),
        EditError::UnmatchedPlaceholder { path } | EditError::MarkerText { path } => {
            line_of_field(text, path)
        }
        EditError::IdentityChanged { field } => line_of_field(text, field),
        _ => None,
    }
}

/// One non-blank, non-comment line of the document, seen as the key it starts.
struct YamlLine {
    /// 1-based line number in the text.
    number: usize,
    /// The column of the key; after a `- ` item marker, the column of what follows it.
    indent: usize,
    /// The column of the `-` that starts a sequence item on this line.
    dash: Option<usize>,
    /// The key this line starts, `None` for a scalar item or a continuation.
    key: Option<String>,
}

/// The 1-based line of the field `path` (`spec.template.spec.containers[0].image`) in `text`: the
/// line of the key, or of the deepest part of the path that exists when the rest is missing.
/// Best effort over block-style YAML (what the editor shows): flow collections, anchors, and
/// `[name]` selectors are not followed.
pub(crate) fn line_of_field(text: &str, path: &str) -> Option<usize> {
    let lines = yaml_lines(text);
    let mut scope = 0..lines.len();
    // The scope of a sequence item starts on its `-` line, which also holds its first key.
    let mut is_item = false;
    let mut found = None;
    for segment in segments(path)? {
        let Some(first) = lines.get(scope.start).filter(|_| !scope.is_empty()) else {
            break;
        };
        let at = match segment {
            Segment::Key(key) => {
                if first.dash.is_some() && !is_item {
                    break;
                }
                is_item = false;
                let base = first.indent;
                let Some(at) = scope.clone().find(|&at| {
                    lines[at].indent == base && lines[at].key.as_deref() == Some(key.as_str())
                }) else {
                    break;
                };
                scope = children(&lines, at, scope.end);
                at
            }
            Segment::Index(index) => {
                let Some(dash) = first.dash else {
                    break;
                };
                let items: Vec<usize> = scope
                    .clone()
                    .filter(|&at| lines[at].dash == Some(dash))
                    .collect();
                let Some(&at) = items.get(index) else {
                    break;
                };
                let end = items.get(index + 1).copied().unwrap_or(scope.end);
                scope = at..end;
                is_item = true;
                at
            }
        };
        found = Some(lines[at].number);
    }
    found
}

/// The lines after `at` that belong to the key on it: the deeper ones. The items of a sequence
/// written at the key's own column count as deeper, since their content follows the `- `.
fn children(lines: &[YamlLine], at: usize, end: usize) -> std::ops::Range<usize> {
    let indent = lines[at].indent;
    let stop = (at + 1..end)
        .find(|&next| lines[next].indent <= indent)
        .unwrap_or(end);
    at + 1..stop
}

fn yaml_lines(text: &str) -> Vec<YamlLine> {
    text.lines()
        .enumerate()
        .filter_map(|(index, line)| {
            let trimmed = line.trim_start();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                return None;
            }
            let column = line.len() - trimmed.len();
            let (dash, rest, indent) = match trimmed.strip_prefix('-') {
                Some(after) if after.is_empty() || after.starts_with(' ') => {
                    let rest = after.trim_start();
                    (Some(column), rest, column + 1 + (after.len() - rest.len()))
                }
                _ => (None, trimmed, column),
            };
            Some(YamlLine {
                number: index + 1,
                indent,
                dash,
                key: key_of(rest),
            })
        })
        .collect()
}

/// The key a `key: value` or `key:` line starts; quoted keys lose their quotes.
fn key_of(rest: &str) -> Option<String> {
    let (key, after) = match rest.chars().next()? {
        quote @ ('"' | '\'') => {
            let close = rest[1..].find(quote)? + 1;
            (&rest[1..close], &rest[close + 1..])
        }
        _ => {
            let colon = rest.find(':')?;
            (&rest[..colon], &rest[colon..])
        }
    };
    let is_key = after
        .strip_prefix(':')
        .is_some_and(|value| value.is_empty() || value.starts_with(' '));
    is_key.then(|| key.trim_end().to_owned())
}

/// The byte range of the text of the 1-based `line` in `text`, without its line break.
pub(crate) fn line_byte_range(text: &str, line: usize) -> Option<std::ops::Range<usize>> {
    let mut start = 0;
    for (index, row) in text.split_inclusive('\n').enumerate() {
        if index + 1 == line {
            return Some(start..start + row.trim_end_matches(['\n', '\r']).len());
        }
        start += row.len();
    }
    None
}

enum Segment {
    Key(String),
    Index(usize),
}

/// `spec.containers[0].image` as keys and indexes; `None` when a `[..]` is not a number (a
/// `[name]` selector has no place in the text to look for).
fn segments(path: &str) -> Option<Vec<Segment>> {
    let mut segments = Vec::new();
    for part in path.split('.') {
        let (key, mut indexes) = match part.find('[') {
            Some(open) => (&part[..open], &part[open..]),
            None => (part, ""),
        };
        if !key.is_empty() {
            segments.push(Segment::Key(key.to_owned()));
        }
        while let Some(rest) = indexes.strip_prefix('[') {
            let close = rest.find(']')?;
            segments.push(Segment::Index(rest[..close].parse().ok()?));
            indexes = &rest[close + 1..];
        }
    }
    Some(segments)
}

#[cfg(test)]
#[path = "edit_error_line_tests.rs"]
mod edit_error_line_tests;
