//! Email two-factor codes for the sensitive reveals.
//!
//! A six-digit code is generated, emailed, and held only in memory with a
//! short life. The attempt policy follows the spec exactly: three wrong tries,
//! then a one-minute lockout on the same code, two more tries after it, and on
//! the fifth failure the code is abandoned and the caller falls back to the
//! seed phrase. Getting it right hands back a short-lived pass that a reveal
//! consumes.
//!
//! Six digits, not four, so the space is a million rather than ten thousand.

use std::time::{SystemTime, UNIX_EPOCH};

use rand::Rng;

/// How long a freshly issued code stays valid.
const CODE_TTL: i64 = 5 * 60;
/// The lockout after the third wrong try.
const LOCKOUT: i64 = 60;
/// How long a passed check authorises a reveal before it must be redone.
const PASS_TTL: i64 = 2 * 60;
const MAX_ATTEMPTS: u32 = 5;
const LOCK_AT: u32 = 3;

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// A code in flight.
#[derive(Clone)]
pub struct Pending {
    code: String,
    created: i64,
    attempts: u32,
    locked_until: Option<i64>,
}

/// The outcome of checking an entered code.
#[derive(Debug, PartialEq, Eq)]
pub enum Verify {
    /// Correct. The caller may proceed, and gets a pass valid for a short time.
    Ok,
    /// Wrong, with this many tries left before the lockout or abandonment.
    Wrong { remaining: u32 },
    /// Locked out until this Unix second; the same code stays valid after.
    LockedOut { until: i64 },
    /// The code aged out. A new one must be requested.
    Expired,
    /// Five failures. The code is dead; fall back to the seed phrase.
    Abandoned,
}

/// A fresh six-digit code, zero-padded so every code is six characters.
pub fn new_code() -> String {
    let n: u32 = rand::rngs::OsRng.gen_range(0..1_000_000);
    format!("{n:06}")
}

pub fn begin(code: String) -> Pending {
    Pending {
        code,
        created: now(),
        attempts: 0,
        locked_until: None,
    }
}

impl Pending {
    /// Checks an entered code and advances the ladder.
    pub fn verify(&mut self, input: &str) -> Verify {
        self.verify_at(input, now())
    }

    fn verify_at(&mut self, input: &str, at: i64) -> Verify {
        if at - self.created >= CODE_TTL {
            return Verify::Expired;
        }
        if let Some(until) = self.locked_until {
            if at < until {
                return Verify::LockedOut { until };
            }
        }
        if input == self.code {
            return Verify::Ok;
        }

        self.attempts += 1;

        if self.attempts >= MAX_ATTEMPTS {
            return Verify::Abandoned;
        }
        if self.attempts == LOCK_AT {
            self.locked_until = Some(at + LOCKOUT);
            return Verify::LockedOut { until: at + LOCKOUT };
        }
        Verify::Wrong {
            remaining: MAX_ATTEMPTS - self.attempts,
        }
    }
}

/// The moment a check passed, so a reveal can confirm it was recent.
pub fn pass_expires_at() -> i64 {
    now() + PASS_TTL
}

/// Whether a pass issued to expire at `expiry` is still good.
pub fn pass_valid(expiry: i64) -> bool {
    now() < expiry
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pending() -> Pending {
        Pending {
            code: "123456".into(),
            created: 1000,
            attempts: 0,
            locked_until: None,
        }
    }

    #[test]
    fn a_correct_code_passes() {
        let mut p = pending();
        assert_eq!(p.verify_at("123456", 1010), Verify::Ok);
    }

    #[test]
    fn codes_are_six_digits() {
        for _ in 0..200 {
            let c = new_code();
            assert_eq!(c.len(), 6);
            assert!(c.chars().all(|d| d.is_ascii_digit()));
        }
    }

    #[test]
    fn three_wrong_tries_lock_it_for_a_minute() {
        let mut p = pending();
        assert_eq!(p.verify_at("000000", 1001), Verify::Wrong { remaining: 4 });
        assert_eq!(p.verify_at("000000", 1002), Verify::Wrong { remaining: 3 });
        // The third wrong try triggers the lockout.
        assert_eq!(p.verify_at("000000", 1003), Verify::LockedOut { until: 1063 });
        // During the lockout, even the right code is held off.
        assert_eq!(p.verify_at("123456", 1030), Verify::LockedOut { until: 1063 });
    }

    #[test]
    fn the_same_code_still_works_after_the_lockout() {
        let mut p = pending();
        for t in 1..=3 {
            p.verify_at("000000", 1000 + t);
        }
        // After the minute, the original code is accepted.
        assert_eq!(p.verify_at("123456", 1064), Verify::Ok);
    }

    #[test]
    fn the_fifth_failure_abandons_it() {
        let mut p = pending();
        p.verify_at("000000", 1001); // 1
        p.verify_at("000000", 1002); // 2
        p.verify_at("000000", 1003); // 3 -> lock until 1063
        p.verify_at("000000", 1064); // 4
        // The fifth wrong try, past the lockout, kills the code.
        assert_eq!(p.verify_at("000000", 1065), Verify::Abandoned);
    }

    #[test]
    fn an_expired_code_is_rejected() {
        let mut p = pending();
        assert_eq!(p.verify_at("123456", 1000 + CODE_TTL), Verify::Expired);
    }

    #[test]
    fn a_pass_ages_out() {
        assert!(pass_valid(now() + 30));
        assert!(!pass_valid(now() - 1));
    }
}
