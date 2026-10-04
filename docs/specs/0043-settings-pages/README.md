# 0043 — Settings pages (W2 General, Logs, Terminal & Shell; Clusters completions)

Status: **built 2026-10-04 on main `bdc1f31` (steps 1–5, [as-built.md](as-built.md)); drafted 2026-10-03 against main `52f8f87`, amended after the opus review and the security review** (2 must-fix, 9 should-fix, nits; density decided by the coordinator). Content proposed by the architect (user: "propose the missing Settings pages yourself"). **Local-only: no new Kubernetes calls.** Step 4 changes `crates/cluster` transport only (proxy); step 5 adds a bounded kubeconfig parse there; no write path is touched. Crates: `crates/app`, `crates/cluster` (steps 2, 4, 5). Prerequisites: merged 0024, 0025, 0028, 0030, 0036. One cluster at a time (0046). Roadmap: [wireframe-gap-audit.md](../../roadmap/wireframe-gap-audit.md) gap 5, order item 6. Wireframes: W2 nav and notes 2–5, W2 form (`Color`, `Proxy`), W2 header `⌕ Search`, W2 Add menu, Tokens (density 28 / 36), W8/W8b log toolbar.

## Goal

- **General** (export folder, TLS Secrets watch), **Logs** (tail, timestamps, wrap, JSON defaults), **Terminal & Shell** (default shell, scrollback, font size) pages; **Row density** on Appearance.
- Clusters page: **search**, **drag to reorder** (drives Ctrl 1–9, `Alt ↑/↓` too), **colour swatches**, per-cluster **Proxy**, **Watch a kubeconfig folder** (read-only, untrusted: never auto-started or auto-probed).
- Every setting changes existing behaviour or is drawn in W2/Tokens; full contract per key in [settings-model.md](settings-model.md).

## Non-goals

the Extensions page (backlog; the Metrics page is 0048); anything multi-cluster; a clipboard-clear setting (0016 decision 26 stays); a global proxy, `HTTPS_PROXY`/`NO_PROXY`, `https://` proxies in Settings, proxy credentials in settings or an OS keychain; recursive folder watch; settings not tied to existing behaviour (decision 32); dock height (0044).

## Implementation steps

| Step | Scope | ACs |
|---|---|---|
| 1 | `general` + `appearance` sections; General page; Row density (observer); export folder and TLS readers; `--screen settings-general` | 1–5, 15 |
| 2 | `logs` + `terminal` sections; Logs and Terminal & Shell pages; tab and shell readers; `ShellCommand` serde | 1–4, 6, 7, 15 |
| 3 | Clusters: search, drag and `Alt ↑/↓` order, colour swatches + title-bar border | 1–3, 8–10, 15 |
| 4 | Proxy: `kube` features, `ProxyChoice`/`ProxyUrl`, `open(.., proxy)`, `open_cluster`, Connection row | 1–3, 11, 12, 15 |
| 5 | Watch a kubeconfig folder (bounded reads, start and probe rules); full ui-verifier and coder-lite run | 1–3, 13–16 |

## Files

| File | Contents |
|---|---|
| [settings-model.md](settings-model.md) | sections, per-key contract (type, default, reader, when, validation), scrollback cap, renamed reserved keys, allow-list |
| [pages.md](pages.md) | page order; General, Appearance density, Logs, Terminal & Shell fields and readers |
| [clusters-list.md](clusters-list.md) | search, order (drag, keys), `ClusterColor` and the border |
| [proxy.md](proxy.md) | build features, cluster API, precedence vs kubeconfig `proxy-url`, validation, form, auth |
| [folder-watch.md](folder-watch.md) | untrusted-file rules, bounded reads, debounce, vanished files and folders, UI |
| [decisions.md](decisions.md) · [files-to-touch.md](files-to-touch.md) · [test-plan.md](test-plan.md) · [as-built.md](as-built.md) | decisions; files per step and doc follow-ups; tests and checks; what was built and where it differs |

## Acceptance criteria

