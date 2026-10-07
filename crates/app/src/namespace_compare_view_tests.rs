use cluster::WritePolicy;
use cluster::fake_api::FakeApi;
use gpui_kit::{Entity, TestAppContext};
use serde_json::json;

use super::*;

fn deployment(replicas: u32) -> serde_json::Value {
    json!({
        "metadata": {"name": "web"},
        "spec": {"replicas": replicas},
    })
}

/// The dialog child over a fake server where `lab-shop` runs one replica of `web` and
/// `lab-shop-stg` two.
fn open_view(
    runtime: &tokio::runtime::Runtime,
    cx: &mut TestAppContext,
) -> Entity<NamespaceCompareView> {
    cx.executor().allow_parking();
    cx.update(|cx| cx.set_global(ClusterRuntime::new(runtime.handle().clone())));
    let (connection, _api) = {
        let _guard = runtime.enter();
        FakeApi::connection(WritePolicy::Blocked, |request| {
            let items = if request.path.ends_with("/namespaces/lab-shop/deployments") {
                vec![deployment(1)]
            } else if request
                .path
                .ends_with("/namespaces/lab-shop-stg/deployments")
            {
                vec![deployment(2)]
            } else {
                Vec::new()
            };
            let list = json!({"apiVersion": "v1", "kind": "List", "metadata": {}, "items": items});
            (200, list.to_string())
        })
    };
    let names = ["lab-shop", "lab-shop-stg", "kube-system"].map(str::to_owned);
    cx.update(|cx| {
        gpui_kit::init(cx);
        gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| {
                NamespaceCompareView::new(
                    "lab-shop".to_owned(),
                    names.to_vec(),
                    connection,
                    window,
                    cx,
                )
            })
        })
        .expect("open the test window")
        .1
    })
}

fn wait_until_ready(view: &Entity<NamespaceCompareView>, cx: &mut TestAppContext) {
    for _ in 0..400 {
        cx.run_until_parked();
        if view.read_with(cx, |view, _| view.ready_lines().is_some()) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    panic!("the comparison did not become ready");
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .expect("a tokio runtime")
}

#[gpui_kit::test]
fn the_dialog_starts_on_the_choice_and_leaves_the_left_side_out(cx: &mut TestAppContext) {
    let runtime = runtime();
    let view = open_view(&runtime, cx);
    view.read_with(cx, |view, _| {
        assert!(view.ready_lines().is_none());
        assert_eq!(view.candidates, ["kube-system", "lab-shop-stg"]);
    });
}

#[gpui_kit::test]
fn picking_a_namespace_reads_both_and_lists_the_difference(cx: &mut TestAppContext) {
    let runtime = runtime();
    let view = open_view(&runtime, cx);
    view.update(cx, |view, cx| view.choose_for_test("lab-shop-stg", cx));
    wait_until_ready(&view, cx);
    view.read_with(cx, |view, _| {
        let lines = view.ready_lines().expect("ready");
        assert!(lines.contains(&CompareLine::Change("spec.replicas: 1 → 2".to_owned())));
        assert!(
            !lines
                .iter()
                .any(|line| matches!(line, CompareLine::Diff(_))),
            "no diff is open yet"
        );
    });
}

#[gpui_kit::test]
fn open_diff_shows_the_line_diff_and_a_second_toggle_hides_it(cx: &mut TestAppContext) {
    let runtime = runtime();
    let view = open_view(&runtime, cx);
    view.update(cx, |view, cx| view.choose_for_test("lab-shop-stg", cx));
    wait_until_ready(&view, cx);
    let diff_rows = |view: &Entity<NamespaceCompareView>, cx: &mut TestAppContext| {
        view.read_with(cx, |view, _| {
            view.ready_lines()
                .expect("ready")
                .iter()
                .filter(|line| matches!(line, CompareLine::Diff(_)))
                .count()
        })
    };
    view.update(cx, |view, cx| {
        view.toggle_diff_for_test(ObjectKind::Deployment, "web", cx);
    });
    assert!(diff_rows(&view, cx) > 0);
    view.update(cx, |view, cx| {
        view.toggle_diff_for_test(ObjectKind::Deployment, "web", cx);
    });
    assert_eq!(diff_rows(&view, cx), 0);
}
