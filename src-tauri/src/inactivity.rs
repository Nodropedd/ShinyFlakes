//! Clearing this machine after a long absence.
//!
//! If the wallet is not opened for the chosen period, one of two things
//! happens on a later launch: either the encrypted vault and its key are
//! deleted (delete mode), or the balances are first swept to fixed donation
//! addresses and then the vault is deleted (donate mode).
//!
//! Two properties are worth stating plainly, because they are the whole
//! shape of this feature:
//!
//! 1. It can only act when the app is launched. Nothing runs while the
//!    machine sits unused, so a wallet whose owner is simply gone is never
//!    swept; the coins stay on their chains. The switch only does anything
//!    if someone launches the wallet after the deadline.
//!
//! 2. A sweep sends real money and cannot be undone. So donate mode never
//!    acts on the launch that first notices the deadline. It starts a grace
//!    window and warns instead, and any successful unlock cancels the whole
//!    thing and resets the clock. Sweeping only happens on a launch after the
//!    grace has also passed with still no unlock, which is the only situation
//!    that actually means "gone and cannot get in".
//!
//! The check needs no keys, so it runs before unlocking and still works for
//! someone locked out.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::error::{Result, WalletError};

/// Average Gregorian month. Close enough for a period measured in months, and
/// it does not drift the way a flat thirty days would over two years.
const SECONDS_PER_MONTH: i64 = 2_629_746;

/// After the deadline passes in donate mode, this long must also pass, across
/// at least one more launch with no unlock, before anything is sent. It is the
/// window in which a returning owner cancels the sweep simply by logging in.
const GRACE_SECONDS: i64 = 14 * 86_400;

/// Periods the user can choose. Zero means never.
pub const CHOICES: [u32; 5] = [0, 3, 6, 12, 24];

pub const DEFAULT_MONTHS: u32 = 12;

/// What happens when the deadline passes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Delete the local wallet. Reversible with the seed phrase.
    Delete,
    /// Sweep balances to the donation addresses, then delete. Irreversible.
    Donate,
}

