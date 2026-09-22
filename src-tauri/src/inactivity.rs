//! Dormancy wipe.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::error::{Result, WalletError};

const SECONDS_PER_MONTH: i64 = 2_629_746;

const GRACE_SECONDS: i64 = 14 * 86_400;

pub const CHOICES: [u32; 5] = [0, 3, 6, 12, 24];

pub const DEFAULT_MONTHS: u32 = 12;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {

    Delete,

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

            _ => Action::Delete,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Record {

    last_seen: i64,

    months: u32,

    #[serde(default)]
    action: String,

    #[serde(default)]
    grace_started: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {

    pub last_seen: Option<i64>,
    pub months: u32,

    pub action: String,
    pub days_since: i64,

    pub days_remaining: Option<i64>,

    pub grace_active: bool,

    pub grace_days_remaining: Option<i64>,

    pub sweep_due: bool,

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

pub fn last_seen(app_data: &Path) -> Option<i64> {
    read(app_data).map(|record| record.last_seen)
}

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

        grace_started: None,
    };
    save(app_data, &record)?;
    Ok(describe(&record, false))
}

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

pub enum Outcome {

    Idle(Status),

    Wiped(Status),

    SweepDue(Status),
}

pub fn check(app_data: &Path, vault_path: &Path) -> Outcome {
    let Some(mut record) = read(app_data) else {

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

    if elapsed < limit || elapsed < 0 {

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

            let mut status = describe(&record, false);
            status.sweep_due = true;
            Outcome::SweepDue(status)
        }
    }
}

fn wipe(app_data: &Path, vault_path: &Path) -> bool {
    let mut wiped = false;
    match std::fs::remove_file(vault_path) {
        Ok(()) => wiped = true,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => wiped = true,
        Err(_) => {}
    }
    if wiped {
        let _ = crate::keychain::forget();

        let _ = record_seen(app_data);
    }
    wiped
}

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

        record_seen(&dir).unwrap();

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

        plant(&dir, rec(now() - 86_400, 12, Action::Donate, Some(now() - 3 * 86_400)));

        assert!(matches!(check(&dir, &vault), Outcome::Idle(_)));
        assert!(!status_of(&check(&dir, &vault)).grace_active);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn old_files_without_the_action_field_default_to_delete() {
        let dir = temp();

        std::fs::write(
            store_path(&dir),
            format!(r#"{{"last_seen":{},"months":12}}"#, now() - 13 * SECONDS_PER_MONTH),
        )
        .unwrap();
        let vault = dir.join("wallet.vault");
        std::fs::write(&vault, b"vault").unwrap();

        assert!(matches!(check(&dir, &vault), Outcome::Wiped(_)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn finish_sweep_only_deletes_on_full_success() {
        let dir = temp();
        let vault = dir.join("wallet.vault");
        std::fs::write(&vault, b"vault").unwrap();
        plant(&dir, rec(now(), 12, Action::Donate, None));

        assert!(!finish_sweep(&dir, &vault, false));
        assert!(vault.is_file());

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
