use k8s_openapi::apimachinery::pkg::apis::meta::v1::APIGroupList;

use crate::connection::{ClusterConnection, ClusterError};

pub(crate) const METRICS_GROUP: &str = "metrics.k8s.io";

/// Whether `metrics.k8s.io` is served. Discovery only; metric values are never read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MetricsApi {
    Available {
        group_version: String,
    },
    /// The group is registered but its group-version call fails, for example a 503
    /// from an `APIService` whose metrics-server is down.
    Unavailable {
        group_version: String,
        reason: String,
    },
    NotInstalled,
}

impl ClusterConnection {
    pub async fn metrics_api(&self) -> Result<MetricsApi, ClusterError> {
        let groups = self
            .run("listing API groups", self.client().list_api_groups())
            .await?;
        let Some(group_version) = metrics_group_version(&groups) else {
            return Ok(MetricsApi::NotInstalled);
        };
        let resources = self
            .run(
                "checking the metrics API",
                self.client().list_api_group_resources(&group_version),
            )
            .await;
        Ok(match resources {
            Ok(_) => MetricsApi::Available { group_version },
            Err(error) => MetricsApi::Unavailable {
                group_version,
                reason: error.to_string(),
            },
        })
    }
}

/// The preferred group version of `metrics.k8s.io`, else its first listed version.
fn metrics_group_version(groups: &APIGroupList) -> Option<String> {
    let group = groups
        .groups
        .iter()
        .find(|group| group.name == METRICS_GROUP)?;
    let version = group
        .preferred_version
        .as_ref()
        .or_else(|| group.versions.first())?;
    Some(version.group_version.clone())
}

#[cfg(test)]
mod tests {
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::{APIGroup, GroupVersionForDiscovery};

    use super::*;

    fn version(group_version: &str) -> GroupVersionForDiscovery {
        GroupVersionForDiscovery {
            group_version: group_version.to_owned(),
            version: group_version
                .rsplit('/')
                .next()
                .unwrap_or_default()
                .to_owned(),
        }
    }

    fn group(name: &str, preferred: Option<&str>, versions: &[&str]) -> APIGroup {
        APIGroup {
            name: name.to_owned(),
            preferred_version: preferred.map(version),
            versions: versions.iter().copied().map(version).collect(),
            ..Default::default()
        }
    }

    fn list(groups: Vec<APIGroup>) -> APIGroupList {
        APIGroupList { groups }
    }

    #[test]
    fn metrics_group_uses_preferred_version() {
        let groups = list(vec![
            group("apps", Some("apps/v1"), &["apps/v1"]),
            group(
                "metrics.k8s.io",
                Some("metrics.k8s.io/v1beta1"),
                &["metrics.k8s.io/v1", "metrics.k8s.io/v1beta1"],
            ),
        ]);
        assert_eq!(
            metrics_group_version(&groups).as_deref(),
            Some("metrics.k8s.io/v1beta1")
        );
    }

    #[test]
    fn metrics_group_falls_back_to_first_version() {
        let groups = list(vec![group(
            "metrics.k8s.io",
            None,
            &["metrics.k8s.io/v1", "metrics.k8s.io/v1beta1"],
        )]);
        assert_eq!(
            metrics_group_version(&groups).as_deref(),
            Some("metrics.k8s.io/v1")
        );
    }

    #[test]
    fn metrics_group_absent_returns_none() {
        let groups = list(vec![group("apps", Some("apps/v1"), &["apps/v1"])]);
        assert_eq!(metrics_group_version(&groups), None);
        assert_eq!(metrics_group_version(&list(Vec::new())), None);
    }
}
