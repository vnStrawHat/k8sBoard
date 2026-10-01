# Risks

[Back to index](README.md). Ranked by impact on delivering the whole wireframe set.

| # | Risk | Impact | Mitigation | Affects |
|---|---|---|---|---|
| R1 | **Secret exposure.** UAT allows `list secrets`, so Secrets, YAML view, Helm values and manifests, and the audit log can carry real credentials into memory, the UI, screenshots, the clipboard, or logs | High | Settle C1 before 0007; lazy drawer content (C7); masked by default; no tracing of payloads; screenshot runs never reveal; extend the 0001 credential script to Secret values in each spec's live check | 0007, 0016, 0017, 0030, 0031 |
| R2 | **No write-capable test cluster.** UAT is read-only and denies exec/port-forward, so every mutating spec (0030–0038) can only be unit-tested and checked as "disabled" | High | Before 0030, the user provides a disposable cluster (for example kind or k3d) and a kubeconfig placed in the project folder; until then mutating specs stop at unit tests plus SSAR-disabled rendering | 0030–0038 |
| R3 | **Custom rendering with no kit component**: terminal (`oneterm-vt` + grid element), topology canvas with layout, Monitor charts | High | Prototype each in its spec's first coder step; keep the dependency choice in the spec (C6); accept a smaller first cut (no Traffic mode, no LSP) | 0010, 0022, 0036 |
| R4 | **Memory and connection budget.** Always-on watches for Issues/Overview, metrics ring buffers, kubelet polling, many log streams, and several live clusters versus the < 150 MB idle target | Med–High | C4, C11, C13; lazy watches per visible screen; caps on buffers and tabs; measure with the UAT pod count before merging 0020 and 0027 | 0010, 0011, 0019, 0020, 0027 |
| R5 | **Scope and platform churn.** 32 specs on a pre-1.0 GPUI Kit (0.7); API changes can stall several specs at once | Med | Pin the kit version; keep kit-specific code in thin view modules; one spec in flight per area; update the roadmap status table after each merge | all |

## Secondary risks

| Risk | Mitigation |
|---|---|
| Generic custom resources need a JSONPath subset and schema-driven drawers; arbitrary CRDs may break the table model | Support the printer-column JSONPath forms that appear in common CRDs; fall back to Name/Age plus YAML |
| `nodes/proxy` grants more than stats (kubelet APIs) | Fixed allow-list of GET paths (`/stats/summary`, `/logs/…`) in the cluster crate; read-only grep extended |
| Persistence outside the project folder conflicts with agent rules | `--config-dir` override is mandatory in every agent run (C2) |
| Unknown environment of UAT context hides prod-style guardrails | C5 default STG for unknown; Settings lets the user set it |
| Drain is long-running and partially failing | Dock progress tab, cancel, stop on first stuck node, resumable state shown (0034) |
