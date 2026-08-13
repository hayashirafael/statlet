use std::time::Duration;

use crate::indicator_preferences::MetricsRefreshInterval;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MetricsSamplingSchedule {
    interval: Duration,
    next_due: Duration,
}

impl MetricsSamplingSchedule {
    pub fn new_due_now(now: Duration, interval: MetricsRefreshInterval) -> Self {
        Self {
            interval: duration(interval),
            next_due: now,
        }
    }

    pub fn reschedule(&mut self, now: Duration, interval: MetricsRefreshInterval) {
        self.interval = duration(interval);
        self.next_due = now.saturating_add(self.interval);
    }

    pub fn take_due(&mut self, now: Duration) -> bool {
        if now < self.next_due {
            return false;
        }

        self.next_due = now.saturating_add(self.interval);
        true
    }

    pub fn remaining(&self, now: Duration) -> Duration {
        self.next_due.saturating_sub(now)
    }
}

fn duration(interval: MetricsRefreshInterval) -> Duration {
    Duration::from_secs(u64::from(interval.seconds()))
}
