//! The Metrics section: its pure rules, and the section in a headless window over a fake API server.
//! Nothing here touches a real cluster, config folder, or clipboard.

use std::path::PathBuf;
use std::time::Duration;

use cluster::fake_api::FakeApi;
use cluster::{MetricsFlavor, WritePolicy};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, TestAppContext, WeakEntity, WindowOptions};

use super::*;
use crate::cluster_catalog::CatalogHandle;
use crate::cluster_form::RowOrigin;
use crate::cluster_registry::{ClusterProfile, ClusterProxy, StoredMetrics};
use crate::environment::{Environment, EnvironmentTier};
use crate::settings::Settings;
use crate::settings_store::{LoadedSettings, WriteMode};
use crate::write_guard::ConfirmMode;

fn fields(namespace: &str, service: &str, port: &str, prefix: &str) -> MetricsSourceFields {
    MetricsSourceFields {
        namespace: namespace.to_owned(),
        service: service.to_owned(),
        port: port.to_owned(),
        scheme: MetricsScheme::Http,
        prefix: prefix.to_owned(),
    }
}

fn candidate(service: &str) -> MetricsCandidate {
    MetricsCandidate {
        flavor: MetricsFlavor::VictoriaMetricsCluster,
        fields: fields("monitoring", service, "8481", "/select/0/prometheus"),
    }
}

#[test]
fn other_service_validates_each_field() {
    let all_bad = fields("Bad NS", "bad.svc", "0", "no-slash");
    assert_eq!(
        invalid_fields(&all_bad),
        [
            OtherField::Namespace,
            OtherField::Service,
            OtherField::Port,
            OtherField::Prefix
        ]
    );
    assert!(invalid_fields(&fields("monitoring", "vmselect", "8481", "/select/0")).is_empty());
    assert_eq!(
        invalid_fields(&fields("monitoring", "vmselect", "http", "/a/../b")),
        [OtherField::Prefix]
    );
    assert_eq!(
        OtherField::Port.error().to_string(),
        "Use a port number or the service's port name."
    );
}

#[test]
fn saved_source_is_preselected() {
    let candidates = [candidate("vmselect"), candidate("vmquery")];
    let saved = candidates[1].fields.clone();
    assert_eq!(
        initial_choice(Some(&saved), &candidates),
        Choice::Candidate(saved.clone())
    );
    let elsewhere = fields("obs", "thanos", "10902", "");
    assert_eq!(initial_choice(Some(&elsewhere), &candidates), Choice::Other);
    assert_eq!(initial_choice(None, &candidates), Choice::ServerOnly);
    assert_eq!(initial_choice(Some(&saved), &[]), Choice::Other);
}

#[test]
fn test_lines_per_outcome() {
    let check = |cpu_series| {
        Ok(SourceCheck {
            latency: Duration::from_millis(35),
            cpu_series,
        })
    };
    assert_eq!(
        test_line(&check(1_234)),
        (
            "Reachable · 35 ms · 1,234 CPU series".to_owned(),
            StatusTone::Ok
        )
    );
    assert_eq!(
        test_line(&check(0)),
        (
            "Reachable, but it has no container_cpu_usage_seconds_total series".to_owned(),
            StatusTone::Warn
        )
    );
    assert_eq!(
        test_line(&Err(MetricsError::NoApiAtPrefix)),
        (
            "nothing answers at this path prefix; check the prefix and port".to_owned(),
            StatusTone::Bad
        )
    );
}

#[test]
fn saved_line_names_the_source_and_its_state() {
    let saved = fields("monitoring", "vmselect", "8481", "/select/0/prometheus");
    let stored = StoredMetrics::Fields(saved.clone());
    let name = "monitoring/vmselect:8481 /select/0/prometheus";
    assert_eq!(saved_line(None, None), None);
    assert_eq!(
        saved_line(Some(&stored), None).as_deref(),
        Some(format!("Saved: {name}").as_str())
    );
    let ready = SourceState::Ready {
        source: MetricsSource::new(&saved).expect("valid"),
        check: SourceCheck {
            latency: Duration::from_millis(5),
            cpu_series: 3,
        },
    };
    assert_eq!(
        saved_line(Some(&stored), Some(&ready)).as_deref(),
        Some(format!("Saved: {name} · Ready").as_str())
    );
    let failed = SourceState::Failed {
        source: MetricsSource::new(&saved).expect("valid"),
        error: MetricsError::TimedOut,
    };
    assert_eq!(
        saved_line(Some(&stored), Some(&failed)).as_deref(),
        Some(
            format!("Saved: {name} · Failed: the metrics backend did not answer within 20 s")
                .as_str()
        )
    );
    assert_eq!(
        saved_line(Some(&stored), Some(&SourceState::Invalid)).as_deref(),
        Some(format!("Saved: {name} · Invalid entry in settings").as_str())
    );
    let invalid = fields("Bad", "x", "80", "");
    assert_eq!(
        saved_line(Some(&StoredMetrics::Fields(invalid)), None).as_deref(),
        Some("Saved: invalid entry in settings")
    );
}

