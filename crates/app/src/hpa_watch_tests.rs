use super::*;

fn hpa() -> ManagingHpa {
    ManagingHpa {
        name: "web".to_owned(),
        min: 1,
        max: 3,
    }
}

#[test]
fn replicas_that_stay_as_asked_are_not_a_revert() {
    let mut follow = ScaleFollow::new(5);
    assert_eq!(follow.moved_to(5), None);
    assert_eq!(follow.moved_to(5), None);
}

#[test]
fn a_watch_that_still_shows_the_old_count_is_not_a_revert() {
    let mut follow = ScaleFollow::new(5);
    assert_eq!(follow.moved_to(3), None);
    assert_eq!(follow.moved_to(3), None);
    assert_eq!(follow.moved_to(5), None);
}

#[test]
fn replicas_that_leave_the_asked_count_are_a_revert() {
    let mut follow = ScaleFollow::new(5);
    assert_eq!(follow.moved_to(5), None);
    assert_eq!(follow.moved_to(3), Some(3));
}

#[test]
fn replicas_that_never_showed_the_asked_count_but_moved_are_a_revert() {
    let mut follow = ScaleFollow::new(5);
    assert_eq!(follow.moved_to(1), None);
    assert_eq!(follow.moved_to(3), Some(3));
}

#[test]
fn the_notice_names_the_bound_the_hpa_stopped_at() {
    assert_eq!(
        hpa_revert_text(&hpa(), 3),
        "HPA web set replicas back to 3 (max 3)"
    );
    assert_eq!(
        hpa_revert_text(&hpa(), 1),
        "HPA web set replicas back to 1 (min 1)"
    );
    assert_eq!(hpa_revert_text(&hpa(), 2), "HPA web set replicas back to 2");
}
