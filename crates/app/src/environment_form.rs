//! What the Environments page of Settings edits, without any view code: the name rules, the
//! registry edits, and the dialog texts. Rows are addressed by their index in
//! `registry.environments`; cluster entries refer to a custom environment by its exact name.

use crate::cluster_form::FieldError;
use crate::cluster_registry::ClusterRegistry;
use crate::environment::{
    CustomEnvironment, EnvironmentColor, EnvironmentKey, EnvironmentTier, is_reserved,
    usable_environments,
};

const MAX_NAME_CHARS: usize = 16;

/// The trimmed name, or the message to show under the field. `own` is the index of the row being
/// renamed, which the uniqueness check skips.
pub(crate) fn validate_environment_name(
    text: &str,
    own: Option<usize>,
    custom: &[CustomEnvironment],
) -> Result<String, FieldError> {
    let name = text.trim();
    if name.is_empty() {
        return Err(FieldError("Enter a name.".into()));
    }
    // A reserved word is reported as such even when it is long (`Development · Local`).
    if is_reserved(name) {
        return Err(FieldError(
            format!("'{name}' is used by a built-in environment.").into(),
        ));
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return Err(FieldError("Use at most 16 characters.".into()));
    }
    if name.chars().any(char::is_control) {
        return Err(FieldError("Remove line breaks and tabs.".into()));
    }
    let lowered = name.to_lowercase();
    let is_taken = custom.iter().enumerate().any(|(index, environment)| {
        Some(index) != own && environment.name.trim().to_lowercase() == lowered
    });
    if is_taken {
        return Err(FieldError(
            format!("Another environment is already named '{name}'.").into(),
        ));
    }
    Ok(name.to_owned())
}

/// Whether the row at `at` takes part in resolution (a hand-edited reserved or repeated name does
/// not), so that references to its name ever pointed at it.
pub(crate) fn is_usable(custom: &[CustomEnvironment], at: usize) -> bool {
    custom
        .get(at)
        .is_some_and(|row| usable_environments(custom).any(|usable| std::ptr::eq(usable, row)))
}

/// Appends a new environment: the strictest tier and the first hue no built-in uses (decision 9).
pub(crate) fn add_environment(registry: &mut ClusterRegistry, name: String) {
    registry.environments.push(CustomEnvironment {
        name,
        color: EnvironmentColor::Purple,
        tier: EnvironmentTier::Production,
    });
}

/// Renames the environment at `at`; the cluster entries that referred to it follow.
pub(crate) fn rename_environment(registry: &mut ClusterRegistry, at: usize, to: String) {
    if at >= registry.environments.len() {
        return;
    }
    if is_usable(&registry.environments, at) {
        let old = EnvironmentKey::Custom(registry.environments[at].name.clone());
        for entry in &mut registry.clusters {
            if entry.environment.as_ref() == Some(&old) {
                entry.environment = Some(EnvironmentKey::Custom(to.clone()));
            }
        }
    }
    registry.environments[at].name = to;
}

pub(crate) fn edit_environment(
    registry: &mut ClusterRegistry,
    at: usize,
    edit: impl FnOnce(&mut CustomEnvironment),
) {
    if let Some(environment) = registry.environments.get_mut(at) {
        edit(environment);
    }
}

/// Removes the environment at `at`; the clusters that used it move to the built-in of its tier,
/// which has the same guardrails (decision 10).
pub(crate) fn delete_environment(registry: &mut ClusterRegistry, at: usize) {
    let Some(environment) = registry.environments.get(at) else {
        return;
    };
    if is_usable(&registry.environments, at) {
        let old = EnvironmentKey::Custom(environment.name.clone());
        let tier = environment.tier;
        for entry in &mut registry.clusters {
            if entry.environment.as_ref() == Some(&old) {
                entry.environment = Some(EnvironmentKey::BuiltIn(tier));
            }
        }
    }
    registry.environments.remove(at);
}

/// The cluster entries that refer to `name` exactly, loaded or not.
pub(crate) fn clusters_using(registry: &ClusterRegistry, name: &str) -> usize {
    registry
        .clusters
        .iter()
        .filter(|entry| {
            matches!(&entry.environment, Some(EnvironmentKey::Custom(stored)) if stored == name)
        })
        .count()
}

/// Whether moving from tier `from` to `to` loosens the rules (`Ord` on the tier is risk order).
pub(crate) fn is_weaker(from: EnvironmentTier, to: EnvironmentTier) -> bool {
    to < from
}

fn cluster_count_text(count: usize) -> String {
    match count {
        1 => "1 cluster".to_owned(),
        count => format!("{count} clusters"),
    }
}

/// The title and body of the delete confirm.
pub(crate) fn delete_dialog_text(
    environment: &CustomEnvironment,
    using: usize,
) -> (String, String) {
    let title = format!("Delete environment {}?", environment.name);
    let body = match using {
        0 => "No cluster uses it.".to_owned(),
        1 => format!(
            "1 cluster uses it and moves to {}, the built-in environment with the same confirm rules.",
            environment.tier.name()
        ),
        count => format!(
            "{count} clusters use it and move to {}, the built-in environment with the same confirm rules.",
            environment.tier.name()
        ),
    };
    (title, body)
}

/// The title and body of the confirm for a weaker tier on an environment that clusters use.
pub(crate) fn weaken_dialog_text(
    environment: &CustomEnvironment,
    to: EnvironmentTier,
    using: usize,
) -> (String, String) {
    let title = format!("Change {} to {} rules?", environment.name, to.name());
    let body = format!(
        "{} using {} will follow the {} rules instead of the {} rules.",
        cluster_count_text(using),
        environment.name,
        to.name(),
        environment.tier.name()
    );
    (title, body)
}

#[cfg(test)]
#[path = "environment_form_tests.rs"]
mod environment_form_tests;
