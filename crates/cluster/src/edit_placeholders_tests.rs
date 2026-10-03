use serde_json::json;

use super::*;

fn items(value: &Value) -> &[Value] {
    value.as_array().expect("a list")
}

#[test]
fn counterpart_matches_by_unique_name() {
    let base = json!([{"name": "a", "v": 1}, {"name": "b", "v": 2}]);
    let edited = json!([{"name": "b", "v": 0}, {"name": "a", "v": 0}]);
    let found = counterpart(items(&base), &items(&edited)[0], 0, items(&edited));
    assert_eq!(found, Some(&json!({"name": "b", "v": 2})));
    let missing = json!([{"name": "c"}]);
    assert_eq!(
        counterpart(items(&base), &items(&missing)[0], 0, items(&missing)),
        None
    );
}

#[test]
fn counterpart_falls_back_to_the_index() {
    let duplicate_names = json!([{"name": "x", "v": 1}, {"name": "x", "v": 2}]);
    let unnamed = json!(["a", "b"]);
    let one_unnamed = json!([{"name": "x"}, {"other": 1}]);
    for list in [&duplicate_names, &unnamed, &one_unnamed] {
        assert_eq!(
            counterpart(items(list), &items(list)[1], 1, items(list)),
            Some(&items(list)[1])
        );
    }
    // Both lists must be named for a name match: one duplicate in either side means position.
    let named = json!([{"name": "b", "v": 1}, {"name": "a", "v": 2}]);
    let found = counterpart(
        items(&named),
        &items(&duplicate_names)[0],
        0,
        items(&duplicate_names),
    );
    assert_eq!(found, Some(&json!({"name": "b", "v": 1})));
}

#[test]
fn restore_puts_the_server_value_back() {
    let fresh = json!({"spec": {"env": [{"name": "A", "value": "raw-a"}], "note": "keep"}});
    let mut edited = json!({"spec": {"env": [{"name": "A", "value": HIDDEN}], "note": "new"}});
    let restored = restore(&mut edited, &fresh).expect("a counterpart exists");
    assert_eq!(edited["spec"]["env"][0]["value"], "raw-a");
    assert_eq!(edited["spec"]["note"], "new");
    assert!(restored.moved.is_empty());
}

#[test]
fn restore_reports_the_first_unmatched_path() {
    let fresh = json!({"spec": {"a": 1}});
    let mut edited = json!({"spec": {"a": HIDDEN, "b": HIDDEN, "c": HIDDEN}});
    let error = restore(&mut edited, &fresh).expect_err("b has no counterpart");
    assert!(matches!(&error, Unrestorable::Unmatched(_)), "{error:?}");
    assert_eq!(error.path().to_string(), "spec.b");
}

#[test]
fn check_leaves_the_edit_untouched() {
    let base = json!({"data": {"k": HIDDEN}});
    let edited = json!({"data": {"k": HIDDEN}});
    assert!(check(&edited, &base).is_ok());
    assert_eq!(edited, json!({"data": {"k": HIDDEN}}));
    let unmatched = json!({"data": {"other": HIDDEN}});
    assert_eq!(
        check(&unmatched, &base)
            .expect_err("unmatched")
            .path()
            .to_string(),
        "data.other"
    );
}

#[test]
fn a_named_placeholder_follows_its_item_through_a_reorder() {
    let fresh = json!({"env": [{"name": "A", "value": "raw-a"}, {"name": "B", "value": "raw-b"}]});
    let mut edited =
        json!({"env": [{"name": "B", "value": HIDDEN}, {"name": "A", "value": HIDDEN}]});
    let restored = restore(&mut edited, &fresh).expect("matched by name");
    assert_eq!(edited["env"][0]["value"], "raw-b");
    assert_eq!(edited["env"][1]["value"], "raw-a");
    assert!(restored.moved.is_empty());
}

#[test]
fn an_index_matched_placeholder_is_moved_when_its_item_differs() {
    let fresh = json!({"env": [
        {"name": "X", "value": "raw-1", "tag": 1},
        {"name": "X", "value": "raw-2", "tag": 2},
    ]});
    let mut edited = json!({"env": [
        {"name": "X", "value": HIDDEN, "tag": 2},
        {"name": "X", "value": HIDDEN, "tag": 1},
    ]});
    let restored = restore(&mut edited, &fresh).expect("matched by index");
    let moved: Vec<String> = restored.moved.iter().map(ToString::to_string).collect();
    assert_eq!(moved, ["env[0].value", "env[1].value"]);
    assert_eq!(edited["env"][0]["value"], "raw-1");
}

#[test]
fn an_index_matched_placeholder_of_an_unchanged_item_is_not_moved() {
    let fresh = json!({"env": [{"name": "X", "value": "raw-1"}, {"name": "X", "value": "raw-2"}]});
    let mut edited =
        json!({"env": [{"name": "X", "value": HIDDEN}, {"name": "X", "value": HIDDEN}]});
    let restored = restore(&mut edited, &fresh).expect("matched by index");
    assert!(restored.moved.is_empty());
}

#[test]
fn a_placeholder_inside_a_moved_item_is_moved_at_any_depth() {
    let fresh = json!({"items": [{"label": "one", "inner": {"secret": "raw-1"}}]});
    let mut edited = json!({"items": [{"label": "two", "inner": {"secret": HIDDEN}}]});
    let restored = restore(&mut edited, &fresh).expect("matched by index");
    assert_eq!(restored.moved.len(), 1);
    assert_eq!(restored.moved[0].to_string(), "items[0].inner.secret");
}

#[test]
fn diff_marker_text_is_refused_wherever_it_sits() {
    let fresh = json!({"a": "raw", "list": [{"v": "raw"}]});
    for marker in [
        HIDDEN_CHANGED,
        HIDDEN_MOVED,
        "<hidden",
        "<hidden>x",
        "<hidden >",
    ] {
        let mut edited = json!({"a": HIDDEN, "list": [{"v": marker}]});
        let error = restore(&mut edited, &fresh).expect_err(marker);
        assert!(
            matches!(&error, Unrestorable::MarkerText(_)),
            "{marker}: {error:?}"
        );
        assert_eq!(error.path().to_string(), "list[0].v");
    }
}

#[test]
fn the_placeholder_itself_and_ordinary_text_are_not_markers() {
    let fresh = json!({"a": "raw", "b": "raw"});
    let mut edited = json!({"a": HIDDEN, "b": "hidden <hidden> text"});
    assert!(restore(&mut edited, &fresh).is_ok());
    assert_eq!(edited["b"], "hidden <hidden> text");
}
