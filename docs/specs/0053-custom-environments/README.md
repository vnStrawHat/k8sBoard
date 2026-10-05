# 0053 — Custom environments

Status: draft (architect, 2026-10-05). Crate: `crates/app` only, no dependency change, no Kubernetes change. User request (2026-10-05, Vietnamese): "Remove the colour select in Settings › Clusters. Add a Settings section to create more custom Environments, each with a Color (today it is fixed: Production, Staging, Development, Local)."

## Goal

- Settings › Clusters loses its `Color` row. `ClusterEntry.color` / `ClusterProfile.color` go away (the title-bar stripe that read them was removed on 2026-10-04, commit 301f4f0).
- Environments become data. The four built-ins stay exactly as they are. Users add **custom environments**, each with a name, a colour from a fixed theme-token palette, and a **tier** ("Behaves like": one of the four built-ins). All guardrails read the tier, so a custom environment is never weaker than the built-in it names.
- A new Settings page, **Environments** (after Clusters): built-ins listed read-only, custom ones with rename, recolour, tier, move up/down, delete, and an add row.
- Custom environments show everywhere an environment shows today: the Environment dropdown, every badge, the Unlocked frame, the group headers of Settings › Clusters and the switcher, search text, and the Safety tier table.

## Non-goals

- Editing the built-ins (name, badge, colour, or tier).
- A free RGB/hex picker, or colours outside the GPUI Kit theme.
- Guessing a custom environment from names: `guess_environment` stays built-in only.
- A separate badge text: a custom badge is its name in upper case.
- Re-locking an open session when its environment or tier changes (same as today: the lock is chosen at open).

## Files in this spec

| File | Contents |
|---|---|
| [decisions.md](decisions.md) | numbered decisions with rationale |
| [model.md](model.md) | types, signatures, persistence, resolution, groups, palette |
| [environments-page.md](environments-page.md) | the new Settings page: layout, actions, validation, delete dialog |
| [files-to-touch.md](files-to-touch.md) | three coder steps, every call site, docs |
| [test-plan.md](test-plan.md) | unit, gpui, and ui-verifier checks per step |

Also changed in this commit: [0024 persisted-prefs.md](../0024-settings-store/persisted-prefs.md) (rows for `environment`, `environments`, `color` removed) and [0043 clusters-list.md](../0043-settings-pages/clusters-list.md) (Colour section marked removed).

## Acceptance criteria

- [ ] 1. After each step: the quality gate passes, plus `cargo clippy -p k8sboard --features screenshot --all-targets -- -D warnings`. No new `#[allow]`, no `Cargo.toml` change.
- [ ] 2. The 0003 colour-literal grep of `crates/app/src` stays clean: every environment colour is a `cx.theme()` token.
- [ ] 3. Settings › Clusters has no `Color` row. A `settings.json` with `registry.clusters[].color` loads without a reset notice, and the next save drops the key.
- [ ] 4. `registry.environments` round-trips; the settings key allow-list gains `registry.environments{,.name,.color,.tier}` and loses `registry.clusters.color`. A default file has no `environments` key.
- [ ] 5. A cluster in a custom environment gets every default of its tier: Production tier opens Read-only, confirms by typing the name, and has the node shell off; other tiers match the built-in set explicitly. A reference to a missing custom environment resolves to Production.
- [ ] 6. The Environments page adds, renames (cluster references follow), recolours, re-tiers, reorders, and deletes custom environments. Invalid names show the message under the field and save nothing. Delete moves its clusters to the built-in of the same tier, after a confirm dialog that names how many.
- [ ] 7. Custom badges (upper-case name, chosen colour) show in the title bar, switcher, Settings › Clusters list, palette, and confirm/drain/port-forward dialogs. Custom groups follow the built-in groups in the page's order, in both Settings › Clusters and the switcher. The Safety tier table lists custom names under their tier.
- [ ] 8. ui-verifier shots (test-plan §5) in Default light/dark and One Dark: no unreadable badge, palette swatches distinct.

## Open items (defaulted, user may override)

1. New custom environments default to **Production tier** and **Purple** (decision 9).
2. Custom badge = upper-cased name, max 16 characters (decision 6). A short badge field is a follow-up.
3. A custom badge colour is the user's choice, so colour alone no longer signals risk; the tier still drives the confirm step (decision 12).
