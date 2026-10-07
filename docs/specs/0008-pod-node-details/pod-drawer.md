# 0008 · App: pod drawer details

[Back to index](README.md) · Step 2 · Modules: `pod_drawer.rs`, `container_detail.rs` (new), `pod_diagnosis.rs` (new), `drawer.rs`, `kind_drawer.rs`, `table_selection.rs`, `event_rows.rs`, `app_shell.rs`, `resource_actions.rs`, `pod_table.rs`. Builds on 0007 (`DrawerTab`, Pod tabs Overview · Containers · YAML · Events).

## Shared pieces (moved, not rewritten)

| Item | From → to | Note |
|---|---|---|
| `link_value` → `pub(crate) fn link_text(id, text, target: ResourceKey, cx: &Context<AppShell>)` | `kind_drawer.rs` → `drawer.rs` | click calls `AppShell::reveal(target)` |
| `port_row` → `pub(crate)` | `kind_drawer.rs` → `drawer.rs` | disabled Forward with `port_forward_reason` |
| `chips` → `pub(crate)` | `kind_drawer.rs` → `drawer.rs` | node labels use it |
| `ResourceKey::of_object(kind: &str, namespace: Option<&str>, name: &str) -> Option<ResourceKey>` | body of `event_rows::object_key` → `table_selection.rs` | `object_key` delegates to it; `Pod`, `Node`, and every `ResourceKind` except Events |

## Overview tab

1. **WHY box** first, when `pod_diagnosis(pod, events, now)` is Some ([pod-diagnosis.md](pod-diagnosis.md)); `events` = `live.events_of(&subject)` items when Ready. `gpui_kit::component::Alert::error` (Bad) or `Alert::warning` (Warn), `.title("WHY · CONTAINER {name}")` or `"WHY · POD"`, message = text. `Alert` has no children, so for a container the `Open container {name} →` link (theme `link` color, calls `open_container(name)`) is a **sibling** right under it, both in one `v_flex`. No hardcoded colors.
   The container name is quoted in the title and the link (`WHY · CONTAINER "report"`, `Open container "report" →`). An Unschedulable message is shown one sentence per line with one `• N reason` bullet per counted reason (`PodDiagnosis::display_text`); the issue rules keep the one-sentence `text`. The kind drawer's Age fields (Started, Created) read `2026-10-06 17:26 +07 (49m ago)` in the system zone (`format_local_time`).
2. **Pod** rows: Node → `link_text` to `ResourceKey::Node` ("—" when unscheduled); Pod IP; QoS class; Service account (plain text; the ServiceAccounts screen is 0015); Controlled by → `link_text` to `of_object(kind, Some(ns), name)`, plain `{kind}/{name}` when `None`.
3. **Conditions** chips as today; a chip that is not true gets a tooltip `{reason}: {message}` (either part may be missing; no tooltip when both are).
4. **Containers** summary unchanged. Since 0007 the Containers arm must wrap its content in `DrawerBody::Scrolling(..)`. UX fix: each container row also shows "N restarts" in the warning tone (hidden at 0) and, once it ran before, a muted "Last exit: {last state text}" line; the Pods Restarts cell is warning-toned above 0.

## Containers tab

