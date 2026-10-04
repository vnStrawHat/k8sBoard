//! The Metrics page: its pure rules, and the page in a headless window over a fake API server.
//! Nothing here touches a real cluster, config folder, or clipboard.

use std::time::Duration;

use cluster::fake_api::FakeApi;
use cluster::{MetricsFlavor, WritePolicy};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, TestAppContext, WeakEntity, WindowOptions};

use super::*;
use crate::cluster_registry::StoredMetrics;
use crate::settings::Settings;
use crate::settings_store::{LoadedSettings, WriteMode};

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

// ---- The page in a window ----

fn install(cx: &mut TestAppContext) {
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
    });
    cx.run_until_parked();
}

/// The page as the root view of a window, drawn once: detection starts with the first draw.
fn open_page(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<MetricsPage>) {
    let (window, page) = cx.update(|cx| {
        gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| MetricsPage::new(window, cx))
        })
        .expect("open the test window")
    });
    render(window, cx);
    (window, page)
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
    page: &Entity<MetricsPage>,
    done: impl Fn(&MetricsPage) -> bool,
) {
    for _ in 0..1_500 {
        cx.run_until_parked();
        if page.read_with(cx, |page, _| done(page)) {
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
fn metrics_page_without_cluster_shows_the_hint(cx: &mut TestAppContext) {
    install(cx);
    let (_window, page) = open_page(cx);
    page.read_with(cx, |page, _| {
        assert!(page.cluster.is_none());
        assert!(
            !page.is_settled(),
            "a page without a cluster is not settled"
        );
        assert!(matches!(page.detection, Detection::Idle));
    });
    assert_eq!(
        NO_CLUSTER_TEXT,
        "Connect to a cluster to choose its metrics source."
    );
}

#[gpui_kit::test]
fn detection_lists_the_candidates_of_the_connected_cluster(cx: &mut TestAppContext) {
    install(cx);
    let (_runtime, api) = publish_connection(cx);
    let (_window, page) = open_page(cx);
    wait_for("detection", cx, &page, |page| page.is_settled());
    page.read_with(cx, |page, _| {
        let found: Vec<&str> = page
            .candidates()
            .iter()
            .map(|candidate| candidate.fields.service.as_str())
            .collect();
        assert_eq!(found, ["vmselect-x"], "grafana is not a metrics source");
        assert_eq!(page.choice, Choice::ServerOnly);
        assert_eq!(page.label, "readonly@Monitor");
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
    let (_window, page) = open_page(cx);
    wait_for("detection", cx, &page, |page| page.is_settled());
    page.read_with(cx, |page, _| {
        assert_eq!(page.choice, Choice::Candidate(vmselect_fields()));
    });
}

#[gpui_kit::test]
fn save_writes_fields_only_and_server_only_clears_them(cx: &mut TestAppContext) {
    install(cx);
    let (_runtime, _api) = publish_connection(cx);
    let (_window, page) = open_page(cx);
    wait_for("detection", cx, &page, |page| page.is_settled());
    page.update(cx, |page, cx| {
        page.choose(1, cx);
        page.save(cx);
    });
    assert_eq!(
        stored_entry(&cluster_ref(), cx),
        Some(StoredMetrics::Fields(vmselect_fields()))
    );
    page.update(cx, |page, cx| {
        page.choose(0, cx);
        page.save(cx);
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
    let (window, page) = open_page(cx);
    wait_for("detection", cx, &page, |page| page.is_settled());
    save_vmselect(cx);
    page.update(cx, |page, cx| {
        page.choose(1, cx);
        page.start_detection(cx);
    });
    // Save and Test do nothing while the list is being read.
    page.update(cx, |page, cx| {
        assert!(page.is_detecting());
        page.save(cx);
        page.start_test(cx);
        assert!(matches!(page.test, TestResult::Idle));
    });
    wait_for("the second detection", cx, &page, |page| page.is_settled());
    render(window, cx);
    page.update(cx, |page, cx| {
        assert_eq!(page.choice, Choice::Candidate(vmselect_fields()));
        page.save(cx);
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
    let (_window, page) = open_page(cx);
    wait_for("detection", cx, &page, |page| page.is_settled());
    page.update(cx, |page, cx| {
        page.choose(1, cx);
        // The list changes under the choice: nothing is listed any more.
        page.detection = Detection::Done(Vec::new());
        page.save(cx);
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
    let (window, page) = open_page(cx);
    wait_for("detection", cx, &page, |page| page.is_settled());
    cx.update_window(window, |_, window, cx| {
        page.update(cx, |page, cx| {
            page.set_inputs(&fields("Bad NS", "svc", "80", ""), window, cx);
            page.choose(2, cx);
            page.save(cx);
            page.start_test(cx);
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
    let (_window, page) = open_page(cx);
    wait_for("detection", cx, &page, |page| page.is_settled());
    page.update(cx, |page, cx| {
        page.choose(1, cx);
        page.start_test(cx);
    });
    wait_for("the test", cx, &page, |page| {
        matches!(page.test, TestResult::Done(_))
    });
    page.read_with(cx, |page, _| match &page.test {
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
fn switching_clusters_clears_inputs_and_drops_a_running_detection(cx: &mut TestAppContext) {
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
    let (window, page) = open_page(cx);
    page.read_with(cx, |page, cx| {
        assert!(page.is_detecting());
        assert_eq!(page.service.read(cx).value().as_ref(), "vmselect-x");
    });
    // The user moves to a cluster with no saved source.
    cx.update(|cx| set_connection(other_cluster_ref(), answering, cx));
    cx.run_until_parked();
    render(window, cx);
    wait_for("the second detection", cx, &page, |page| page.is_settled());
    page.read_with(cx, |page, cx| {
        assert_eq!(page.cluster, Some(other_cluster_ref()));
        let found: Vec<&str> = page
            .candidates()
            .iter()
            .map(|candidate| candidate.fields.service.as_str())
            .collect();
        assert_eq!(found, ["prometheus-operated"], "the old list was dropped");
        assert_eq!(page.choice, Choice::ServerOnly);
        assert_eq!(
            page.service.read(cx).value().as_ref(),
            "",
            "inputs are cleared"
        );
        assert_eq!(page.namespace.read(cx).value().as_ref(), "");
    });
}

#[gpui_kit::test]
fn fixture_page_is_settled_and_sends_nothing(cx: &mut TestAppContext) {
    install(cx);
    let (window, page) = cx.update(|cx| {
        gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| MetricsPage::fixture(window, cx))
        })
        .expect("open the test window")
    });
    render(window, cx);
    page.read_with(cx, |page, _| {
        assert!(page.is_settled());
        assert_eq!(page.candidates().len(), 3);
    });
}
