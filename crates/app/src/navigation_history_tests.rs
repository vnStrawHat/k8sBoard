use super::*;

fn place(screen: Screen) -> Place {
    Place {
        screen,
        selection: None,
        is_drawer_open: false,
        tab: DrawerTab::Overview,
        container: None,
        container_tab: ContainerTab::Info,
        filter: None,
    }
}

fn served(_: &Place) -> bool {
    true
}

#[test]
fn back_returns_the_recorded_place_and_forward_redoes_the_current() {
    let mut history = NavigationHistory::default();
    history.record(place(Screen::Pods));
    let back = history.back(place(Screen::Nodes), served);
    assert_eq!(back, Some(place(Screen::Pods)));
    assert!(history.previous().is_none());
    let forward = history.forward(place(Screen::Pods), served);
    assert_eq!(forward, Some(place(Screen::Nodes)));
    assert_eq!(history.previous(), Some(&place(Screen::Pods)));
}

#[test]
fn empty_stacks_return_nothing() {
    let mut history = NavigationHistory::default();
    assert_eq!(history.back(place(Screen::Pods), served), None);
    assert_eq!(history.forward(place(Screen::Pods), served), None);
    assert!(history.previous().is_none());
}

#[test]
fn a_new_record_clears_forward() {
    let mut history = NavigationHistory::default();
    history.record(place(Screen::Pods));
    history.back(place(Screen::Nodes), served);
    history.record(place(Screen::Issues));
    assert_eq!(history.forward(place(Screen::Topology), served), None);
}

#[test]
fn back_keeps_at_most_fifty_places() {
    let mut history = NavigationHistory::default();
    for index in 0..60 {
        let mut from = place(Screen::Pods);
        from.container = Some(index.to_string());
        history.record(from);
    }
    let mut seen = 0;
    let mut oldest = None;
    while let Some(back) = history.back(place(Screen::Nodes), served) {
        seen += 1;
        oldest = back.container;
    }
    assert_eq!(seen, 50);
    assert_eq!(oldest.as_deref(), Some("10"));
}

#[test]
fn forward_is_bounded_by_the_same_cap() {
    let mut history = NavigationHistory::default();
    for _ in 0..50 {
        history.record(place(Screen::Pods));
    }
    for _ in 0..50 {
        history.back(place(Screen::Nodes), served);
    }
    // Walking forward and back again must not grow either stack past the cap.
    for _ in 0..50 {
        history.forward(place(Screen::Nodes), served);
    }
    history.record(place(Screen::Issues));
    let mut seen = 0;
    while history.back(place(Screen::Nodes), served).is_some() {
        seen += 1;
    }
    assert_eq!(seen, 50);
}

#[test]
fn clear_forgets_both_stacks() {
    let mut history = NavigationHistory::default();
    history.record(place(Screen::Pods));
    history.record(place(Screen::Nodes));
    history.back(place(Screen::Issues), served);
    history.clear();
    assert!(history.previous().is_none());
    assert_eq!(history.forward(place(Screen::Pods), served), None);
}

#[test]
fn unserved_places_are_consumed_and_never_reach_forward() {
    let mut history = NavigationHistory::default();
    history.record(place(Screen::Pods));
    history.record(place(Screen::Issues));
    let back = history.back(place(Screen::Nodes), |place| place.screen != Screen::Issues);
    assert_eq!(back, Some(place(Screen::Pods)));
    // Only the current place was pushed forward, not the skipped one.
    assert_eq!(
        history.forward(place(Screen::Pods), served),
        Some(place(Screen::Nodes))
    );
    assert_eq!(history.forward(place(Screen::Pods), served), None);
}

#[test]
fn no_served_place_leaves_the_current_one_off_the_forward_stack() {
    let mut history = NavigationHistory::default();
    history.record(place(Screen::Issues));
    assert_eq!(history.back(place(Screen::Nodes), |_| false), None);
    assert_eq!(history.forward(place(Screen::Pods), served), None);
}
