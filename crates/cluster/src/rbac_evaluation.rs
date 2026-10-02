//! Client-side RBAC evaluation over one `RbacSnapshot`: the authorizer's rule matching, who can
//! do a request, and what an identity can do. Pure: it never talks to the cluster.

use std::collections::{BTreeSet, HashMap};

use crate::rbac_snapshot::RbacSnapshot;
use crate::role::{RbacRule, RoleSummary};
use crate::role_binding::{
    AUTHENTICATED_GROUP, BindingSummary, RoleKind, SERVICE_ACCOUNTS_GROUP,
    SERVICE_ACCOUNTS_GROUP_PREFIX, Subject, SubjectKind, UNAUTHENTICATED_GROUP,
};

const WILDCARD: &str = "*";
const ANONYMOUS_USER: &str = "system:anonymous";

/// One question for the authorizer: may a caller do `verb` on `target`?
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccessRequest {
    pub verb: String,
    pub target: RequestTarget,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RequestTarget {
    Resource(ResourceRequest),
    NonResource { path: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResourceRequest {
    /// The API group; empty is the core group.
    pub group: String,
    pub resource: String,
    pub subresource: Option<String>,
    pub name: Option<String>,
    /// `None` is a cluster-wide (or cluster-scoped) request.
    pub namespace: Option<String>,
}

/// Who a request is evaluated for: the user name and the groups the authenticator would add.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identity {
    user: Option<String>,
    groups: Vec<String>,
}

impl Identity {
    /// The token authenticator's identity of a service account.
    pub fn service_account(namespace: &str, name: &str) -> Self {
        Self {
            user: Some(format!("system:serviceaccount:{namespace}:{name}")),
            groups: vec![
                SERVICE_ACCOUNTS_GROUP.to_owned(),
                format!("{SERVICE_ACCOUNTS_GROUP_PREFIX}{namespace}"),
                AUTHENTICATED_GROUP.to_owned(),
            ],
        }
    }

    /// A user plus `system:authenticated`; the anonymous user gets `system:unauthenticated`
    /// instead. The user's other groups are unknowable here.
    pub fn user(name: &str) -> Self {
        let group = if name == ANONYMOUS_USER {
            UNAUTHENTICATED_GROUP
        } else {
            AUTHENTICATED_GROUP
        };
        Self {
            user: Some(name.to_owned()),
            groups: vec![group.to_owned()],
        }
    }

    /// Only the named group.
    pub fn group(name: &str) -> Self {
        Self {
            user: None,
            groups: vec![name.to_owned()],
        }
    }

    /// The authorizer's `appliesTo` for one binding subject.
    pub(crate) fn applies_to(&self, subject: &Subject) -> bool {
        match subject.kind {
            SubjectKind::User => self.user.as_deref() == Some(subject.name.as_str()),
            SubjectKind::Group => self.groups.contains(&subject.name),
            SubjectKind::ServiceAccount => {
                let Some(namespace) = &subject.namespace else {
                    return false;
                };
                let name = format!("system:serviceaccount:{namespace}:{}", subject.name);
                self.user.as_deref() == Some(name.as_str())
            }
        }
    }
}

/// Which objects a matching rule reaches.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GrantNames {
    Any,
    /// The rule lists `resourceNames` and the request names no object: only those objects.
    Only(Vec<String>),
}

/// A subject that a binding gives a matching rule.
#[derive(Clone, Debug)]
pub struct Grant<'a> {
    pub subject: &'a Subject,
    pub binding: &'a BindingSummary,
    pub names: GrantNames,
}

/// One rule that reaches an identity through `binding`.
#[derive(Clone, Debug)]
pub struct EffectiveRule<'a> {
    pub rule: &'a RbacRule,
    pub binding: &'a BindingSummary,
    pub subject: &'a Subject,
}

/// The authorizer's `RuleAllows`.
fn rule_match(rule: &RbacRule, request: &AccessRequest) -> Option<GrantNames> {
    if !has_or_wildcard(&rule.verbs, &request.verb) {
        return None;
    }
    match &request.target {
        RequestTarget::NonResource { path } => rule
            .non_resource_urls
            .iter()
            .any(|url| url_matches(url, path))
            .then_some(GrantNames::Any),
        RequestTarget::Resource(resource) => {
            let is_match = has_or_wildcard(&rule.api_groups, &resource.group)
                && rule.resources.iter().any(|rule_resource| {
                    resource_matches(
                        rule_resource,
                        &resource.resource,
                        resource.subresource.as_deref(),
                    )
                });
            if !is_match {
                return None;
            }
            names_match(&rule.resource_names, resource.name.as_deref())
        }
    }
}

fn has_or_wildcard(values: &[String], wanted: &str) -> bool {
    values
        .iter()
        .any(|value| value == WILDCARD || value == wanted)
}

fn resource_matches(rule_resource: &str, resource: &str, subresource: Option<&str>) -> bool {
    if rule_resource == WILDCARD {
        return true;
    }
    // An empty subresource is no subresource, as the authorizer reads it.
    let Some(subresource) = subresource.filter(|subresource| !subresource.is_empty()) else {
        return rule_resource == resource;
    };
    rule_resource
        .strip_prefix(resource)
        .and_then(|rest| rest.strip_prefix('/'))
        == Some(subresource)
        || rule_resource.strip_prefix("*/") == Some(subresource)
}

