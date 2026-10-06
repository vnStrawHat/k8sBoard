use super::*;

const DOCUMENT: &str = "\
apiVersion: apps/v1
kind: Deployment
metadata:
  name: api
  labels:
    app: api
spec:
  replicas: 3
  template:
    spec:
      containers:
      - name: api
        image: api:1
        ports:
        - containerPort: 80
        - containerPort: 443
      - name: sidecar
        image: proxy:1
        env:
          - name: A
            value: \"1\"
          - name: B
            value: \"2\"
  # a comment
  paused: false
";

#[test]
fn a_plain_key_path_finds_its_line() {
    assert_eq!(line_of_field(DOCUMENT, "spec.replicas"), Some(8));
    assert_eq!(line_of_field(DOCUMENT, "metadata.labels.app"), Some(6));
    assert_eq!(line_of_field(DOCUMENT, "spec.paused"), Some(25));
}

#[test]
fn a_sequence_index_picks_the_item() {
    let image = |index| format!("spec.template.spec.containers[{index}].image");
    assert_eq!(line_of_field(DOCUMENT, &image(0)), Some(13));
    assert_eq!(line_of_field(DOCUMENT, &image(1)), Some(18));
}

#[test]
fn nested_sequences_and_indented_items_resolve() {
    let port = "spec.template.spec.containers[0].ports[1].containerPort";
    assert_eq!(line_of_field(DOCUMENT, port), Some(16));
    let value = "spec.template.spec.containers[1].env[1].value";
    assert_eq!(line_of_field(DOCUMENT, value), Some(23));
}

#[test]
fn a_missing_part_falls_back_to_the_deepest_line_that_exists() {
    let path = "spec.template.spec.containers[1].resources.limits.cpu";
    assert_eq!(line_of_field(DOCUMENT, path), Some(17));
    assert_eq!(
        line_of_field(DOCUMENT, "spec.template.spec.containers[7].image"),
        Some(11)
    );
}

#[test]
fn an_unknown_path_has_no_line() {
    assert_eq!(line_of_field(DOCUMENT, "status.replicas"), None);
    assert_eq!(line_of_field("", "spec"), None);
}

#[test]
fn a_name_selector_is_not_followed() {
    // The server spells indexes as numbers; a `[name]` selector stops at the part before it.
    assert_eq!(
        line_of_field(DOCUMENT, "spec.template.spec.containers[api].image"),
        None
    );
}

#[test]
fn a_syntax_error_carries_its_own_line() {
    let error = EditError::Syntax {
        line: 4,
        column: 2,
        message: "bad".to_owned(),
    };
    assert_eq!(local_error_line(&error, DOCUMENT), Some(4));
    let unknown = EditError::Syntax {
        line: 0,
        column: 0,
        message: "bad".to_owned(),
    };
    assert_eq!(local_error_line(&unknown, DOCUMENT), None);
}

#[test]
fn a_field_error_is_looked_up_in_the_text() {
    let error = EditError::UnmatchedPlaceholder {
        path: "spec.template.spec.containers[1].image".to_owned(),
    };
    assert_eq!(local_error_line(&error, DOCUMENT), Some(18));
    assert_eq!(local_error_line(&EditError::NoChanges, DOCUMENT), None);
}

#[test]
fn a_line_range_leaves_out_the_line_break() {
    let text = "a: 1\r\nbb: 2\n\nc: 3";
    assert_eq!(line_byte_range(text, 1), Some(0..4));
    assert_eq!(line_byte_range(text, 2), Some(6..11));
    assert_eq!(line_byte_range(text, 3), Some(12..12));
    assert_eq!(line_byte_range(text, 4), Some(13..17));
    assert_eq!(line_byte_range(text, 5), None);
}
