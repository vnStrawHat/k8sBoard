use std::time::Instant;

use super::*;
use crate::secret_clipboard::ClipboardWriteError;

fn release(revision: u32) -> HelmReleaseSummary {
    HelmReleaseSummary {
        namespace: "shop".to_owned(),
        name: "api".to_owned(),
        revision,
        status: cluster::HelmStatus::Deployed,
        chart: None,
        updated_at: None,
        description: None,
        deployed_revision: None,
    }
}

fn release_key() -> ResourceKey {
    ResourceKey::Kind {
        kind: ResourceKind::HelmReleases,
        namespace: Some("shop".to_owned()),
        name: "api".to_owned(),
    }
}

fn slots(has_earlier: bool, detail: SlotState, diff: SlotState) -> SlotStates {
    SlotStates {
        has_earlier,
        detail,
        diff,
        revealed: None,
    }
}

fn revealing(base: SlotStates, payload: SlotState, diff: SlotState) -> SlotStates {
    SlotStates {
        revealed: Some(RevealedSlots { payload, diff }),
        ..base
    }
}

const ABSENT: SlotState = SlotState::Absent;

#[test]
fn next_need_for_each_tab_layout_and_reveal() {
    use HelmTab::{Manifest, Notes, Overview, Values};
    use ValueVisibility::{Masked, Revealed};
    use ValuesLayout::{Diff, Document};
    let masked = slots(true, ABSENT, ABSENT);
    let revealed = revealing(masked, ABSENT, ABSENT);
    let cases = [
        (Overview, Document, masked, Need::Diff(Masked)),
        (Overview, Document, revealed, Need::Diff(Revealed)),
        (Values, Document, masked, Need::Detail),
        (Values, Document, revealed, Need::Revealed),
        (Values, Diff, masked, Need::Diff(Masked)),
        (Values, Diff, revealed, Need::Diff(Revealed)),
        (Manifest, Document, masked, Need::Detail),
        (Manifest, Document, revealed, Need::Detail),
        (Notes, Document, masked, Need::Detail),
        (Notes, Document, revealed, Need::Revealed),
    ];
    for (tab, layout, slots, expected) in cases {
        assert_eq!(
            next_need(tab, layout, slots),
            Some(expected),
            "{tab:?} {layout:?}"
        );
    }
}

#[test]
fn failed_slot_waits_for_refresh() {
    for state in [SlotState::Running, SlotState::Ready, SlotState::Failed] {
        let slots = slots(true, state, state);
        for tab in [
            HelmTab::Overview,
            HelmTab::Values,
            HelmTab::Manifest,
            HelmTab::Notes,
        ] {
            assert_eq!(
                next_need(tab, ValuesLayout::Document, slots),
                None,
                "{tab:?}"
            );
        }
    }
}

#[test]
fn manifest_never_needs_revealed_values() {
    let revealed = revealing(slots(true, ABSENT, ABSENT), ABSENT, ABSENT);
    assert_eq!(
        next_need(HelmTab::Manifest, ValuesLayout::Document, revealed),
        Some(Need::Detail)
    );
    let ready = revealing(slots(true, SlotState::Ready, ABSENT), ABSENT, ABSENT);
    assert_eq!(
        next_need(HelmTab::Manifest, ValuesLayout::Document, ready),
        None
    );
}

#[test]
fn notes_open_masked_and_expire() {
    let masked = slots(false, ABSENT, ABSENT);
    assert_eq!(
        next_need(HelmTab::Notes, ValuesLayout::Document, masked),
        Some(Need::Detail)
    );
    let ready = slots(false, SlotState::Ready, ABSENT);
    assert_eq!(
        next_need(HelmTab::Notes, ValuesLayout::Document, ready),
        None
    );
    // A Reveal asks for the notes once.
    let revealed = revealing(ready, ABSENT, ABSENT);
    assert_eq!(
        next_need(HelmTab::Notes, ValuesLayout::Document, revealed),
        Some(Need::Revealed)
    );
    // After 30 s the Reveal is gone and the masked state needs nothing more.
    assert_eq!(
        next_need(HelmTab::Notes, ValuesLayout::Document, ready),
        None
    );
}