// ---- The section in a window ----

fn install(cx: &mut TestAppContext) {
    install_with(&[], cx);
}

/// Installs the globals; `chain` are the kubeconfig files the catalog loads.
fn install_with(chain: &[PathBuf], cx: &mut TestAppContext) {
    let chain = chain.to_vec();
    cx.update(|cx| {
        gpui_kit::init(cx);
        cx.set_reduce_motion(true);
        AppSettings::install(
            LoadedSettings {
                settings: Settings::default(),
                writes: WriteMode::Disabled,
                notice: None,
            },
            cx,
        );
        CatalogHandle::install(chain, cx);
    });
    cx.run_until_parked();
}

/// The row the Clusters page would hand the section for `cluster`.
fn row_of(cluster: ClusterRef) -> ClusterRow {
    ClusterRow {
        profile: ClusterProfile {
            display_name: cluster.context.clone(),
            environment: Environment::BuiltIn(EnvironmentTier::Development),
            default_namespace: None,
            read_only: false,
            confirm: ConfirmMode::Click,
            allow_node_shell: false,
            debug_image: cluster::DEFAULT_DEBUG_IMAGE.to_owned(),
            node_shell_namespace: "kube-system".to_owned(),
            proxy: Ok(ProxyChoice::Kubeconfig),
            metrics: None,
        },
        label: cluster.context.clone(),
        meta: String::new(),
        guessed: EnvironmentTier::Development,
        origin: RowOrigin::Chain,
        trust_note: None,
        cluster,
    }
}

/// The section as the root view of a window with `row` shown, drawn once: the open cluster
/// detects with the first draw.
fn open_section(
    row: Option<ClusterRow>,
    cx: &mut TestAppContext,
) -> (AnyWindowHandle, Entity<ClusterMetricsSection>) {
    let (window, section) = cx.update(|cx| {
        let catalog = CatalogHandle::of(cx);
        gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| {
                let mut section = ClusterMetricsSection::new(catalog, window, cx);
                section.show_cluster(row.as_ref(), window, cx);
                section
            })
        })
        .expect("open the test window")
    });
    render(window, cx);
    (window, section)
}

/// The section showing the open cluster of `publish_connection`.
fn open_open_section(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<ClusterMetricsSection>) {
    open_section(Some(row_of(cluster_ref())), cx)
}

fn render(window: AnyWindowHandle, cx: &mut TestAppContext) {
    cx.update_window(window, |_, window, cx| window.render_frame(cx))
        .expect("the window is open");
    cx.run_until_parked();
}

fn cluster_ref() -> ClusterRef {
    ClusterRef {
        kubeconfig: std::path::PathBuf::from("a.yaml"),
        context: "readonly@Monitor".to_owned(),
    }
}

fn other_cluster_ref() -> ClusterRef {
    ClusterRef {
        kubeconfig: std::path::PathBuf::from("a.yaml"),
        context: "other".to_owned(),
    }
}

fn vmselect_fields() -> MetricsSourceFields {
    fields("monitoring", "vmselect-x", "8481", "/select/0/prometheus")
}

const SERVICES: &str = r#"{"apiVersion":"v1","kind":"ServiceList","metadata":{},"items":[
 {"metadata":{"name":"vmselect-x","namespace":"monitoring","labels":{"app.kubernetes.io/name":"vmselect"}},
  "spec":{"ports":[{"name":"http","port":8481}]}},
 {"metadata":{"name":"vm-grafana","namespace":"monitoring","labels":{"app.kubernetes.io/name":"grafana"}},
  "spec":{"ports":[{"name":"http","port":80}]}}]}"#;

const OTHER_SERVICES: &str = r#"{"apiVersion":"v1","kind":"ServiceList","metadata":{},"items":[
 {"metadata":{"name":"prometheus-operated","namespace":"obs"},
  "spec":{"ports":[{"name":"web","port":9090}]}}]}"#;

