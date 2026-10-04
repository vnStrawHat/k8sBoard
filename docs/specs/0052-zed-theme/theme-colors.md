# 0052 · Theme colours

[Back to index](README.md) · File: `crates/app/themes/zed-one.json`, one `ThemeSet` named `Zed One` with two themes: `One Light` (`"mode": "light"`) and `One Dark` (`"mode": "dark"`), both `is_default: false`. The source is Zed `one.json` at the pinned commit (decisions §6). "Zed key" names the `style` key; `ansi.*` means `terminal.ansi.*`.

## Required tokens (the test list)

Write every row in both themes, with the hex as shown (alpha digits included). Rows marked † are written even though the kit fallback is close, because they have no derived fallback or because Zed differs from the derivation.

| Kit key | One Dark | One Light | Zed key / reason |
|---|---|---|---|
| `background` † | `#282c33` | `#fafafa` | `editor.background` (content area; Zed's `background` is the chrome) |
| `foreground` † | `#dce0e5` | `#242529` | `text` |
| `muted.background` † | `#2f343e` | `#ebebec` | `surface.background` |
| `muted.foreground` | `#a9afbc` | `#58585a` | `text.muted` |
| `border` † | `#464b57` | `#c9c9ca` | `border` |
| `ring` | `#47679e` | `#7d82e8` | `border.focused` |
| `primary.background` † | `#74ade8` | `#5c78e2` | `text.accent` |
| `primary.foreground` | `#282c33` | `#fafafa` | editor background on the accent |
| `secondary.background` † | `#2e343e` | `#ebebec` | `element.background` |
| `secondary.hover.background` | `#363c46` | `#dfdfe0` | `element.hover` |
| `secondary.active.background` | `#454a56` | `#cacaca` | `element.active` |
| `accent.background` | `#363c46` | `#dfdfe0` | `ghost_element.hover` (cluster switcher active row, dock drag-over) |
| `popover.background` | `#2f343e` | `#ebebec` | `elevated_surface.background` |
| `selection.background` | `#74ade83d` | `#5c78e23d` | `players[0].selection` (also `table_active` via the 0043 patch) |
| `caret` | `#74ade8` | `#5c78e2` | `players[0].cursor` (also the terminal cursor) |
| `drop_target.background` | `#83899480` | `#7e808780` | `drop_target.background` |
| `title_bar.background` | `#3b414d` | `#dcdcdd` | `title_bar.background` (status bar falls back to it, as in Zed) |
| `sidebar.background` | `#2f343e` | `#ebebec` | `panel.background` |
| `sidebar.accent.background` | `#454a56` | `#cacaca` | `element.selected` |
| `tab_bar.background` | `#2f343e` | `#ebebec` | `tab_bar.background` |
| `tab.background` | `#2f343e` | `#ebebec` | `tab.inactive_background` |
| `tab.active.background` | `#282c33` | `#fafafa` | `tab.active_background` |
| `tab.foreground` | `#a9afbc` | `#58585a` | `text.muted` |
| `table.head.background` | `#2f343e` | `#ebebec` | `panel.background` (Zed has no table) |
| `table.row.border` | `#363c46` | `#dfdfe0` | `border.variant` |
| `list.hover.background` | `#363c46` | `#dfdfe0` | `element.hover` (`table.hover` falls back to it) |
| `list.active.background` | `#74ade81a` | `#5c78e21a` | `editor.document_highlight.read_background` |
| `list.active.border` | `#74ade8` | `#5c78e2` | `text.accent` |
| `scrollbar.thumb.background` | `#c8ccd44c` | `#383a414c` | `scrollbar.thumb.background` |
| `success.background` | `#a1c181` | `#669f59` | `success` |
| `warning.background` | `#dec184` | `#a48819` | `warning` |
| `danger.background` | `#d07277` | `#d36151` | `error` |
| `info.background` | `#74ade8` | `#5c78e2` | `info` |
| `success/warning/danger/info.foreground` (4 keys) | `#282c33` | `#fafafa` | text on a solid fill |
| `base.red` † / `base.red.light` | `#e06c75` / `#ea858b` | `#de3e35` / `#e06c75` | `ansi.red` / dark `ansi.bright_red`, light: One Dark `ansi.red` |
| `base.green` † / `.light` | `#98c379` / `#aad581` | `#3f953a` / `#98c379` | the same pattern |
| `base.yellow` † / `.light` | `#e5c07b` / `#ffd885` | `#d2b67c` / `#e5c07b` | the same pattern |
| `base.blue` † / `.light` | `#61afef` / `#85c1ff` | `#2f5af3` / `#61afef` | the same pattern |
| `base.magenta` † / `.light` | `#c678dd` / `#d398eb` | `#950095` / `#c678dd` | the same pattern |
| `base.cyan` † / `.light` | `#56b6c2` / `#6ed5de` | `#0997b3` / `#56b6c2` | the same pattern |
| `chart.1` … `chart.5` | `#74ade8` `#6eb4bf` `#b477cf` `#a1c181` `#bf956a` | `#5c78e2` `#3882b7` `#a449ab` `#649f57` `#ad6e25` | syntax `attribute`, `type`, `keyword`, `string`, `number` |
| `chart.bullish` / `chart.bearish` | `#a1c181` / `#d07277` | `#669f59` / `#d36151` | `success` / `error` |

Why One Light's `base.*.light` uses One Dark's ANSI values: One Light's bright ANSI colours equal its normal ones. Topology tells Pod from Workload, Secret from ConfigMap, and so on, through `X` vs `X.light`, so the two must differ. The Atom One pastels are the canonical lighter forms. This mirrors Default light, where `-600` / `-400` pairs are used.

Left to the kit's derived fallbacks (they match Zed or there is no Zed equivalent): `input.border`, `link*`, `drag.border`, `secondary/accent/popover/sidebar/tab.active/group_box/description_list foregrounds`, `sidebar.border`, `sidebar.primary*`, `status_bar*`, `title_bar.border`, `list`, `list.even`, `list.head`, `table*` (others), `button.*`, `*.hover` / `*.active` of the status colours, `progress_bar`, `slider*`, `switch*`, `skeleton`, `accordion`, `chart.grid`, `scrollbar` (track), `scrollbar.thumb.hover`, `tab_bar.segmented`, and `window.border`. `overlay` is the kit default (`#00000033` / `#0000000d`); Zed has none.

## `highlight` (YAML views and editors)

Copy from Zed's `style`, per theme:
- The 7 keys `editor.foreground`, `editor.background`, `editor.gutter.background`, `editor.active_line.background`, `editor.line_number`, `editor.active_line_number`, `editor.invisible`.
- The 15 status keys `error|warning|info|success|hint` with ` `, `.background`, `.border`.
- The whole `syntax` object (46 entries).

Drop `players`, `accents`, `terminal.*`, `version_control.*`, and everything else. The kit ignores unknown keys, but extra keys are noise. Generate the block with a throwaway script in `.tmp/`, not by hand. Fetch the source with `curl -sSL -o .tmp/zed/one.json https://raw.githubusercontent.com/zed-industries/zed/a3f6ef252b6de19d22a1223952fc335253163642/assets/themes/one/one.json`.

## Where each derived colour lands (checked)

| Consumer | Tokens | One Dark | One Light |
|---|---|---|---|
| `status_tone::tone_color` (dark: raw; light: mixed 0.6 / 0.7 toward fg) | success, warning, danger, info | on bg: 7.0, 8.1, 4.2, 5.9 :1 | after the mix: 5.6, 5.1, 6.4, 6.5 :1 |
| `muted_foreground` text (≈ 240 uses) | — | 6.4 :1 on bg, 4.7 on the title bar | 6.8 :1 on bg |
| Env badge `Tag::custom(color, background, color)`: PROD = danger, STG = warning | — | 4.2 / 8.1 :1 (Default dark PROD: 7.2) | 3.6 / 3.3 :1 (Default light: 3.8 / 1.9) |
| Primary button text | primary.foreground on primary | 5.9 :1 | 3.8 :1 |
| Topology `CanvasColors` | base.* hues, muted, border, ring, warn/bad tones | 8 distinct hues; red and yellow stay reserved | the same |
| `usage_chart` series 0 / 1, `log_legend` pods (`readable_chart_color`) | chart.1, chart.bullish; chart.1–5 | blue / green; 5 distinct hues | the same |
| Terminal palette (`terminal_session::terminal_palette`) | muted, base.*, base.*.light, fg, bg, caret | Zed's own ANSI colours | ANSI yellow is 1.9 :1 on `#fafafa`, as in Zed itself (accepted) |
| `log_rows` / `log_volume` / terminal selection | selection | 24 % accent | 24 % accent |
| Cluster colours (W2 swatches) | danger, warning, info, magenta, cyan, muted_foreground | as above | Purple = `#950095` (dark, very legible) |

Gaps in the kit schema: none needed. Zed keys with no kit token (`border.variant`, `text.placeholder`, `ghost_element.*`, `toolbar.background`, `editor.subheader`) are folded into the rows above or dropped.
