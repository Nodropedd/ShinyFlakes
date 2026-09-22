//! Two-factor policy.

use std::time::{SystemTime, UNIX_EPOCH};

const PASS_TTL: i64 = 2 * 60;

pub const LOCK_AT: u32 = 3;

pub const LOCKOUT: i64 = 60;

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub fn pass_expires_at() -> i64 {
    now() + PASS_TTL
}

pub fn pass_valid(expiry: i64) -> bool {
    now() < expiry
}

pub fn lock_after(wrong: u32, at: i64) -> Option<i64> {
    if wrong >= LOCK_AT {
        Some(at + LOCKOUT)
    } else {
        None
    }
}

pub fn locked(until: Option<i64>, at: i64) -> bool {
    matches!(until, Some(u) if at < u)
}

use crate::appconfig::{AppConfig, LARGE_SEND_SHARE, LARGE_SEND_USD};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Guarded {

    RevealSecret,

    LargeSend,

    Dormant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Need {
    pub totp: bool,
}

impl Need {
    pub fn any(self) -> bool {
        self.totp
    }
}

pub fn required(config: &AppConfig, action: Guarded) -> Need {
    let have = Need {
        totp: config.two_factor && !config.totp_secret.is_empty(),
    };
    if !have.any() {
        return Need::default();
    }

    if action == Guarded::Dormant && config.step_up.dormant_days == 0 {
        return Need::default();
    }
    have
}

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

        assert!(is_large_send(Some(60.0), Some(500.0)));

        assert!(!is_large_send(Some(60.0), Some(10_000.0)));

        assert!(!is_large_send(Some(20.0), Some(100.0)));

        assert!(!is_large_send(Some(50.0), Some(100.0)));
    }

    #[test]
    fn an_unpriceable_wallet_fails_closed() {
        assert!(is_large_send(Some(60.0), None));
        assert!(is_large_send(Some(60.0), Some(0.0)));

        assert!(!is_large_send(Some(10.0), None));
    }

    #[test]
    fn an_unpriceable_transfer_fails_closed() {

        assert!(is_large_send(None, Some(10_000.0)));
        assert!(is_large_send(None, None));
    }

    #[test]
    fn a_nan_amount_does_not_slip_through() {

        assert!(is_large_send(Some(f64::NAN), Some(100.0)));
        assert!(is_large_send(Some(f64::INFINITY), Some(100.0)));
        assert!(is_large_send(Some(-1.0), Some(100.0)));

        assert!(is_large_send(Some(9_999.0), Some(f64::NAN)));
    }

    #[test]
    fn dormancy_counts_from_the_last_unlock() {
        let now = 100 * 86_400;
        assert!(dormant(Some(now - 15 * 86_400), 15, now));
        assert!(dormant(Some(now - 40 * 86_400), 15, now));
        assert!(!dormant(Some(now - 14 * 86_400), 15, now));

        assert!(!dormant(None, 15, now));

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
