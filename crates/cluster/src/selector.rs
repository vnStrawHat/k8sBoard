//! The one label matcher: Kubernetes label selectors against `key=value` label terms.

use k8s_openapi::apimachinery::pkg::apis::meta::v1::LabelSelector;

/// A parsed label selector. An empty selector selects everything.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Selector {
    requirements: Vec<Requirement>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Requirement {
    Equals(String, String),
    In(String, Vec<String>),
    NotIn(String, Vec<String>),
    Exists(String),
    DoesNotExist(String),
    /// An unknown operator or a malformed term: matches nothing.
    Invalid,
}

impl Selector {
    /// `matchLabels` in key order, then `matchExpressions` in order.
    pub(crate) fn of(selector: &LabelSelector) -> Self {
        let equalities = selector
            .match_labels
            .iter()
            .flatten()
            .map(|(key, value)| Requirement::Equals(key.clone(), value.clone()));
        let expressions = selector
            .match_expressions
            .iter()
            .flatten()
            .map(|expression| {
                let key = expression.key.clone();
                let values = expression.values.clone().unwrap_or_default();
                match expression.operator.as_str() {
                    "In" => Requirement::In(key, values),
                    "NotIn" => Requirement::NotIn(key, values),
                    "Exists" => Requirement::Exists(key),
                    "DoesNotExist" => Requirement::DoesNotExist(key),
                    _ => Requirement::Invalid,
                }
            });
        Self {
            requirements: equalities.chain(expressions).collect(),
        }
    }

    /// The selector with no requirements: it selects every pod or namespace.
    pub fn everything() -> Self {
        Self {
            requirements: Vec::new(),
        }
    }

    /// A Service selector: `key=value` terms. A term without `=` never matches. `None` for no
    /// terms: a Service without a selector selects nothing (its endpoints are managed by hand),
    /// while an empty `Selector` would select every pod.
    pub fn of_labels(terms: &[String]) -> Option<Self> {
        if terms.is_empty() {
            return None;
        }
        let requirements = terms
            .iter()
            .map(|term| match term.split_once('=') {
                Some((key, value)) => Requirement::Equals(key.to_owned(), value.to_owned()),
                None => Requirement::Invalid,
            })
            .collect();
        Some(Self { requirements })
    }

    pub fn selects_everything(&self) -> bool {
        self.requirements.is_empty()
    }

    /// Whether `labels` (`key=value` terms in key order, as `PodSummary.labels` holds them)
    /// satisfy every requirement.
    pub fn matches(&self, labels: &[String]) -> bool {
        self.requirements
            .iter()
            .all(|requirement| requirement.matches(labels))
    }

    /// kubectl syntax, for example `app=api`, `env in (a,b)`, `!legacy`.
    pub fn terms(&self) -> Vec<String> {
        self.requirements.iter().map(Requirement::term).collect()
    }
}

impl Requirement {
    fn matches(&self, labels: &[String]) -> bool {
        match self {
            Self::Equals(key, value) => value_of(labels, key) == Some(value.as_str()),
            Self::In(key, values) => {
                value_of(labels, key).is_some_and(|value| values.iter().any(|item| item == value))
            }
            Self::NotIn(key, values) => {
                value_of(labels, key).is_none_or(|value| !values.iter().any(|item| item == value))
            }
            Self::Exists(key) => value_of(labels, key).is_some(),
            Self::DoesNotExist(key) => value_of(labels, key).is_none(),
            Self::Invalid => false,
        }
    }

    fn term(&self) -> String {
        match self {
            Self::Equals(key, value) => format!("{key}={value}"),
            Self::In(key, values) => format!("{key} in ({})", values.join(",")),
            Self::NotIn(key, values) => format!("{key} notin ({})", values.join(",")),
            Self::Exists(key) => key.clone(),
            Self::DoesNotExist(key) => format!("!{key}"),
            Self::Invalid => "<invalid>".to_owned(),
        }
    }
}

/// The text before the first `=`; label keys never contain `=`.
fn key_of(term: &str) -> &str {
    term.split_once('=').map_or(term, |(key, _)| key)
}

