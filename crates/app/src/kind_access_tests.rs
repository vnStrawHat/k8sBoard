use cluster::{AccessCheck, AccessDecision, AccessReport, AccessReview};

use super::*;

fn report(kind: ObjectKind, decision: AccessDecision) -> AccessReport {
    AccessReport {
        reviews: vec![AccessReview {
            check: AccessCheck::Update(kind),
            decision,
        }],
    }
}

#[test]
fn an_unreviewed_kind_has_no_entry() {
    let map = KindAccessMap::new();
    assert!(map.get(ObjectKind::Deployment).is_none());
    assert!(KindAccessMap::EMPTY.get(ObjectKind::Deployment).is_none());
}

#[test]
fn set_replaces_the_entry_of_a_kind() {
    let mut map = KindAccessMap::new();
    map.set(ObjectKind::Deployment, KindAccess::Unknown);
    map.set(
        ObjectKind::Deployment,
        KindAccess::Known(report(ObjectKind::Deployment, AccessDecision::Allowed)),
    );
    map.set(ObjectKind::ConfigMap, KindAccess::Unknown);
    assert_eq!(map.len(), 2);
    assert!(matches!(
        map.get(ObjectKind::Deployment),
        Some(KindAccess::Known(_))
    ));
}

#[test]
fn clear_forgets_every_answer() {
    let mut map = KindAccessMap::new();
    map.set(ObjectKind::Deployment, KindAccess::Unknown);
    map.clear();
    assert!(map.get(ObjectKind::Deployment).is_none());
}

#[test]
fn kind_access_includes_delete() {
    assert_eq!(
        lazy_checks(ObjectKind::Deployment),
        [
            AccessCheck::Update(ObjectKind::Deployment),
            AccessCheck::Delete(ObjectKind::Deployment)
        ]
    );
    // Not editable: only the delete right is asked.
    assert_eq!(
        lazy_checks(ObjectKind::Node),
        [AccessCheck::Delete(ObjectKind::Node)]
    );
}

#[test]
fn config_maps_and_secrets_also_ask_patch() {
    assert_eq!(
        lazy_checks(ObjectKind::Secret),
        [
            AccessCheck::Update(ObjectKind::Secret),
            AccessCheck::Patch(ObjectKind::Secret),
            AccessCheck::Delete(ObjectKind::Secret)
        ]
    );
    // No other kind asks for it: a merge patch of values exists for these two only.
    for kind in ObjectKind::ALL {
        if !matches!(kind, ObjectKind::ConfigMap | ObjectKind::Secret) {
            assert!(
                !lazy_checks(kind).contains(&AccessCheck::Patch(kind)),
                "{kind:?}"
            );
        }
    }
}

#[test]
fn lazy_checks_ask_create_for_creatable_kinds() {
    let config_map = ObjectKind::ConfigMap;
    assert_eq!(
        lazy_checks(config_map),
        [
            AccessCheck::Update(config_map),
            AccessCheck::Patch(config_map),
            AccessCheck::Delete(config_map),
            AccessCheck::Create(config_map)
        ]
    );
    // Not editable, but creatable: delete, then create.
    assert_eq!(
        lazy_checks(ObjectKind::Namespace),
        [
            AccessCheck::Delete(ObjectKind::Namespace),
            AccessCheck::Create(ObjectKind::Namespace)
        ]
    );
    for kind in ObjectKind::ALL {
        assert_eq!(
            lazy_checks(kind).contains(&AccessCheck::Create(kind)),
            kind.is_creatable(),
            "{kind:?}"
        );
    }
}
