# 0019 — Workload logs and dock polish (read-only)

Status: amended after the advisor review (must-fix 1–2, should-fix 3–8, nice-to-haves 9–13). HEAD `d2f98f9`; **start after 0011 step 3 merges** (it edits `container_detail.rs`, `pod_drawer.rs`). Requires 0004, 0008, 0010, 0012 (`PodOwner`, `KindRow.related_pods`). Crates: `crates/app`, plus **one field** in `crates/cluster` (`LogRequest.tail_lines`, step 2a; decision 33). Wireframes: W8, W8b, W4b note 3, W7 workload menus. Applies C1 (no change), C6 (no new package), C9 (Export, user-approved).

## Goal

- **Workload log tabs** (`deploy/`, `sts/`, `ds/`, `rs/`, `job/`): merge up to 10 pods' streams with a colored `{pod}/{container}` prefix; follow pod churn (joins, deletions, StatefulSet rejoins).
- **Filters:** Regex toggle beside the 0004 plain filter; ERROR/WARN/INFO/DEBUG chips with heuristic level detection; JSON toggle (message + pretty fields); error rows tinted.
- **Zoomed (full) layout:** pod legend and a log volume histogram (kit `BarChart`).
- **Export…** of the visible lines through the native save dialog only (C9).
- **Dock polish:** container **Logs** sub-tab (W4b), "+ ▾" new-tab menu (Logs of selected, Shell slot disabled), tab drag-reorder.

## Non-goals

Kubelet/node logs (decision 24); Pop out; CronJob "View logs of last job"; Shell tab (0036); keyboard shortcuts (0028); per-pod hide toggles; a Follow toggle (tail-follow is follow); histogram brush selection; local time; auto-reconnect; dashed max-height line and double-click reset (0004). **Follow-up:** synthetic `SYS` marker lines (`── pod x2k4q joined ──`, `── pod x2k4q deleted ──`, container restarts) in workload tabs.

## Implementation steps

| Step | Scope | ACs |
|---|---|---|
| 1 | `log_level.rs`, `line_matcher.rs`, `log_json.rs`, `log_rows.rs`, buffer view, toolbar controls, Cargo edges | 1–5, 8 |
| 2a | `log_target.rs`, `log_workload.rs`, `LogRequest.tail_lines`, multi-stream `LogTab`, merge, stream budget, rejoin, status/tone, prefixes, `open_workload_logs`, `kind_menu` item | 1–6, 8 |
| 2b | workload container picker, legend, `selected_log_target` + `NoLogTarget`, `logs-workload` launch screen and screenshot | 1–8 |
| 3 | `LogLayout`, histogram (`log_volume.rs`), Export (`log_export.rs`), "+ ▾", tab reorder, container Logs sub-tab | 1–8 |

## Files

| File | Contents |
|---|---|
| [decisions.md](decisions.md) | numbered decisions with one-line rationale, deferrals, ceilings |
| [filters-and-levels.md](filters-and-levels.md) | step 1: level detection, `line_matcher.rs` |
| [buffer-and-rows.md](buffer-and-rows.md) | step 1: buffer view, JSON view, toolbar, rows |
| [workload-targets.md](workload-targets.md) | 2a/2b: `log_target.rs`, pure membership, opening paths |
| [workload-streams.md](workload-streams.md) | 2a/2b: cluster field, tab state, sync, merge, status, legend, async contract |
| [dock-polish.md](dock-polish.md) | step 3: layouts, histogram, Export flow, "+ ▾", reorder, Logs sub-tab |
| [files-to-touch.md](files-to-touch.md) | Cargo, modules per step, screenshot screen |
| [test-plan.md](test-plan.md) | unit tests per step, live checks, ui-verifier checklist and accepted deviations |

## Acceptance criteria

- [ ] 1. The quality gate passes, plus `cargo clippy -p k8sboard --features screenshot --all-targets -- -D warnings`. No new `#[allow]`.
- [ ] 2. Every test in [test-plan.md](test-plan.md) exists under that name and passes; none touches a network or the file system.
- [ ] 3. `Cargo.lock` gains no `[[package]]`; `regex` stays 1.13.x, `serde_json` 1.0.x.
- [ ] 4. No `tracing` call in the 0019 log modules receives line text, a file path, or export content (review + the scoped grep in [test-plan.md](test-plan.md), which lists files explicitly and leaves out the pre-existing `log_filter.rs` tracing pin). The only new file write is in `log_export.rs`, reached only from the Export button after the dialog returns a path.
- [ ] 5. `crates/cluster` diff = `LogRequest.tail_lines`, `log_params`, its tests, and the probe call site only; the 0001 read-only guard holds; `ws` stays off.
- [ ] 6. App on UAT: a Deployment's "View logs (all pods)" merges prefixed lines from its pods; a new pod (if observable) joins; regex `error|timeout`, level chips, and JSON work; Export writes only after Save; Logs sub-tab focuses the dock.
- [ ] 7. Screenshots `logs-dock`, `logs-zoomed`, `logs-workload`, `pod-containers` in light and dark; no high-severity ui-verifier defect against W8, W8b, W4b beyond the accepted deviations.
- [ ] 8. The 0003 AC4 color-literal grep is still clean (pod colors `chart_1..chart_5`; tints derive from tone tokens).

## Open items

1. **Ordering ceiling:** exact only inside the 2 s start window. Afterwards: arrival order (≤ ~100 ms batch skew); late joiners' 50-line tails append at the bottom even when older; kubelet timestamps come from each node's clock, so cross-node skew shows even when sorted. Upgrade: ordered insert with scroller splices.
2. The per-tab stream table grows with churn (one small entry per joined pod/container) until Reconnect.
3. **Buffer fairness ceiling:** one 10k-line buffer per tab; a chatty pod can evict a quiet pod's lines. Upgrade: per-source quotas.
4. Kubelet logs: revisit on ≥ 1.30 with `NodeLogQuery` + `enableSystemLogQuery` on; needs a probe and a fixed path allow-list like 0011.
5. Rejoin needs a snapshot without the pod; a StatefulSet pod recreated between two pods-watch batches keeps its ended stream until Reconnect (no pod UID in `PodSummary`).
