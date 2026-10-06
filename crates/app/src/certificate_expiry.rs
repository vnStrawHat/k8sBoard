//! When a certificate expires, and how that reads. Expiry is always the leaf's not-after: an
//! intermediate that expires earlier gets its own note and never replaces the leaf date.

use cluster::CertificateInfo;
use jiff::{SignedDuration, Timestamp};

use crate::age::format_age;
use crate::status_tone::{StatusLabel, StatusTone};

/// A certificate this close to its not-after is a warning.
pub(crate) const EXPIRY_WARNING: SignedDuration = SignedDuration::from_hours(14 * 24);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ExpiryState {
    Valid,
    ExpiringSoon,
    Expired,
    NotYetValid,
}

/// `leaf` is `chain[0]`; `SecretDetails::Certificate` guarantees a non-empty chain. The first
/// matching case wins.
pub(crate) fn expiry_state(leaf: &CertificateInfo, now: Timestamp) -> ExpiryState {
    if leaf.not_after <= now {
        return ExpiryState::Expired;
    }
    if leaf.not_before > now {
        return ExpiryState::NotYetValid;
    }
    if is_within_warning(leaf.not_after, now) {
        return ExpiryState::ExpiringSoon;
    }
    ExpiryState::Valid
}

fn is_within_warning(not_after: Timestamp, now: Timestamp) -> bool {
    not_after.duration_since(now) <= EXPIRY_WARNING
}

/// The cell and field text, read at paint time so days left never go stale.
pub(crate) fn expiry_label(not_after: Timestamp, now: Timestamp) -> StatusLabel {
    if not_after <= now {
        return StatusLabel {
            text: format!("expired {} ago", format_age(Some(not_after), now)).into(),
            tone: StatusTone::Bad,
        };
    }
    let left = format_age(Some(now), not_after);
    if is_within_warning(not_after, now) {
        return StatusLabel {
            text: format!("expires in {left}").into(),
            tone: StatusTone::Warn,
        };
    }
    StatusLabel {
        text: format!("{left} left").into(),
        tone: StatusTone::Ok,
    }
}

/// The drawer field text: the absolute date, then the relative one, `Dec 25, 2026 (81d left)`.
pub(crate) fn expiry_detail_label(not_after: Timestamp, now: Timestamp) -> StatusLabel {
    let relative = expiry_label(not_after, now);
    StatusLabel {
        text: format!("{} ({})", date_text(not_after), relative.text).into(),
        tone: relative.tone,
    }
}

/// The earliest not-after among `chain[1..]` when it is before the leaf's.
pub(crate) fn intermediate_expires_first(chain: &[CertificateInfo]) -> Option<Timestamp> {
    let (leaf, rest) = chain.split_first()?;
    let earliest = rest.iter().map(|certificate| certificate.not_after).min()?;
    (earliest < leaf.not_after).then_some(earliest)
}

/// `Oct 7, 2026`, in UTC, for the text of a box.
pub(crate) fn date_text(time: Timestamp) -> String {
    time.strftime("%b %-d, %Y").to_string()
}

/// `2026-10-07 14:30 UTC`, for a drawer field.
pub(crate) fn date_time_text(time: Timestamp) -> String {
    time.strftime("%Y-%m-%d %H:%M UTC").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: i64 = 24 * 3600;

    fn at(seconds: i64) -> Timestamp {
        Timestamp::from_second(seconds).expect("valid timestamp")
    }

    fn certificate(not_before: i64, not_after: i64) -> CertificateInfo {
        CertificateInfo {
            subject: "CN=leaf".to_owned(),
            issuer: "CN=ca".to_owned(),
            alt_names: Vec::new(),
            not_before: at(not_before),
            not_after: at(not_after),
        }
    }

    #[test]
    fn expiry_uses_leaf_not_after() {
        let now = at(100 * DAY);
        let leaf = certificate(0, 200 * DAY);
        let intermediate = certificate(0, 101 * DAY);
        assert_eq!(expiry_state(&leaf, now), ExpiryState::Valid);
        // The chain is not an input of the state at all, only the leaf.
        assert_eq!(
            intermediate_expires_first(&[leaf, intermediate]),
            Some(at(101 * DAY))
        );
    }

    #[test]
    fn intermediate_expiring_first_is_reported() {
        let chain = [
            certificate(0, 300 * DAY),
            certificate(0, 250 * DAY),
            certificate(0, 200 * DAY),
        ];
        assert_eq!(intermediate_expires_first(&chain), Some(at(200 * DAY)));
    }

    #[test]
    fn later_intermediate_is_not_reported() {
        let chain = [certificate(0, 200 * DAY), certificate(0, 300 * DAY)];
        assert_eq!(intermediate_expires_first(&chain), None);
        assert_eq!(intermediate_expires_first(&chain[..1]), None);
        assert_eq!(intermediate_expires_first(&[]), None);
    }

    #[test]
    fn expired_state_and_label() {
        let now = at(100 * DAY);
        let leaf = certificate(0, 97 * DAY);
        assert_eq!(expiry_state(&leaf, now), ExpiryState::Expired);
        let label = expiry_label(leaf.not_after, now);
        assert_eq!(label.text, "expired 3d ago");
        assert_eq!(label.tone, StatusTone::Bad);
        // Expiry at this very second counts as expired.
        assert_eq!(
            expiry_state(&certificate(0, 100 * DAY), now),
            ExpiryState::Expired
        );
    }

    #[test]
    fn expiring_within_fourteen_days() {
        let now = at(100 * DAY);
        let leaf = certificate(0, 114 * DAY);
        assert_eq!(expiry_state(&leaf, now), ExpiryState::ExpiringSoon);
        let label = expiry_label(leaf.not_after, now);
        assert_eq!(label.text, "expires in 14d");
        assert_eq!(label.tone, StatusTone::Warn);
    }

    #[test]
    fn fifteen_days_is_valid() {
        let now = at(100 * DAY);
        let leaf = certificate(0, 115 * DAY);
        assert_eq!(expiry_state(&leaf, now), ExpiryState::Valid);
        let label = expiry_label(leaf.not_after, now);
        assert_eq!(label.text, "15d left");
        assert_eq!(label.tone, StatusTone::Ok);
    }

    #[test]
    fn not_yet_valid_state() {
        let now = at(100 * DAY);
        let leaf = certificate(110 * DAY, 400 * DAY);
        assert_eq!(expiry_state(&leaf, now), ExpiryState::NotYetValid);
        // The label still follows the not-after.
        assert_eq!(expiry_label(leaf.not_after, now).tone, StatusTone::Ok);
    }

    #[test]
    fn detail_label_has_date_and_relative_text() {
        let now: Timestamp = "2026-10-05T00:00:00Z".parse().expect("timestamp");
        let later: Timestamp = "2026-12-25T00:00:00Z".parse().expect("timestamp");
        let label = expiry_detail_label(later, now);
        assert_eq!(label.text, "Dec 25, 2026 (81d left)");
        assert_eq!(label.tone, StatusTone::Ok);
        let earlier: Timestamp = "2026-09-01T00:00:00Z".parse().expect("timestamp");
        let label = expiry_detail_label(earlier, now);
        assert_eq!(label.text, "Sep 1, 2026 (expired 34d ago)");
        assert_eq!(label.tone, StatusTone::Bad);
    }

    #[test]
    fn dates_are_utc() {
        let time: Timestamp = "2026-10-07T23:30:00Z".parse().expect("timestamp");
        assert_eq!(date_text(time), "Oct 7, 2026");
        assert_eq!(date_time_text(time), "2026-10-07 23:30 UTC");
    }
}
