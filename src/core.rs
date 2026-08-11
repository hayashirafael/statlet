use std::time::Duration;

use crate::disk::DiskObservation;
use crate::history::HistoryEventKind;
use crate::mole::MoleStatus;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MemoryPressure {
    Normal,
    Warning,
    Critical,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UnknownMemoryPressure(pub i32);

impl TryFrom<i32> for MemoryPressure {
    type Error = UnknownMemoryPressure;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::Normal),
            2 => Ok(Self::Warning),
            4 => Ok(Self::Critical),
            other => Err(UnknownMemoryPressure(other)),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SystemSnapshot {
    pub cpu_percent: f64,
    pub ram_percent: f64,
    pub memory_pressure: MemoryPressure,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MetricSeverity {
    Good,
    Warning,
    Critical,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MetricPresentation {
    pub label: &'static str,
    pub value: String,
    pub severity: MetricSeverity,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StatusPresentation {
    pub top: MetricPresentation,
    pub bottom: MetricPresentation,
    pub disk_badge: Option<DiskBadge>,
    pub accessibility_label: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiskBadge {
    Warning,
    Error,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppState {
    pub status: StatusPresentation,
    pub preferences: Preferences,
    pub latest_disk_observation: Option<DiskObservation>,
    pub mole_status: MoleStatus,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AppEvent {
    MetricsSample(SystemSnapshot),
    OpenPreferences,
    OpenHistory,
    Quit,
    SetMoleIntegrationEnabled(bool),
    SetWarningThreshold(WarningThreshold),
    DiskObserved(DiskObservation),
    DiskMonitoringFailed,
    ReviewSpace,
    NotificationActivated,
    MoleStatusObserved(MoleStatus),
    OpenMoleInTerminal,
    ClearHistoryConfirmed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WindowKind {
    Preferences,
    History,
    FreeSpace,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AppEffect {
    ShowWindow(WindowKind),
    SavePreferences(Preferences),
    SetDiskSamplingEnabled(bool),
    DiskPressureAlert(DiskObservation),
    RequestNotificationAuthorization,
    CheckMoleCompatibility,
    LaunchMoleInTerminal,
    RecordHistory(HistoryEventKind),
    ClearHistory,
    Quit,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Preferences {
    pub mole_integration_enabled: bool,
    pub warning_threshold: WarningThreshold,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WarningThreshold(u8);

impl WarningThreshold {
    pub const fn get(self) -> u8 {
        self.0
    }
}

impl Default for WarningThreshold {
    fn default() -> Self {
        Self(90)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InvalidWarningThreshold(pub u8);

impl TryFrom<u8> for WarningThreshold {
    type Error = InvalidWarningThreshold;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        if (70..=95).contains(&value) && value.is_multiple_of(5) {
            Ok(Self(value))
        } else {
            Err(InvalidWarningThreshold(value))
        }
    }
}

pub struct StatletCore {
    state: AppState,
    system_snapshot: SystemSnapshot,
    disk_episode: DiskEpisode,
    last_mole_block: Option<HistoryEventKind>,
    monitoring_failure_active: bool,
}

impl StatletCore {
    pub fn new() -> Self {
        Self::with_preferences(Preferences::default()).0
    }

    pub fn with_preferences(preferences: Preferences) -> (Self, Vec<AppEffect>) {
        let disk_sampling_enabled = preferences.mole_integration_enabled;
        let system_snapshot = SystemSnapshot {
            cpu_percent: 0.0,
            ram_percent: 0.0,
            memory_pressure: MemoryPressure::Normal,
        };
        let core = Self {
            state: AppState {
                status: present(system_snapshot, None),
                preferences,
                latest_disk_observation: None,
                mole_status: MoleStatus::Unknown,
            },
            system_snapshot,
            disk_episode: DiskEpisode::default(),
            last_mole_block: None,
            monitoring_failure_active: false,
        };
        let mut effects = vec![AppEffect::SetDiskSamplingEnabled(disk_sampling_enabled)];
        if disk_sampling_enabled {
            effects.push(AppEffect::CheckMoleCompatibility);
        }
        (core, effects)
    }

    pub fn state(&self) -> &AppState {
        &self.state
    }

    pub fn handle(&mut self, event: AppEvent) -> Vec<AppEffect> {
        match event {
            AppEvent::MetricsSample(snapshot) => {
                self.system_snapshot = snapshot;
                self.refresh_status();
                Vec::new()
            }
            AppEvent::OpenPreferences => {
                vec![AppEffect::ShowWindow(WindowKind::Preferences)]
            }
            AppEvent::OpenHistory => vec![AppEffect::ShowWindow(WindowKind::History)],
            AppEvent::ReviewSpace | AppEvent::NotificationActivated => {
                let mut effects = Vec::with_capacity(2);
                if self.state.preferences.mole_integration_enabled {
                    self.state.mole_status = MoleStatus::Unknown;
                    self.refresh_status();
                    effects.push(AppEffect::CheckMoleCompatibility);
                }
                effects.push(AppEffect::ShowWindow(WindowKind::FreeSpace));
                effects
            }
            AppEvent::Quit => vec![AppEffect::Quit],
            AppEvent::SetMoleIntegrationEnabled(enabled) => {
                if self.state.preferences.mole_integration_enabled == enabled {
                    return Vec::new();
                }
                self.state.preferences.mole_integration_enabled = enabled;
                if !enabled {
                    self.disk_episode = DiskEpisode::default();
                    self.state.latest_disk_observation = None;
                    self.state.mole_status = MoleStatus::Unknown;
                    self.last_mole_block = None;
                    self.monitoring_failure_active = false;
                    self.refresh_status();
                }
                let mut effects = vec![
                    AppEffect::SavePreferences(self.state.preferences),
                    AppEffect::SetDiskSamplingEnabled(enabled),
                ];
                if enabled {
                    effects.push(AppEffect::RequestNotificationAuthorization);
                    effects.push(AppEffect::CheckMoleCompatibility);
                }
                effects
            }
            AppEvent::SetWarningThreshold(threshold) => {
                if self.state.preferences.warning_threshold == threshold {
                    return Vec::new();
                }
                self.state.preferences.warning_threshold = threshold;
                vec![AppEffect::SavePreferences(self.state.preferences)]
            }
            AppEvent::DiskObserved(observation) => {
                if !self.state.preferences.mole_integration_enabled {
                    return Vec::new();
                }
                self.state.latest_disk_observation = Some(observation);
                self.monitoring_failure_active = false;
                let transition = self
                    .disk_episode
                    .observe(observation, self.state.preferences.warning_threshold);
                self.refresh_status();
                match transition {
                    DiskEpisodeTransition::Started => vec![
                        AppEffect::RecordHistory(HistoryEventKind::DiskPressureStarted),
                        AppEffect::DiskPressureAlert(observation),
                    ],
                    DiskEpisodeTransition::Recovered => vec![AppEffect::RecordHistory(
                        HistoryEventKind::DiskPressureRecovered,
                    )],
                    DiskEpisodeTransition::None => Vec::new(),
                }
            }
            AppEvent::DiskMonitoringFailed => {
                if !self.state.preferences.mole_integration_enabled
                    || self.monitoring_failure_active
                {
                    return Vec::new();
                }
                self.monitoring_failure_active = true;
                vec![AppEffect::RecordHistory(HistoryEventKind::MonitoringFailed)]
            }
            AppEvent::MoleStatusObserved(status) => {
                if !self.state.preferences.mole_integration_enabled {
                    return Vec::new();
                }
                self.state.mole_status = status;
                self.refresh_status();
                let block = match status {
                    MoleStatus::Missing => Some(HistoryEventKind::MoleMissing),
                    MoleStatus::Unavailable => Some(HistoryEventKind::MoleUnavailable),
                    MoleStatus::Incompatible(_) => Some(HistoryEventKind::MoleIncompatible),
                    MoleStatus::Compatible(_) => {
                        self.last_mole_block = None;
                        None
                    }
                    MoleStatus::Unknown => None,
                };
                match block {
                    Some(block) if Some(block) != self.last_mole_block => {
                        self.last_mole_block = Some(block);
                        vec![AppEffect::RecordHistory(block)]
                    }
                    _ => Vec::new(),
                }
            }
            AppEvent::OpenMoleInTerminal => {
                if self.state.preferences.mole_integration_enabled
                    && self.state.mole_status.is_compatible()
                {
                    vec![AppEffect::LaunchMoleInTerminal]
                } else {
                    Vec::new()
                }
            }
            AppEvent::ClearHistoryConfirmed => vec![AppEffect::ClearHistory],
        }
    }

    fn refresh_status(&mut self) {
        let disk_badge = if self.state.preferences.mole_integration_enabled
            && self.state.mole_status.is_error()
        {
            Some(DiskBadge::Error)
        } else {
            self.disk_episode.is_active().then_some(DiskBadge::Warning)
        };
        self.state.status = present(self.system_snapshot, disk_badge);
    }
}

impl Default for StatletCore {
    fn default() -> Self {
        Self::new()
    }
}

const REQUIRED_PRESSURE_DURATION: Duration = Duration::from_secs(5 * 60);
const MAX_OBSERVED_GAP: Duration = Duration::from_secs(90);

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum DiskEpisode {
    #[default]
    Ready,
    Debouncing {
        started_at: Duration,
        last_observed_at: Duration,
    },
    Active,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DiskEpisodeTransition {
    None,
    Started,
    Recovered,
}

impl DiskEpisode {
    fn observe(
        &mut self,
        observation: DiskObservation,
        threshold: WarningThreshold,
    ) -> DiskEpisodeTransition {
        if !observation.is_at_or_above(threshold.get()) {
            let recovered = matches!(self, Self::Active);
            *self = Self::Ready;
            return if recovered {
                DiskEpisodeTransition::Recovered
            } else {
                DiskEpisodeTransition::None
            };
        }

        let observed_at = observation.observed_at();
        match *self {
            Self::Ready => {
                *self = Self::Debouncing {
                    started_at: observed_at,
                    last_observed_at: observed_at,
                };
                DiskEpisodeTransition::None
            }
            Self::Debouncing {
                started_at,
                last_observed_at,
            } => {
                let continuous = observed_at
                    .checked_sub(last_observed_at)
                    .is_some_and(|gap| gap <= MAX_OBSERVED_GAP);
                if !continuous {
                    *self = Self::Debouncing {
                        started_at: observed_at,
                        last_observed_at: observed_at,
                    };
                    return DiskEpisodeTransition::None;
                }

                if observed_at.saturating_sub(started_at) >= REQUIRED_PRESSURE_DURATION {
                    *self = Self::Active;
                    DiskEpisodeTransition::Started
                } else {
                    *self = Self::Debouncing {
                        started_at,
                        last_observed_at: observed_at,
                    };
                    DiskEpisodeTransition::None
                }
            }
            Self::Active => DiskEpisodeTransition::None,
        }
    }

    fn is_active(self) -> bool {
        matches!(self, Self::Active)
    }
}

fn present(snapshot: SystemSnapshot, disk_badge: Option<DiskBadge>) -> StatusPresentation {
    let cpu = rounded_percent(snapshot.cpu_percent);
    let ram = rounded_percent(snapshot.ram_percent);
    let (memory_severity, pressure_description) =
        memory_pressure_presentation(snapshot.memory_pressure);
    let disk_description = match disk_badge {
        Some(DiskBadge::Warning) => ", disco acima do limite",
        Some(DiskBadge::Error) => ", Mole indisponível",
        None => "",
    };
    StatusPresentation {
        top: MetricPresentation {
            label: "C",
            value: format!("{cpu:>3}%"),
            severity: cpu_severity(snapshot.cpu_percent),
        },
        bottom: MetricPresentation {
            label: "R",
            value: format!("{ram:>3}%"),
            severity: memory_severity,
        },
        disk_badge,
        accessibility_label: format!(
            "CPU {cpu}%, RAM {ram}%, pressão de memória {pressure_description}{disk_description}"
        ),
    }
}

fn rounded_percent(value: f64) -> u8 {
    value.clamp(0.0, 100.0).round() as u8
}

fn cpu_severity(percent: f64) -> MetricSeverity {
    match percent {
        value if value < 40.0 => MetricSeverity::Good,
        value if value < 70.0 => MetricSeverity::Warning,
        _ => MetricSeverity::Critical,
    }
}

fn memory_pressure_presentation(pressure: MemoryPressure) -> (MetricSeverity, &'static str) {
    match pressure {
        MemoryPressure::Normal => (MetricSeverity::Good, "normal"),
        MemoryPressure::Warning => (MetricSeverity::Warning, "em atenção"),
        MemoryPressure::Critical => (MetricSeverity::Critical, "crítica"),
    }
}
