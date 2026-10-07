//! What is wrong behind the rules of an Ingress: a backend Service that does not exist, a port the
//! Service does not expose, a Service with no ready pod. These are the causes of a 502 or 503 that
//! the address and the certificate say nothing about. Pure: the drawer passes the Services of the
//! namespace and the pods list.

use cluster::{IngressPath, IngressSummary, PodSummary, ServiceSummary};

use crate::kind_join::matching_pods;

/// The lists the checks read, once both have loaded.
#[derive(Clone, Copy)]
pub(crate) struct IngressBackends<'a> {
    pub(crate) services: &'a [ServiceSummary],
    pub(crate) pods: &'a [PodSummary],
}

/// One problem line for each rule (and the default backend) whose Service is missing, lacks the
/// port, or has no ready pod, in rule order, with the Service the first existing problem is about.
pub(crate) fn backend_problems<'a>(
    ingress: &IngressSummary,
    backends: &IngressBackends<'a>,
) -> (Vec<String>, Option<&'a ServiceSummary>) {
    let rules = ingress
        .rules
        .iter()
        .filter_map(|rule| Some((rule_label(rule), rule.service.as_deref()?, &rule.backend)));
    let default = ingress
        .default_service
        .as_deref()
        .zip(ingress.default_backend.as_ref())
        .map(|(service, backend)| ("Default backend".to_owned(), service, backend));
    let mut lines = Vec::new();
    let mut service_of_problem = None;
    for (label, name, backend) in rules.chain(default) {
        let service = backends
            .services
            .iter()
            .find(|service| service.namespace == ingress.namespace && service.name == name);
        let Some(problem) = service_problem(ingress, name, backend, service, backends.pods) else {
            continue;
        };
        lines.push(format!("{label} → {backend}: {problem}"));
        service_of_problem = service_of_problem.or(service);
    }
    (lines, service_of_problem)
}

/// `Rule shop.lab/v2`; a rule without a path is the root.
fn rule_label(rule: &IngressPath) -> String {
    let host = rule.host.as_deref().unwrap_or_default();
    format!("Rule {host}{}", rule.path.as_deref().unwrap_or("/"))
}

fn service_problem(
    ingress: &IngressSummary,
    name: &str,
    backend: &str,
    service: Option<&ServiceSummary>,
    pods: &[PodSummary],
) -> Option<String> {
    let Some(service) = service else {
        return Some(format!(
            "Service {name} does not exist in {}",
            ingress.namespace
        ));
    };
    // The backend text is `name:port`; a port-less backend has no port to check.
    let port = backend.strip_prefix(name)?.strip_prefix(':');
    if let Some(port) = port
        && service.service_type != "ExternalName"
        && !has_port(service, port)
    {
        return Some(format!(
            "Service {name} has no port {port} (has {})",
            ports_text(service)
        ));
    }
    if service.selector.is_empty() || service.service_type == "ExternalName" {
        return None;
    }
    let matching = matching_pods(service, pods);
    let ready = matching.iter().filter(|pod| is_ready(pod)).count();
    if ready > 0 {
        return None;
    }
    let why = if matching.is_empty() {
        format!("no pod matches {}", service.selector.join(","))
    } else {
        format!("{} pods match, none ready", matching.len())
    };
    Some(format!("Service {name} has 0 ready endpoints ({why})"))
}

/// A port number matches a Service port; a port name matches a Service port name.
fn has_port(service: &ServiceSummary, port: &str) -> bool {
    service
        .ports
        .iter()
        .any(|candidate| match port.parse::<u16>() {
            Ok(number) => candidate.port == number,
            Err(_) => candidate.name.as_deref() == Some(port),
        })
}

/// `80, 8080 (metrics)`; `no ports` for a Service that exposes none.
fn ports_text(service: &ServiceSummary) -> String {
    if service.ports.is_empty() {
        return "no ports".to_owned();
    }
    service
        .ports
        .iter()
        .map(|port| match &port.name {
            Some(name) => format!("{} ({name})", port.port),
            None => port.port.to_string(),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn is_ready(pod: &PodSummary) -> bool {
    pod.conditions
        .iter()
        .any(|condition| condition.name == "Ready" && condition.is_true)
}

#[cfg(test)]
#[path = "ingress_backends_tests.rs"]
mod ingress_backends_tests;
