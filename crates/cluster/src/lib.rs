//! Cluster access for k8sBoard: kubeconfig loading, context selection, and
//! access to Kubernetes clusters. Every write goes through `object_write`; the exec and
//! port-forward connections live in `pod_shell` and `port_forward`.

// The fake API server builds a connection whose write policy the caller picks, so it must never
// reach a release build, where writes are allowed.
#[cfg(all(feature = "test-support", not(debug_assertions)))]
compile_error!("the `test-support` feature is for debug test builds only");

mod access_review;
mod autoscaler;
mod cadvisor_text;
mod certificate;
mod certificate_renewal;
mod column_path;
mod config_map;
mod config_values;
mod connection;
mod container_spec;
mod cron_job;
mod cron_schedule;
mod custom_object;
mod custom_resource_definition;
mod daemon_set;
mod debug_pod_bodies;
mod debug_shell;
mod deployment;
mod disruption_budget;
mod dns_name;
mod edit_placeholders;
mod edit_preview;
mod endpoint_slice;
mod event;
#[cfg(any(test, feature = "test-support"))]
#[doc(hidden)]
pub mod fake_api;
mod helm_release;
mod helm_release_detail;
mod helm_values_diff;
mod ingress;
mod job;
mod kubeconfig;
mod kubelet_stats;
mod limit_range;
mod metrics_api;
mod metrics_query;
mod metrics_source;
mod namespace;
mod network_policy;
mod network_policy_traffic;
mod node;
mod node_maintenance_bodies;
mod node_shell_leftovers;
mod object_count;
mod object_create;
mod object_edit;
mod object_metadata;
mod object_names;
mod object_write;
mod object_yaml;
mod pending_pod;
mod persistent_volume;
mod persistent_volume_claim;
mod pinned_volume;
mod pod;
mod pod_log;
mod pod_shell;
mod pod_status;
mod port_forward;
mod port_forward_target;
mod promql;
mod proxy;
mod quantity;
mod quota_demand;
mod rbac_evaluation;
mod rbac_snapshot;
mod reason_text;
mod registry_secret;
mod replica_set;
mod resource_metrics;
mod resource_quota;
mod resource_watch;
mod role;
mod role_binding;
mod secret;
mod selector;
mod service;
mod service_account;
mod stateful_set;
mod storage_class;
mod traffic;
mod traffic_metrics;
mod volume_write_bodies;
mod workload;
mod workload_write_bodies;