#[test]
fn tab_change_drops_revealed() {
    assert!(tab_change_hides(HelmTab::Values, HelmTab::Notes));
    assert!(tab_change_hides(HelmTab::Values, HelmTab::Overview));
    assert!(!tab_change_hides(HelmTab::Values, HelmTab::Values));
}

#[test]
fn earlier_revision_is_highest_below() {
    assert_eq!(earlier_revision([10, 9, 5, 2], 9), Some(5));
    assert_eq!(earlier_revision([10, 9, 5, 2], 10), Some(9));
    assert_eq!(earlier_revision([10, 9, 5, 2], 2), None);
    // A pruned history still compares with what is left.
    assert_eq!(earlier_revision([38, 36], 38), Some(36));
    assert_eq!(earlier_revision([], 3), None);
}

#[test]
fn diff_needs_an_earlier_revision() {
    let none = slots(false, ABSENT, ABSENT);
    for tab in [HelmTab::Overview, HelmTab::Values] {
        assert_eq!(next_need(tab, ValuesLayout::Diff, none), None, "{tab:?}");
    }
    let loaded = |earlier| HistoryState::Loaded { earlier };
    assert_eq!(values_change(HistoryState::Loading), ValuesChange::Loading);
    assert_eq!(values_change(loaded(None)), ValuesChange::FirstRevision);
    assert_eq!(values_change(loaded(Some(2))), ValuesChange::Diff);
}

#[test]
fn values_changed_section_notes_first_revision() {
    // No earlier revision in a loaded History reads "first stored revision", not an endless load.
    assert_eq!(
        values_change(HistoryState::Loaded { earlier: None }),
        ValuesChange::FirstRevision
    );
    assert_ne!(
        values_change(HistoryState::Loading),
        ValuesChange::FirstRevision
    );
}

#[test]
fn shown_text_tracks_source_env_and_reveal() {
    use ValuesLayout::{Diff, Document};
    assert_eq!(
        shown_text(
            HelmTab::Values,
            Document,
            ValuesSource::User,
            EnvValues::Hidden,
            false
        ),
        Some(ShownText::Values(
            ValuesSource::User,
            ValueVisibility::Masked
        ))
    );
    assert_eq!(
        shown_text(
            HelmTab::Values,
            Document,
            ValuesSource::Computed,
            EnvValues::Hidden,
            true
        ),
        Some(ShownText::Values(
            ValuesSource::Computed,
            ValueVisibility::Revealed
        ))
    );
    assert_eq!(
        shown_text(
            HelmTab::Manifest,
            Document,
            ValuesSource::User,
            EnvValues::Shown,
            true
        ),
        Some(ShownText::Manifest(EnvValues::Shown))
    );
    for (tab, layout) in [
        (HelmTab::Overview, Document),
        (HelmTab::Notes, Document),
        (HelmTab::Values, Diff),
    ] {
        assert_eq!(
            shown_text(tab, layout, ValuesSource::User, EnvValues::Hidden, false),
            None
        );
    }
}

#[test]
fn reveal_expires_after_thirty_seconds() {
    let start = Instant::now();
    let hides_at = start + REVEAL_DURATION;
    assert!(!is_expired(None, start + REVEAL_DURATION * 2));
    assert!(!is_expired(Some(hides_at), start));
    assert!(!is_expired(
        Some(hides_at),
        hides_at - Duration::from_millis(1)
    ));
    assert!(is_expired(Some(hides_at), hides_at));
    assert_eq!(REVEAL_DURATION, Duration::from_secs(30));
    assert_eq!(seconds_left(hides_at, start), 30);
    assert_eq!(seconds_left(hides_at, hides_at), 0);
}

#[test]
fn revealed_copy_emits_private_clipboard_mark() {
    let mut written = String::new();
    let result = private_copy(Zeroizing::new("replicas".to_owned()), |text| {
        written.push_str(text);
        Ok::<_, ClipboardWriteError>(ClipboardMark::of(text))
    });
    let Some(Ok(mark)) = result else {
        panic!("expected a mark");
    };
    assert_eq!(written, "replicas");
    assert!(mark.matches("replicas"));
}