fn new_runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .expect("a tokio runtime")
}

fn set_connection(cluster: ClusterRef, connection: cluster::ClusterConnection, cx: &mut App) {
    cx.set_global(ActiveConnection {
        label: cluster.context.clone(),
        cluster,
        connection,
        session: WeakEntity::new_invalid(),
        generation: 1,
    });
}

/// A runtime that stays alive for the test, with the connection of the first cluster published as
/// the shell would, answering the service list and a count of one series per query.
fn publish_connection(cx: &mut TestAppContext) -> (tokio::runtime::Runtime, FakeApi) {
    let runtime = new_runtime();
    cx.executor().allow_parking();
    let handle = runtime.handle().clone();
    let (connection, api) = {
        let _guard = runtime.enter();
        FakeApi::connection(WritePolicy::Blocked, |request| {
            if request.path == "/api/v1/services" {
                (200, SERVICES.to_owned())
            } else {
                (
                    200,
                    r#"{"status":"success","data":{"resultType":"vector","result":[{"metric":{},"value":[1,"7"]}]}}"#
                        .to_owned(),
                )
            }
        })
    };
    cx.update(|cx| {
        cx.set_global(ClusterRuntime::new(handle));
        set_connection(cluster_ref(), connection, cx);
    });
    (runtime, api)
}

