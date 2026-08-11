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
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AppEvent {
    MetricsSample(SystemSnapshot),
}

pub struct StatletCore {
    state: AppState,
}

impl StatletCore {
    pub fn new() -> Self {
        Self {
            state: AppState {
                status: present(SystemSnapshot {
                    cpu_percent: 0.0,
                    ram_percent: 0.0,
                    memory_pressure: MemoryPressure::Normal,
                }),
            },
        }
    }

    pub fn handle(&mut self, event: AppEvent) -> &AppState {
        match event {
            AppEvent::MetricsSample(snapshot) => self.state.status = present(snapshot),
        }
        &self.state
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