pub use access_review::{
    AccessCheck, AccessDecision, AccessReport, AccessReview, NamespaceAccess, RulesReview,
};
pub use autoscaler::{HorizontalPodAutoscalerSummary, HpaMetric, MetricSource, MetricValue};
pub use cadvisor_text::{ContainerDiskIo, DiskIoCounters, DiskIoSample};
pub use certificate::{
    CertificateInfo, CertificateIssue, KeyCheck, TlsPairInfo, TlsPairIssue, check_tls_pair,
};
pub use config_map::{
    ConfigMapKey, ConfigMapSummary, ConfigMapValue, ConfigMapValues, ValuePreview,
};
pub use config_values::{
    BaseNotes, DataField, DataFieldChange, HELM_MANAGED_WARNING, KeyChange, KeyContent,
    MAX_INLINE_VALUE, NewValue, ValueKey, ValuesBase, ValuesBaseError, ValuesEdit, ValuesEditError,
    is_helm_managed, is_valid_key_name, terms_are_helm_managed,
};
pub use connection::{ClusterConnection, ClusterError, ServerVersion};
pub use container_spec::{
    ContainerProbes, ContainerResource, EnvEntry, EnvFromEntry, EnvFromSource, EnvSource,
    MountEntry, ProbeAction, ProbeSummary, VolumeSource,
};
pub use cron_job::CronJobSummary;
pub use cron_schedule::{CronSchedule, ScheduleError};
pub use custom_object::{
    ColumnValue, CustomObjectFields, CustomObjectSummary, FieldEntry, FieldList, FieldValue,
    ObjectCondition,
};
pub use custom_resource_definition::{
    ColumnType, CrdState, CrdSummary, CrdVersion, CustomResourceType, PrinterColumn, ResourceScope,
    SchemaField, SchemaOutline,
};
pub use daemon_set::DaemonSetSummary;
pub use debug_pod_bodies::{
    DEFAULT_DEBUG_IMAGE, debug_container_name, is_valid_debug_image, node_shell_pod_name,
    random_suffix, run_id,
};
pub use debug_shell::{AttachPermit, AttachRequest, AttachWait};
pub use deployment::{DeploymentSummary, FieldWriter};
pub use disruption_budget::{BlockCause, DisruptionState, PodDisruptionBudgetSummary};
pub use edit_preview::{EditCheck, EditPreview, FieldChange, FieldPath};
pub use endpoint_slice::{EndpointPort, EndpointSliceSummary, EndpointSummary};
pub use event::{
    ChangeEventKind, EVENT_LIMIT, EventFilter, EventSummary, EventType, InvolvedObject,
};
pub use helm_release::RELEASE_TYPE as HELM_RELEASE_SECRET_TYPE;
pub use helm_release::{HelmChart, HelmReleaseSummary, HelmRevision, HelmRevisionRef, HelmStatus};
pub use helm_release_detail::{HelmReleaseDetail, HelmRevealed, HelmText};
pub use helm_values_diff::{HelmValuesDiff, ValueChange, ValueVisibility};
pub use ingress::{IngressPath, IngressSummary, IngressTls};
pub use job::{JobStatus, JobSummary};
pub use kubeconfig::{
    AuthKind, ConnectionInfo, ContextOrigin, ContextSummary, EntryNames, Kubeconfig,
    KubeconfigError, LoadedKubeconfig,
};
pub use kubelet_stats::{
    KubeletSummary, KubeletTargets, NetworkCounters, NodeKubeletStats, PodKubeletStats, PvcUsage,
};
pub use limit_range::{LimitRangeLimit, LimitRangeSummary};
pub use metrics_api::MetricsApi;
pub use metrics_query::{MetricsError, SourceCheck, UsageSeries};
pub use metrics_source::{
    MetricsCandidate, MetricsFlavor, MetricsScheme, MetricsSource, MetricsSourceError,
    MetricsSourceFields, metrics_candidates,
};
pub use namespace::{NamespaceDeletionCondition, NamespacePhase, NamespaceScope, NamespaceSummary};
pub use network_policy::{
    NetworkPolicySummary, PolicyDirection, PolicyPeer, PolicyPort, PolicyRule,
};
pub use network_policy_traffic::{
    DirectionVerdict, PolicyInputs, RequestPort, RuleRef, TrafficDestination, TrafficEndpoint,
    TrafficError, TrafficRequest, TrafficSource, TrafficVerdict, evaluate_traffic,
};
pub use node::{
    ConditionStatus, NodeAddress, NodeCondition, NodeEdit, NodeReadiness, NodeResource,
    NodeScheduling, NodeStatus, NodeSummary, NodeSystemInfo, NodeTaint,
};
pub use node_maintenance_bodies::{GracePeriod, LabelChange};
pub use node_shell_leftovers::{LeftoverPhase, NodeShellLeftover};
pub use object_create::{DraftError, DraftFix, DraftWarning, ObjectDraft};
pub use object_edit::{EditBase, EditError, ObjectEdit, Rebased, format_yaml, rebase};
pub use object_metadata::ObjectMetadata;
pub use object_names::{NameList, ObjectName};
pub use object_write::{
    ChangedField, DeletePropagation, WriteEffect, WriteError, WriteMode, WriteOperation,
    WriteOutcome, WritePolicy, WriteRequest,
};
pub use object_yaml::{
    EnvValues, ObjectIdentity, ObjectKind, ObjectRef, ObjectYaml, is_secret_key,
};
pub use pending_pod::PendingPod;
pub use persistent_volume::{ClaimRef, PersistentVolumeSummary, VolumeBackend};
pub use persistent_volume_claim::PersistentVolumeClaimSummary;
pub use pod::{
    ContainerKind, ContainerState, ContainerSummary, ContainerTerminal, DrainPod, PodCondition,
    PodSummary, ReadyCount, Termination,
};
pub use pod_log::{LogLine, LogRequest, LogSource, LogUpdate};
pub use pod_shell::{
    ExecPermit, GridSize, ShellCommand, ShellExit, ShellInput, ShellRequest, ShellUpdate,
};
pub use pod_status::{InitStatus, PodStatus, StatusReason};
pub use port_forward::{
    ForwardControl, ForwardError, ForwardEvent, ForwardRequest, ForwardTarget, ForwardTraffic,
    ForwardUpdate, LocalPort, PortForwardPermit, default_local_port, free_local_port,
};
pub use promql::{
    MAX_POINTS, RANGE_STEPS, RangeError, RangeSpec, UsageMetric, UsageTarget, WorkloadKind,
};
pub use proxy::{ProxyChoice, ProxyUrl, ProxyUrlError};
pub use quantity::{ByteAmount, CpuAmount, quantity_ratio};
pub use quota_demand::{
    DemandChange, QuotaCheck, QuotaResource, QuotaShortfall, WorkloadDemand, quota_check,
    scale_demand,
};
pub use rbac_evaluation::{
    AccessRequest, EffectiveRule, Grant, GrantNames, Identity, RequestTarget, ResourceRequest,
};
pub use rbac_snapshot::{NamespaceCoverage, RbacCoverage, RbacSnapshot};
pub use registry_secret::docker_config_json;
pub use replica_set::ReplicaSetSummary;
pub use resource_metrics::{
    ContainerMetrics, METRICS_INTERVAL, NodeMetrics, PodMetrics, ResourceUsage,
};
pub use resource_quota::{QuotaItem, ResourceQuotaSummary};
pub use resource_watch::WatchUpdate;
pub use role::{RbacRule, RoleSummary};
pub use role_binding::{
    BindingSummary, BroadGroup, RoleKind, RoleRef, Subject, SubjectKind, SubjectMatch,
};
pub use secret::{SecretDetails, SecretKey, SecretSummary, SecretValue};
pub use selector::Selector;
pub use service::{ServicePortSummary, ServiceSummary};
pub use service_account::{CloudIdentity, CloudProvider, ServiceAccountSummary};
pub use stateful_set::{ClaimTemplate, StatefulSetSummary};
pub use storage_class::{StorageClassSummary, StorageParameter};
pub use traffic::TrafficCounter;
pub use traffic_metrics::{
    TrafficEnd, TrafficMetricSource, TrafficRate, TrafficReading, TrafficSourceKind,
};
pub use volume_write_bodies::ReclaimPolicy;
pub use workload::{
    AnnotationTerms, ContainerPort, ControllerRef, TemplateContainer, WorkloadCondition,
};
