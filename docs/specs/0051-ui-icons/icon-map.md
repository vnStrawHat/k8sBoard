# 0051 · Icon map

[Back to index](README.md). Names are `gpui_kit::assets::IconName` variants (Lucide file `kebab-case` → `PascalCase`, e.g. `undo-2` → `Undo2`). All exist in `gpui-kit-assets` 0.7.0. "=" means unchanged from today.

## Kinds (`KindSpec.icon`, `POD_ICON`, `NODE_ICON` in `resource_kind.rs`)

| Kind | Icon | | Kind | Icon |
|---|---|---|---|---|
| Pods (`POD_ICON`) | `Box` | | ConfigMaps | `FileCog` |
| Nodes (`NODE_ICON`) | `Server` | | Secrets | `KeyRound` |
| Namespaces | `Folder` | | HorizontalPodAutoscalers | `Gauge` |
| Events | `Bell` | | ResourceQuotas | `ChartPie` |
| Deployments | `Layers` | | PodDisruptionBudgets | `ShieldCheck` |
| StatefulSets | `Database` | | PersistentVolumeClaims | `Ticket` (a claim) |
| DaemonSets | `Radio` (runs on every node) | | PersistentVolumes | `HardDrive` |
| ReplicaSets | `Grid2x2` | | StorageClasses | `Archive` |
| Jobs | `BriefcaseBusiness` | | ServiceAccounts | `Bot` |
| CronJobs | `CalendarClock` | | Roles | `ScrollText` |
| Services | `Network` | | ClusterRoles | `BookKey` |
| Ingresses | `Globe` | | RoleBindings | `Link` |
| NetworkPolicies | `BrickWallShield` | | ClusterRoleBindings | `Cable` |
| HelmReleases | `ShipWheel` | | Crds | `Blocks` |
| Custom(_) | `Puzzle` | | | |

Rule: the 26 built-in kinds, `POD_ICON`, and `NODE_ICON` are pairwise distinct (test) and not near-twins at 12 px (no `X` / `X2` pairs, no Server / ServerCog, no Database / Cylinder). The custom spec in `custom_kind.rs` sets `Puzzle`. `ResourceKind::icon` reads `spec().icon`; `TopologyKind::icon` = `resource_kind().map_or(POD_ICON, ResourceKind::icon)`.

## Screens and sidebar (`navigation.rs`)

| `screen_icon(Screen)` | Icon | Sidebar group (`NavigationSection.icon`) | Icon |
|---|---|---|---|
| Overview | `LayoutDashboard` | Cluster | `Cloud` |
| Issues | `Flag` (= title-bar issues button) | Workloads | `Boxes` |
| Topology | `Waypoints` | Network | `Network` |
| PortForwarding | `ArrowLeftRight` (= drawer Forward) | Config | `SlidersHorizontal` |
| Pods / Nodes | `POD_ICON` / `NODE_ICON` | Storage | `HardDrive` |
| Kind(k) | `k.icon()` | Access Control | `ShieldUser` |
| | | Helm | `ShipWheel` |
| | | Custom Resources | `Puzzle` |

Screen header (`workspace.rs`, before the title) and palette rows (`command_palette.rs::row_icon`): header and Screen rows → `screen_icon`; Resource pod/node/kind → `POD_ICON` / `NODE_ICON` / `kind.icon()`; Namespace → `Folder`; Cluster → `Building2` (= switcher command). `RowIcon` is deleted; `row_icon` returns `IconName`.

## Row actions (`RowAction::icon`, moved to `resource_actions.rs`)

