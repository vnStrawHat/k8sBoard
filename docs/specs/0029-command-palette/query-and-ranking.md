# 0029 · Query modes and ranking

[Back to index](README.md) · Step 1 · Modules: `palette_search.rs` (new; tests in `palette_search_tests.rs`), `fuzzy_score.rs` (new; tests in module), `resource_kind.rs`. Decisions 1–7.

## Modes (W9 note 1, footer)

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PaletteMode { All, Kinds, Clusters, Namespaces, Actions }
pub(crate) struct PaletteQuery<'a> { pub(crate) mode: PaletteMode, pub(crate) text: &'a str }
/// The first character picks the mode (`:` `@` `#` `>`); the rest, trimmed, is the text.
pub(crate) fn parse_query(raw: &str) -> PaletteQuery<'_>;
```

| Prefix | Mode | Groups searched |
|---|---|---|
| none | `All` | Actions, Resources, Go to (kinds, namespaces, clusters) |
| `:` | `Kinds` | Go to: screens only (k9s style `:po`, `:deploy`) |
| `@` | `Clusters` | Go to: registered clusters (0026 rows) |
| `#` | `Namespaces` | Go to: namespaces of the live list, plus "All namespaces" |
| `>` | `Actions` | Actions only |

- Leading spaces are skipped before the prefix check; `"  :po"` is `Kinds`, `po`.
- An empty text shows each group's default list ([entries.md](entries.md)); `All` with empty text shows no Resources (decision 6).

## Kind aliases (`resource_kind.rs`)

`ResourceKind::short_names(self) -> &'static [&'static str]` with the kubectl short names (`po` is a `Screen`, not a kind, so the palette keeps its own two rows for Pods `po` and Nodes `no`):

| Screen | Search words |
|---|---|
| Pods | pods, pod, po |
| Nodes | nodes, node, no |
| every `ResourceKind::ALL` kind | `plural()` and `singular()`, plus the kubectl short names: `ns`, `ev`, `deploy`, `sts`, `ds`, `rs`, `cj`, `svc`, `ing`, `cm`, `netpol`, `pdb`, `hpa`, `quota` |

An exact alias match (`:po`, `:deploy`) always ranks first (decision 4). A kind added later (0014–0018) lists its short names in its own spec; `short_names` is an exhaustive `match`, so the compiler forces it.

## Fuzzy score (`fuzzy_score.rs`)

```rust
/// `None` when `needle` is not a subsequence of `haystack` (ASCII case-insensitive).
pub(crate) fn fuzzy_score(needle: &str, haystack: &str) -> Option<u32>;
```

- Greedy left-to-right subsequence match, preferring a word start when the next needle char occurs at one. Word starts: index 0, and after `-` `/` `.` `_` space.
- No numeric weights in the spec: the named tests in [test-plan.md](test-plan.md) are the contract, and the coder tunes the weights under the ponytail note below. Ranking properties the weights must give:
  1. **exact** (case-insensitive) beats **prefix**, and prefix beats an **inner** match;
  2. a **consecutive** run beats scattered word-start hits (`api` ranks `payments-api` above `a-p-i`);
  3. a word-start hit beats a mid-word hit;
  4. index 0 counts as a word start **once** (no extra bonus for being both). Saturating arithmetic.
- `// ponytail: greedy alignment, not optimal (no DP); switch to a Smith-Waterman pass or nucleo if users report misses.`
- Chars compare with `eq_ignore_ascii_case`; non-ASCII chars compare exactly (Kubernetes names are ASCII).

## Entry score

```rust
pub(crate) fn entry_score(text: &str, fields: &[&str]) -> Option<u32>;
```

- `text` splits on whitespace into tokens; every token must match some field; the entry score is the sum of each token's best field score. Tokens are scored independently, so two tokens may match the same field (W9: `rest pay` → `Restart rollout` + `payments-api`; `pay api` → `payments-api`).
- Fields per entry: action → label, target (`deployment/payments-api`); resource → name, `namespace/name`, kind words; kind → its search words; namespace → name; cluster → label, context, env badge (0026 `search_text`).
- Empty text → every entry scores 0 (kept, in source order).

## Order and caps

- Groups keep the W9 order: Actions, Resources, Go to. Inside a group: score descending, then source order (stable sort).
- Caps: Actions 20, Resources 50, Go to 30 per query; the footer shows "+N more" text when a cap cuts (decision 7).
- Budget: one full rescoring at most 4 ms for 5,000 candidates in a release build (trace, counts only). The caps are also the measurement budget: the 4 ms covers scoring, sorting, and cutting to these caps. Rescoring happens on query change and on shell notify, never per frame.