#[test]
fn revealed_copy_fails_closed() {
    let result = private_copy(Zeroizing::new("replicas".to_owned()), |_| {
        Err::<ClipboardMark, _>(ClipboardWriteError::Unavailable)
    });
    assert!(matches!(
        result,
        Some(Err(ClipboardWriteError::Unavailable))
    ));
    // The view shows this fixed line and writes nothing else.
    assert_eq!(COPY_FAILED, "Copy failed: the clipboard is unavailable.");
}

#[test]
fn empty_selection_copies_nothing() {
    let mut calls = 0;
    let result = private_copy(Zeroizing::new(String::new()), |text| {
        calls += 1;
        Ok::<_, ClipboardWriteError>(ClipboardMark::of(text))
    });
    assert!(result.is_none());
    assert_eq!(calls, 0);
}

#[test]
fn helm_subject_only_on_helm_tabs() {
    let key = release_key();
    let summary = release(7);
    for tab in [
        DrawerTab::Containers,
        DrawerTab::Monitor,
        DrawerTab::Yaml,
        DrawerTab::Events,
    ] {
        assert_eq!(
            helm_subject(Some(&key), tab, Some(&summary), None),
            None,
            "{tab:?}"
        );
    }
    let other = ResourceKey::Kind {
        kind: ResourceKind::Secrets,
        namespace: Some("shop".to_owned()),
        name: "api".to_owned(),
    };
    assert_eq!(
        helm_subject(Some(&other), DrawerTab::Values, Some(&summary), None),
        None
    );
    assert_eq!(
        helm_subject(None, DrawerTab::Values, Some(&summary), None),
        None
    );
    assert_eq!(
        helm_subject(Some(&key), DrawerTab::Values, None, None),
        None
    );
    let tabs = [
        (DrawerTab::Overview, HelmTab::Overview),
        (DrawerTab::Values, HelmTab::Values),
        (DrawerTab::Manifest, HelmTab::Manifest),
        (DrawerTab::Notes, HelmTab::Notes),
    ];
    for (drawer, helm) in tabs {
        let (_, tab) = helm_subject(Some(&key), drawer, Some(&summary), None).expect("a subject");
        assert_eq!(tab, helm);
    }
}

#[test]
fn helm_subject_overview_uses_latest() {
    let (revision, tab) = helm_subject(
        Some(&release_key()),
        DrawerTab::Overview,
        Some(&release(7)),
        Some(3),
    )
    .expect("a subject");
    assert_eq!(tab, HelmTab::Overview);
    assert_eq!(revision.revision, 7);
    assert_eq!(
        (revision.namespace.as_str(), revision.release.as_str()),
        ("shop", "api")
    );
}

#[test]
fn helm_subject_follows_chosen_revision() {
    let chosen = helm_subject(
        Some(&release_key()),
        DrawerTab::Values,
        Some(&release(7)),
        Some(3),
    )
    .expect("a subject");
    assert_eq!(chosen.0.revision, 3);
    let latest = helm_subject(
        Some(&release_key()),
        DrawerTab::Manifest,
        Some(&release(7)),
        None,
    )
    .expect("a subject");
    assert_eq!(latest.0.revision, 7);
}

#[test]
fn failed_history_is_settled_and_says_why() {
    assert_eq!(
        values_change(HistoryState::Failed),
        ValuesChange::HistoryFailed
    );
    assert_eq!(
        no_earlier_text(HistoryState::Failed),
        "The history could not be read."
    );
    assert_eq!(
        no_earlier_text(HistoryState::Loading),
        "Loading the history…"
    );
    assert_eq!(
        no_earlier_text(HistoryState::Loaded { earlier: None }),
        "No earlier revision"
    );
    // A failed History offers no diff, so nothing is fetched and nothing keeps loading.
    let none = slots(false, ABSENT, ABSENT);
    assert_eq!(next_need(HelmTab::Overview, ValuesLayout::Diff, none), None);
}
