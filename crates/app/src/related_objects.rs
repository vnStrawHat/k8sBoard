//! What a drawer needs beyond its own row: the objects related to it, watched while it is open.
//! Pure: the session starts the watch from the subject.

use crate::custom_kind::CustomKind;
use crate::kind_row::{KindObject, KindRow};
use crate::resource_kind::ResourceKind;
use crate::table_selection::ResourceKey;

/// The objects whose watch the open drawer needs. One watch runs at a time, like the object
/// events watch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RelatedSubject {
    /// The ReplicaSets of a Deployment, selected by its label selector.
    ReplicaSets {
        namespace: String,
        deployment: String,
        /// The selector terms joined with `,`.
        selector: String,
    },
    /// Every Job of the namespace; the drawer keeps those the CronJob owns.
    Jobs { namespace: String, cron_job: String },
    /// The value previews of one ConfigMap.
    ConfigMapValues { namespace: String, name: String },
    /// The FailedCreate events of the namespace; the drawer keeps those that name the quota.
    QuotaRejections { namespace: String, quota: String },
    /// The ResourceQuotas of one namespace, for the Namespace drawer.
    NamespaceQuotas { namespace: String },
    /// Every revision of one Helm release, from metadata only.
    HelmHistory { namespace: String, release: String },
    /// The masked spec and status of one custom object.
    CustomFields {
        kind: CustomKind,
        namespace: Option<String>,
        name: String,
    },
    /// Every Service of a Pod drawer's namespace; the drawer keeps those that select the pod.
    PodServices { namespace: String },
    /// Every Ingress of a Service drawer's namespace; the drawer keeps those that route to it.
    ServiceIngresses { namespace: String },
}

/// The related subject a drawer key names without a row: a Pod or a Service. These drawers also
/// open over Topology, where the explorer holds no such list, so the subject cannot wait for a row.
pub(crate) fn key_related_subject(key: &ResourceKey) -> Option<RelatedSubject> {
    match key {
        ResourceKey::Pod { namespace, .. } => Some(RelatedSubject::PodServices {
            namespace: namespace.clone(),
        }),
        ResourceKey::Kind {
            kind: ResourceKind::Services,
            namespace: Some(namespace),
            ..
        } => Some(RelatedSubject::ServiceIngresses {
            namespace: namespace.clone(),
        }),
        ResourceKey::Node { .. } | ResourceKey::Kind { .. } => None,
    }
}

