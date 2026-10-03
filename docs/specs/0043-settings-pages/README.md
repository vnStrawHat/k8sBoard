# 0043 — Settings pages (W2 General, Logs, Terminal & Shell; Clusters completions)

Status: **draft 2026-10-03 against main `52f8f87`**; content proposed by the architect (user: "propose the missing Settings pages yourself"). **Local-only: no new Kubernetes calls.** Step 4 changes `crates/cluster` transport only (proxy, flagged below); no write path is touched. Crates: `crates/app`, `crates/cluster` (steps 2, 4). Prerequisites: merged 0024, 0025, 0028, 0030, 0036. One cluster at a time (0046). Roadmap: [wireframe-gap-audit.md](../../roadmap/wireframe-gap-audit.md) gap 5, order item 6. Wireframes: W2 nav and notes 2–5, W2 form (`Color`, `Proxy`), W2 header `⌕ Search`, W2 Add menu, Tokens (density 28 / 36), W8/W8b log toolbar.

## Goal

- **General** (export folder, clipboard clear time, TLS Secrets watch), **Logs** (tail, timestamps, wrap, JSON defaults), **Terminal & Shell** (default shell, scrollback, font size) pages; **Row density** on Appearance.
- Clusters page: **search**, **drag to reorder** (drives Ctrl 1–9, `Alt ↑/↓` too), **colour swatches**, per-cluster **Proxy**, **Watch a kubeconfig folder** (read-only).
- Every setting changes existing behaviour or is drawn in W2/Tokens; full contract per key in [settings-model.md](settings-model.md).

## Non-goals

Metrics and Extensions pages (backlog); anything multi-cluster; a global proxy, `HTTPS_PROXY`/`NO_PROXY`, proxy credentials in settings or an OS keychain; recursive folder watch; settings not tied to existing behaviour (cursor style, copy on select, log buffer size, …, decision 25); dock height (0044).

## Implementation steps

| Step | Scope | ACs |
|---|---|---|
| 1 | `general` + `appearance` sections; General page; Row density; export folder, clipboard, TLS readers; `--screen settings-general` | 1–5, 15 |
| 2 | `logs` + `terminal` sections; Logs and Terminal & Shell pages; tab and shell readers; `ShellCommand` serde | 1–4, 6, 7, 15 |
| 3 | Clusters: search, drag and `Alt ↑/↓` order, colour swatches + title-bar border | 1–3, 8–10, 15 |
| 4 | Proxy: `kube` features, `ProxyChoice`/`ProxyUrl`, `open(.., proxy)`, Connection row | 1–3, 11, 12, 15 |
| 5 | Watch a kubeconfig folder; full ui-verifier and coder-lite run | 1–3, 13–15 |

## Files

| File | Contents |
|---|---|
| [settings-model.md](settings-model.md) | sections, per-key contract (type, default, reader, when, validation), renamed reserved keys, allow-list |
| [pages.md](pages.md) | page order; General, Appearance density, Logs, Terminal & Shell fields and readers |
| [clusters-list.md](clusters-list.md) | search, order (drag, keys), `ClusterColor` and the border |
| [proxy.md](proxy.md) | build features, cluster API, precedence vs kubeconfig `proxy-url`, validation, auth |
| [folder-watch.md](folder-watch.md) | candidates, debounce, vanished files and folders, UI |
| [decisions.md](decisions.md) · [files-to-touch.md](files-to-touch.md) · [test-plan.md](test-plan.md) | decisions; files per step and doc follow-ups; tests and checks |

## Acceptance criteria

- [ ] 1. Gate passes, plus `cargo clippy -p k8sboard --features screenshot --all-targets -- -D warnings`; no new `#[allow]`, no `unsafe`.
- [ ] 2. Every test in [test-plan.md](test-plan.md) for the step exists under its name and passes offline; `Cargo.lock` gains no `[[package]]` (steps 4, 5 add features and a locked crate only).
- [ ] 3. No new Kubernetes call: no new `ClusterConnection` method or request path; `object_write.rs`, `clippy.toml`, connect files unchanged; `settings_keys_are_the_allow_list` lists exactly the [settings-model.md](settings-model.md) keys.
- [ ] 4. Settings nav reads General, Clusters, Appearance, Keyboard Shortcuts, Safety, Terminal & Shell, Logs, About (W2 order); `Ctrl ,` still opens Clusters.
- [ ] 5. General: Export starts in the saved folder and remembers the last one; a copied secret clears after the chosen time; TLS watch off removes the Secrets condition watch at the next session and names it in coverage. Appearance: Compact 28 px (default) and Comfortable 36 px apply live to every table.
- [ ] 6. Logs: a new log tab opens with the configured tail (`tailLines` in the request), timestamps, wrap, and JSON; tab toggles never change settings.
- [ ] 7. Terminal & Shell: Open shell uses the default shell (named in the confirm dialog and audit line); new tabs keep the chosen scrollback; font size changes the open terminal at once.
- [ ] 8. Search filters the Clusters list by name, context, environment, or file, like the switcher filter.
- [ ] 9. Dragging a row (or `Alt ↑/↓`) reorders it inside its environment group only, persists in `registry.clusters`, and the switcher's Ctrl 1–9 follow at once.
- [ ] 10. A colour swatch changes the title-bar top border at once; the environment badge keeps its colour; picking the environment's colour stores nothing; only theme tokens.
- [ ] 11. Proxy: From kubeconfig / None / Custom URL apply on the next connection (switch, Reconnect, Test connection, probe); a kubeconfig `proxy-url` now works; `HTTPS_PROXY` stays ignored.
- [ ] 12. No proxy credential is ever stored, shown, or traced: a URL with userinfo shows the Credentials message and is not saved; `ConnectionInfo.proxy` and `Debug` show `scheme://host:port` only; an invalid stored URL fails `open`, never falls back to direct.
- [ ] 13. A watched folder lists its kubeconfig files within ~1 s of a change (500 ms debounce); a deleted file drops its rows but not a running session; a broken rewrite keeps the last good version; a missing folder shows its notice.
- [ ] 14. k8sBoard never writes in a watched folder; Stop watching deletes nothing; folder rows cannot be removed.
- [ ] 15. ui-verifier (light, dark): `settings-general`, `settings-appearance`, `settings-logs`, `settings-terminal`, `settings`, `pods` at 28 and 36 px show no high-severity defect against W2 and Tokens; coder-lite UAT run per [test-plan.md](test-plan.md).

## Open items

1. Default density 28 px changes today's 32 px look for everyone (Tokens say compact is default). Keep, or default to Comfortable?
2. Proxy credentials beyond the kubeconfig `proxy-url` (OS keychain) are deferred: needs a dependency choice and a C1 review.
3. A missing watched folder is retried only at the next start; add a re-check on Settings open if users ask.
