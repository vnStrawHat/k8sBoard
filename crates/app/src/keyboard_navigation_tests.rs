use super::*;

#[test]
fn step_row_moves_one_row() {
    assert_eq!(step_row(Some(3), 10, RowStep::Next, 5), Some(4));
    assert_eq!(step_row(Some(3), 10, RowStep::Previous, 5), Some(2));
}

#[test]
fn step_row_wraps_next_and_previous() {
    assert_eq!(step_row(Some(9), 10, RowStep::Next, 5), Some(0));
    assert_eq!(step_row(Some(0), 10, RowStep::Previous, 5), Some(9));
}

#[test]
fn step_row_starts_at_the_top_without_a_cursor() {
    for step in [
        RowStep::Next,
        RowStep::Previous,
        RowStep::First,
        RowStep::NextPage,
        RowStep::PreviousPage,
    ] {
        assert_eq!(step_row(None, 10, step, 5), Some(0), "{step:?}");
    }
    assert_eq!(step_row(None, 10, RowStep::Last, 5), Some(9));
}

#[test]
fn step_row_pages_and_ends_clamp() {
    assert_eq!(step_row(Some(5), 12, RowStep::NextPage, 10), Some(11));
    assert_eq!(step_row(Some(5), 12, RowStep::PreviousPage, 10), Some(0));
    assert_eq!(step_row(Some(5), 12, RowStep::First, 10), Some(0));
    assert_eq!(step_row(Some(5), 12, RowStep::Last, 10), Some(11));
}

#[test]
fn step_row_has_no_target_in_an_empty_table() {
    for step in [RowStep::Next, RowStep::Last, RowStep::PreviousPage] {
        assert_eq!(step_row(None, 0, step, 5), None, "{step:?}");
        assert_eq!(step_row(Some(2), 0, step, 5), None, "{step:?}");
    }
}

#[test]
fn step_row_treats_a_stale_cursor_as_the_last_row() {
    assert_eq!(step_row(Some(9), 3, RowStep::Previous, 5), Some(1));
    assert_eq!(step_row(Some(9), 3, RowStep::NextPage, 5), Some(2));
}

#[test]
fn dismiss_step_follows_the_ladder() {
    let all = DismissState {
        is_dock_zoomed: true,
        is_drawer_open: true,
        has_selection: true,
    };
    assert_eq!(dismiss_step(all), DismissStep::UnzoomDock);
    let drawer = DismissState {
        is_dock_zoomed: false,
        ..all
    };
    assert_eq!(dismiss_step(drawer), DismissStep::CloseDrawer);
    let row = DismissState {
        is_drawer_open: false,
        ..drawer
    };
    assert_eq!(dismiss_step(row), DismissStep::ClearSelection);
    let nothing = DismissState {
        has_selection: false,
        ..row
    };
    assert_eq!(dismiss_step(nothing), DismissStep::Propagate);
}

#[test]
fn step_container_clamps_in_display_order() {
    // Display order: container 2 first, then 0, then 1.
    let order = [2, 0, 1];
    assert_eq!(
        step_container(&order, Some(2), ContainerStep::Next),
        Some(0)
    );
    assert_eq!(
        step_container(&order, Some(1), ContainerStep::Next),
        Some(1)
    );
    assert_eq!(
        step_container(&order, Some(2), ContainerStep::Previous),
        Some(2)
    );
    assert_eq!(
        step_container(&order, Some(1), ContainerStep::Previous),
        Some(0)
    );
    assert_eq!(step_container(&order, None, ContainerStep::Next), Some(2));
    assert_eq!(
        step_container(&order, None, ContainerStep::Previous),
        Some(2)
    );
    assert_eq!(step_container(&[], Some(0), ContainerStep::Next), None);
}

// ---- Edit values (spec 0047) ----

#[test]
fn values_screen_context_follows_the_visible_screen() {
    use crate::resource_kind::ResourceKind;
    for kind in [ResourceKind::ConfigMaps, ResourceKind::Secrets] {
        assert_eq!(
            shell_key_context(Screen::Kind(kind)),
            "AppShell ValuesScreen",
            "{kind:?}"
        );
    }
    for screen in [
        Screen::Kind(ResourceKind::HelmReleases),
        Screen::Kind(ResourceKind::Deployments),
        Screen::Pods,
        Screen::Nodes,
        Screen::Overview,
    ] {
        assert_eq!(shell_key_context(screen), "AppShell", "{screen:?}");
    }
}