impl Action {
    fn as_str(self) -> &'static str {
        match self {
            Action::Delete => "delete",
            Action::Donate => "donate",
        }
    }

    fn parse(text: &str) -> Action {
        match text {
            "donate" => Action::Donate,
            // Anything unrecognised, including an old file without the field,
            // falls back to the safe, reversible behaviour.
            _ => Action::Delete,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Record {
    /// Unix seconds of the last unlock.
    last_seen: i64,
    /// Months of silence before the switch fires. Zero is never.
    months: u32,
    /// "delete" or "donate". Absent in files written before donate existed,
    /// so it defaults to the reversible choice.
    #[serde(default)]
    action: String,
    /// When the deadline was first observed as passed, in donate mode. The
    /// grace window is measured from here. Cleared by any unlock.
    #[serde(default)]
    grace_started: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    /// Unix seconds, absent on a machine that has never been used.
    pub last_seen: Option<i64>,
    pub months: u32,
    /// "delete" or "donate".
    pub action: String,
    pub days_since: i64,
    /// Days left before the switch fires. Absent when set to never.
    pub days_remaining: Option<i64>,
    /// Donate mode only: the deadline has passed and the grace window is
    /// counting down. Unlocking now cancels it.
    pub grace_active: bool,
    /// Days left in the grace window, when it is active.
    pub grace_days_remaining: Option<i64>,
    /// Donate mode: the grace has also passed and a sweep is now owed. The
    /// caller runs the async sweep on seeing this.
    pub sweep_due: bool,
    /// Delete mode: the wallet was cleared by this check, just now.
    pub wiped: bool,
}

fn store_path(app_data: &Path) -> PathBuf {
    app_data.join("activity.json")
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn read(app_data: &Path) -> Option<Record> {
    let text = std::fs::read_to_string(store_path(app_data)).ok()?;
    serde_json::from_str(&text).ok()
}

fn save(app_data: &Path, record: &Record) -> Result<()> {
    let text =
        serde_json::to_string_pretty(record).map_err(|e| WalletError::Storage(e.to_string()))?;
    std::fs::write(store_path(app_data), text).map_err(|e| WalletError::Storage(e.to_string()))
}

/// Unix second of the last unlock, or `None` on a machine that has never been
/// used. Read before [`record_seen`] moves it, when the question is how long
/// the wallet sat untouched.
pub fn last_seen(app_data: &Path) -> Option<i64> {
    read(app_data).map(|record| record.last_seen)
}

/// Marks the wallet as used, and cancels any sweep in progress.
///
/// Called on every unlock. Clearing grace_started here is what makes a
/// successful login call off a pending sweep: the owner is demonstrably back.
pub fn record_seen(app_data: &Path) -> Result<()> {
    let existing = read(app_data);
    save(
        app_data,
        &Record {
            last_seen: now(),
            months: existing.as_ref().map(|r| r.months).unwrap_or(DEFAULT_MONTHS),
            action: existing
                .as_ref()
                .map(|r| r.action.clone())
                .unwrap_or_else(|| Action::Delete.as_str().into()),
            grace_started: None,
        },
    )
}

/// Changes the period without disturbing the last-seen time or the action.
pub fn set_months(app_data: &Path, months: u32) -> Result<Status> {
    if !CHOICES.contains(&months) {
        return Err(WalletError::Unsupported(format!(
            "{months} is not one of the periods this wallet offers"
        )));
    }
    let existing = read(app_data);
    let record = Record {
        last_seen: existing.as_ref().map(|r| r.last_seen).unwrap_or_else(now),
        months,
        action: existing
            .as_ref()
            .map(|r| r.action.clone())
            .unwrap_or_else(|| Action::Delete.as_str().into()),
        // Changing the period is a fresh decision, so any pending grace is
        // dropped rather than carried across a new deadline.
        grace_started: None,
    };
    save(app_data, &record)?;
    Ok(describe(&record, false))
}

/// Switches between deleting and donating when the deadline passes.
pub fn set_action(app_data: &Path, action: Action) -> Result<Status> {
    let existing = read(app_data);
    let record = Record {
        last_seen: existing.as_ref().map(|r| r.last_seen).unwrap_or_else(now),
        months: existing.as_ref().map(|r| r.months).unwrap_or(DEFAULT_MONTHS),
        action: action.as_str().into(),
        grace_started: None,
    };
    save(app_data, &record)?;
    Ok(describe(&record, false))
}

fn describe(record: &Record, wiped: bool) -> Status {
    let days_since = ((now() - record.last_seen).max(0)) / 86_400;

    let days_remaining = if record.months == 0 {
        None
    } else {
        let deadline_days = record.months as i64 * SECONDS_PER_MONTH / 86_400;
        Some((deadline_days - days_since).max(0))
    };

    let grace_active = record.grace_started.is_some();
    let grace_days_remaining = record.grace_started.map(|started| {
        let left = GRACE_SECONDS - (now() - started);
        (left.max(0)) / 86_400
    });

    Status {
        last_seen: Some(record.last_seen),
        months: record.months,
        action: record.action.clone(),
        days_since,
        days_remaining,
        grace_active,
        grace_days_remaining,
        sweep_due: false,
        wiped,
    }
}

/// The verdict a launch reaches, before any slow work is done.
pub enum Outcome {
    /// Nothing to do. Carries the status for display.
    Idle(Status),
    /// Delete mode fired and cleared the wallet.
    Wiped(Status),
    /// Donate mode: deadline and grace both passed. The caller must run the
    /// async sweep, then call `finish_sweep`.
    SweepDue(Status),
}

/// Runs the check. Deletes in delete mode; only flags a sweep in donate mode,
/// since sweeping is async and must not happen inside this sync call.
pub fn check(app_data: &Path, vault_path: &Path) -> Outcome {
    let Some(mut record) = read(app_data) else {
        // No history yet. Start the clock rather than treating the unknown
        // past as an expired one.
        let _ = record_seen(app_data);
        let fresh = Record {
            last_seen: now(),
            months: DEFAULT_MONTHS,
            action: Action::Delete.as_str().into(),
            grace_started: None,
        };
        return Outcome::Idle(describe(&fresh, false));
    };

    if record.months == 0 {
        return Outcome::Idle(describe(&record, false));
    }

    let elapsed = now() - record.last_seen;
    let limit = record.months as i64 * SECONDS_PER_MONTH;

    // Not expired. A clock that jumped backwards shows negative elapsed, which
    // is a date change and not an absence, so it is treated as not expired.
    if elapsed < limit || elapsed < 0 {
        // Deadline no longer passed, so drop any stale grace marker.
        if record.grace_started.is_some() {
            record.grace_started = None;
            let _ = save(app_data, &record);
        }
        return Outcome::Idle(describe(&record, false));
    }

    match Action::parse(&record.action) {
        Action::Delete => {
            let wiped = wipe(app_data, vault_path);
            Outcome::Wiped(describe(&record, wiped))
        }
        Action::Donate => {
            // First launch past the deadline: open the grace window and warn.
            // Nothing is sent yet, and an unlock now cancels it.
            let Some(started) = record.grace_started else {
                record.grace_started = Some(now());
                let _ = save(app_data, &record);
                let mut status = describe(&record, false);
                status.sweep_due = false;
                return Outcome::Idle(status);
            };

            if now() - started < GRACE_SECONDS {
                return Outcome::Idle(describe(&record, false));
            }

            // Deadline and grace both passed, still no unlock. The sweep is
            // owed. The actual sending happens in the async command.
            let mut status = describe(&record, false);
            status.sweep_due = true;
            Outcome::SweepDue(status)
        }
    }
}

/// Deletes the vault and forgets the key. Both go or neither is useful.
fn wipe(app_data: &Path, vault_path: &Path) -> bool {
    let mut wiped = false;
    match std::fs::remove_file(vault_path) {
        Ok(()) => wiped = true,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => wiped = true,
        Err(_) => {}
    }
    if wiped {
        let _ = crate::keychain::forget();
        // Restart the clock so a later launch does not keep reporting a wipe
        // that already ran.
        let _ = record_seen(app_data);
    }
    wiped
}

/// Called after the async sweep has run.
///
/// If every chain that could be swept succeeded, the local wallet is deleted,
/// completing the switch. If a chain failed, usually a network blip, nothing
/// is deleted and the next launch tries again; a chain already emptied simply
/// has nothing to send and is skipped, so retrying is safe.
pub fn finish_sweep(app_data: &Path, vault_path: &Path, all_succeeded: bool) -> bool {
    if all_succeeded {
        wipe(app_data, vault_path)
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp() -> PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "sf-inactivity-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn plant(dir: &Path, record: Record) {
        save(dir, &record).unwrap();
    }

    fn rec(last_seen: i64, months: u32, action: Action, grace: Option<i64>) -> Record {
        Record {
            last_seen,
            months,
            action: action.as_str().into(),
            grace_started: grace,
        }
    }

    fn status_of(outcome: &Outcome) -> &Status {
        match outcome {
            Outcome::Idle(s) | Outcome::Wiped(s) | Outcome::SweepDue(s) => s,
        }
    }

    #[test]
    fn a_fresh_machine_starts_the_clock() {
        let dir = temp();
        let vault = dir.join("wallet.vault");
        std::fs::write(&vault, b"vault").unwrap();

        assert!(matches!(check(&dir, &vault), Outcome::Idle(_)));
        assert!(vault.is_file());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_recent_visit_does_nothing() {
        let dir = temp();
        let vault = dir.join("wallet.vault");
        std::fs::write(&vault, b"vault").unwrap();
        plant(&dir, rec(now() - 30 * 86_400, 12, Action::Delete, None));

        assert!(matches!(check(&dir, &vault), Outcome::Idle(_)));
        assert!(vault.is_file());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn delete_mode_clears_after_the_deadline() {
        let dir = temp();
        let vault = dir.join("wallet.vault");
        std::fs::write(&vault, b"vault").unwrap();
        plant(&dir, rec(now() - 13 * SECONDS_PER_MONTH, 12, Action::Delete, None));

        assert!(matches!(check(&dir, &vault), Outcome::Wiped(_)));
        assert!(!vault.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn donate_mode_opens_grace_before_sweeping() {
        let dir = temp();
        let vault = dir.join("wallet.vault");
        std::fs::write(&vault, b"vault").unwrap();
        plant(&dir, rec(now() - 13 * SECONDS_PER_MONTH, 12, Action::Donate, None));

        // First launch past the deadline: grace opens, nothing swept, vault
        // untouched.
        let first = check(&dir, &vault);
        assert!(matches!(first, Outcome::Idle(_)));
        assert!(status_of(&first).grace_active);
        assert!(!status_of(&first).sweep_due);
        assert!(vault.is_file());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn donate_sweeps_once_grace_has_also_passed() {
        let dir = temp();
        let vault = dir.join("wallet.vault");
        std::fs::write(&vault, b"vault").unwrap();
        // Deadline long gone, grace started more than fourteen days ago.
        plant(
            &dir,
            rec(
                now() - 13 * SECONDS_PER_MONTH,
                12,
                Action::Donate,
                Some(now() - GRACE_SECONDS - 86_400),
            ),
        );

        assert!(matches!(check(&dir, &vault), Outcome::SweepDue(_)));
        // The vault is still there: sweeping and deleting is the caller's job.
        assert!(vault.is_file());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unlocking_cancels_a_pending_sweep() {
        let dir = temp();
        let vault = dir.join("wallet.vault");
        std::fs::write(&vault, b"vault").unwrap();
        plant(
            &dir,
            rec(
                now() - 13 * SECONDS_PER_MONTH,
                12,
                Action::Donate,
                Some(now() - GRACE_SECONDS - 86_400),
            ),
        );

        // The owner is back.
        record_seen(&dir).unwrap();

        // No sweep now, and the grace marker is gone.
        assert!(matches!(check(&dir, &vault), Outcome::Idle(_)));
        assert!(!status_of(&check(&dir, &vault)).grace_active);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_backwards_clock_does_not_fire() {
        let dir = temp();
        let vault = dir.join("wallet.vault");
        std::fs::write(&vault, b"vault").unwrap();
        plant(&dir, rec(now() + 5 * SECONDS_PER_MONTH, 6, Action::Donate, None));

        assert!(matches!(check(&dir, &vault), Outcome::Idle(_)));
        assert!(vault.is_file());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn never_means_never_even_in_donate_mode() {
        let dir = temp();
        let vault = dir.join("wallet.vault");
        std::fs::write(&vault, b"vault").unwrap();
        plant(&dir, rec(now() - 120 * SECONDS_PER_MONTH, 0, Action::Donate, None));

        assert!(matches!(check(&dir, &vault), Outcome::Idle(_)));
        assert!(vault.is_file());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn recovery_from_a_dropped_deadline_clears_grace() {
        let dir = temp();
        let vault = dir.join("wallet.vault");
        std::fs::write(&vault, b"vault").unwrap();
        // Grace was open, but the last_seen is recent, so the deadline is no
        // longer passed. The stale grace marker must be dropped.
        plant(&dir, rec(now() - 86_400, 12, Action::Donate, Some(now() - 3 * 86_400)));

        assert!(matches!(check(&dir, &vault), Outcome::Idle(_)));
        assert!(!status_of(&check(&dir, &vault)).grace_active);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn old_files_without_the_action_field_default_to_delete() {
        let dir = temp();
        // A file as written before donate mode existed.
        std::fs::write(
            store_path(&dir),
            format!(r#"{{"last_seen":{},"months":12}}"#, now() - 13 * SECONDS_PER_MONTH),
        )
        .unwrap();
        let vault = dir.join("wallet.vault");
        std::fs::write(&vault, b"vault").unwrap();

        // Defaulting to delete means an old wallet is cleared, not swept.
        assert!(matches!(check(&dir, &vault), Outcome::Wiped(_)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn finish_sweep_only_deletes_on_full_success() {
        let dir = temp();
        let vault = dir.join("wallet.vault");
        std::fs::write(&vault, b"vault").unwrap();
        plant(&dir, rec(now(), 12, Action::Donate, None));

        // A failed chain leaves everything in place to retry.
        assert!(!finish_sweep(&dir, &vault, false));
        assert!(vault.is_file());

        // Full success completes the switch.
        assert!(finish_sweep(&dir, &vault, true));
        assert!(!vault.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_period_must_be_an_offered_choice() {
        let dir = temp();
        assert!(set_months(&dir, 6).is_ok());
        assert!(set_months(&dir, 7).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn setting_the_action_survives_a_reload() {
        let dir = temp();
        set_action(&dir, Action::Donate).unwrap();
        assert_eq!(read(&dir).unwrap().action, "donate");
        set_action(&dir, Action::Delete).unwrap();
        assert_eq!(read(&dir).unwrap().action, "delete");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
