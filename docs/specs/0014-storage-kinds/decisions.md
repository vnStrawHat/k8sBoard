# 0014 · Decisions

[Back to index](README.md). Architect defaults; the user asked not to stop for questions. 0005, 0012, and 0013 decisions apply unless replaced here.

## Data

| # | Decision | Rationale |
|---|---|---|
| 1 | One module and one watch per kind: `watch_persistent_volume_claims(scope)`, `watch_persistent_volumes()`, `watch_storage_classes()` | 0005 decision 5; PVs and StorageClasses are cluster-scoped like Namespaces |
| 2 | Phases stay API text (`Bound`, `Pending`, `Lost`, `Available`, `Released`, `Failed`) plus `is_terminating`; tones live in the app | the values are read, not derived; no extra enum to keep in sync |
| 3 | PV source is `VolumeBackend` (CSI, NFS, HostPath, Local, else the in-tree field name). CSI `volumeAttributes`, every `*SecretRef`, and flexVolume `options` are never copied; `mountOptions` are kept on both PVs and StorageClasses, with the value of a `key=value` option hidden (`password=<hidden>`) when the key passes `is_secret_parameter` (CIFS/SMB); the YAML view masks the same options | attributes and options are free-form and may hold credentials (C1); driver and handle are what users look up |
| 4 | **One annotation exception**: `storageclass.kubernetes.io/is-default-class` (and its beta key) is read once into `is_default`; no other annotation is read or kept | the default class exists only as this annotation; it holds `true`/`false`, never a secret. Approved by the user; the "never read annotations" rule holds otherwise |
| 5 | StorageClass parameter values are hidden when the normalized key contains `password`, `passwd`, `token`, `credential`, `secretkey`, `accesskey`, `userkey` (which covers `restuserkey`), or `privatekey`; `*-secret-name` and `*-secret-namespace` references stay visible; the YAML view masks the same keys | some provisioners take plaintext credentials as parameters (e.g. glusterfs `restuserkey`); a bare `key` would hide harmless ids such as `kmsKeyId`; a name list is a heuristic with a known ceiling | some provisioners take plaintext credentials as parameters (e.g. glusterfs `restuserkey`); a name list is a heuristic with a known ceiling |
| 6 | 3 list `AccessCheck`s: PVCs namespaced, PVs and StorageClasses cluster-scoped | 0005 decision 8; denied kinds disabled by `kind_availability` |

## App

