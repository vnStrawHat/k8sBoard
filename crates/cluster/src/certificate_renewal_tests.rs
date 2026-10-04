use serde_json::json;

use super::*;

fn requested_at() -> jiff::Timestamp {
    "2026-10-04T08:30:15.987Z"
        .parse()
        .expect("a valid timestamp")
}

fn certificate() -> Value {
    json!({
        "apiVersion": "cert-manager.io/v1",
        "kind": "Certificate",
        "metadata": {
            "name": "tls", "namespace": "shop", "resourceVersion": "812", "generation": 4,
            "managedFields": [{"manager": "cert-manager-certificates-readiness"}],
        },
        "spec": {"secretName": "tls-secret", "issuerRef": {"name": "letsencrypt"}},
        "status": {
            "conditions": [
                {"type": "Ready", "status": "True", "reason": "Ready", "observedGeneration": 4},
            ],
            "notAfter": "2026-12-01T00:00:00Z",
            "renewalTime": "2026-11-01T00:00:00Z",
            "revision": 3,
        },
    })
}

fn conditions(body: &Value) -> Vec<Value> {
    body.pointer("/status/conditions")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

#[test]
fn renewal_body_adds_one_issuing_condition() {
    let fresh = certificate();
    let body = renewal_status_body(fresh.clone(), requested_at()).expect("renewable");
    let conditions = conditions(&body);
    assert_eq!(conditions.len(), 2);
    assert_eq!(conditions[0], fresh["status"]["conditions"][0]);
    assert_eq!(
        conditions[1],
        json!({
            "type": "Issuing",
            "status": "True",
            "reason": "ManuallyTriggered",
            "message": "Certificate re-issuance manually triggered",
            "lastTransitionTime": "2026-10-04T08:30:15Z",
            "observedGeneration": 4,
        })
    );
    assert_eq!(body["spec"], fresh["spec"]);
    for field in ["notAfter", "renewalTime", "revision"] {
        assert_eq!(body["status"][field], fresh["status"][field], "{field}");
    }
}

#[test]
fn renewal_body_replaces_a_false_issuing_condition() {
    let mut fresh = certificate();
    fresh["status"]["conditions"] = json!([
        {"type": "Issuing", "status": "False", "reason": "Issued"},
        {"type": "Ready", "status": "True"},
    ]);
    let body = renewal_status_body(fresh, requested_at()).expect("renewable");
    let conditions = conditions(&body);
    let types: Vec<_> = conditions
        .iter()
        .filter_map(|condition| condition["type"].as_str())
        .collect();
    assert_eq!(types, ["Ready", "Issuing"]);
    assert_eq!(conditions[1]["status"], "True");
}

#[test]
fn renewal_body_creates_missing_status_and_conditions() {
    let mut fresh = certificate();
    fresh.as_object_mut().expect("an object").remove("status");
    let body = renewal_status_body(fresh.clone(), requested_at()).expect("renewable");
    assert_eq!(conditions(&body).len(), 1);

    fresh["status"] = json!({"revision": 1, "conditions": null});
    let body = renewal_status_body(fresh, requested_at()).expect("renewable");
    assert_eq!(conditions(&body).len(), 1);
    assert_eq!(body["status"]["revision"], 1);
}

#[test]
fn renewal_body_without_generation_omits_observed_generation() {
    let mut fresh = certificate();
    fresh["metadata"]
        .as_object_mut()
        .expect("an object")
        .remove("generation");
    let body = renewal_status_body(fresh, requested_at()).expect("renewable");
    assert!(conditions(&body)[1].get("observedGeneration").is_none());
}

#[test]
fn renewal_refuses_issuing_and_deleting() {
    let mut issuing = certificate();
    issuing["status"]["conditions"] = json!([{"type": "Issuing", "status": "True"}]);
    assert_eq!(
        renewal_status_body(issuing, requested_at()),
        Err(RenewRefusal::AlreadyIssuing)
    );
    let mut deleting = certificate();
    deleting["metadata"]["deletionTimestamp"] = json!("2026-10-04T08:00:00Z");
    assert_eq!(
        renewal_status_body(deleting, requested_at()),
        Err(RenewRefusal::Deleting)
    );
}

#[test]
fn renewal_body_drops_managed_fields_keeps_version() {
    let body = renewal_status_body(certificate(), requested_at()).expect("renewable");
    assert!(body["metadata"].get("managedFields").is_none());
    assert_eq!(body["metadata"]["resourceVersion"], "812");
    assert_eq!(body["metadata"]["name"], "tls");
}

#[test]
fn renewal_refuses_unreadable_objects() {
    let mut no_version = certificate();
    no_version["metadata"]
        .as_object_mut()
        .expect("an object")
        .remove("resourceVersion");
    assert_eq!(
        renewal_status_body(no_version, requested_at()),
        Err(RenewRefusal::Unreadable)
    );
    let mut odd_conditions = certificate();
    odd_conditions["status"]["conditions"] = json!("none");
    assert_eq!(
        renewal_status_body(odd_conditions, requested_at()),
        Err(RenewRefusal::Unreadable)
    );
    let mut odd_status = certificate();
    odd_status["status"] = json!(["x"]);
    assert_eq!(
        renewal_status_body(odd_status, requested_at()),
        Err(RenewRefusal::Unreadable)
    );
    assert_eq!(
        renewal_status_body(json!([]), requested_at()),
        Err(RenewRefusal::Unreadable)
    );
}
