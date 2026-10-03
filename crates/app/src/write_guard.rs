//! The pure rules of the write guard (spec 0030): the per-cluster read-only lock, the confirm
//! tiers, and the `ClusterGuard` that carries one cluster's inputs to the gate and the confirm
//! step. Nothing here reads a session or sends a request; the caller names the cluster.

use cluster::ContextSummary;
use serde::{Deserialize, Serialize};

use crate::cluster_registry::{ClusterProfile, ClusterRef};
use crate::cluster_session::AccessState;
use crate::environment::Environment;

/// Whether a session may offer changes. Starts from the profile and toggles for the session only.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WriteLock {
    Locked,
    Unlocked,
}

impl WriteLock {
    /// The state a session opens in: `profile.read_only`, which is on for Production unless the
    /// cluster's entry says otherwise.
    pub(crate) fn at_open(profile: &ClusterProfile) -> Self {
        if profile.read_only {
            Self::Locked
        } else {
            Self::Unlocked
        }
    }
}

/// How a cluster confirms a change. Stored as `type-name` or `click`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum ConfirmMode {
    TypeName,
    Click,
}

impl ConfirmMode {
    /// Production types the name; every other environment (an unknown one is Staging) clicks.
    pub(crate) fn for_environment(environment: Environment) -> Self {
        match environment {
            Environment::Production => Self::TypeName,
            Environment::Staging | Environment::Development | Environment::Local => Self::Click,
        }
    }
}

/// What an action can do to the cluster; only the dialog's button style follows from it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ActionRisk {
    Change,
    Destructive,
}

/// How the confirm dialog is confirmed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum DialogConfirm {
    Click,
    TypeName { expected: String },
}

/// Every guarded action opens a confirm dialog; this only picks how it is confirmed. `expected` is
/// the text to type in the `TypeName` tier: the cluster display name unless the action names its
/// object.
pub(crate) fn confirm_step(mode: ConfirmMode, risk: ActionRisk, expected: &str) -> DialogConfirm {
    match risk {
        ActionRisk::Change | ActionRisk::Destructive => match mode {
            ConfirmMode::TypeName => DialogConfirm::TypeName {
                expected: expected.to_owned(),
            },
            ConfirmMode::Click => DialogConfirm::Click,
        },
    }
}

/// Everything the gate and the confirm step need, taken from one cluster's session and profile.
/// Always built for the cluster of the row or action, never for an implicit current session.
pub(crate) struct ClusterGuard<'a> {
    pub(crate) cluster: ClusterRef,
    pub(crate) access: &'a AccessState,
    pub(crate) lock: WriteLock,
    /// Holds the display name, environment, and confirm mode.
    pub(crate) profile: ClusterProfile,
    /// Read by the audit lines (`lock_entry`).
    pub(crate) summary: ContextSummary,
}

impl<'a> ClusterGuard<'a> {
    pub(crate) fn new(
        access: &'a AccessState,
        lock: WriteLock,
        profile: ClusterProfile,
        summary: ContextSummary,
    ) -> Self {
        Self {
            cluster: ClusterRef::of(&summary),
            access,
            lock,
            profile,
            summary,
        }
    }

    pub(crate) fn display_name(&self) -> &str {
        &self.profile.display_name
    }
}

/// A guard for the tests of the modules that read one: a cluster `name` in `environment`.
#[cfg(test)]
pub(crate) fn test_guard<'a>(
    access: &'a AccessState,
    lock: WriteLock,
    name: &str,
    environment: Environment,
) -> ClusterGuard<'a> {
    let profile = ClusterProfile {
        display_name: name.to_owned(),
        environment,
        default_namespace: None,
        read_only: lock == WriteLock::Locked,
        confirm: ConfirmMode::for_environment(environment),
    };
    let summary = ContextSummary {
        name: name.to_owned(),
        cluster: format!("{name}-cluster"),
        user: Some("tester".to_owned()),
        namespace: None,
        source: std::path::PathBuf::from("test.yaml"),
    };
    ClusterGuard::new(access, lock, profile, summary)
}

#[cfg(test)]
#[path = "write_guard_tests.rs"]
mod write_guard_tests;
