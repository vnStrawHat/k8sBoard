use super::*;
use crate::cluster_form::cluster_groups;
use crate::cluster_registry::{ClusterEntry, ClusterRef, ClusterRegistry};

const TOKEN: &str = "fixture-token-value";

fn kubeconfig(source: &str, context: &str, cluster: &str, user: &str) -> Kubeconfig {
    let yaml = format!(
        "clusters:\n  - name: {cluster}\n    cluster: {{ server: 'https://u:p@127.0.0.1:1/x' }}\nusers:\n  - name: {user}\n    user: {{ token: {TOKEN} }}\ncontexts:\n  - name: {context}\n    context: {{ cluster: {cluster}, user: {user} }}\n"
    );
    Kubeconfig::parse(&yaml, Path::new(source)).expect("fixture parses")
}

fn rows_of(kubeconfigs: &[&Kubeconfig], registry: &ClusterRegistry) -> Vec<ClusterRow> {
    cluster_groups(kubeconfigs, registry, |_| false, |_| None, None)
        .into_iter()
        .flat_map(|group| group.rows)
        .collect()
}

fn messages(collisions: &[NameCollision]) -> Vec<String> {
    collisions.iter().map(ToString::to_string).collect()
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("k8sboard-0025-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

#[test]
fn collisions_report_context_display_name_cluster_and_user() {
    let existing = kubeconfig("old.yaml", "prod", "old-cluster", "old-user");
    let candidate = kubeconfig("new.yaml", "prod", "old-cluster", "old-user");
    let rows = rows_of(&[&existing], &ClusterRegistry::default());
    let collisions = name_collisions(&candidate, &rows, None, &[&existing]);
    assert_eq!(
        messages(&collisions),
        [
            "Context 'prod' also exists in old.yaml. Both are listed; the switcher adds the file name.",
            "Cluster 'old-cluster' also exists in old.yaml. k8sBoard reads this file on its own, so nothing is replaced; kubectl would use only the first one if both were in KUBECONFIG.",
            "User 'old-user' also exists in old.yaml. k8sBoard reads this file on its own, so nothing is replaced; kubectl would use only the first one if both were in KUBECONFIG.",
        ]
    );

    // A cluster shown as "Shown" matches a candidate context named "shown".
    let renamed = ClusterRegistry {
        clusters: vec![ClusterEntry {
            cluster: ClusterRef {
                kubeconfig: PathBuf::from("old.yaml"),
                context: "prod".to_owned(),
            },
            display_name: Some("Shown".to_owned()),
            environment: None,
            read_only: None,
            confirm: None,
            default_namespace: None,
            allow_node_shell: None,
            debug_image: None,
            node_shell_namespace: None,
            color: None,
            proxy: None,
        }],
        ..ClusterRegistry::default()
    };
    let rows = rows_of(&[&existing], &renamed);
    let candidate = kubeconfig("new.yaml", "shown", "other-cluster", "other-user");
    assert_eq!(
        messages(&name_collisions(&candidate, &rows, None, &[])),
        [
            "Context 'shown' matches the name shown for a cluster from old.yaml. Both are listed; rename one in Settings."
        ]
    );
}

#[test]
fn context_collision_names_its_source_file() {
    // The collision names the file that defines the context, not the candidate.
    let first = kubeconfig("a.yaml", "one", "c1", "u1");
    let second = kubeconfig("b.yaml", "two", "c2", "u2");
    let rows = rows_of(&[&first, &second], &ClusterRegistry::default());
    let candidate = kubeconfig("new.yaml", "two", "c9", "u9");
    let collisions = name_collisions(&candidate, &rows, None, &[]);
    assert_eq!(
        collisions,
        [NameCollision {
            kind: EntryKind::Context,
            name: "two".to_owned(),
            place: CollisionPlace::File(PathBuf::from("b.yaml")),
        }]
    );
}

#[test]
fn chain_collision_says_kubeconfig_chain() {
    let chain = kubeconfig("chain.yaml", "one", "shared-cluster", "shared-user");
    let candidate = kubeconfig("new.yaml", "two", "shared-cluster", "shared-user");
    let collisions = name_collisions(&candidate, &[], Some(&chain), &[]);
    let texts = messages(&collisions);
    assert_eq!(texts.len(), 2);
    assert!(
        texts[0].starts_with("Cluster 'shared-cluster' also exists in your KUBECONFIG chain."),
        "{texts:?}"
    );
    assert!(
        texts[1].starts_with("User 'shared-user' also exists in your KUBECONFIG chain."),
        "{texts:?}"
    );
}

#[test]
fn no_collision_for_distinct_names() {
    let existing = kubeconfig("old.yaml", "one", "c1", "u1");
    let candidate = kubeconfig("new.yaml", "two", "c2", "u2");
    let rows = rows_of(&[&existing], &ClusterRegistry::default());
    assert!(name_collisions(&candidate, &rows, Some(&existing), &[&existing]).is_empty());
}

#[test]
fn already_added_file_is_rejected() {
    let registered = [PathBuf::from("a.yaml")];
    let error = check_new_file(Path::new("a.yaml"), &registered, false).expect_err("registered");
    assert_eq!(error.to_string(), "This file is already added.");
    assert!(check_new_file(Path::new("b.yaml"), &registered, false).is_ok());
}

#[test]
fn chain_file_is_rejected() {
    let error = check_new_file(Path::new("a.yaml"), &[], true).expect_err("chain file");
    assert_eq!(
        error.to_string(),
        "This file is already loaded from KUBECONFIG or --kubeconfig."
    );
}

#[test]
fn preview_without_contexts_is_an_error() {
    let empty = Kubeconfig::parse("clusters: []\n", Path::new("x.yaml")).expect("parses");
    let error = import_preview(
        ImportSource::File(PathBuf::from("x.yaml")),
        &empty,
        &[],
        None,
        &[],
    )
    .expect_err("no contexts");
    assert_eq!(error.to_string(), "This kubeconfig has no contexts.");
}

#[test]
fn preview_debug_has_no_credentials() {
    let candidate = kubeconfig("new.yaml", "prod", "c", "u");
    let preview = import_preview(
        ImportSource::Pasted {
            target: PathBuf::from("config/kubeconfigs/prod.yaml"),
        },
        &candidate,
        &[],
        None,
        &[],
    )
    .expect("a preview");
    let text = format!("{preview:?}");
    assert!(!text.contains(TOKEN), "{text}");
    // Userinfo of the server URL is dropped too.
    assert!(!text.contains("u:p@"), "{text}");
    assert!(text.contains("https://127.0.0.1:1"), "{text}");
    assert_eq!(preview.contexts[0].environment, Environment::Production);
}

#[test]
fn clipboard_over_one_mebibyte_is_rejected() {
    assert!(matches!(
        check_clipboard_text(None),
        Err(ImportError::NoClipboardText)
    ));
    let big = "x".repeat(MAX_CLIPBOARD_BYTES + 1);
    assert!(matches!(
        check_clipboard_text(Some(&big)),
        Err(ImportError::ClipboardTooLarge)
    ));
    let limit = "x".repeat(MAX_CLIPBOARD_BYTES);
    assert!(check_clipboard_text(Some(&limit)).is_ok());
}

#[test]
fn pasted_file_path_slugs_the_first_context() {
    let dir = temp_dir("slug");
    let long = "arn:aws:eks:eu-west-1:123456789012:cluster/Prod-EU-1";
    let path = pasted_file_path(&dir, Some(long));
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .expect("name");
    assert!(
        name.starts_with("arn-aws-eks-eu-west-1-123456789012-"),
        "{name}"
    );
    assert_eq!(name.len(), MAX_SLUG_CHARS + ".yaml".len());
    assert_eq!(path.parent(), Some(dir.join(PASTED_DIR).as_path()));
    let none = pasted_file_path(&dir, None);
    assert!(none.ends_with("pasted.yaml"), "{none:?}");
}

#[test]
fn pasted_file_path_skips_existing_names() {
    let dir = temp_dir("skip");
    std::fs::create_dir_all(dir.join(PASTED_DIR)).expect("folder");
    for name in ["prod.yaml", "prod-2.yaml"] {
        std::fs::write(dir.join(PASTED_DIR).join(name), "x").expect("file");
    }
    assert!(pasted_file_path(&dir, Some("prod")).ends_with("prod-3.yaml"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn write_pasted_never_overwrites() {
    let dir = temp_dir("overwrite");
    let first = write_pasted_kubeconfig(&dir, Some("prod"), "first").expect("first write");
    let second = write_pasted_kubeconfig(&dir, Some("prod"), "second").expect("second write");
    assert_ne!(first, second);
    assert_eq!(std::fs::read_to_string(&first).expect("read"), "first");
    assert_eq!(std::fs::read_to_string(&second).expect("read"), "second");
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn write_pasted_sets_owner_only_mode() {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = temp_dir("mode");
    let path = write_pasted_kubeconfig(&dir, Some("prod"), "x").expect("write");
    let file_mode = std::fs::metadata(&path).expect("file").permissions().mode();
    let folder_mode = std::fs::metadata(dir.join(PASTED_DIR))
        .expect("folder")
        .permissions()
        .mode();
    assert_eq!(file_mode & 0o077, 0);
    assert_eq!(folder_mode & 0o777, 0o700);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn app_owned_only_under_kubeconfigs_folder() {
    let dir = Path::new("config");
    assert!(is_app_owned(&dir.join(PASTED_DIR).join("a.yaml"), dir));
    assert!(!is_app_owned(&dir.join("a.yaml"), dir));
    assert!(!is_app_owned(
        &dir.join(PASTED_DIR).join("sub").join("a.yaml"),
        dir
    ));
    assert!(!is_app_owned(Path::new("other/kubeconfigs/a.yaml"), dir));
    assert!(!is_app_owned(Path::new("a.yaml"), dir));
}

#[test]
fn pasted_file_names_have_the_shape_we_give_them() {
    for name in [
        "prod.yaml",
        "prod-2.yaml",
        "arn-aws-eks.eu-1_x.yaml",
        "pasted.yaml",
    ] {
        assert!(is_pasted_file_name(name), "{name}");
    }
    let too_long = format!("{}.yaml", "a".repeat(MAX_SLUG_CHARS + 11));
    for name in [
        "My Config.yaml",
        "UPPER.yaml",
        "prod.yml",
        "prod",
        ".yaml",
        "a/b.yaml",
        too_long.as_str(),
    ] {
        assert!(!is_pasted_file_name(name), "{name}");
    }
}

#[test]
fn a_file_with_another_name_is_not_app_owned() {
    let dir = Path::new("config");
    assert!(!is_app_owned(
        &dir.join(PASTED_DIR).join("My Config.yaml"),
        dir
    ));
    assert!(!is_app_owned(&dir.join(PASTED_DIR).join("notes.txt"), dir));
}

#[test]
fn missing_config_dir_error_says_why() {
    assert_eq!(
        ImportError::NoConfigDir.to_string(),
        "Settings are not saved this session, so a pasted kubeconfig cannot be stored."
    );
}