/// The value of `key` in terms sorted by key.
fn value_of<'a>(labels: &'a [String], key: &str) -> Option<&'a str> {
    let index = labels.binary_search_by(|term| key_of(term).cmp(key)).ok()?;
    labels.get(index)?.split_once('=').map(|(_, value)| value)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use k8s_openapi::apimachinery::pkg::apis::meta::v1::LabelSelectorRequirement;

    use super::*;

    fn terms(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| (*item).to_owned()).collect()
    }

    fn labels_selector(items: &[&str]) -> Selector {
        Selector::of_labels(&terms(items)).expect("non-empty terms")
    }

    fn expression(key: &str, operator: &str, values: &[&str]) -> LabelSelectorRequirement {
        LabelSelectorRequirement {
            key: key.to_owned(),
            operator: operator.to_owned(),
            values: Some(terms(values)),
        }
    }

    fn selector(
        match_labels: &[(&str, &str)],
        expressions: Vec<LabelSelectorRequirement>,
    ) -> LabelSelector {
        LabelSelector {
            match_labels: Some(
                match_labels
                    .iter()
                    .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
                    .collect::<BTreeMap<_, _>>(),
            ),
            match_expressions: Some(expressions),
        }
    }

    #[test]
    fn match_labels_and_expressions_all_hold() {
        let selector = Selector::of(&selector(
            &[("app", "api")],
            vec![
                expression("env", "In", &["prod", "stage"]),
                expression("canary", "Exists", &[]),
            ],
        ));
        assert!(selector.matches(&terms(&["app=api", "canary=", "env=stage"])));
        assert!(!selector.matches(&terms(&["app=api", "env=stage"])));
        assert!(!selector.matches(&terms(&["app=api", "canary=", "env=dev"])));
        assert!(!selector.matches(&terms(&["app=web", "canary=", "env=prod"])));
    }

    #[test]
    fn not_in_and_does_not_exist_hold_for_missing_key() {
        let selector = Selector::of(&selector(
            &[],
            vec![
                expression("zone", "NotIn", &["a"]),
                expression("legacy", "DoesNotExist", &[]),
            ],
        ));
        assert!(selector.matches(&terms(&["app=api"])));
        assert!(selector.matches(&terms(&["zone=b"])));
        assert!(!selector.matches(&terms(&["zone=a"])));
        assert!(!selector.matches(&terms(&["legacy=1"])));
    }

    #[test]
    fn empty_selector_selects_everything() {
        let selector = Selector::of(&LabelSelector::default());
        assert!(selector.selects_everything());
        assert!(selector.matches(&[]));
        assert!(selector.matches(&terms(&["app=api"])));
        assert!(Selector::of_labels(&[]).is_none());
    }

    #[test]
    fn unknown_operator_matches_nothing() {
        let selector = Selector::of(&selector(&[], vec![expression("app", "Gt", &["1"])]));
        assert!(!selector.selects_everything());
        assert!(!selector.matches(&terms(&["app=2"])));
        assert!(!selector.matches(&[]));
    }

    #[test]
    fn terms_match_kubectl_syntax() {
        let selector = Selector::of(&selector(
            &[("tier", "web"), ("app", "api")],
            vec![
                expression("env", "In", &["prod", "stage"]),
                expression("zone", "NotIn", &["a"]),
                expression("canary", "Exists", &[]),
                expression("legacy", "DoesNotExist", &[]),
                expression("odd", "Gt", &["1"]),
            ],
        ));
        assert_eq!(
            selector.terms(),
            [
                "app=api",
                "tier=web",
                "env in (prod,stage)",
                "zone notin (a)",
                "canary",
                "!legacy",
                "<invalid>",
            ]
        );
        assert!(Selector::of(&LabelSelector::default()).terms().is_empty());
    }

    #[test]
    fn of_labels_builds_equalities() {
        let selector = labels_selector(&["app=api", "tier=web"]);
        assert_eq!(selector.terms(), ["app=api", "tier=web"]);
        assert!(selector.matches(&terms(&["app=api", "extra=1", "tier=web"])));
        assert!(!selector.matches(&terms(&["app=api"])));
        let malformed = labels_selector(&["app"]);
        assert!(!malformed.matches(&terms(&["app=api"])));
        assert_eq!(malformed.terms(), ["<invalid>"]);
    }

    #[test]
    fn matches_by_key_binary_search() {
        // Key order puts `a` before `a.b`, though the term texts sort the other way.
        let labels = terms(&["a=1", "a.b=2", "c=x=y"]);
        assert!(labels_selector(&["a=1"]).matches(&labels));
        assert!(labels_selector(&["a.b=2"]).matches(&labels));
        assert!(labels_selector(&["c=x=y"]).matches(&labels));
        assert!(!labels_selector(&["a=2"]).matches(&labels));
        assert!(!labels_selector(&["c=x"]).matches(&labels));
        assert!(!labels_selector(&["b=1"]).matches(&labels));
    }
}
