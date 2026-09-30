//! Bounded structured application logs. Never accepts credential payloads.
use crate::dto::{LogEntry, LogFilter};
use std::{collections::VecDeque, sync::Mutex};
use tokio::sync::broadcast;
use uuid::Uuid;
pub struct LogBook {
    entries: Mutex<VecDeque<LogEntry>>,
    events: broadcast::Sender<LogEntry>,
}
impl Default for LogBook {
    fn default() -> Self {
        let (events, _) = broadcast::channel(256);
        Self {
            entries: Mutex::new(VecDeque::new()),
            events,
        }
    }
}
impl LogBook {
    pub fn push(&self, level: &str, tunnel_id: Option<Uuid>, message: impl Into<String>) {
        let entry = LogEntry {
            id: Uuid::new_v4(),
            timestamp: chrono::Utc::now().to_rfc3339(),
            level: level.into(),
            tunnel_id,
            message: crate::session_redact::redact_command(&message.into()),
        };
        let mut entries = self.entries.lock().unwrap_or_else(|p| p.into_inner());
        if entries.len() == 1000 {
            entries.pop_front();
        }
        entries.push_back(entry.clone());
        let _ = self.events.send(entry);
    }
    pub fn get(&self, filter: Option<LogFilter>) -> Vec<LogEntry> {
        let filter = filter.unwrap_or_default();
        self.entries
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .filter(|e| {
                filter.tunnel_id.is_none_or(|id| e.tunnel_id == Some(id))
                    && filter.level.as_ref().is_none_or(|l| *l == e.level)
            })
            .cloned()
            .collect()
    }
    pub fn subscribe(&self) -> broadcast::Receiver<LogEntry> {
        self.events.subscribe()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn logs_are_bounded_and_filters_are_applied() {
        let logs = LogBook::default();
        let id = Uuid::new_v4();
        for n in 0..1010 {
            logs.push("info", None, format!("entry {n}"));
        }
        logs.push("error", Some(id), "fixture error");
        assert_eq!(logs.get(None).len(), 1000);
        let filtered = logs.get(Some(LogFilter {
            tunnel_id: Some(id),
            level: Some("error".into()),
        }));
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].message, "fixture error");
    }
}