/// The related subject of a row. A kind without related content, and a Deployment without a
/// selector (which would match every ReplicaSet), have none.
pub(crate) fn related_subject(kind: ResourceKind, row: &KindRow) -> Option<RelatedSubject> {
    if let ResourceKind::Custom(custom) = kind {
        // A cluster-scoped object has no namespace.
        return Some(RelatedSubject::CustomFields {
            kind: custom,
            namespace: row.namespace.clone(),
            name: row.name.clone(),
        });
    }
    if kind == ResourceKind::Namespaces {
        return Some(RelatedSubject::NamespaceQuotas {
            namespace: row.name.clone(),
        });
    }
    let namespace = row.namespace.clone()?;
    match (kind, &row.object) {
        (ResourceKind::Deployments, KindObject::Deployment(deployment)) => {
            if deployment.selector.is_empty() {
                return None;
            }
            Some(RelatedSubject::ReplicaSets {
                namespace,
                deployment: deployment.name.clone(),
                selector: deployment.selector.join(","),
            })
        }
        (ResourceKind::CronJobs, KindObject::CronJob(cron_job)) => Some(RelatedSubject::Jobs {
            namespace,
            cron_job: cron_job.name.clone(),
        }),
        (ResourceKind::ConfigMaps, KindObject::ConfigMap(config_map)) => {
            Some(RelatedSubject::ConfigMapValues {
                namespace,
                name: config_map.name.clone(),
            })
        }
        (ResourceKind::ResourceQuotas, KindObject::ResourceQuota(quota)) => {
            Some(RelatedSubject::QuotaRejections {
                namespace,
                quota: quota.name.clone(),
            })
        }
        (ResourceKind::HelmReleases, KindObject::HelmRelease(release)) => {
            Some(RelatedSubject::HelmHistory {
                namespace,
                release: release.name.clone(),
            })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use cluster::{CronJobSummary, CronSchedule, DeploymentSummary};

    use super::*;
    use crate::batch_rows::cron_job_row;
    use crate::workload_rows::deployment_row;

    fn deployment(selector: &[&str]) -> DeploymentSummary {
        DeploymentSummary {
            namespace: "team-a".to_owned(),
            name: "api".to_owned(),
            created_at: None,
            labels: Vec::new(),
            desired: 1,
            ready: 1,
            up_to_date: 1,
            available: 1,
            strategy: String::new(),
            max_surge: None,
            max_unavailable: None,
            progress_deadline_seconds: 600,
            is_paused: false,
            generation: 1,
            observed_generation: 1,
            revision: Some("7".to_owned()),
            selector: selector.iter().map(|term| (*term).to_owned()).collect(),
            containers: Vec::new(),
            conditions: Vec::new(),
            template_change: None,
        }
    }

    fn cron_job() -> CronJobSummary {
        CronJobSummary {
            namespace: "team-a".to_owned(),
            name: "reconcile".to_owned(),
            created_at: None,
            labels: Vec::new(),
            schedule: "*/5 * * * *".to_owned(),
            time_zone: None,
            timetable: CronSchedule::parse("*/5 * * * *", None),
            is_suspended: false,
            concurrency_policy: String::new(),
            starting_deadline_seconds: None,
            successful_history_limit: None,
            failed_history_limit: None,
            active_jobs: Vec::new(),
            last_schedule_at: None,
            last_success_at: None,
            containers: Vec::new(),
        }
    }

    #[test]
    fn related_subject_per_kind() {
        let deployment = deployment_row(&deployment(&["app=api", "tier in (a,b)"]));
        assert_eq!(
            related_subject(ResourceKind::Deployments, &deployment),
            Some(RelatedSubject::ReplicaSets {
                namespace: "team-a".to_owned(),
                deployment: "api".to_owned(),
                selector: "app=api,tier in (a,b)".to_owned(),
            })
        );
        let cron_job = cron_job_row(&cron_job());
        assert_eq!(
            related_subject(ResourceKind::CronJobs, &cron_job),
            Some(RelatedSubject::Jobs {
                namespace: "team-a".to_owned(),
                cron_job: "reconcile".to_owned(),
            })
        );
        // A row is read as the kind it was asked for.
        assert_eq!(related_subject(ResourceKind::Services, &cron_job), None);
        assert_eq!(related_subject(ResourceKind::Deployments, &cron_job), None);
        let quota = crate::policy_rows::resource_quota_row(&cluster::ResourceQuotaSummary {
            namespace: "team-a".to_owned(),
            name: "compute-quota".to_owned(),
            created_at: None,
            labels: Vec::new(),
            items: Vec::new(),
            scopes: Vec::new(),
        });
        assert_eq!(
            related_subject(ResourceKind::ResourceQuotas, &quota),
            Some(RelatedSubject::QuotaRejections {
                namespace: "team-a".to_owned(),
                quota: "compute-quota".to_owned(),
            })
        );
        // The Namespaces row is cluster-scoped: its name is the namespace.
        let namespace = crate::namespace_rows::namespace_row(&cluster::NamespaceSummary {
            name: "team-a".to_owned(),
            created_at: None,
            labels: Vec::new(),
            phase: cluster::NamespacePhase::Active,
            deleting_since: None,
            deletion_conditions: Vec::new(),
        });
        assert_eq!(
            related_subject(ResourceKind::Namespaces, &namespace),
            Some(RelatedSubject::NamespaceQuotas {
                namespace: "team-a".to_owned(),
            })
        );
    }

    #[test]
    fn deployment_without_selector_has_no_subject() {
        let row = deployment_row(&deployment(&[]));
        assert_eq!(related_subject(ResourceKind::Deployments, &row), None);
    }

    #[test]
    fn config_map_subject_names_the_object() {
        let row = crate::config_map_rows::config_map_row(&cluster::ConfigMapSummary {
            namespace: "team-a".to_owned(),
            name: "settings".to_owned(),
            created_at: None,
            labels: Vec::new(),
            keys: Vec::new(),
            is_immutable: false,
        });
        assert_eq!(
            related_subject(ResourceKind::ConfigMaps, &row),
            Some(RelatedSubject::ConfigMapValues {
                namespace: "team-a".to_owned(),
                name: "settings".to_owned(),
            })
        );
        assert_eq!(related_subject(ResourceKind::Services, &row), None);
    }

    #[test]
    fn helm_release_row_has_history_subject() {
        let row = crate::helm_rows::helm_release_row(&cluster::HelmReleaseSummary {
            namespace: "team-a".to_owned(),
            name: "api".to_owned(),
            revision: 3,
            status: cluster::HelmStatus::Deployed,
            chart: None,
            updated_at: None,
            description: None,
            deployed_revision: None,
        });
        assert_eq!(
            related_subject(ResourceKind::HelmReleases, &row),
            Some(RelatedSubject::HelmHistory {
                namespace: "team-a".to_owned(),
                release: "api".to_owned(),
            })
        );
        assert_eq!(related_subject(ResourceKind::Secrets, &row), None);
    }

    #[test]
    fn a_service_key_names_its_namespace_ingresses() {
        let service = ResourceKey::Kind {
            kind: ResourceKind::Services,
            namespace: Some("team-a".to_owned()),
            name: "api".to_owned(),
        };
        assert_eq!(
            key_related_subject(&service),
            Some(RelatedSubject::ServiceIngresses {
                namespace: "team-a".to_owned(),
            })
        );
        // Another kind in the same namespace has no key-derived subject.
        let config_map = ResourceKey::Kind {
            kind: ResourceKind::ConfigMaps,
            namespace: Some("team-a".to_owned()),
            name: "settings".to_owned(),
        };
        assert_eq!(key_related_subject(&config_map), None);
    }

    #[test]
    fn a_pod_key_names_its_namespace_services() {
        let pod = ResourceKey::Pod {
            namespace: "team-a".to_owned(),
            name: "api-1".to_owned(),
        };
        assert_eq!(
            key_related_subject(&pod),
            Some(RelatedSubject::PodServices {
                namespace: "team-a".to_owned(),
            })
        );
        // Two pods of a namespace share the one watch.
        let sibling = ResourceKey::Pod {
            namespace: "team-a".to_owned(),
            name: "api-2".to_owned(),
        };
        assert_eq!(key_related_subject(&pod), key_related_subject(&sibling));
        assert_eq!(
            key_related_subject(&ResourceKey::Node {
                name: "node-1".to_owned()
            }),
            None
        );
    }

    fn custom_kind(scope: cluster::ResourceScope) -> crate::custom_kind::CustomKind {
        let crd = cluster::CrdSummary {
            name: "widgets.x.io".to_owned(),
            group: "x.io".to_owned(),
            kind: "Widget".to_owned(),
            plural: "widgets".to_owned(),
            singular: "widget".to_owned(),
            scope,
            versions: vec![cluster::CrdVersion {
                name: "v1".to_owned(),
                is_served: true,
                is_storage: true,
                is_deprecated: false,
                deprecation_warning: None,
                printer_columns: Vec::new(),
                schema: cluster::SchemaOutline::default(),
            }],
            state: cluster::CrdState::Established,
            created_at: None,
        };
        crate::custom_kind::custom_kinds(
            &[crd],
            &mut crate::custom_kind::CustomKindCache::default(),
        )[0]
    }

    fn custom_row(
        kind: crate::custom_kind::CustomKind,
        namespace: Option<&str>,
    ) -> crate::kind_row::KindRow {
        crate::custom_rows::custom_object_row(
            kind,
            &cluster::CustomObjectSummary {
                namespace: namespace.map(str::to_owned),
                name: "w1".to_owned(),
                created_at: None,
                labels: Vec::new(),
                columns: Vec::new(),
                conditions: Vec::new(),
                phase: None,
            },
        )
    }

    #[test]
    fn custom_rows_have_a_fields_subject() {
        let kind = custom_kind(cluster::ResourceScope::Namespaced);
        let row = custom_row(kind, Some("shop"));
        assert_eq!(
            related_subject(ResourceKind::Custom(kind), &row),
            Some(RelatedSubject::CustomFields {
                kind,
                namespace: Some("shop".to_owned()),
                name: "w1".to_owned(),
            })
        );
    }

    #[test]
    fn fields_subject_carries_namespace_for_namespaced_kinds() {
        let namespaced = custom_kind(cluster::ResourceScope::Namespaced);
        let cluster_scoped = custom_kind(cluster::ResourceScope::Cluster);
        let subject = |kind, namespace| {
            related_subject(ResourceKind::Custom(kind), &custom_row(kind, namespace))
        };
        let Some(RelatedSubject::CustomFields { namespace, .. }) = subject(namespaced, Some("a"))
        else {
            panic!("expected a fields subject");
        };
        assert_eq!(namespace.as_deref(), Some("a"));
        // A cluster-scoped object has no namespace, and still gets a subject.
        let Some(RelatedSubject::CustomFields { namespace, .. }) = subject(cluster_scoped, None)
        else {
            panic!("expected a fields subject");
        };
        assert_eq!(namespace, None);
        // Two kinds with the same CRD but another scope are different subjects.
        assert_ne!(
            subject(namespaced, Some("a")),
            subject(cluster_scoped, None)
        );
    }
}
