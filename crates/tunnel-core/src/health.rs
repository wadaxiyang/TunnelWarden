use std::time::Duration;

#[derive(Clone, Debug)]
pub enum WorkerHealth {
    Healthy { rtts: Vec<Duration> },
    Degraded { failed_pings: u8, reason: String },
}