- Master list unchanged. Detail header: name, state label, kind tag, then muted `next retry in {d}` when `next_retry` is Some (`d` in Go style via `age::format_countdown`: `40s`, `3m20s`, `1h0m5s`).
- Sub-tab bar under the header: kit `TabBar::new("container-tabs").segmented().xsmall()` with `Info`, `Env {env.len() + env_from.len()}`, `Mounts {mounts.len()}`. Logs (0019) and Monitor (0010) are not shown.

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ContainerTab { Info, Env, Mounts }  // drawer.rs
pub(crate) struct DrawerState { /* … */ pub(crate) container_tab: ContainerTab }
impl AppShell { pub(crate) fn set_container_tab(&mut self, tab: ContainerTab, cx: &mut Context<Self>); }
```

`container_tab` survives container and subject changes (like `tab`); `show_screen` resets it to `Info`. The ui-verifier therefore expects a newly selected pod to open on the sub-tab last used, not on Info.

### Info (single column, sections in this order)

| Section | Rows |
|---|---|
| State | State (as today); Last state `{reason} · exit {code}` (+ `· signal {n}`); Last run `ran {run}, ended {age} ago` (only with both timestamps); Restarts |
| Image | Image; Digest (mono, truncated with tooltip; "—"); Pull policy ("—") |
| Ports | one `port_row` per port, text `{port}/{protocol}` + ` · {name}`; "—" when none |
| Resources | one row per `ContainerResource`: label `resource_label` (CPU, Memory, Ephemeral storage, else the raw name), value `resource_text`; muted "No requests or limits" when empty |
| Probes | three rows, Liveness, Readiness, Startup: left `probe_text`, right `probe_result(pod, container, kind, events)` text in its tone |
| Env & mounts | `env_summary` with an `Env →` link and `mount_summary` with a `Mounts →` link (each calls `set_container_tab`); a row is left out when its summary is None |

Text builders (pure, `container_detail.rs`):

| Fn | Example |
|---|---|
| `resource_text(&ContainerResource)` | `request 250m · limit 1`, `request 250m · no limit`, `no request · limit 512Mi` |
| `probe_text(ProbeKind, Option<&ProbeSummary>)` | `Readiness · HTTP GET :8080/ready · every 5s`, `Liveness · TCP :5432 · every 10s`, `Startup · gRPC :9090 · every 10s`, `Liveness · exec `test -f /tmp/ready` · every 10s` (`exec command` when the argv is empty), `Startup` (not set). HTTPS → `HTTPS GET` |
| `env_summary(&ContainerSummary)` | `14 env vars · 6 from configmap/api-config · 3 from secret/api-db · all of secret/extra`; sources in first-seen order, at most 3, then ` · +{n} more`; `1 env var`; None when both lists are empty |
| `mount_summary(&ContainerSummary)` | `/etc/api ← configmap/api-config (read-only) · +3 more`; first mount in spec order; None when empty |
| `last_run_text(&Termination, now)` | `ran 4m, ended 2m ago` |

### Env (names and sources only, C1)

```rust
pub(crate) struct SourceRow { pub(crate) name: String, pub(crate) source: String, pub(crate) target: Option<ResourceKey> }
pub(crate) fn env_rows(container: &ContainerSummary, namespace: &str) -> Vec<SourceRow>;
pub(crate) fn mount_rows(container: &ContainerSummary, namespace: &str) -> Vec<SourceRow>;
```

| Entry | `name` | `source` | `target` |
|---|---|---|---|
| envFrom ConfigMap | `{prefix}*` or `*` | `all keys of configmap/{name}` | ConfigMap key |
| envFrom Secret | same | `all keys of secret/{name}` | None (0016) |
| Literal | name | `literal · value in the YAML tab` | None |
| ConfigMapKey | name | `configmap/{name} · {key}` | ConfigMap key |
| SecretKey | name | `secret/{name} · {key}` | None |
| Field / ResourceField / Unknown | name | `field {path}` / `resource {resource}` / `unknown source` | None |

envFrom rows come first. Each row: name (mono, truncated, tooltip) and source (muted; `link_text` when `target` is Some). Empty: muted "No environment variables".

### Mounts

`mount_rows`: `name` = mount path; `source` = `source_text` + ` · read-only` + ` · subPath {p}`; `target` = ConfigMap key for ConfigMap sources. `source_text`: `configmap/{n}`, `secret/{n}`, `pvc/{claim}` (text until 0014), `emptyDir {volume}`, `hostPath {path}`, `projected {volume}`, `downwardAPI {volume}`, `volume {volume}`. Empty: muted "No mounts".

## Menu: Copy kubectl command

- `pod_menu` gains `context: &str` (callers pass `session.read(cx).context()`), and a `Copy kubectl command` item after Copy name, always enabled.
- `pub(crate) fn kubectl_describe_command(context, namespace, name) -> String` in `resource_actions.rs`: `kubectl --context {c} -n {ns} describe pod {name}`. Each part is single-quoted when it has a character outside `[A-Za-z0-9@%+=:,./_-]`, with `'` written as `'\''`. The function carries `// ponytail: POSIX sh quoting only; PowerShell and cmd need other rules — add a per-shell variant if Windows users paste into them.`
