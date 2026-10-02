//! The printer-column JSONPath subset: `.field` chains, `['key']`, `[n]`, `[*]`, and
//! `[?(@.a.b == "lit")]` filters, evaluated to the first match like the API server does.
//!
//! Pure, no I/O, and no logging: a path or a value never leaves this module as text except
//! through `column_value`, which applies the secret rules of `object_yaml`.

use serde_json::Value;

use crate::custom_object::ColumnValue;
use crate::custom_resource_definition::{ColumnType, PrinterColumn};
use crate::object_yaml::{is_secret_key, mask_url_userinfo};

/// Longest text a column cell keeps.
const MAX_TEXT_CHARS: usize = 200;

/// A compiled printer-column path. Only the subset below parses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ColumnPath {
    root: PathRoot,
    steps: Vec<Step>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PathRoot {
    /// The first field was `metadata`; it is dropped from `steps`.
    Metadata,
    Data,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Step {
    Field(String),
    Index(usize),
    Wildcard,
    Filter { path: Vec<String>, literal: String },
}

/// No payload on purpose: the column is simply `—`.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct UnsupportedPath;

impl ColumnPath {
    pub(crate) fn parse(text: &str) -> Result<Self, UnsupportedPath> {
        let mut parser = Parser {
            chars: text.chars().collect(),
            position: 0,
        };
        let mut steps = parser.steps()?;
        let root = match steps.first() {
            Some(Step::Field(name)) if name == "metadata" => {
                steps.remove(0);
                PathRoot::Metadata
            }
            _ => PathRoot::Data,
        };
        Ok(Self { root, steps })
    }

    /// The first value the path reaches with the container it was read from, or `None`.
    pub(crate) fn first_match<'a>(
        &self,
        metadata: &'a Value,
        data: &'a Value,
    ) -> Option<Found<'a>> {
        let root = match self.root {
            PathRoot::Metadata => metadata,
            PathRoot::Data => data,
        };
        walk(root, root, &self.steps)
    }

    pub(crate) fn reads_metadata(&self) -> bool {
        self.root == PathRoot::Metadata
    }

    /// `…conditions[?(@.type == "X")].status`: the cell is toned.
    pub(crate) fn is_condition_status(&self) -> bool {
        let [
            ..,
            Step::Field(conditions),
            Step::Filter { .. },
            Step::Field(status),
        ] = self.steps.as_slice()
        else {
            return false;
        };
        conditions == "conditions" && status == "status"
    }

    /// The last field name, for the secret-key rule.
    pub(crate) fn last_field(&self) -> Option<&str> {
        self.steps.iter().rev().find_map(|step| match step {
            Step::Field(name) => Some(name.as_str()),
            _ => None,
        })
    }

    /// A path ending in `.value` read from a `{name, value}` pair whose `name` is secret-like,
    /// such as an env entry `DB_PASSWORD`.
    fn is_secret_named_value(&self, parent: &Value) -> bool {
        let reads_value = matches!(self.steps.last(), Some(Step::Field(name)) if name == "value");
        reads_value
            && parent
                .get("name")
                .and_then(Value::as_str)
                .is_some_and(is_secret_key)
    }

    /// Whether the path starts at `.status` or `.metadata`, for the secret-like kind rule.
    pub(crate) fn is_status_or_metadata(&self) -> bool {
        self.root == PathRoot::Metadata
            || matches!(self.steps.first(), Some(Step::Field(name)) if name == "status")
    }
}

/// A value a path reached and the array or object it was read from (the value itself for an
/// empty path), which tells an env-style `{name, value}` pair apart from a plain field.
#[derive(Clone, Copy)]
pub(crate) struct Found<'a> {
    value: &'a Value,
    parent: &'a Value,
}

fn walk<'a>(value: &'a Value, parent: &'a Value, steps: &[Step]) -> Option<Found<'a>> {
    let Some((step, rest)) = steps.split_first() else {
        return Some(Found { value, parent });
    };
    match step {
        Step::Field(name) => walk(value.get(name.as_str())?, value, rest),
        Step::Index(index) => walk(value.as_array()?.get(*index)?, value, rest),
        Step::Wildcard => match value {
            Value::Array(items) => items.iter().find_map(|item| walk(item, value, rest)),
            Value::Object(map) => map.values().find_map(|item| walk(item, value, rest)),
            _ => None,
        },
        Step::Filter { path, literal } => value
            .as_array()?
            .iter()
            .filter(|item| text_at(item, path) == Some(literal.as_str()))
            .find_map(|item| walk(item, value, rest)),
    }
}

/// The JSON string at `path`, or `None` for a missing path or a non-string value.
fn text_at<'a>(value: &'a Value, path: &[String]) -> Option<&'a str> {
    let mut current = value;
    for name in path {
        current = current.get(name.as_str())?;
    }
    current.as_str()
}

