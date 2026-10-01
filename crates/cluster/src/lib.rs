//! Cluster access for k8sBoard: kubeconfig loading, context selection, and
//! read-only access to Kubernetes clusters.

mod access_review;
mod connection;
mod kubeconfig;
mod metrics_api;
mod namespace;
mod node;
mod pod;
mod pod_log;
mod pod_status;
mod resource_watch;

pub use access_review::{AccessCheck, AccessDecision, AccessReport, AccessReview};
pub use connection::{ClusterConnection, ClusterError, ServerVersion};
pub use kubeconfig::{ContextOrigin, ContextSummary, Kubeconfig, KubeconfigError};
pub use metrics_api::MetricsApi;
pub use namespace::{NamespacePhase, NamespaceScope, NamespaceSummary};
pub use node::{NodeReadiness, NodeScheduling, NodeStatus, NodeSummary, NodeTaint};
pub use pod::{
    ContainerKind, ContainerState, ContainerSummary, PodCondition, PodController, PodSummary,
    ReadyCount, Termination,
};
pub use pod_log::{LogLine, LogRequest, LogSource, LogUpdate};
pub use pod_status::{InitStatus, PodStatus, StatusReason};
pub use resource_watch::WatchUpdate;