fn wait_for(
    what: &str,
    cx: &mut TestAppContext,
    section: &Entity<ClusterMetricsSection>,
    done: impl Fn(&ClusterMetricsSection) -> bool,
) {
    for _ in 0..1_500 {
        cx.run_until_parked();
        if section.read_with(cx, |section, _| done(section)) {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("timed out waiting for {what}");
}

fn stored_entry(cluster: &ClusterRef, cx: &mut TestAppContext) -> Option<StoredMetrics> {
    cx.read(|cx| {
        AppSettings::get(cx)
            .registry
            .clusters
            .iter()
            .find(|entry| entry.cluster == *cluster)
            .and_then(|entry| entry.metrics.clone())
    })
}

fn save_vmselect(cx: &mut TestAppContext) {
    cx.update(|cx| {
        AppSettings::update(cx, |settings| {
            edit_entry(&mut settings.registry, &cluster_ref(), |entry| {
                entry.metrics = Some(StoredMetrics::Fields(vmselect_fields()));
            });
        });
    });
}

#[gpui_kit::test]
fn section_without_cluster_is_empty_and_idle(cx: &mut TestAppContext) {
    install(cx);
    let (_window, section) = open_section(None, cx);
    section.read_with(cx, |section, _| {
        assert!(section.cluster.is_none());
        assert!(
            !section.is_settled(),
            "a section without a cluster is not settled"
        );
        assert!(matches!(section.detection, Detection::Idle));
    });
}

#[gpui_kit::test]
fn open_cluster_detects_at_first_draw(cx: &mut TestAppContext) {
    install(cx);
    let (_runtime, api) = publish_connection(cx);
    let (_window, section) = open_open_section(cx);
    wait_for("detection", cx, &section, |section| section.is_settled());
    section.read_with(cx, |section, _| {
        let found: Vec<&str> = section
            .candidates()
            .iter()
            .map(|candidate| candidate.fields.service.as_str())
            .collect();
        assert_eq!(found, ["vmselect-x"], "grafana is not a metrics source");
        assert_eq!(section.choice, Choice::ServerOnly);
        assert_eq!(section.label, "readonly@Monitor");
    });
    let requests = api.requests();
    assert!(requests.iter().all(|request| request.method == "GET"));
    assert_eq!(requests.len(), 1, "one list, no source request yet");
}

#[gpui_kit::test]
fn saved_candidate_is_preselected_after_detection(cx: &mut TestAppContext) {
    install(cx);
    let (_runtime, _api) = publish_connection(cx);
    save_vmselect(cx);
    let (_window, section) = open_open_section(cx);
    wait_for("detection", cx, &section, |section| section.is_settled());
    section.read_with(cx, |section, _| {
        assert_eq!(section.choice, Choice::Candidate(vmselect_fields()));
    });
}

#[gpui_kit::test]
fn save_writes_fields_only_and_server_only_clears_them(cx: &mut TestAppContext) {
    install(cx);
    let (_runtime, _api) = publish_connection(cx);
    let (_window, section) = open_open_section(cx);
    wait_for("detection", cx, &section, |section| section.is_settled());
    section.update(cx, |section, cx| {
        section.choose(1, cx);
        section.save(cx);
    });
    assert_eq!(
        stored_entry(&cluster_ref(), cx),
        Some(StoredMetrics::Fields(vmselect_fields()))
    );
    section.update(cx, |section, cx| {
        section.choose(0, cx);
        section.save(cx);
    });
    assert_eq!(
        stored_entry(&cluster_ref(), cx),
        None,
        "metrics-server only stores nothing"
    );
}

#[gpui_kit::test]
fn detect_again_keeps_the_picked_row_and_save_writes_it(cx: &mut TestAppContext) {
    install(cx);
    let (_runtime, _api) = publish_connection(cx);
    let (window, section) = open_open_section(cx);
    wait_for("detection", cx, &section, |section| section.is_settled());
    save_vmselect(cx);
    section.update(cx, |section, cx| {
        section.choose(1, cx);
        section.start_detection(cx);
    });
    // Save and Test do nothing while the list is being read.
    section.update(cx, |section, cx| {
        assert!(section.is_detecting());
        section.save(cx);
        section.start_test(cx);
        assert!(matches!(section.test, TestResult::Idle));
    });
    wait_for("the second detection", cx, &section, |section| {
        section.is_settled()
    });
    render(window, cx);
    section.update(cx, |section, cx| {
        assert_eq!(section.choice, Choice::Candidate(vmselect_fields()));
        section.save(cx);
    });
    assert_eq!(
        stored_entry(&cluster_ref(), cx),
        Some(StoredMetrics::Fields(vmselect_fields())),
        "the stored entry is unchanged, not wiped"
    );
}

#[gpui_kit::test]
fn a_picked_row_that_detection_drops_still_saves_what_was_picked(cx: &mut TestAppContext) {
    install(cx);
    let (_runtime, _api) = publish_connection(cx);
    let (_window, section) = open_open_section(cx);
    wait_for("detection", cx, &section, |section| section.is_settled());
    section.update(cx, |section, cx| {
        section.choose(1, cx);
        // The list changes under the choice: nothing is listed any more.
        section.detection = Detection::Done(Vec::new());
        section.save(cx);
    });
    assert_eq!(
        stored_entry(&cluster_ref(), cx),
        Some(StoredMetrics::Fields(vmselect_fields()))
    );
}

#[gpui_kit::test]
fn invalid_other_service_cannot_be_saved_or_tested(cx: &mut TestAppContext) {
    install(cx);
    let (_runtime, api) = publish_connection(cx);
    let (window, section) = open_open_section(cx);
    wait_for("detection", cx, &section, |section| section.is_settled());
    cx.update_window(window, |_, window, cx| {
        section.update(cx, |section, cx| {
            section.set_inputs(&fields("Bad NS", "svc", "80", ""), window, cx);
            section.choose(2, cx);
            section.save(cx);
            section.start_test(cx);
        });
    })
    .expect("window");
    cx.run_until_parked();
    let saved = cx.read(|cx| AppSettings::get(cx).registry.clusters.len());
    assert_eq!(saved, 0, "nothing was saved");
    assert_eq!(api.requests().len(), 1, "no test request was sent");
}

#[gpui_kit::test]
fn test_checks_the_selected_source(cx: &mut TestAppContext) {
    install(cx);
    let (_runtime, api) = publish_connection(cx);
    let (_window, section) = open_open_section(cx);
    wait_for("detection", cx, &section, |section| section.is_settled());
    section.update(cx, |section, cx| {
        section.choose(1, cx);
        section.start_test(cx);
    });
    wait_for("the test", cx, &section, |section| {
        matches!(section.test, TestResult::Done(_))
    });
    section.read_with(cx, |section, _| match &section.test {
        TestResult::Done(Ok(check)) => assert_eq!(check.cpu_series, 7),
        _ => panic!("the test should have passed"),
    });
    let path = api
        .requests()
        .into_iter()
        .map(|request| request.path)
        .find(|path| path.contains("/proxy/"))
        .expect("a source request");
    assert!(path.starts_with(
        "/api/v1/namespaces/monitoring/services/http:vmselect-x:8481/proxy/select/0/prometheus/api/v1/query"
    ));
}

#[gpui_kit::test]
fn show_cluster_resets_inputs_and_drops_a_running_detection(cx: &mut TestAppContext) {
    install(cx);
    // The first cluster never answers its list.
    let runtime = new_runtime();
    cx.executor().allow_parking();
    let (hung, _hung_api) = {
        let _guard = runtime.enter();
        FakeApi::failing(WritePolicy::Blocked, cluster::fake_api::Failure::Hang)
    };
    let (answering, _answering_api) = {
        let _guard = runtime.enter();
        FakeApi::connection(WritePolicy::Blocked, |_| (200, OTHER_SERVICES.to_owned()))
    };
    let handle = runtime.handle().clone();
    cx.update(|cx| {
        cx.set_global(ClusterRuntime::new(handle));
        set_connection(cluster_ref(), hung, cx);
    });
    // The first cluster has a saved source, so its fields fill the inputs.
    save_vmselect(cx);
    let (window, section) = open_open_section(cx);
    section.read_with(cx, |section, cx| {
        assert!(section.is_detecting());
        assert_eq!(section.service.read(cx).value().as_ref(), "vmselect-x");
    });
    // The page moves to a cluster with no saved source, which is the open one now.
    cx.update(|cx| set_connection(other_cluster_ref(), answering, cx));
    cx.update_window(window, |_, window, cx| {
        section.update(cx, |section, cx| {
            section.show_cluster(Some(&row_of(other_cluster_ref())), window, cx);
        });
    })
    .expect("window");
    cx.run_until_parked();
    render(window, cx);
    wait_for("the second detection", cx, &section, |section| {
        section.is_settled()
    });
    section.read_with(cx, |section, cx| {
        assert_eq!(section.cluster, Some(other_cluster_ref()));
        let found: Vec<&str> = section
            .candidates()
            .iter()
            .map(|candidate| candidate.fields.service.as_str())
            .collect();
        assert_eq!(found, ["prometheus-operated"], "the old list was dropped");
        assert_eq!(section.choice, Choice::ServerOnly);
        assert_eq!(
            section.service.read(cx).value().as_ref(),
            "",
            "inputs are cleared"
        );
        assert_eq!(section.namespace.read(cx).value().as_ref(), "");
    });
}

#[gpui_kit::test]
fn fixture_section_is_settled_and_sends_nothing(cx: &mut TestAppContext) {
    install(cx);
    cx.update(|cx| cx.set_global(ClusterMetricsFixture));
    let (_window, section) = open_section(Some(row_of(cluster_ref())), cx);
    section.read_with(cx, |section, _| {
        assert!(section.is_settled());
        assert_eq!(section.candidates().len(), 3);
        assert_eq!(section.label, "readonly@Monitor");
    });
}

// ---- A cluster that is not open ----

/// A kubeconfig file with two contexts whose server refuses at once; returns the directory and the
/// refs of the contexts.
fn two_context_file(name: &str) -> (PathBuf, ClusterRef, ClusterRef) {
    let dir = std::env::temp_dir().join(format!("k8sboard-0058-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let path = dir.join("chain.yaml");
    let text = "apiVersion: v1\nkind: Config\nclusters:\n  - name: c\n    cluster: { server: \"https://127.0.0.1:1\" }\nusers:\n  - name: u\n    user: { token: fixture-token-value }\ncontexts:\n  - name: open-ctx\n    context: { cluster: c, user: u }\n  - name: other-ctx\n    context: { cluster: c, user: u }\n";
    std::fs::write(&path, text).expect("write fixture");
    let cluster = |context: &str| ClusterRef {
        kubeconfig: path.clone(),
        context: context.to_owned(),
    };
    let (open, other) = (cluster("open-ctx"), cluster("other-ctx"));
    (dir, open, other)
}

fn valid_other_fields() -> MetricsSourceFields {
    fields("monitoring", "thanos-query", "10902", "")
}

fn type_other_service(
    window: AnyWindowHandle,
    section: &Entity<ClusterMetricsSection>,
    cx: &mut TestAppContext,
) {
    cx.update_window(window, |_, window, cx| {
        section.update(cx, |section, cx| {
            section.set_inputs(&valid_other_fields(), window, cx);
            // Typing does this; `set_value` itself raises no change event.
            section.on_other_changed(cx);
        });
    })
    .expect("window");
    cx.run_until_parked();
}

/// The open cluster of the shell is `cluster_ref()` on the fake API; the section shows `other`.
#[gpui_kit::test]
fn cluster_that_is_not_open_sends_nothing_until_detect(cx: &mut TestAppContext) {
    install(cx);
    let (_runtime, api) = publish_connection(cx);
    let (window, section) = open_section(Some(row_of(other_cluster_ref())), cx);
    render(window, cx);
    std::thread::sleep(Duration::from_millis(100));
    cx.run_until_parked();
    section.read_with(cx, |section, cx| {
        assert!(matches!(section.detection, Detection::Idle));
        assert!(!section.is_open_cluster(cx));
    });
    assert!(api.requests().is_empty(), "{:?}", api.requests());
    // Save needs no connection.
    type_other_service(window, &section, cx);
    section.update(cx, |section, cx| section.save(cx));
    assert_eq!(
        stored_entry(&other_cluster_ref(), cx),
        Some(StoredMetrics::Fields(valid_other_fields()))
    );
}

#[gpui_kit::test]
fn detect_on_a_cluster_that_is_not_open_opens_a_one_time_client(cx: &mut TestAppContext) {
    let (dir, _open, other) = two_context_file("detect");
    install_with(&[dir.join("chain.yaml")], cx);
    let (_runtime, api) = publish_connection(cx);
    let (_window, section) = open_section(Some(row_of(other)), cx);
    section.update(cx, |section, cx| section.start_detection(cx));
    wait_for("the failed detection", cx, &section, |section| {
        matches!(section.detection, Detection::Failed(_))
    });
    assert!(
        api.requests().is_empty(),
        "the open cluster's connection is not used"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn test_on_a_cluster_that_is_not_open_reports_cannot_connect(cx: &mut TestAppContext) {
    let (dir, _open, other) = two_context_file("test");
    install_with(&[dir.join("chain.yaml")], cx);
    let (_runtime, api) = publish_connection(cx);
    // An unusable stored proxy fails the open at once, with no network involved.
    cx.update(|cx| {
        AppSettings::update(cx, |settings| {
            edit_entry(&mut settings.registry, &other, |entry| {
                entry.proxy = Some(ClusterProxy::Url("ftp://proxy.example".to_owned()));
            });
        });
    });
    let (window, section) = open_section(Some(row_of(other)), cx);
    type_other_service(window, &section, cx);
    section.update(cx, |section, cx| section.start_test(cx));
    wait_for("the failed test", cx, &section, |section| {
        matches!(section.test, TestResult::Done(_))
    });
    section.read_with(cx, |section, _| match &section.test {
        TestResult::Done(Err(MetricsError::Unexpected(message))) => {
            assert!(message.starts_with("cannot connect: "), "{message}");
        }
        _ => panic!("the test should have failed to connect"),
    });
    assert!(api.requests().is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn show_cluster_again_for_the_same_row_resets_the_choice(cx: &mut TestAppContext) {
    install(cx);
    let (_runtime, _api) = publish_connection(cx);
    let (window, section) = open_section(Some(row_of(other_cluster_ref())), cx);
    type_other_service(window, &section, cx);
    section.read_with(cx, |section, _| assert_eq!(section.choice, Choice::Other));
    // Reset to defaults clears the entry, and the page shows the same row again.
    cx.update_window(window, |_, window, cx| {
        section.update(cx, |section, cx| {
            section.show_cluster(Some(&row_of(other_cluster_ref())), window, cx);
        });
    })
    .expect("window");
    section.read_with(cx, |section, cx| {
        assert_eq!(section.choice, Choice::ServerOnly);
        assert_eq!(section.service.read(cx).value().as_ref(), "");
        assert!(matches!(section.detection, Detection::Idle));
        assert!(matches!(section.test, TestResult::Idle));
    });
}

#[gpui_kit::test]
fn section_follows_the_cluster_becoming_open(cx: &mut TestAppContext) {
    install(cx);
    let (runtime, api) = publish_connection(cx);
    let (window, section) = open_section(Some(row_of(other_cluster_ref())), cx);
    render(window, cx);
    section.read_with(cx, |section, _| {
        assert!(matches!(section.detection, Detection::Idle));
    });
    assert!(api.requests().is_empty());
    // The shell opens the cluster the section shows.
    let (answering, answering_api) = {
        let _guard = runtime.enter();
        FakeApi::connection(WritePolicy::Blocked, |_| (200, OTHER_SERVICES.to_owned()))
    };
    cx.update(|cx| set_connection(other_cluster_ref(), answering, cx));
    cx.run_until_parked();
    render(window, cx);
    wait_for("detection", cx, &section, |section| section.is_settled());
    let requests = answering_api.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].path, "/api/v1/services");
}