- [x] 1. Gate passes, plus `cargo clippy -p k8sboard --features screenshot --all-targets -- -D warnings`; no new `#[allow]`, no `unsafe`.
- [x] 2. Every test in [test-plan.md](test-plan.md) for the step exists under its name and passes offline; `Cargo.lock` gains no `[[package]]` (steps 4, 5 add features and a locked crate only).
- [x] 3. No new Kubernetes call: no new `ClusterConnection` method or request path; `object_write.rs`, `clippy.toml`, connect files unchanged; `settings_keys_are_the_allow_list` lists exactly the [settings-model.md](settings-model.md) keys.
- [x] 4. Settings nav reads General, Clusters, Appearance, Keyboard Shortcuts, Safety, Terminal & Shell, Logs, About (W2 order); `Ctrl ,` still opens Clusters.
- [x] 5. General: Export starts in the saved folder and remembers the last one; TLS watch off removes the Secrets condition watch at the next session and names it in coverage; the secret clipboard clear stays 30 s. Appearance: Compact 28 px (default) and Comfortable 36 px apply live to every table, header included.
- [x] 6. Logs: a new log tab opens with the configured tail (`tailLines` in the request), timestamps, wrap, and JSON; tab toggles never change settings.
- [x] 7. Terminal & Shell: Open shell uses the default shell (named in the confirm dialog and audit line); new tabs keep the chosen scrollback (max 10,000); font size changes the open terminal at once.
- [x] 8. Search filters the Clusters list by name, context, environment, or file, like the switcher filter.
- [x] 9. Dragging a row (or `Alt ↑/↓`) reorders it inside its environment group only, persists in `registry.clusters`, and the switcher's Ctrl 1–9 follow at once.
- [x] 10. A colour swatch changes the title-bar top border at once; the environment badge keeps its colour; picking the environment's colour stores nothing; only theme tokens.
- [x] 11. Proxy: From kubeconfig / None / Custom URL (`http://`, `socks5://`) apply on the next connection (switch, Reconnect, Test connection, probe); the Custom URL commits on Enter or blur and shows `Not applied` until then; a kubeconfig `proxy-url` now works; `HTTPS_PROXY` stays ignored by the client.
- [x] 12. No proxy credential is ever stored, shown, or traced: a URL with userinfo shows the Credentials message and is not saved; the form, `ConnectionInfo.proxy`, and every `Debug` (`ProxyChoice`, `ProxyUrl`, `ClusterProxy`) show `scheme://host:port` only; an unparsable stored URL fails with `InvalidProxy`, never falls back to direct.
- [x] 13. A watched folder rescans 500 ms after its last change and at least every 2 s during a burst (fake-clock tests); reads are capped at 1 MiB; a deleted file drops its rows but not a running session; a broken rewrite keeps the last good version; a missing folder shows its notice.
- [x] 14. k8sBoard never writes in a watched folder; Stop watching deletes nothing; folder rows cannot be removed.
- [x] 15. ui-verifier (light, dark): `settings-general`, `settings-appearance`, `settings-logs`, `settings-terminal`, `settings`, `pods` at 28 and 36 px show no high-severity defect against W2 and Tokens; nothing clips at 28 px (status pills, usage bars, checkboxes; a clip is fixed in the cell). coder-lite UAT: direct Test connection → `Connected` with one `GET /version` in the trace; Custom URL `http://127.0.0.1:9` → `Failed` naming the proxy connect error, with the trace showing the attempt to `127.0.0.1:9` and no request to the API server host.
- [x] 16. A folder file never connects on its own: it starts only as the exact `last_used`, never through `--context`, `current-context`, or the first-file fallback (otherwise the switcher opens), and the switcher never probes folder rows automatically, whatever their auth kind.

## Open items

1. Proxy credentials beyond the kubeconfig `proxy-url` (OS keychain, SOCKS5 auth) are deferred: needs a dependency choice and a C1 review.
2. A missing watched folder is retried only at the next start; add a re-check on Settings open if users ask.
