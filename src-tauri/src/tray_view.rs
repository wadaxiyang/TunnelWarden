//! Native tray presentation; no network or forwarding decisions.
use std::{collections::HashMap, time::Instant};
use tunnelwarden_core::dto::{RuntimeSnapshot, TunnelDto, TunnelStatus, TunnelTraffic};
use tunnelwarden_core::i18n::{Language, text};
use uuid::Uuid;

pub fn status_text(status: TunnelStatus, language: Language) -> &'static str {
    text(
        language,
        match status {
            TunnelStatus::Stopped => "Stopped",
            TunnelStatus::Connecting => "Connecting",
            TunnelStatus::Running => "Running",
            TunnelStatus::Retrying => "Retrying",
            TunnelStatus::Error => "Error",
        },
    )
}
pub fn can_stop(status: TunnelStatus) -> bool {
    matches!(
        status,
        TunnelStatus::Running | TunnelStatus::Connecting | TunnelStatus::Retrying
    )
}
pub fn name(text: &str) -> String {
    text.chars().filter(|c| !c.is_control()).take(80).collect()
}
pub fn menu_name(text: &str) -> String {
    name(text).replace('&', "&&")
}

#[derive(Default)]
pub struct RateSampler {
    previous: HashMap<Uuid, (Instant, Instant, u64, u64)>,
}
impl RateSampler {
    pub fn sample(
        &mut self,
        now: Instant,
        samples: Vec<TunnelTraffic>,
    ) -> HashMap<Uuid, (f64, f64)> {
        let mut previous = HashMap::new();
        let rates = samples
            .into_iter()
            .map(|s| {
                let rate = self
                    .previous
                    .get(&s.id)
                    .and_then(|&(started, then, sent, received)| {
                        let seconds = now.saturating_duration_since(then).as_secs_f64();
                        (started == s.started_at
                            && seconds > 0.0
                            && s.sent_bytes >= sent
                            && s.received_bytes >= received)
                            .then(|| {
                                (
                                    (s.sent_bytes - sent) as f64 / seconds,
                                    (s.received_bytes - received) as f64 / seconds,
                                )
                            })
                    })
                    .unwrap_or((0.0, 0.0));
                previous.insert(s.id, (s.started_at, now, s.sent_bytes, s.received_bytes));
                (s.id, rate)
            })
            .collect();
        self.previous = previous;
        rates
    }
}
fn rate(bytes: f64) -> String {
    if bytes >= 1024.0 * 1024.0 {
        format!("{:.1} MiB/s", bytes / (1024.0 * 1024.0))
    } else if bytes >= 1024.0 {
        format!("{:.1} KiB/s", bytes / 1024.0)
    } else {
        format!("{bytes:.0} B/s")
    }
}
pub fn panel_text(
    tunnels: &[TunnelDto],
    runtime: &RuntimeSnapshot,
    rates: &HashMap<Uuid, (f64, f64)>,
    language: Language,
) -> String {
    let active: Vec<_> = tunnels
        .iter()
        .filter(|tunnel| {
            runtime
                .tunnels
                .iter()
                .any(|state| state.id == tunnel.id && state.status == TunnelStatus::Running)
        })
        .collect();
    if active.is_empty() {
        return format!("TunnelWarden\r\n{}", text(language, "No running tunnels"));
    }
    let count = text(language, "{count} running").replace("{count}", &active.len().to_string());
    let mut output = format!("TunnelWarden · {count}\r\n");
    for tunnel in active {
        let (up, down) = rates.get(&tunnel.id).copied().unwrap_or_default();
        output.push_str(&format!(
            "\r\n{}\r\n  ↑ {}    ↓ {}\r\n",
            name(&tunnel.name),
            rate(up),
            rate(down)
        ));
    }
    output.trim_end().to_string()
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Bounds {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}
impl Bounds {
    pub fn contains(self, x: i32, y: i32) -> bool {
        x >= self.left && x < self.right && y >= self.top && y < self.bottom
    }
}
pub fn place(anchor: Bounds, work: Bounds, width: i32, height: i32, gap: i32) -> Bounds {
    let width = width.min(work.right - work.left).max(1);
    let height = height.min(work.bottom - work.top).max(1);
    let x = (anchor.right - width).clamp(work.left, work.right - width);
    let y = if anchor.top - gap - height >= work.top {
        anchor.top - gap - height
    } else {
        anchor.bottom + gap
    };
    let y = y.clamp(work.top, work.bottom - height);
    Bounds {
        left: x,
        top: y,
        right: x + width,
        bottom: y + height,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    #[test]
    fn compact_panel_only_lists_running_rules_in_both_languages() {
        use tunnelwarden_core::dto::RuntimeTunnel;
        let mut tunnels = Vec::new();
        let mut runtime = RuntimeSnapshot::default();
        for status in [
            TunnelStatus::Running,
            TunnelStatus::Stopped,
            TunnelStatus::Connecting,
            TunnelStatus::Retrying,
            TunnelStatus::Error,
        ] {
            let id = Uuid::new_v4();
            tunnels.push(TunnelDto {
                id,
                name: format!("rule-{status:?}"),
                kind: serde_json::from_value(serde_json::json!("Local")).unwrap(),
                host_id: Uuid::new_v4(),
                listen_address: "127.0.0.1".into(),
                listen_port: 8000,
                target_host: "localhost".into(),
                target_port: 80,
                auto_start: false,
            });
            runtime.tunnels.push(RuntimeTunnel {
                id,
                status,
                last_error: Some("hidden diagnostic".into()),
                next_retry_at: None,
            });
        }
        let rates = HashMap::from([(tunnels[0].id, (1024.0, 2048.0))]);
        for language in [Language::English, Language::Chinese] {
            let panel = panel_text(&tunnels, &runtime, &rates, language);
            assert!(panel.contains("rule-Running"));
            assert!(panel.contains("↑ 1.0 KiB/s    ↓ 2.0 KiB/s"));
            assert!(!panel.contains("hidden diagnostic"));
            for omitted in ["Stopped", "Connecting", "Retrying", "Error"] {
                assert!(!panel.contains(&format!("rule-{omitted}")));
            }
            runtime.tunnels[0].status = TunnelStatus::Stopped;
            let empty = panel_text(&tunnels, &runtime, &rates, language);
            assert_eq!(
                empty,
                format!("TunnelWarden\r\n{}", text(language, "No running tunnels"))
            );
            runtime.tunnels[0].status = TunnelStatus::Running;
        }
    }

    #[test]
    fn rates_reset_on_reattach_and_use_actual_sample_interval() {
        let now = Instant::now();
        let id = Uuid::new_v4();
        let mut sampler = RateSampler::default();
        let sample = |started_at, sent_bytes| {
            vec![TunnelTraffic {
                id,
                started_at,
                sent_bytes,
                received_bytes: sent_bytes * 2,
            }]
        };
        assert_eq!(sampler.sample(now, sample(now, 1024))[&id], (0.0, 0.0));
        assert_eq!(
            sampler.sample(now + Duration::from_secs(2), sample(now, 3072))[&id],
            (1024.0, 2048.0)
        );
        // Even if a new connection exceeds the old byte count, it is a fresh baseline.
        assert_eq!(
            sampler.sample(
                now + Duration::from_secs(3),
                sample(now + Duration::from_secs(2), 9999)
            )[&id],
            (0.0, 0.0)
        );
        sampler.sample(now, vec![]);
        assert!(sampler.previous.is_empty());
    }
    #[test]
    fn placement_handles_negative_monitor_coordinates_and_top_taskbars() {
        let work = Bounds {
            left: -1920,
            top: 40,
            right: 0,
            bottom: 1080,
        };
        for anchor in [
            Bounds {
                left: -30,
                top: 0,
                right: -10,
                bottom: 40,
            },
            Bounds {
                left: -1900,
                top: 1060,
                right: -1870,
                bottom: 1080,
            },
        ] {
            let p = place(anchor, work, 460, 420, 8);
            assert!(
                p.left >= work.left
                    && p.right <= work.right
                    && p.top >= work.top
                    && p.bottom <= work.bottom
            );
        }
    }
    #[test]
    fn labels_and_actions_are_safe_and_match_runtime_intent() {
        assert_eq!(menu_name("A&B\n\t"), "A&&B");
        for s in [
            TunnelStatus::Running,
            TunnelStatus::Connecting,
            TunnelStatus::Retrying,
        ] {
            assert!(can_stop(s));
        }
        for s in [TunnelStatus::Stopped, TunnelStatus::Error] {
            assert!(!can_stop(s));
        }
    }
}
