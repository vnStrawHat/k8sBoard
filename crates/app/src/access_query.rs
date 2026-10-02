//! The one-line request syntax of the RBAC tools: `verb resource[.group][/subresource] [name]`
//! or `verb /url`. Pure; the dialogs show the hints and errors it returns.

use cluster::{AccessRequest, RbacRule, RequestTarget, ResourceRequest};

const WILDCARD: &str = "*";

/// The built-in kinds of Kubernetes v1.29 as `(plural, API group, is namespaced)`. There is no API
/// discovery, so this fills in the group a request leaves out and tells cluster-scoped kinds,
/// which take no namespace.
const BUILT_IN_RESOURCES: &[(&str, &str, bool)] = &[
    ("pods", "", true),
    ("services", "", true),
    ("endpoints", "", true),
    ("configmaps", "", true),
    ("secrets", "", true),
    ("serviceaccounts", "", true),
    ("persistentvolumeclaims", "", true),
    ("events", "", true),
    ("limitranges", "", true),
    ("resourcequotas", "", true),
    ("replicationcontrollers", "", true),
    ("podtemplates", "", true),
    ("bindings", "", true),
    ("nodes", "", false),
    ("componentstatuses", "", false),
    ("namespaces", "", false),
    ("persistentvolumes", "", false),
    ("deployments", "apps", true),
    ("statefulsets", "apps", true),
    ("daemonsets", "apps", true),
    ("replicasets", "apps", true),
    ("controllerrevisions", "apps", true),
    ("jobs", "batch", true),
    ("cronjobs", "batch", true),
    ("horizontalpodautoscalers", "autoscaling", true),
    ("poddisruptionbudgets", "policy", true),
    ("ingresses", "networking.k8s.io", true),
    ("networkpolicies", "networking.k8s.io", true),
    ("ingressclasses", "networking.k8s.io", false),
    ("endpointslices", "discovery.k8s.io", true),
    ("roles", "rbac.authorization.k8s.io", true),
    ("rolebindings", "rbac.authorization.k8s.io", true),
    ("clusterroles", "rbac.authorization.k8s.io", false),
    ("clusterrolebindings", "rbac.authorization.k8s.io", false),
    ("leases", "coordination.k8s.io", true),
    ("storageclasses", "storage.k8s.io", false),
    ("volumeattachments", "storage.k8s.io", false),
    ("csidrivers", "storage.k8s.io", false),
    ("csinodes", "storage.k8s.io", false),
    ("csistoragecapacities", "storage.k8s.io", true),
    ("customresourcedefinitions", "apiextensions.k8s.io", false),
    (
        "mutatingwebhookconfigurations",
        "admissionregistration.k8s.io",
        false,
    ),
    (
        "validatingwebhookconfigurations",
        "admissionregistration.k8s.io",
        false,
    ),
    (
        "validatingadmissionpolicies",
        "admissionregistration.k8s.io",
        false,
    ),
    (
        "validatingadmissionpolicybindings",
        "admissionregistration.k8s.io",
        false,
    ),
    ("certificatesigningrequests", "certificates.k8s.io", false),
    ("priorityclasses", "scheduling.k8s.io", false),
    ("runtimeclasses", "node.k8s.io", false),
    ("apiservices", "apiregistration.k8s.io", false),
    ("flowschemas", "flowcontrol.apiserver.k8s.io", false),
    (
        "prioritylevelconfigurations",
        "flowcontrol.apiserver.k8s.io",
        false,
    ),
    ("tokenreviews", "authentication.k8s.io", false),
    ("selfsubjectreviews", "authentication.k8s.io", false),
    ("selfsubjectaccessreviews", "authorization.k8s.io", false),
    ("selfsubjectrulesreviews", "authorization.k8s.io", false),
    ("subjectaccessreviews", "authorization.k8s.io", false),
    ("localsubjectaccessreviews", "authorization.k8s.io", true),
];

