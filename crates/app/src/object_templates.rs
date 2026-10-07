//! The starting text of a new object (spec 0042): one YAML template per creatable kind. The
//! template is the form: the user edits it and the server checks it.

use cluster::ObjectKind;

/// The namespace a template names when the scope has none (a scope of `All`).
const DEFAULT_NAMESPACE: &str = "default";

/// The starting text of a new object of `kind`; `None` for a kind that is not creatable.
/// `namespace` is ignored for a Namespace.
pub(crate) fn template_text(kind: ObjectKind, namespace: &str) -> Option<String> {
    let text = match kind {
        ObjectKind::Namespace => {
            "apiVersion: v1\nkind: Namespace\nmetadata:\n  name: new-namespace\n  labels: {}\n"
                .to_owned()
        }
        ObjectKind::ConfigMap => format!(
            "apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: new-config\n  namespace: {namespace}\ndata:\n  KEY: value\n"
        ),
        ObjectKind::ResourceQuota => format!(
            "apiVersion: v1\nkind: ResourceQuota\nmetadata:\n  name: compute-quota\n  namespace: {namespace}\nspec:\n  hard:\n    requests.cpu: \"4\"\n    requests.memory: 8Gi\n    limits.cpu: \"8\"\n    limits.memory: 16Gi\n    pods: \"20\"\n"
        ),
        ObjectKind::PodDisruptionBudget => format!(
            "apiVersion: policy/v1\nkind: PodDisruptionBudget\nmetadata:\n  name: new-pdb\n  namespace: {namespace}\nspec:\n  minAvailable: 1\n  selector:\n    matchLabels:\n      app: my-app\n"
        ),
        ObjectKind::Secret => format!(
            "apiVersion: v1
kind: Secret
metadata:
  name: new-secret
  namespace: {namespace}
type: Opaque
stringData:
  KEY: value
"
        ),
        ObjectKind::RoleBinding => format!(
            "apiVersion: rbac.authorization.k8s.io/v1\nkind: RoleBinding\nmetadata:\n  name: new-binding\n  namespace: {namespace}\nroleRef:\n  apiGroup: rbac.authorization.k8s.io\n  kind: ClusterRole\n  name: view\nsubjects:\n  - kind: ServiceAccount\n    name: default\n    namespace: {namespace}\n"
        ),
        _ => return None,
    };
    Some(text)
}

/// The namespace of a template: the first of the scope (`Named`, or the first of the sorted
/// `Several`), else `default` (decision 8).
pub(crate) fn template_namespace(scope: &cluster::NamespaceScope) -> &str {
    scope
        .namespaces()
        .first()
        .map_or(DEFAULT_NAMESPACE, String::as_str)
}

#[cfg(test)]
mod tests {
    use cluster::{NamespaceScope, ObjectDraft};

    use super::*;

    #[test]
    fn every_template_is_a_valid_draft() {
        let creatable: Vec<_> = ObjectKind::ALL
            .into_iter()
            .filter(|kind| kind.is_creatable())
            .collect();
        assert_eq!(creatable.len(), 6);
        for kind in creatable {
            let text = template_text(kind, "payments").expect("a template");
            let draft = ObjectDraft::new(kind, &text);
            assert!(draft.is_ok(), "{kind:?}: {:?}", draft.err());
        }
    }

    #[test]
    fn a_kind_that_is_not_creatable_has_no_template() {
        assert_eq!(template_text(ObjectKind::Service, "payments"), None);
        assert_eq!(template_text(ObjectKind::Deployment, "payments"), None);
    }

    #[test]
    fn a_namespace_template_has_no_namespace_field() {
        let text = template_text(ObjectKind::Namespace, "payments").expect("a template");
        assert!(!text.contains("payments"), "{text}");
        let config_map = template_text(ObjectKind::ConfigMap, "payments").expect("a template");
        assert!(config_map.contains("namespace: payments"), "{config_map}");
    }

    #[test]
    fn template_uses_first_scoped_namespace_else_default() {
        assert_eq!(
            template_namespace(&NamespaceScope::Named("payments".to_owned())),
            "payments"
        );
        assert_eq!(
            template_namespace(&NamespaceScope::Several(vec![
                "a".to_owned(),
                "b".to_owned()
            ])),
            "a"
        );
        assert_eq!(template_namespace(&NamespaceScope::All), "default");
    }
}
