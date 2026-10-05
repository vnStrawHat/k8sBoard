# 0053 · Model

[Back to index](README.md) · Module: `environment.rs` (types, palette, badge), `cluster_registry.rs` (persistence, resolution), `cluster_form.rs` (groups).

## Types (`environment.rs`)

```rust
/// The four built-ins. Every environment behaves as one of them (its tier).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum EnvironmentTier { Local, Development, Staging, Production } // today's enum, renamed
impl EnvironmentTier {
    pub(crate) const ALL: [Self; 4];           // Production, Staging, Development, Local (display order)
    pub(crate) fn name(self) -> &'static str;  // unchanged
    pub(crate) fn badge(self) -> &'static str; // unchanged
    pub(crate) fn color(self) -> EnvironmentColor; // Red, Amber, Blue, Gray (was ClusterColor::of)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum EnvironmentColor { Red, Amber, Green, Blue, Teal, Purple, Gray } // was ClusterColor
impl EnvironmentColor {
    pub(crate) const ALL: [Self; 7];          // swatch order as declared (step 3)
    pub(crate) fn name(self) -> &'static str; // swatch tooltip (step 3)
}
/// The only place an environment colour touches the theme (was `cluster_color`).
pub(crate) fn palette_color(color: EnvironmentColor, cx: &App) -> Hsla;
// Red danger · Amber warning · Green success · Blue info · Teal cyan · Purple magenta · Gray muted_foreground

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct CustomEnvironment {
    pub(crate) name: String,
    pub(crate) color: EnvironmentColor,
    pub(crate) tier: EnvironmentTier,
}

/// What a registry entry stores. Untagged: `"production"` is a built-in, any other string a custom name.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub(crate) enum EnvironmentKey { BuiltIn(EnvironmentTier), Custom(String) }

/// An environment as a cluster wears it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Environment { BuiltIn(EnvironmentTier), Custom(CustomEnvironment) }
impl Environment {
    pub(crate) const PRODUCTION: Self = Self::BuiltIn(EnvironmentTier::Production); // + STAGING, DEVELOPMENT, LOCAL
    pub(crate) fn tier(&self) -> EnvironmentTier;
    pub(crate) fn name(&self) -> &str;
    pub(crate) fn badge(&self) -> Cow<'static, str>;  // built-in: Borrowed; custom: Owned(name.to_uppercase())
    pub(crate) fn color(&self) -> EnvironmentColor;
    pub(crate) fn key(&self) -> EnvironmentKey;
}

/// Built-in names and badges, `BUILT_IN_GROUP_TITLES`, and `auto`, compared trimmed and lower-case
/// (decision 6). Shared by the form and resolution.
pub(crate) fn is_reserved(name: &str) -> bool;
/// The custom environments resolution may use, in list order: a reserved name, or one repeating an
/// earlier custom name ignoring case, is skipped. Groups, the dropdown, and the Safety table read this too.
pub(crate) fn usable_environments(custom: &[CustomEnvironment]) -> impl Iterator<Item = &CustomEnvironment>;
/// Exact (case-sensitive) match of a `Custom` key among `usable_environments`; none → Production (decision 5).
pub(crate) fn resolve_environment(key: &EnvironmentKey, custom: &[CustomEnvironment]) -> Environment;
pub(crate) fn guess_environment(context: &str, cluster: &str) -> EnvironmentTier; // rules unchanged
pub(crate) fn environment_color(environment: &Environment, cx: &App) -> Hsla;     // palette_color(environment.color())
pub(crate) fn environment_badge(environment: &Environment, cx: &App) -> impl IntoElement; // unchanged look
```

Constants (`Environment::PRODUCTION` …) replace `Environment::Production` in fixtures, so most test edits are a mechanical rename. No dead code: a const whose only callers are tests is `#[cfg(test)]`, and one used only by screenshot fixtures follows the gate of its caller (the `test_guard` precedent).

## Persistence (`cluster_registry.rs`)

```rust
pub(crate) struct ClusterRegistry {
    /// Custom environments, in creation order = display order (0053).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) environments: Vec<CustomEnvironment>,
    /* kubeconfigs, kubeconfig_folders, clusters, last_used, last_used_stamp: unchanged */
}
pub(crate) struct ClusterEntry { /* … */ pub(crate) environment: Option<EnvironmentKey>, /* color: removed */ }
pub(crate) struct ClusterProfile { /* … */ pub(crate) environment: Environment, /* color: removed */ }
```

```json
"registry": {
  "environments": [ { "name": "QA", "color": "purple", "tier": "staging" } ],
  "clusters": [ { "kubeconfig": "D:/kube/qa.yaml", "context": "qa-1", "environment": "QA" } ]
}
```

An unknown `color` or `tier` string fails the parse, as an unknown `confirm` does today (reset with backup). No leniency is added: only a hand edit can produce it.

## Resolution (`ClusterRegistry::profile`)

| Field | Rule |
|---|---|
| `environment` | `entry.environment` → `resolve_environment(key, &self.environments)`, else `Environment::BuiltIn(guess_environment(..))` |
| `is_environment_set` | `entry.environment.is_some()` (a custom one is always set) |
| `read_only` | stored, else `environment.tier() == Production` |
| `confirm` | stored, else `ConfirmMode::for_tier(environment.tier())` |
| `allow_node_shell` | stored, else `match environment.tier()` exactly as today |

`write_guard.rs`: `ConfirmMode::for_tier(tier: EnvironmentTier)` replaces `for_environment`; the match is unchanged. `test_guard(.., environment: Environment)` builds `confirm` with `for_tier(environment.tier())`.

## Groups (`cluster_form.rs`)

```rust
pub(crate) struct ClusterGroup { pub(crate) title: SharedString, pub(crate) rows: Vec<ClusterRow> }
fn group_index(environment: &Environment, custom: &[CustomEnvironment]) -> usize;
// BuiltIn: Production 0, Staging 1, Development | Local 2. Custom: 3 + its position in `usable_environments`.
```

`cluster_groups` builds `BUILT_IN_GROUP_TITLES` (moved from `cluster_form.rs` to `environment.rs`, so `is_reserved` reads it) plus one title per usable custom environment, fills by `group_index`, and drops empty groups (unchanged). `ClusterRow.guessed: EnvironmentTier`. `search_text(label, context, environment: &Environment, file)` reads `environment.badge()`.

## Where a custom environment shows

| Surface | File | How |
|---|---|---|
| Badge | title bar, switcher, Clusters list, palette, confirm, drain, port-forward dialogs and page | `environment_badge(&profile.environment, cx)` |
| Unlocked dashed frame | `title_bar.rs` | `environment_color(&open.profile.environment, cx)` |
| Group headers | Settings › Clusters, switcher | `ClusterGroup.title` |
| Environment dropdown | `clusters_page.rs` | Auto, 4 built-ins, separator, `"{name} · like {tier name}"` per usable custom (only when any exist). Label: the resolved name, except a dangling `Custom(key)` shows the stored key string with nothing checked (behaviour still Production) |
| Safety tier table | `settings_window.rs` | `tier_rows(&registry.environments)`: each row lists built-ins then usable customs whose `for_tier(tier)` matches |
| Import preview | `clusters_page_import.rs` | `ContextPreview.environment: EnvironmentTier` (guess only), drawn as `Environment::BuiltIn(..)` |

## Reference matching

A `Custom(name)` key matches a custom environment by **exact, case-sensitive** string equality, in `resolve_environment`, `rename_environment`, `delete_environment`, and `clusters_using` alike. The case-insensitive comparison is only for validation and `usable_environments` (uniqueness and reserved words).