| Action | Icon | Action | Icon |
|---|---|---|---|
| ViewLogs | `FileText` = | RestartRollout, RestartPod | `RotateCw` = |
| ViewYaml | `FileCode` (was `Eye`; Eye is Reveal) | RenewCertificate | `RefreshCw` |
| CopyName | `Copy` = | EvictPod | `LogOut` = |
| OpenShell | `SquareTerminal` = | Scale, EditHpaRange | `ChevronsUpDown` = |
| Attach | `Plug` (was `SquareTerminal`) | Delete | `Trash` (was kit `Delete`) |
| DebugContainer | `Bug` | | |
| PortForward | `ArrowLeftRight` (was `Network`, now Services) | PauseRollout | `Pause` = |
| Cordon | `Ban` = | RollBack | `Undo2` = |
| Drain | `ArrowDown` = (dock drain tab) | SuspendCronJob | `Timer` = |
| EditTaints, EditLabels | `Tag` | TriggerCronJob | `Play` = |
| EditYaml | `FilePenLine` | RerunJob | `Repeat` = |
| EditValues | `Pencil` | ExpandClaim | `HardDriveUpload` |
| | | SetDefaultStorageClass | `Star` = |

## Unkeyed menu items (set at their helper)

| Item(s) | Icon |
|---|---|
| Who can… | `UserSearch` |
| Check permissions | `ShieldQuestionMark` |
| Test traffic… | `Activity` |
| Go to owner / target / pod / claim / role / object; Issues `Open <kind>` | `CornerDownRight` |
| Show in Topology | `Waypoints` |
| Show remaining resources, Show selected pods, Browse instances, View pods on node | `List` |
| View values / View manifest (Helm) | `FileText` / `FileCode` |
| Open URL, Open in browser | `ExternalLink` |
| Set as default namespace | `Star` |
| Copy kubectl command | `Terminal` |
| Copy image, Copy message, Copy object name, Copy value ▸, Copy local address | `Copy` |
| Filter similar | `ListFilter` |
| Reveal values (30s) | `Eye` |
| Open node shell | `SquareTerminal` (keyed as OpenShell) |
| Port forward menu: Stop forward / Start, Restart / Change local port… / Save as preset / Remove from list, Remove preset… | `CircleStop` / `Play`, `RotateCw` / `Pencil` / `Star` / `X` |
| Container menu: View logs, Open shell, Attach, Copy image | as the row actions; `Copy` |
| Submenus View logs ▸ (`resource_actions.rs` ~1511), Open shell ▸ (~2822), Port forward ▸ (`port_forward_menu.rs` ~283) | through `row_keyed` (`.icon()` applies to a Submenu; `.action()` is a no-op there) |
| Any `.checked()` item | none (the icon would replace the check mark) |

## Toolbar buttons

| Button (file) | Icon | Label change |
|---|---|---|
| + Filter (`filter_bar.rs`) | `ListFilterPlus` | `Filter` (the `+` becomes the icon) |
| Columns (`filter_bar.rs`) | `Columns3` | — |
| Clear filters (`filter_bar.rs`) | `X` | — |
| Namespace: all trigger (~140) and each picked chip (~160) (`filter_bar.rs`) | `Folder` | — |
| Fit (`workspace.rs` Topology header) | `Maximize` (= canvas fit) | — |
| New (`workspace.rs`) | `Plus` | — |
| Header Who can… / Check permissions / Test traffic / Renew (`workspace.rs` ~520–619) | `UserSearch` / `ShieldQuestionMark` / `Activity` / `RefreshCw` (= menu items) | — |
| + New forward / Stop all (`port_forward_page.rs`) | `Plus` / `CircleStop` | `New forward`; `EMPTY_TEXT` "…or + New forward." → "…or New forward." |
| + Add (`node_editor.rs`, 2) | `Plus` | `Add` |
| Refresh (`helm_release_view.rs`, `permissions_view.rs`, `who_can_view.rs` ~699) | `RefreshCw` | — |
| Reveal, Reveal all (30s) / Hide / Copy (`secret_values.rs`, `helm_release_view.rs`) | `Eye` / `EyeOff` / `Copy` | — |

## Settings nav (`SettingsPage::icon`)

General `Settings` (= title-bar gear) · Clusters `Building2` · Appearance `Palette` · Keyboard Shortcuts `Keyboard` · Safety `ShieldCheck` · Terminal & Shell `SquareTerminal` · Logs `FileText` · About `Info` · Metrics (0048, if merged first) `ChartLine`.