/// The cell for `found`, typed by the column. `column` carries the type and the password
/// format; an unsupported path never reaches here.
pub(crate) fn column_value(
    column: &PrinterColumn,
    path: &ColumnPath,
    found: Option<Found>,
) -> ColumnValue {
    let Some(found) = found.filter(|found| !found.value.is_null()) else {
        return ColumnValue::Absent;
    };
    let value = found.value;
    let is_secret = column.is_password
        || path.last_field().is_some_and(is_secret_key)
        || path.is_secret_named_value(found.parent);
    if is_secret {
        return ColumnValue::Hidden;
    }
    match (column.column_type, value) {
        (ColumnType::String, Value::String(text)) => ColumnValue::Text(shown_text(text)),
        (ColumnType::String, Value::Number(_) | Value::Bool(_)) => {
            ColumnValue::Text(shown_text(&value.to_string()))
        }
        (ColumnType::Integer, value) => value
            .as_i64()
            .map_or(ColumnValue::Absent, ColumnValue::Integer),
        (ColumnType::Number, Value::Number(number)) => ColumnValue::Number(number.to_string()),
        (ColumnType::Boolean, Value::Bool(flag)) => ColumnValue::Boolean(*flag),
        (ColumnType::Date, Value::String(text)) => text
            .parse::<jiff::Timestamp>()
            .map_or(ColumnValue::Absent, ColumnValue::Date),
        _ => ColumnValue::Absent,
    }
}

/// URL userinfo hidden first so a cut can never split a credential, then cut at the limit.
pub(crate) fn shown_text(text: &str) -> String {
    shown_text_within(text, MAX_TEXT_CHARS)
}

/// `shown_text` with a caller-chosen limit.
pub(crate) fn shown_text_within(text: &str, limit: usize) -> String {
    let masked = mask_url_userinfo(text);
    let text = masked.as_deref().unwrap_or(text);
    cut_text(text, limit)
}

/// At most `limit` characters; a longer text ends with `…` within the limit.
pub(crate) fn cut_text(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_owned();
    }
    let mut cut: String = text.chars().take(limit.saturating_sub(1)).collect();
    cut.push('…');
    cut
}

struct Parser {
    chars: Vec<char>,
    position: usize,
}

impl Parser {
    fn steps(&mut self) -> Result<Vec<Step>, UnsupportedPath> {
        if self.peek() != Some('.') {
            return Err(UnsupportedPath);
        }
        let mut steps = Vec::new();
        while let Some(next) = self.peek() {
            match next {
                '.' => {
                    self.position += 1;
                    steps.push(Step::Field(self.name()?));
                }
                '[' => {
                    self.position += 1;
                    steps.push(self.bracket()?);
                }
                _ => return Err(UnsupportedPath),
            }
        }
        Ok(steps)
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.position).copied()
    }

    fn skip_spaces(&mut self) {
        while self.peek().is_some_and(char::is_whitespace) {
            self.position += 1;
        }
    }

    fn expect(&mut self, expected: char) -> Result<(), UnsupportedPath> {
        if self.peek() != Some(expected) {
            return Err(UnsupportedPath);
        }
        self.position += 1;
        Ok(())
    }

    /// `[A-Za-z0-9_-]+`; an empty name covers `..`, `.*`, and a trailing dot.
    fn name(&mut self) -> Result<String, UnsupportedPath> {
        let start = self.position;
        while self
            .peek()
            .is_some_and(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-'))
        {
            self.position += 1;
        }
        if start == self.position {
            return Err(UnsupportedPath);
        }
        Ok(self.chars[start..self.position].iter().collect())
    }

    /// After `[`: a quoted key, an index, `*`, or a filter, then `]`.
    fn bracket(&mut self) -> Result<Step, UnsupportedPath> {
        self.skip_spaces();
        let step = match self.peek() {
            Some('\'' | '"') => Step::Field(self.quoted()?),
            Some('*') => {
                self.position += 1;
                Step::Wildcard
            }
            Some('?') => {
                self.position += 1;
                self.filter()?
            }
            Some(digit) if digit.is_ascii_digit() => Step::Index(self.index()?),
            _ => return Err(UnsupportedPath),
        };
        self.skip_spaces();
        self.expect(']')?;
        Ok(step)
    }

    fn index(&mut self) -> Result<usize, UnsupportedPath> {
        let start = self.position;
        while self.peek().is_some_and(|ch| ch.is_ascii_digit()) {
            self.position += 1;
        }
        let digits: String = self.chars[start..self.position].iter().collect();
        digits.parse().map_err(|_| UnsupportedPath)
    }

    /// A string in single or double quotes; `\'`, `\"`, and `\\` are escapes.
    fn quoted(&mut self) -> Result<String, UnsupportedPath> {
        let quote = self.peek().ok_or(UnsupportedPath)?;
        self.position += 1;
        let mut text = String::new();
        loop {
            let ch = self.peek().ok_or(UnsupportedPath)?;
            self.position += 1;
            match ch {
                ch if ch == quote => return Ok(text),
                '\\' => {
                    let escaped = self.peek().ok_or(UnsupportedPath)?;
                    if !matches!(escaped, '\'' | '"' | '\\') {
                        return Err(UnsupportedPath);
                    }
                    self.position += 1;
                    text.push(escaped);
                }
                ch => text.push(ch),
            }
        }
    }

    /// After `?`: `(@.a.b == "literal")`. Only string literals and `==` are supported.
    fn filter(&mut self) -> Result<Step, UnsupportedPath> {
        self.expect('(')?;
        self.skip_spaces();
        self.expect('@')?;
        let mut path = Vec::new();
        while self.peek() == Some('.') {
            self.position += 1;
            path.push(self.name()?);
        }
        if path.is_empty() {
            return Err(UnsupportedPath);
        }
        self.skip_spaces();
        self.expect('=')?;
        self.expect('=')?;
        self.skip_spaces();
        if !matches!(self.peek(), Some('\'' | '"')) {
            return Err(UnsupportedPath);
        }
        let literal = self.quoted()?;
        self.skip_spaces();
        self.expect(')')?;
        Ok(Step::Filter { path, literal })
    }
}

#[cfg(test)]
#[path = "column_path_tests.rs"]
mod column_path_tests;