| # | Decision | Rationale |
|---|---|---|
| 7 | PVC **Used** is a cross-list cell joined on the main thread from `KubeletHistory::pvc_usage`; the join also runs after each kubelet round while PVCs is shown | values must sort and filter; the kubelet store lives on the main thread (0011) |
| 8 | Used tone = 0010 `usage_tone` (Warn ≥ 80 %, Bad ≥ 90 %) | W7 note "warn above 80 %"; 0011 consumer note; one bar rule |
| 9 | Kubelet demand: a PVC drawer adds `KubeletSubject::Claim`, whose nodes are those of the non-Done pods mounting the claim. The table adds no demand | ≤ 10 Ready nodes already poll every node (0011 decision 11); on bigger clusters a table-wide demand would poll every node every 15 s |
| 10 | **Mounted by** is paint-time Live content from live pods (`VolumeSource::PersistentVolumeClaim` mounts) | no extra watch; W7 "Mounted by" |
| 11 | StorageClass **PVs** count comes from a PV **companion** watch alive only while StorageClasses is the explorer kind | the column needs every class; one cluster-scoped watch; same pattern as 0012 endpoint slices |
| 12 | The PV companion is a new `CompanionLists::PersistentVolumes` variant of the general companion that 0012 already defines; nothing is renamed | 0012 was amended so later kinds only add variants |
| 13 | Links: PVC → PV and → StorageClass; PV → claim and → StorageClass; Mounted by → Pod. Menus: PVCs **Go to pod** (first mounting pod by name), PVs **Go to claim** | W7 menus and "related objects"; `go_to_item` from 0013 |
| 14 | PV **RELEASED** box (Warn) explains Retain vs Delete; **RECLAIM FAILED** (Bad) for `Failed`; PVC **VOLUME LOST** (Bad) for `Lost` | W7 note "Released has a clean-up hint, Retain keeps the data"; the Clean up action itself is 0033 |
| 15 | Badges: PVCs `Pc`, PVs `Pv` (W7 uses `Pv` for both) | the drawer header must tell a claim from a volume |
| 16 | Status tones: Bound Ok, Available Ok, Pending Warn, Released Done, Lost and Failed Bad, terminating Info; non-default StorageClass Done | Info and Warn count as "Unhealthy" in the 0009 chip, so idle states use Ok or Done |
| 17 | Columns follow W7; StorageClasses add **PVs** and keep **Age** last | W7 drawer meta "58 volumes"; 0013 decision 20 |
| 18 | Screenshot screens: `<plural>` and `<plural>-drawer` for the three kinds; `--filter` picks a Bound PVC; empty states when UAT has none | 0005 decision 25 |
| 19 | PV Source shows **Node affinity** chips (all required terms, `matchExpressions` and `matchFields`) instead of W7's single "Zone" row | the zone is one affinity term among others, under driver-specific keys (`topology.kubernetes.io/zone`, `topology.ebs.csi.aws.com/zone`, …); chips show it without guessing the key |
| 20 | PVC status: `Resizing` true → Info "Resizing"; `FileSystemResizePending` true → Info "Resize pending" (the drawer Status row adds "; restart the pod"); in the Conditions rows a true `Resizing` or `FileSystemResizePending` reads Info, not Ok | the second needs a pod restart on older drivers; both are transitional, so neither reads as a healthy state |
| 21 | The W7 drawer `meta` line is not rendered (0005 subtitle); no list-level top buttons | the subtitle stays one format; Expand and Set default are 0032 |

## Known ceilings

- Parameter masking is a key-name heuristic (decision 5).
- Used and Mounted by cover filesystem-mode claims only: kubelet stats report no usage for `volumeMode: Block`, and block devices are `volumeDevices`, which pod summaries do not keep; Mounted by also misses claims referenced only by `spec.volumes` with no container mount.
- The Used join reruns every kubelet round (15 s) while PVCs is shown: O(rows) map lookups.
- **Shared filesystems:** a hostPath or local volume has no quota of its own, so its kubelet stats describe the node's disk. `is_shared_filesystem` detects it (the kubelet capacity is larger than the claim's capacity). Such a claim shows no Used cell and no status raise; the drawer keeps the bar, labelled "Node filesystem", with the note "Shared with the node: the claim has no quota of its own". A volume that is exactly as large as the claim is trusted. A shared filesystem of the same size as the claim, or a claim whose capacity is not readable, is not detected.
- **PV YAML is not masked for `csi.volumeAttributes` and flexVolume `options`.** The summaries never copy them (decision 3), but the YAML view shows the object as the API returns it, like every other kind. The C1 scope is therefore: summaries and rows are clean; YAML masking covers Secret data, StorageClass parameters, env literals, and manifest annotations. CSI `volume_context` is not meant to carry secrets (drivers take them from secret references), so no masking rule is added.

## UAT probe (step 1)

| Check | Result |
|---|---|
| `list persistentvolumeclaims` / watch line / count | allowed / 29 items / count 29 |
| `list persistentvolumes` / watch line / count | allowed / 57 items / count 57 |
| `list storageclasses` / watch line / count | allowed / 2 items / count 2 |

UAT note: neither UAT class is the default, so the ★ and the Default status are covered by unit tests only (`default_class_star_and_status`).

AC4 credential check: the 0001 AC7 script was not run, because auto mode blocks token extraction from the kubeconfig. It is covered instead by `kubeconfig_tests::debug_output_never_contains_credentials` and `tests/connection.rs::connection_debug_hides_credentials`, and by a code audit: the new probe lines print only counts and error summaries through `tally_source`, and the new summaries never copy CSI attributes, secret references, or secret-like StorageClass parameter values (`csi_attributes_and_secret_refs_are_not_copied`, `secret_like_parameters_are_hidden`, `storage_class_parameters_are_masked_in_yaml`).
