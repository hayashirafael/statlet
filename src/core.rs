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
    pub accessibility_label: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppState {
    pub status: StatusPresentation,
    pub preferences: Preferences,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AppEvent {
    MetricsSample(SystemSnapshot),
    OpenPreferences,
    OpenHistory,
    Quit,
    SetMoleIntegrationEnabled(bool),
    SetWarningThreshold(WarningThreshold),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WindowKind {
    Preferences,
    History,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AppEffect {
    ShowWindow(WindowKind),
    SavePreferences(Preferences),
    SetDiskSamplingEnabled(bool),
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
}

impl StatletCore {
    pub fn new() -> Self {
        Self::with_preferences(Preferences::default()).0
    }

    pub fn with_preferences(preferences: Preferences) -> (Self, Vec<AppEffect>) {
        let disk_sampling_enabled = preferences.mole_integration_enabled;
        let core = Self {
            state: AppState {
                status: present(SystemSnapshot {
                    cpu_percent: 0.0,
                    ram_percent: 0.0,
                    memory_pressure: MemoryPressure::Normal,
                }),
                preferences,
            },
        };
        (
            core,
            vec![AppEffect::SetDiskSamplingEnabled(disk_sampling_enabled)],
        )
    }

    pub fn state(&self) -> &AppState {
        &self.state
    }

    pub fn handle(&mut self, event: AppEvent) -> Vec<AppEffect> {
        match event {
            AppEvent::MetricsSample(snapshot) => {
                self.state.status = present(snapshot);
                Vec::new()
            }
            AppEvent::OpenPreferences => {
                vec![AppEffect::ShowWindow(WindowKind::Preferences)]
            }
            AppEvent::OpenHistory => vec![AppEffect::ShowWindow(WindowKind::History)],
            AppEvent::Quit => vec![AppEffect::Quit],
            AppEvent::SetMoleIntegrationEnabled(enabled) => {
                if self.state.preferences.mole_integration_enabled == enabled {
                    return Vec::new();
                }
                self.state.preferences.mole_integration_enabled = enabled;
                vec![
                    AppEffect::SavePreferences(self.state.preferences),
                    AppEffect::SetDiskSamplingEnabled(enabled),
                ]
            }
            AppEvent::SetWarningThreshold(threshold) => {
                if self.state.preferences.warning_threshold == threshold {
                    return Vec::new();
                }
                self.state.preferences.warning_threshold = threshold;
                vec![AppEffect::SavePreferences(self.state.preferences)]
            }
        }
    }
}

impl Default for StatletCore {
    fn default() -> Self {
        Self::new()
    }
}

fn present(snapshot: SystemSnapshot) -> StatusPresentation {
    let cpu = rounded_percent(snapshot.cpu_percent);
    let ram = rounded_percent(snapshot.ram_percent);
    let (memory_severity, pressure_description) =
        memory_pressure_presentation(snapshot.memory_pressure);
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
        accessibility_label: format!(
            "CPU {cpu}%, RAM {ram}%, pressão de memória {pressure_description}"
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
