# Agent Workflow — k8sBoard

The main session is the orchestrator. It only orchestrates (assigns work, relays results to the user) and never implements.

## Delivery loop

1. Orchestrator assigns a work item.
2. `architect` writes a spec folder `docs/specs/NNNN-short-title/` (`README.md` index plus one short file per topic group).
3. (Optional) `advisor` reviews the spec.
4. `coder` implements the spec, with unit tests and a passing quality gate.
5. `coder-lite` runs the quality gate, builds, and read-only cluster probes, and reports evidence.
6. For any change touching UI (`crates/app` or views), `ui-verifier` builds and launches the app, captures screenshots, compares them to `docs/k8sboard-wireframes.html`, and reports layout defects. Defects go back to `coder`.
7. `advisor` reviews the diff against the spec and `docs/agents/code-style/` (README.md + all files).
8. Orchestrator reports to the user.

Deviations from the spec go back to the architect (or the orchestrator), never silently into code.

## Agents

| Agent | Model | Effort | May edit code? | Typical tasks |
|---|---|---|---|---|
| architect | opus | high | No (docs/ only) | Crate boundaries, public APIs, async architecture, UI structure, specs, spec compliance review |
| coder | sonnet | high | Yes | Implement specs, unit tests, full quality gate |
| coder-lite | sonnet | low | Yes (small/mechanical) | Config, dependency bumps, scaffolding, fmt/clippy fixes, doc updates, env setup, gate runs, read-only probes |
| advisor | fable | medium | No (read-only) | Spec and diff review, risk analysis, crate/API research with sources |
| ui-verifier | sonnet | medium | No (read-only) | Visual verification of UI changes against the wireframes |

## Documentation size

Docs stay short: split by topic group into files of at most ~120 lines; every folder has a README.md index.
