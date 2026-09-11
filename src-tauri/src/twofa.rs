//! The state around the two-factor gate: the short-lived pass a reveal
//! consumes, and the lockout that slows down guessing.
//!
//! The codes themselves are TOTP now (see [`crate::totp`]), computed from a
//! secret on the user's phone rather than emailed, so there is no code to store
//! here. What remains is timing: a correct check hands out a pass good for a
//! couple of minutes, and a run of wrong codes locks the check briefly so the
//! six-digit space cannot be walked through while a code is valid.

use std::time::{SystemTime, UNIX_EPOCH};

/// How long a passed check authorises reveals before it must be redone.
const PASS_TTL: i64 = 2 * 60;
/// Wrong tries before a lockout kicks in.
pub const LOCK_AT: u32 = 3;
/// How long that lockout lasts.
pub const LOCKOUT: i64 = 60;

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// The moment a check passed plus its lifetime, so a reveal can confirm it was
/// recent.
pub fn pass_expires_at() -> i64 {
    now() + PASS_TTL
}

/// Whether a pass issued to expire at `expiry` is still good.
pub fn pass_valid(expiry: i64) -> bool {
    now() < expiry
}

/// After a wrong code takes the count to `wrong`, the Unix second a lockout
/// should run until, or `None` while still under the threshold.
pub fn lock_after(wrong: u32, at: i64) -> Option<i64> {
    if wrong >= LOCK_AT {
        Some(at + LOCKOUT)
    } else {
        None
    }
}

/// Whether a lockout set to lift at `until` is still in force.
pub fn locked(until: Option<i64>, at: i64) -> bool {
    matches!(until, Some(u) if at < u)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pass_ages_out() {
        assert!(pass_valid(now() + 30));
        assert!(!pass_valid(now() - 1));
    }

    #[test]
    fn the_lockout_starts_on_the_third_wrong_try() {
        assert_eq!(lock_after(1, 1000), None);
        assert_eq!(lock_after(2, 1000), None);
        assert_eq!(lock_after(3, 1000), Some(1060));
        assert_eq!(lock_after(4, 1000), Some(1060));
    }

    #[test]
    fn a_lockout_holds_until_its_moment() {
        assert!(locked(Some(1060), 1030));
        assert!(!locked(Some(1060), 1060));
        assert!(!locked(Some(1060), 1090));
        assert!(!locked(None, 1030));
    }
}