fn names_match(rule_names: &[String], name: Option<&str>) -> Option<GrantNames> {
    if rule_names.is_empty() {
        return Some(GrantNames::Any);
    }
    match name {
        Some(name) => rule_names
            .iter()
            .any(|rule_name| rule_name == name)
            .then_some(GrantNames::Any),
        None => Some(GrantNames::Only(rule_names.to_vec())),
    }
}

/// An exact URL, `*`, or a prefix ending in `*` (every trailing `*` is dropped, as `/apis/**`).
fn url_matches(url: &str, path: &str) -> bool {
    if url == path {
        return true;
    }
    url.ends_with(WILDCARD) && path.starts_with(url.trim_end_matches(WILDCARD))
}

/// Several matching rules of one binding merge: `Any` wins, else the sorted union of names.
fn merge_names(merged: Option<GrantNames>, next: GrantNames) -> GrantNames {
    match (merged, next) {
        (Some(GrantNames::Any), _) | (_, GrantNames::Any) => GrantNames::Any,
        (None, only) => only,
        (Some(GrantNames::Only(first)), GrantNames::Only(second)) => {
            let union: BTreeSet<String> = first.into_iter().chain(second).collect();
            GrantNames::Only(union.into_iter().collect())
        }
    }
}

type RoleKey<'a> = (Option<&'a str>, &'a str);

impl RbacSnapshot {
    /// Every subject a binding gives a rule that matches `request`, ClusterRoleBindings first,
    /// then RoleBindings, in snapshot order.
    // ponytail: O(bindings x rules) per call, built for one-shot checks; index rules by resource
    // if a very large cluster stalls the caller.
    pub fn who_can(&self, request: &AccessRequest) -> Vec<Grant<'_>> {
        let roles = self.role_lookup();
        let mut grants = Vec::new();
        for binding in self.applicable_bindings(request_namespace(request)) {
            let Some(role) = resolve_role(&roles, binding) else {
                continue;
            };
            let names = role
                .rules
                .iter()
                .filter_map(|rule| rule_match(rule, request))
                .fold(None, |merged, next| Some(merge_names(merged, next)));
            let Some(names) = names else {
                continue;
            };
            for subject in &binding.subjects {
                grants.push(Grant {
                    subject,
                    binding,
                    names: names.clone(),
                });
            }
        }
        grants
    }

    /// Every rule that reaches `identity` in `namespace` (`None`: ClusterRoleBindings only).
    /// Resource requests are assumed, so RoleBindings contribute no non-resource rules.
    ///
    /// Limit: the superuser group `system:masters` bypasses RBAC and is not modeled here; callers
    /// treat an identity in that group as always allowed.
    pub fn rules_of(&self, identity: &Identity, namespace: Option<&str>) -> Vec<EffectiveRule<'_>> {
        let roles = self.role_lookup();
        let mut rules = Vec::new();
        for binding in self.applicable_bindings(namespace) {
            let Some(subject) = binding
                .subjects
                .iter()
                .find(|subject| identity.applies_to(subject))
            else {
                continue;
            };
            let Some(role) = resolve_role(&roles, binding) else {
                continue;
            };
            let is_role_binding = binding.namespace.is_some();
            rules.extend(
                role.rules
                    .iter()
                    .filter(|rule| !is_role_binding || rule.non_resource_urls.is_empty())
                    .map(|rule| EffectiveRule {
                        rule,
                        binding,
                        subject,
                    }),
            );
        }
        rules
    }

    /// The grants that allow `identity` to make `request`; empty means RBAC grants nothing.
    ///
    /// Limit: members of `system:masters` are always allowed without any binding; an empty result
    /// does not mean denied for them, so callers short-circuit that group.
    pub fn decide(&self, identity: &Identity, request: &AccessRequest) -> Vec<Grant<'_>> {
        self.who_can(request)
            .into_iter()
            .filter(|grant| grant.names == GrantNames::Any && identity.applies_to(grant.subject))
            .collect()
    }

    fn role_lookup(&self) -> HashMap<RoleKey<'_>, &RoleSummary> {
        self.roles
            .iter()
            .chain(&self.cluster_roles)
            .map(|role| ((role.namespace.as_deref(), role.name.as_str()), role))
            .collect()
    }

    /// Every ClusterRoleBinding, plus the RoleBindings of `namespace` when there is one.
    fn applicable_bindings(&self, namespace: Option<&str>) -> Vec<&BindingSummary> {
        let in_namespace = self
            .role_bindings
            .iter()
            .filter(|binding| namespace.is_some() && binding.namespace.as_deref() == namespace);
        self.cluster_role_bindings
            .iter()
            .chain(in_namespace)
            .collect()
    }
}

/// Only resource requests carry a namespace; a non-resource request reaches no RoleBinding.
fn request_namespace(request: &AccessRequest) -> Option<&str> {
    match &request.target {
        RequestTarget::Resource(resource) => resource.namespace.as_deref(),
        RequestTarget::NonResource { .. } => None,
    }
}

/// A missing role, or a role kind other than Role and ClusterRole, grants nothing.
fn resolve_role<'a>(
    roles: &HashMap<RoleKey<'a>, &'a RoleSummary>,
    binding: &'a BindingSummary,
) -> Option<&'a RoleSummary> {
    let name = binding.role.name.as_str();
    match binding.role.kind {
        RoleKind::Role => roles
            .get(&(Some(binding.namespace.as_deref()?), name))
            .copied(),
        RoleKind::ClusterRole => roles.get(&(None, name)).copied(),
        RoleKind::Other(_) => None,
    }
}

#[cfg(test)]
#[path = "rbac_evaluation_tests.rs"]
mod rbac_evaluation_tests;
