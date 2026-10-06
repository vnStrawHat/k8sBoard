# 0024 · Environments

[Back to index](README.md) · Step 3 · Module: `environment.rs` (new; tests in `environment_tests.rs`), `title_bar.rs`. Decisions 23–26. Wireframe: W1 (`.tb` border-top, `.env` badge in `.ctx`), W2 note 4.

## Model

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Environment { Local, Development, Staging, Production } // Ord = risk, lowest first
impl Environment {
    pub(crate) fn badge(self) -> &'static str;       // "LOCAL" | "DEV" | "STG" | "PROD"
    pub(crate) fn group_label(self) -> &'static str; // "Local" | "Development" | "Staging" | "Production" (0025/0026)
}
pub(crate) fn guess_environment(context: &str, cluster: &str) -> Environment;
pub(crate) fn environment_color(environment: Environment, cx: &App) -> Hsla;
pub(crate) fn environment_badge(environment: Environment, cx: &App) -> impl IntoElement;
```

`Ord` by risk lets 0027 take `max()` for the riskiest-env border. `group_label` lands with its first user (0025/0026), not now (dead-code rule).

## Guessing rules (C5)

Input: the context name and the kubeconfig cluster entry name. Each is lowercased and split into **tokens** on every char that is not ASCII alphanumeric; trailing digits are trimmed from each token (`prod1` → `prod`, `dev01` → `dev`). Each environment below is tested on both names; the **riskiest match wins**.

| Environment | Matches when |
|---|---|
| Production | a token starts with `prod` (`prod`, `production`, `prod1`), or equals `prd` |
| Staging | a token equals `stg` or `uat`, or starts with `stag` (`stage`, `staging`) |
| Development | a token starts with `dev` (`dev`, `develop`, `devops`) or `test` (`test`, `testing`) |
| Local | the whole lowercased name starts with `kind-` or `k3d-`, or contains `docker-desktop`, or a token equals `minikube` or `localhost` |
| none matched | **Staging** (C5 default: a confirm dialog with a click, not a typed name; 0030 decision 9) |

Examples (all in the test table): `prod-eu-1` PROD · `arn:aws:eks:eu-central-1:4471:cluster/prod-eu-1` PROD · `stg-us-1` STG · `uat` STG · `dev-shared` DEV · `kind-k8sboard` LOCAL · `minikube` LOCAL · `docker-desktop` LOCAL · `kind-prod` PROD (riskiest) · `latest` STG (token, not substring) · `readonly@Monitor` STG (unknown) · context `admin`, cluster `prod-1` PROD.

Known ceiling: words that merely start with `prod`/`dev` (`products`, `device`) over-classify. That errs toward more caution for PROD; the user corrects it in 0025.

## Colors

| Environment | Token | Wireframe hue |
|---|---|---|
| Production | `theme.danger` | `--prod` red |
| Staging | `theme.warning` | `--stg` amber |
| Development | `theme.info` | `--dev` blue |
| Local | `theme.muted_foreground` | `--loc` gray |

`environment_color` is the only place an environment touches the theme (like `tone_color`). No color literals.

## Badge

`environment_badge`: `Tag::custom(color, theme.background, color)` with `.small()` (or the kit's smallest size) and the `badge()` text in the mono font, semibold; matches W1 `.env` (filled, light text on the env hue).

## Title bar (`title_bar.rs`)

- **Top border**: `title_bar(..)` styles the kit `TitleBar` itself with `.border_t(px(3.)).border_color(color)` (it is `Styled`). `color` = `environment_color(profile.environment)` with an active cluster, else `theme.title_bar_border`. Always 3 px, so the layout does not shift when a session starts. GPUI has one border color per element, so the kit's 1 px bottom border takes the same color (decision 26). **Removed on user request on 2026-10-04: the title bar has no coloured top or bottom border; it uses the kit's normal `title_bar_border`.**
- **Switcher trigger**: `h_flex` of `environment_badge(profile.environment)` + the button label `profile.display_name` (W1 `.ctx`). No session → no badge, label "No cluster" (unchanged).
- **Switcher menu**: one item per context with `profile.display_name`, checked = active. Badges and env groups in the menu are 0026.
- **Notices button** (decision 30): when `AppSettings::notice` or the shell's kubeconfig notices are non-empty, a ghost small icon button (a kit warning/alert icon), `theme.warning` icon color, before the Read-only badge. Tooltip: one notice per line. Click: `AppSettings::dismiss_notice` and clear the shell notices.
- The Read-only badge is unchanged (its env-colored dashed border and toggle are 0030).
- **Built-in rows of Settings › Environments** (J6, 2026-10-06): Production reads `Opens read-only. Confirms by typing the object name (one object) or the cluster name (several)`; the others read `Click Confirm`. The same wording is the Change and Destructive cell of the Safety table.
