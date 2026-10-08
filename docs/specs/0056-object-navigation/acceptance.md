# 0056 · Acceptance criteria

[Back to index](README.md). Numbered for review reports; the step that delivers each is in brackets.

## History (H3)

1. `reveal_object` records the place it leaves (drawer links, palette Go to, Issues rows, Overview cards, dialog links, menu Go to items); j/k, row clicks, sidebar screens, Topology node clicks, and `when_selected` action runs (palette pairs, keys) do not. [A1]
2. `Alt+Left` restores the previous place: screen and that table's filter; when the place had a selection, also the object (scrolled into view), drawer open or closed, drawer tab, Pod container and container sub-tab. A place without a selection restores screen and filter only. `Alt+Right` redoes it. The mouse Back and Forward buttons do the same as `Alt+Left` and `Alt+Right`. [A1]
3. A reveal of the object already selected on the current screen records nothing. [A1]
4. A new reveal after going back clears the forward stack. Back keeps at most 50 entries. [A1]
5. Cluster switch and scope change clear the history. [A1]
6. Back to a deleted object restores the screen with no drawer and no error; a removed custom kind's entry is skipped. [A1]
7. Back with unsaved Edit YAML / values text asks first, like a reveal. [A1]
8. Back to Topology shows the graph and reopens the drawer once the Topology feeds deliver the row; no `pending_reveal` is set. [A1]
9. `Alt+Left` / `Alt+Right` inside a focused text input (quick filter, palette, forms), the YAML / values editors, and the terminal never navigate, verified on Windows; the coder checked the kit key contexts and added `NoAction` bindings where needed. Both appear in the shortcut sheet and the palette as `Back` / `Forward`. [A1]
10. The drawer header shows `← {previous name}` when history is not empty, with tooltip `Back to {Kind} {name} (Alt+Left)`; a click equals `Alt+Left`. [A2]

## Links and Prev / Next (H4 style, M9)

11. Every drawer text that opens an object is in the link color and underlined, and every such click goes through `open_link`; no plain-text row navigates (endpoints, not-ready pods, revisions, jobs, selected pods, mounted-by, pod lists, the WHY link). [B1]
12. A link to a kind the known access report denies does not navigate; a notification shows `Not permitted: list {resource}`. [A3]
13. A link to a namespaced object outside a `Named` / `Several` scope does not navigate and does not change the scope; a notification shows `{name} is in {namespace}, outside the scope`. [A3]
14. The header shows Previous / Next and `{n} of {visible}` on Pods, Nodes, and kind screens; they move like `K` / `J` (wrap at the ends, filtered and sorted order, tab kept). Hidden on Topology, Overview, Port forwarding, and under the editor; disabled while the subject is not a visible row. [A2]

## Relations (H4, M16)

15. The Ingresses table has a `Backends` column listing backend services in rule order then the default backend; resource backends show `Kind/name`. [B4]
16. The Service drawer has `Exposed by`: each Ingress routing to it (rule or default backend) as a link with its routes; loading, empty, unavailable, and denied notes as specified. [B3b]
17. Endpoint rows link their pod by name; an endpoint without a pod is plain text. [B1]
18. The NetworkPolicy drawer lists its selected pods (`Pods` section, counts, cap with `+N more`), sharing the PDB code. [B4]
19. The Pod drawer: `Service account` is a link; a `Deployment` row appears for a Deployment-owned pod and opens the Deployment in one click. [B2]
20. The Pod drawer lists `Volumes` (configmap / secret / pvc as links) and `Labels` chips [B2], and the `Services` that select it [B3a].
21. Env and Mounts link Secrets and PVCs as they link ConfigMaps. [B2]
22. The two new related watches run only while a Pod / Service drawer is open (any screen, Topology included), are named in `Watching N` and its tooltip, and are not started when the report denies them. [B3a, B3b]

## Name search (M28)

23. A Deployment, DaemonSet, Job, HPA, PDB, quota, PVC, or TLS Secret name is found with no new request, only from feeds that are live, and only for namespaces in the scope. [C1]
24. `list_object_names` pages with `limit=500` via the continue token, stops at 5,000 names with `is_truncated`, and lists each namespace of a `Several` scope. [C2]
25. A Service, Ingress, StatefulSet, CronJob, or NetworkPolicy name is found after the index loads; typing starts at most one run per 120 s, also after a failed run; opening the palette alone starts none. [C3]
26. A scope change aborts a running index run and a stale-scope result is discarded. [C3]
27. The index keeps only namespace and name; no object body, label, or query is stored, logged, or traced. [C2, C3]
28. While the index loads the footer / empty text says what is still being searched; afterwards it lists what was searched, denied kinds, and truncated kinds. [C3]
29. Confirming a name-index hit opens its kind screen with the row selected and the drawer open. [C3]
30. A fixture test ranks ~9,000 subjects under the 4 ms budget in a release build. [C3]

## General

31. No create / update / patch / delete / exec / attach / portforward call is added; new calls are LIST / WATCH only.
32. All UI strings are English; colors come from the theme (`theme.link`, `muted_foreground`).
33. fmt, clippy `-D warnings`, and tests pass on the workspace after every step.
