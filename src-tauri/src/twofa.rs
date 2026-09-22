//! The state around the two-factor gate: the short-lived pass a guarded
//! action consumes, the lockout that slows down guessing, and the policy that
//! decides which factors a given action has to clear.
//!
//! The code is TOTP (see [`crate::totp`]), computed from a secret on the
//! user's phone. It never crosses a network, so there is nothing in transit
//! to intercept and no server involved at any point.
//!
//! What lives here is timing: a correct check hands out a pass good for a
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

// ------------------------------------------------------------------------
// Policy: which factors a guarded action has to clear
// ------------------------------------------------------------------------

use crate::appconfig::{AppConfig, LARGE_SEND_SHARE, LARGE_SEND_USD};

/// Something the wallet will stop and ask about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Guarded {
    /// Showing the seed phrase or a private key.
    RevealSecret,
    /// A send over the threshold, checked by the caller with [`is_large_send`].
    LargeSend,
    /// The first unlock after a long silence.
    Dormant,
}

/// What has to be produced before a guarded action goes ahead.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Need {
    pub totp: bool,
}

impl Need {
    pub fn any(self) -> bool {
        self.totp
    }
}

/// What `action` must clear.
///
/// A factor that is not set up is never demanded — a gate nobody can pass is a
/// lockout, not a protection — so with two-factor off this asks for nothing.
/// Unlocking the wallet already needed the seed, which is the floor this sits
/// on top of.
pub fn required(config: &AppConfig, action: Guarded) -> Need {
    let have = Need {
        totp: config.two_factor && !config.totp_secret.is_empty(),
    };
    if !have.any() {
        return Need::default();
    }

    // Dormancy is the one trigger the user can switch off outright; the
    // others are the wallet's own floor.
    if action == Guarded::Dormant && config.step_up.dormant_days == 0 {
        return Need::default();
    }
    have
}

/// Whether a send is big enough to stop for: worth more than
/// [`LARGE_SEND_USD`] *and* at least [`LARGE_SEND_SHARE`] of the whole wallet.
///
/// Both arguments are optional because either can fail to price, and both
/// fail **closed**: an unpriceable transfer is treated as large. A gate that
/// quietly disappears whenever a price feed is unreachable is a gate an
/// attacker can wait out, and the cost of the safe direction is one
/// authenticator code on a day the feed is down.
///
/// Non-finite values count as unpriced. `NaN` in particular must not slip
/// through: every comparison against it is false, so a naive `amount > limit`
/// test would silently answer "not large".
pub fn is_large_send(amount_usd: Option<f64>, wallet_usd: Option<f64>) -> bool {
    let Some(amount) = amount_usd.filter(|v| v.is_finite() && *v >= 0.0) else {
        return true;
    };
    if amount <= LARGE_SEND_USD {
        return false;
    }
    match wallet_usd {
        Some(total) if total.is_finite() && total > 0.0 => {
            amount >= total * LARGE_SEND_SHARE
        }
        _ => true,
    }
}

/// Whether the wallet has sat unopened long enough to demand a second factor.
///
/// A machine that has never been unlocked has nothing to be dormant from, so
/// it does not trigger: that case is a fresh install, not a return.
pub fn dormant(last_seen: Option<i64>, days: u32, at: i64) -> bool {
    if days == 0 {
        return false;
    }
    match last_seen {
        Some(seen) => at.saturating_sub(seen) >= days as i64 * 86_400,
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(totp: bool) -> AppConfig {
        let mut cfg = AppConfig::default();
        cfg.two_factor = totp;
        if totp {
            cfg.totp_secret = "GEZDGNBVGY3TQOJQ".into();
        }
        cfg
    }

    #[test]
    fn two_factor_off_means_no_gate() {
        let need = required(&config(false), Guarded::RevealSecret);
        assert!(!need.any(), "a gate nobody can pass is a lockout");
    }

    #[test]
    fn two_factor_on_guards_every_action() {
        for action in [Guarded::RevealSecret, Guarded::LargeSend, Guarded::Dormant] {
            assert!(required(&config(true), action).any(), "{action:?} was not guarded");
        }
    }

    #[test]
    fn an_enabled_flag_with_no_secret_is_not_a_usable_factor() {
        let mut cfg = config(true);
        cfg.totp_secret.clear();
        assert!(!required(&cfg, Guarded::RevealSecret).any());
    }

    #[test]
    fn dormancy_can_be_switched_off_but_the_others_cannot() {
        let mut cfg = config(true);
        cfg.step_up.dormant_days = 0;
        assert!(!required(&cfg, Guarded::Dormant).any());
        assert!(required(&cfg, Guarded::RevealSecret).any());
        assert!(required(&cfg, Guarded::LargeSend).any());
    }

    #[test]
    fn a_large_send_needs_both_the_amount_and_the_share() {
        // Over $50 and over a tenth of the wallet.
        assert!(is_large_send(Some(60.0), Some(500.0)));
        // Over $50 but a rounding error against the balance.
        assert!(!is_large_send(Some(60.0), Some(10_000.0)));
        // A tenth of the wallet but pocket change.
        assert!(!is_large_send(Some(20.0), Some(100.0)));
        // Exactly on the dollar threshold is not over it.
        assert!(!is_large_send(Some(50.0), Some(100.0)));
    }

    #[test]
    fn an_unpriceable_wallet_fails_closed() {
        assert!(is_large_send(Some(60.0), None));
        assert!(is_large_send(Some(60.0), Some(0.0)));
        // Still bounded by the dollar threshold, so small sends stay quiet.
        assert!(!is_large_send(Some(10.0), None));
    }

    #[test]
    fn an_unpriceable_transfer_fails_closed() {
        // The interface could not price it and neither could we.
        assert!(is_large_send(None, Some(10_000.0)));
        assert!(is_large_send(None, None));
    }

    #[test]
    fn a_nan_amount_does_not_slip_through() {
        // Every comparison against NaN is false, so the old `!(a > limit)`
        // form answered "not large" and skipped the gate entirely.
        assert!(is_large_send(Some(f64::NAN), Some(100.0)));
        assert!(is_large_send(Some(f64::INFINITY), Some(100.0)));
        assert!(is_large_send(Some(-1.0), Some(100.0)));
        // A NaN total is an unpriceable wallet, which already failed closed.
        assert!(is_large_send(Some(9_999.0), Some(f64::NAN)));
    }

    #[test]
    fn dormancy_counts_from_the_last_unlock() {
        let now = 100 * 86_400;
        assert!(dormant(Some(now - 15 * 86_400), 15, now));
        assert!(dormant(Some(now - 40 * 86_400), 15, now));
        assert!(!dormant(Some(now - 14 * 86_400), 15, now));
        // Never opened is a fresh install, not a return.
        assert!(!dormant(None, 15, now));
        // Zero is off.
        assert!(!dormant(Some(0), 0, now));
    }

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
