use std::time::Duration;

#[cfg(target_os = "macos")]
pub mod macos;

const SAMPLE_INTERVAL: Duration = Duration::from_secs(60);

pub fn format_decimal_gigabytes(bytes: u64) -> String {
    const BYTES_PER_GIGABYTE: f64 = 1_000_000_000.0;
    format!("{:.1} GB", bytes as f64 / BYTES_PER_GIGABYTE)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DiskObservation {
    total_bytes: u64,
    available_bytes: u64,
    observed_at: Duration,
}

impl DiskObservation {
    pub fn new(
        total_bytes: u64,
        available_bytes: u64,
        observed_at: Duration,
    ) -> Result<Self, InvalidDiskObservation> {
        if total_bytes == 0 || available_bytes > total_bytes {
            return Err(InvalidDiskObservation);
        }
        Ok(Self {
            total_bytes,
            available_bytes,
            observed_at,
        })
    }

    pub const fn total_bytes(self) -> u64 {
        self.total_bytes
    }

    pub const fn available_bytes(self) -> u64 {
        self.available_bytes
    }

    pub const fn observed_at(self) -> Duration {
        self.observed_at
    }

    pub fn occupied_percent(self) -> f64 {
        (self.total_bytes - self.available_bytes) as f64 * 100.0 / self.total_bytes as f64
    }

    pub fn is_at_or_above(self, threshold_percent: u8) -> bool {
        u128::from(self.total_bytes - self.available_bytes) * 100
            >= u128::from(self.total_bytes) * u128::from(threshold_percent)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InvalidDiskObservation;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DiskSamplingSchedule {
    next_due: Option<Duration>,
    last_checked: Option<Duration>,
}

impl DiskSamplingSchedule {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_enabled(&mut self, enabled: bool, now: Duration) {
        if enabled {
            self.next_due.get_or_insert(now);
            self.last_checked = Some(now);
        } else {
            self.next_due = None;
            self.last_checked = None;
        }
    }

    pub fn take_due(&mut self, now: Duration) -> bool {
        let Some(mut next_due) = self.next_due else {
            return false;
        };
        if self
            .last_checked
            .is_some_and(|last_checked| now < last_checked)
        {
            next_due = now;
            self.next_due = Some(now);
        }
        self.last_checked = Some(now);
        if now < next_due {
            return false;
        }

        self.next_due = Some(now.saturating_add(SAMPLE_INTERVAL));
        true
    }
}
