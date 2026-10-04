use std::cell::Cell;
use std::path::PathBuf;

use super::*;

const KUBECONFIG: &str = "apiVersion: v1\nkind: Config\nclusters:\n  - name: c\n    cluster: { server: \"https://127.0.0.1:1\" }\ncontexts:\n  - name: ctx\n    context: { cluster: c }\n";

/// A fresh folder under the temp dir, removed at the start and by `finish`.
fn folder(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("k8sboard-0043-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp folder");
    dir
}

fn finish(dir: &Path) {
    let _ = std::fs::remove_dir_all(dir);
}

fn write(dir: &Path, name: &str, text: &str) {
    std::fs::write(dir.join(name), text).expect("write a file");
}

fn names(scan: &FolderScan) -> Vec<String> {
    scan.files
        .iter()
        .filter_map(|file| file.path.file_name()?.to_str().map(str::to_owned))
        .collect()
}

#[test]
fn scan_keeps_kubeconfig_candidates() {
    let dir = folder("scan-candidates");
    for name in [
        "a.yaml",
        "b.yml",
        "config",
        "c.conf",
        "d.kubeconfig",
        "E.YAML",
    ] {
        write(&dir, name, KUBECONFIG);
    }
    for name in [".hidden.yaml", ".config", "x.txt", "notes.md", "x.yaml.bak"] {
        write(&dir, name, KUBECONFIG);
    }
    std::fs::create_dir(dir.join("sub.yaml")).expect("a folder");
    // One byte over the cap.
    std::fs::write(
        dir.join("big.yaml"),
        vec![b'#'; MAX_FILE_BYTES as usize + 1],
    )
    .expect("big");
    // Exactly the cap stays.
    std::fs::write(dir.join("edge.yaml"), vec![b'#'; MAX_FILE_BYTES as usize]).expect("edge");
    let scan = scan_folder(&dir).expect("the folder lists");
    let mut found = names(&scan);
    found.sort();
    assert_eq!(
        found,
        [
            "E.YAML",
            "a.yaml",
            "b.yml",
            "c.conf",
            "config",
            "d.kubeconfig",
            "edge.yaml"
        ]
    );
    assert_eq!(scan.skipped_over_cap, 0);
    finish(&dir);
}

#[test]
fn scan_caps_at_fifty_by_name() {
    let dir = folder("scan-cap");
    for index in 0..52 {
        write(&dir, &format!("f{index:02}.yaml"), KUBECONFIG);
    }
    let scan = scan_folder(&dir).expect("the folder lists");
    assert_eq!(scan.files.len(), MAX_FOLDER_FILES);
    assert_eq!(scan.skipped_over_cap, 2);
    let found = names(&scan);
    assert_eq!(found.first().map(String::as_str), Some("f00.yaml"));
    assert_eq!(found.last().map(String::as_str), Some("f49.yaml"));
    finish(&dir);
}

#[test]
fn diff_scan_reports_added_changed_removed() {
    let file = |name: &str, len: u64, seconds: u64| FolderFile {
        path: PathBuf::from(name),
        len,
        modified: Some(SystemTime::UNIX_EPOCH + Duration::from_secs(seconds)),
    };
    let before = [
        file("keep", 1, 1),
        file("grow", 1, 1),
        file("touch", 1, 1),
        file("gone", 1, 1),
    ];
    let after = [
        file("keep", 1, 1),
        file("grow", 2, 1),
        file("touch", 1, 2),
        file("new", 1, 1),
    ];
    let change = diff_scan(&before, &after);
    assert_eq!(change.added, [PathBuf::from("new")]);
    assert_eq!(
        change.changed,
        [PathBuf::from("grow"), PathBuf::from("touch")]
    );
    assert_eq!(change.removed, [PathBuf::from("gone")]);
    assert_eq!(diff_scan(&after, &after), FolderChange::default());
}

#[test]
fn missing_folder_is_an_error() {
    let dir = folder("scan-missing");
    finish(&dir);
    let error = scan_folder(&dir).expect_err("a removed folder cannot be listed");
    assert_eq!(error.kind(), io::ErrorKind::NotFound);
}

#[test]
fn oversized_file_is_refused_by_the_bounded_read() {
    let dir = folder("bounded");
    write(&dir, "ok.yaml", KUBECONFIG);
    // The file grew past the cap after the scan.
    std::fs::write(
        dir.join("grown.yaml"),
        vec![b'#'; MAX_FILE_BYTES as usize + 1],
    )
    .expect("grown");
    let error = load_folder_file(&dir.join("grown.yaml")).expect_err("over the cap");
    assert!(matches!(error, KubeconfigError::TooLarge { .. }));
    assert!(load_folder_file(&dir.join("ok.yaml")).is_ok());
    finish(&dir);
}

/// A reader that never ends and counts what it handed out.
struct Endless<'a>(&'a Cell<u64>);

impl io::Read for Endless<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        buffer.fill(b'#');
        self.0.set(self.0.get() + buffer.len() as u64);
        Ok(buffer.len())
    }
}

#[test]
fn the_bounded_read_stops_at_one_byte_over_the_cap() {
    let served = Cell::new(0);
    let result = read_bounded(Endless(&served)).expect("reads");
    assert!(result.is_none(), "an endless source is over the cap");
    // `take` hands the reader a buffer no larger than what it still allows.
    assert_eq!(served.get(), MAX_FILE_BYTES + 1);
}

#[test]
fn a_folder_file_that_is_not_a_kubeconfig_is_an_error_without_its_text() {
    let dir = folder("garbage");
    write(&dir, "bad.yaml", "clusters: [token: s3cr3t-value");
    let error = load_folder_file(&dir.join("bad.yaml")).expect_err("not a kubeconfig");
    assert!(!format!("{error} {error:?}").contains("s3cr3t"));
    std::fs::write(dir.join("binary.yaml"), [0xC3, 0x28, 0xFF]).expect("bytes");
    assert!(load_folder_file(&dir.join("binary.yaml")).is_err());
    finish(&dir);
}

#[test]
fn a_utf16_file_with_a_byte_order_mark_reads_like_kube_does() {
    let dir = folder("utf16");
    let mut bytes = vec![0xFF, 0xFE];
    for unit in KUBECONFIG.encode_utf16() {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    std::fs::write(dir.join("u16.yaml"), bytes).expect("utf16");
    let loaded = load_folder_file(&dir.join("u16.yaml")).expect("parses");
    assert_eq!(loaded.contexts().len(), 1);
    finish(&dir);
}