/// The kubectl short names of the built-in kinds.
const SHORT_NAMES: &[(&str, &str)] = &[
    ("po", "pods"),
    ("svc", "services"),
    ("deploy", "deployments"),
    ("cm", "configmaps"),
    ("ns", "namespaces"),
    ("sa", "serviceaccounts"),
    ("no", "nodes"),
    ("pv", "persistentvolumes"),
    ("pvc", "persistentvolumeclaims"),
    ("ds", "daemonsets"),
    ("sts", "statefulsets"),
    ("rs", "replicasets"),
    ("cj", "cronjobs"),
    ("ing", "ingresses"),
    ("netpol", "networkpolicies"),
    ("crd", "customresourcedefinitions"),
    ("sc", "storageclasses"),
    ("pdb", "poddisruptionbudgets"),
    ("hpa", "horizontalpodautoscalers"),
    ("ep", "endpoints"),
    ("ev", "events"),
];

/// The subresources people ask about; anything else gets a spelling hint.
const KNOWN_SUBRESOURCES: &[&str] = &[
    "log",
    "exec",
    "attach",
    "portforward",
    "proxy",
    "status",
    "scale",
    "eviction",
    "binding",
    "ephemeralcontainers",
    "token",
    "approval",
    "finalize",
    "rollback",
];

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ParsedRequest {
    pub(crate) request: AccessRequest,
    pub(crate) hint: Option<QueryHint>,
}

/// Something the parser guessed or dropped, shown as a muted line under the input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum QueryHint {
    /// The resource is not a built-in kind, so the core group was assumed.
    AssumedCoreGroup,
    /// The kind is cluster-scoped, so the picked namespace was dropped.
    ClusterScoped,
    /// The subresource is not one of the common ones, so it may be misspelled.
    UnknownSubresource,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum QueryError {
    Empty,
    MissingTarget,
    EmptyPart,
    NameWithUrl,
    TooManyWords,
}

impl QueryError {
    pub(crate) fn text(self) -> &'static str {
        match self {
            Self::Empty => "Type a verb and a resource, for example get pods",
            Self::MissingTarget => "Add a resource or a /url after the verb",
            Self::EmptyPart => {
                "Resource, group, and subresource cannot be empty (for example deployments.apps, pods/log)"
            }
            Self::NameWithUrl => "A URL takes no object name",
            Self::TooManyWords => "Use: verb resource [name]",
        }
    }
}

