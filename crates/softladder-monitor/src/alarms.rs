//! Alarm journal with acknowledgement — **M6 slice four (pure)**.
//!
//! The journal holds the *active* alarms: [`AlarmJournal::raise`] adds one (or
//! refreshes the active entry for the same code and variable, clearing its
//! acknowledgement, since a new occurrence un-acks), [`AlarmJournal::acknowledge`]
//! records who accepted it, and [`AlarmJournal::resolve`] drops it when the
//! condition is gone. [`AlarmJournal::active`] lists errors first, then
//! warnings, then the rest, oldest first within each severity.

use serde::{Deserialize, Serialize};

use crate::Alarm;

/// One active alarm: the wire [`Alarm`] (whose `acked` flag is the ack state)
/// plus journal bookkeeping.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JournalEntry {
    /// Journal id, assigned on raise.
    pub id: u64,
    /// The alarm itself.
    pub alarm: Alarm,
    /// Simulated timestamp of the latest occurrence.
    pub raised_at_ms: u64,
    /// Bearer id that acknowledged it, if any.
    pub acked_by: Option<String>,
}

impl JournalEntry {
    /// Whether anyone acknowledged the alarm.
    pub fn acked(&self) -> bool {
        self.alarm.acked
    }
}

/// Active alarms with acknowledgement.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AlarmJournal {
    entries: Vec<JournalEntry>,
    next_id: u64,
}

impl AlarmJournal {
    /// Creates an empty journal.
    pub fn new() -> Self {
        Self::default()
    }

    /// Raises `alarm` at `now_ms`, returning its id.
    ///
    /// An active entry for the same code and variable is refreshed instead of
    /// duplicated: severity, message and timestamp update, and a previous
    /// acknowledgement is cleared.
    pub fn raise(&mut self, alarm: Alarm, now_ms: u64) -> u64 {
        if let Some(entry) = self
            .entries
            .iter_mut()
            .find(|entry| entry.alarm.code == alarm.code && entry.alarm.var == alarm.var)
        {
            entry.alarm = alarm;
            entry.alarm.acked = false;
            entry.raised_at_ms = now_ms;
            entry.acked_by = None;
            return entry.id;
        }
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        self.entries.push(JournalEntry {
            id,
            alarm,
            raised_at_ms: now_ms,
            acked_by: None,
        });
        id
    }

    /// Acknowledges `id` as `by`; returns `false` when unknown.
    pub fn acknowledge(&mut self, id: u64, by: &str) -> bool {
        match self.entries.iter_mut().find(|entry| entry.id == id) {
            Some(entry) => {
                entry.alarm.acked = true;
                entry.acked_by = Some(by.to_owned());
                true
            }
            None => false,
        }
    }

    /// Drops `id` (condition gone); returns `false` when unknown.
    pub fn resolve(&mut self, id: u64) -> bool {
        match self.entries.iter().position(|entry| entry.id == id) {
            Some(index) => {
                self.entries.remove(index);
                true
            }
            None => false,
        }
    }

    /// Entry by id.
    pub fn get(&self, id: u64) -> Option<&JournalEntry> {
        self.entries.iter().find(|entry| entry.id == id)
    }

    /// Active entries: errors, then warnings, then the rest; oldest first.
    pub fn active(&self) -> Vec<JournalEntry> {
        let mut entries = self.entries.clone();
        entries.sort_by_key(|entry| {
            (
                severity_rank(&entry.alarm.severity),
                entry.raised_at_ms,
                entry.id,
            )
        });
        entries
    }

    /// Number of active entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// `true` when no alarm is active.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

fn severity_rank(severity: &str) -> u8 {
    match severity {
        "error" => 0,
        "warning" => 1,
        _ => 2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn var(text: &str) -> crate::VarRef {
        text.parse::<crate::VarRef>().expect("test variable parses")
    }

    fn alarm(code: &str, severity: &str) -> Alarm {
        Alarm {
            severity: severity.to_owned(),
            code: code.to_owned(),
            message: format!("{code} happened"),
            var: None,
            acked: false,
        }
    }

    #[test]
    fn raise_acknowledge_and_resolve() {
        let mut journal = AlarmJournal::new();
        assert!(journal.is_empty());
        let id = journal.raise(alarm("SL-E002", "error"), 1000);
        assert_eq!(journal.len(), 1);
        assert!(journal.acknowledge(id, "op-1"));
        assert_eq!(
            journal.get(id).expect("present").acked_by.as_deref(),
            Some("op-1")
        );
        assert!(!journal.acknowledge(999, "op-1"));
        assert!(journal.resolve(id));
        assert!(journal.is_empty());
        assert!(!journal.resolve(id));
    }

    #[test]
    fn a_repeat_occurrence_refreshes_and_un_acks() {
        let mut journal = AlarmJournal::new();
        let id = journal.raise(alarm("SL-E002", "error"), 1000);
        assert!(journal.acknowledge(id, "op-1"));
        let again = journal.raise(alarm("SL-E002", "error"), 2000);
        assert_eq!(again, id);
        assert_eq!(journal.len(), 1);
        let entry = journal.get(id).expect("present");
        assert!(!entry.acked());
        assert_eq!(entry.acked_by, None);
        assert_eq!(entry.raised_at_ms, 2000);
    }

    #[test]
    fn same_code_on_another_variable_is_another_alarm() {
        let mut journal = AlarmJournal::new();
        let mut first = alarm("SL-W010", "warning");
        first.var = Some(var("%Q0"));
        let mut second = alarm("SL-W010", "warning");
        second.var = Some(var("%Q1"));
        let a = journal.raise(first, 1000);
        let b = journal.raise(second, 1000);
        assert_ne!(a, b);
        assert_eq!(journal.len(), 2);
    }

    #[test]
    fn active_lists_errors_first_then_oldest() {
        let mut journal = AlarmJournal::new();
        journal.raise(alarm("SL-W001", "warning"), 1000);
        journal.raise(alarm("SL-I001", "info"), 500);
        journal.raise(alarm("SL-E001", "error"), 2000);
        let active = journal.active();
        let codes: Vec<&str> = active
            .iter()
            .map(|entry| entry.alarm.code.as_str())
            .collect();
        assert_eq!(codes, vec!["SL-E001", "SL-W001", "SL-I001"]);
    }

    #[test]
    fn entries_round_trip_through_json() {
        let mut journal = AlarmJournal::new();
        journal.raise(alarm("SL-E002", "error"), 1000);
        journal.acknowledge(0, "op-1");
        let json = serde_json::to_string(&journal.active()).expect("serializes");
        assert!(json.contains("\"acked\":true"), "{json}");
        let decoded: Vec<JournalEntry> = serde_json::from_str(&json).expect("deserializes");
        assert_eq!(decoded, journal.active());
        assert!(decoded[0].acked());
        assert_eq!(decoded[0].acked_by.as_deref(), Some("op-1"));
    }
}
