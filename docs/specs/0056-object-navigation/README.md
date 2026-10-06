# 0056 — Object navigation: back, links, relations, name search

Status: **draft 2026-10-05, revised after advisor review**, from the task-based UX walkthrough on UAT (findings H3, H4, M16, M28, M9); the user approved the fix. Crates: `crates/app` (all steps), `crates/cluster` (C2 only). Touches 0003 (drawer header), 0005 (Ingress columns, kind drawers), 0028 (keys), 0029 (decision 9 superseded for Resources).

## Goal

- Every link is reversible: Back / Forward history, `Alt+Left` / `Alt+Right`, a Back button with the origin's name.
- Ingress -> Service -> Pod can be traced both ways; the Pod drawer links its Deployment, Services, Secrets, PVCs, volumes.
- One link style; denied and out-of-scope targets refuse with a notice.
- The palette finds common kinds by name without watching them.
- The wide Pod drawer can walk its list (Previous / Next).

## Non-goals

- Drawer over the current screen for links (history.md explains why; Topology keeps its own model).
- Links changing the namespace scope; an Endpoints screen; Pod annotations; NetworkPolicies that select a pod (reverse); breadcrumb trail.
- Palette search of ConfigMaps, all Secrets, ServiceAccounts (possible follow-up), RBAC, storage, CRD, and custom kinds (still `:kind`).
- Narrowing the Pod drawer width.

## Files

| File | Content |
|---|---|
| [history.md](history.md) | H3: navigate vs swap decision, history entry, record / restore, resets, header Back, keys |
| [links.md](links.md) | link style rule, `open_link` / `follow_link` (denied, out of scope), Prev / Next (M9), `DrawerNavigation` |
| [relations.md](relations.md) | H4 + M16: sources and cost, Backends column, Exposed by, NetworkPolicy pods, Pod drawer sections, Env / Mounts links |
| [name-search.md](name-search.md) | M28: loaded feeds, one-shot name index, lifecycle, cost, results, texts |
| [plan.md](plan.md) | eleven steps in three lanes, dependencies, files per step, tests |
| [acceptance.md](acceptance.md) | 33 numbered acceptance criteria |

## Acceptance checklist (details in acceptance.md)

- [x] A1 history core and keys (AC 1-9; as built: `Back` takes an `is_served` predicate, the shell lives in `app_shell_history.rs`, and the terminal `NoAction` list is not needed because the workspace context already excludes `Terminal`)
- [ ] A2 header Back, Prev / Next (AC 10, 14)
- [ ] A3 `follow_link`: denied, out of scope (AC 12, 13)
- [x] B1 one link style, `open_link`, endpoint pod links (AC 11, 17; as built: list rows keep their row click and draw the name with `link_name`; the Revisions row links its `rev N` title)
- [x] B2 Pod Overview without new watches (AC 19-21; as built: `Volumes` is always shown, with a note when no container mounts a volume; Env / Mounts links open through `open_link`, A3 adds the denied refusal)
- [x] B3a Pod Services (AC 20, 22); as built: `key_related_subject` (related_objects.rs) gives the Pod subject from the key, `services_selecting` lives in `kind_join.rs`, and the section is capped at 20 rows
- [x] B3b Service Exposed by (AC 16, 22); as built: `exposing_ingresses` lives in `network_rows.rs`, a repeated host is spelled once in the routes text, and the list is capped at 20 ingresses
- [x] B4 Ingress Backends, NetworkPolicy Pods (AC 15, 18); as built: `selected_pods_rows(namespace, selector)` serves PDB and NetworkPolicy, and `INGRESS_TLS` moved to cell 4
- [x] C1 palette searches the live condition feeds in scope (AC 23)
- [x] C2 `list_object_names` (AC 24; AC 27 for the cluster half)
- [ ] C3 name index in the palette (AC 25-30)
- [ ] General (AC 31-33)

## Cost summary

Zero new calls for history, links, Prev / Next, Backends, NetworkPolicy pods, Deployment / volume / label / secret links. Two drawer-scoped watches (services or ingresses of one namespace) while a Pod or Service drawer is open. Palette: up to 5 metadata LISTs per namespace of the scope, at most once per 120 s (failures included), only while searching.

## Decided (were open)

- Q1: a link outside the scope refuses with `{name} is in {ns}, outside the scope`; scope never widens.
- Q5: name index = Services, Ingresses, StatefulSets, CronJobs, NetworkPolicies.

## Open questions (defaults let work start)

| # | Question | Default |
|---|---|---|
| Q2 | Should sidebar screen changes also record history (browser-like)? | No: only reveals record; Back = "back to where the link was followed from" |
| Q3 | Show Pod annotations (requires reading them for every pod)? | No; the YAML tab has them |
| Q4 | Add an Endpoints / EndpointSlices screen? | No; Service drawer Endpoints + Services table cover it |
| Q6 | Prev / Next at list ends: wrap (as J / K) or stop? | Wrap, matching the keys; the `n of N` text shows the jump |
| Q7 | Forward button in the header, or key only? | Key only (`Alt+Right`, palette `Forward`) |