/// `namespace` is the picked one; `None` asks about every namespace (cluster-wide grants).
pub(crate) fn parse_request(
    text: &str,
    namespace: Option<&str>,
) -> Result<ParsedRequest, QueryError> {
    let words: Vec<&str> = text.split_whitespace().collect();
    let (verb, target, name) = match words.as_slice() {
        [] => return Err(QueryError::Empty),
        [_] => return Err(QueryError::MissingTarget),
        [verb, target] => (*verb, *target, None),
        [verb, target, name] => (*verb, *target, Some(*name)),
        _ => return Err(QueryError::TooManyWords),
    };
    let verb = verb.to_lowercase();
    if target.starts_with('/') {
        if name.is_some() {
            return Err(QueryError::NameWithUrl);
        }
        let request = AccessRequest {
            verb,
            target: RequestTarget::NonResource {
                path: target.to_owned(),
            },
        };
        return Ok(ParsedRequest {
            request,
            hint: None,
        });
    }
    // Resources, groups, and subresources are lowercase in the API; object names are not.
    let target = target.to_lowercase();
    let (resource_part, subresource) = match target.split_once('/') {
        Some((resource, subresource)) => (resource, Some(subresource)),
        None => (target.as_str(), None),
    };
    let (resource, group) = match resource_part.split_once('.') {
        Some((resource, group)) => (resource, Some(group)),
        None => (resource_part, None),
    };
    let has_empty_part = resource.is_empty()
        || group.is_some_and(str::is_empty)
        || subresource.is_some_and(str::is_empty);
    if has_empty_part {
        return Err(QueryError::EmptyPart);
    }
    let resource = canonical_resource(resource);
    let resource = resource.as_str();
    // A typed group must also match the table entry, so `pods.metrics.k8s.io` is not the core
    // `pods`.
    let known = BUILT_IN_RESOURCES.iter().find(|(plural, known_group, _)| {
        *plural == resource && group.is_none_or(|group| group == *known_group)
    });
    let (group, is_namespaced, mut hint) = match (group, known) {
        (Some(group), known) => (
            group,
            known.is_none_or(|(_, _, is_namespaced)| *is_namespaced),
            None,
        ),
        (None, _) if resource == WILDCARD => (WILDCARD, true, None),
        (None, Some((_, group, is_namespaced))) => (*group, *is_namespaced, None),
        (None, None) => ("", true, Some(QueryHint::AssumedCoreGroup)),
    };
    // A kind the table does not know is assumed namespaced.
    let is_cluster_scoped = !is_namespaced && namespace.is_some();
    if is_cluster_scoped {
        hint = Some(QueryHint::ClusterScoped);
    }
    let is_unknown_subresource =
        subresource.is_some_and(|subresource| !KNOWN_SUBRESOURCES.contains(&subresource));
    if hint.is_none() && is_unknown_subresource {
        hint = Some(QueryHint::UnknownSubresource);
    }
    let request = AccessRequest {
        verb,
        target: RequestTarget::Resource(ResourceRequest {
            group: group.to_owned(),
            resource: resource.to_owned(),
            subresource: subresource.map(str::to_owned),
            name: name.map(str::to_owned),
            namespace: namespace.filter(|_| is_namespaced).map(str::to_owned),
        }),
    };
    Ok(ParsedRequest { request, hint })
}

/// The kubectl short name or the singular form of a built-in kind becomes its plural; any other
/// resource stays as typed.
fn canonical_resource(resource: &str) -> String {
    if let Some((_, plural)) = SHORT_NAMES.iter().find(|(short, _)| *short == resource) {
        return (*plural).to_owned();
    }
    BUILT_IN_RESOURCES
        .iter()
        .map(|(plural, ..)| *plural)
        .find(|plural| *plural == resource || singular_of(plural) == resource)
        .unwrap_or(resource)
        .to_owned()
}

/// `networkpolicies` is `networkpolicy`, `ingresses` is `ingress`, `pods` is `pod`.
fn singular_of(plural: &str) -> String {
    if let Some(stem) = plural.strip_suffix("ies") {
        format!("{stem}y")
    } else if let Some(stem) = plural.strip_suffix("sses") {
        format!("{stem}ss")
    } else {
        plural.strip_suffix('s').unwrap_or(plural).to_owned()
    }
}

/// The query a Role menu prefills: the first verb and resource of its first resource rule. The
/// group goes before a subresource, as the parser reads it: `deployments.apps/scale`.
pub(crate) fn who_can_prefill(rules: &[RbacRule]) -> Option<String> {
    let rule = rules
        .iter()
        .find(|rule| !rule.resources.is_empty() && !rule.verbs.is_empty())?;
    let verb = rule.verbs.first()?;
    let resource = rule.resources.first()?;
    let (kind, subresource) = match resource.split_once('/') {
        Some((kind, subresource)) => (kind, Some(subresource)),
        None => (resource.as_str(), None),
    };
    let mut target = kind.to_owned();
    if let Some(group) = rule.api_groups.first().filter(|group| !group.is_empty()) {
        target = format!("{target}.{group}");
    }
    if let Some(subresource) = subresource {
        target = format!("{target}/{subresource}");
    }
    Some(format!("{verb} {target}"))
}

#[cfg(test)]
#[path = "access_query_tests.rs"]
mod access_query_tests;
