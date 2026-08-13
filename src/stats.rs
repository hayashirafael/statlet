use std::collections::VecDeque;
use std::time::Duration;

use crate::core::MemoryPressure;
use crate::metrics::MemoryReading;

pub const USAGE_HISTORY_CAPACITY: usize = 150;
const MAX_CONTINUOUS_SAMPLE_GAP: Duration = Duration::from_secs(6);
const SAMPLE_INTERVAL: Duration = Duration::from_secs(2);
const PROCESS_SAMPLE_INTERVAL: Duration = Duration::from_secs(4);

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UsagePoint {
    pub observed_at: Duration,
    pub value: Option<f64>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct UsageHistory {
    points: VecDeque<UsagePoint>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GpuReading {
    pub utilization_percent: f64,
    pub renderer_percent: Option<f64>,
    pub tiler_percent: Option<f64>,
    pub in_use_system_memory_bytes: Option<u64>,
    pub allocated_system_memory_bytes: Option<u64>,
    pub device_name: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum GpuSampleOutcome {
    Available(GpuReading),
    Unavailable,
    Failed,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SystemUsageSection {
    #[default]
    Ram,
    Gpu,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StatsDetailRow {
    pub label: &'static str,
    pub value: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProcessMemory {
    pub pid: u32,
    pub name: String,
    pub memory_bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProcessRowViewModel {
    pub pid: u32,
    pub name: String,
    pub memory: String,
    pub accessibility_label: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SystemUsageViewModel {
    pub section: SystemUsageSection,
    pub primary_value: String,
    pub secondary_value: String,
    pub status: String,
    pub details: Vec<StatsDetailRow>,
    pub history: Vec<UsagePoint>,
    pub history_accessibility_label: String,
    pub process_rows: Vec<ProcessRowViewModel>,
}

#[derive(Clone, Debug, PartialEq)]
enum GpuReadingState {
    Collecting,
    Available(GpuReading),
    Stale(GpuReading),
    Unavailable,
    Failed,
}

impl GpuReading {
    pub fn normalized(
        utilization_percent: f64,
        renderer_percent: Option<f64>,
        tiler_percent: Option<f64>,
        in_use_system_memory_bytes: Option<u64>,
        allocated_system_memory_bytes: Option<u64>,
        device_name: Option<String>,
    ) -> Option<Self> {
        Some(Self {
            utilization_percent: normalize_required_percent(utilization_percent)?,
            renderer_percent: renderer_percent.and_then(normalize_optional_percent),
            tiler_percent: tiler_percent.and_then(normalize_optional_percent),
            in_use_system_memory_bytes,
            allocated_system_memory_bytes,
            device_name,
        })
    }
}

fn normalize_required_percent(value: f64) -> Option<f64> {
    value.is_finite().then(|| value.clamp(0.0, 100.0))
}

fn normalize_optional_percent(value: f64) -> Option<f64> {
    normalize_required_percent(value)
}

#[derive(Clone, Debug, PartialEq)]
pub struct SystemUsageModel {
    visible: bool,
    memory: Option<MemoryReading>,
    memory_stale: bool,
    ram_history: UsageHistory,
    gpu_state: GpuReadingState,
    gpu_history: UsageHistory,
    processes: Vec<ProcessMemory>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ProcessSamplingSchedule {
    visible: bool,
    next_due: Option<Duration>,
}

impl UsageHistory {
    pub fn new() -> Self {
        Self {
            points: VecDeque::with_capacity(USAGE_HISTORY_CAPACITY),
        }
    }

    pub fn push(&mut self, observed_at: Duration, value: Option<f64>) {
        if self
            .points
            .back()
            .is_some_and(|last| observed_at <= last.observed_at)
        {
            return;
        }
        if self
            .points
            .back()
            .and_then(|last| observed_at.checked_sub(last.observed_at))
            .is_some_and(|gap| gap > MAX_CONTINUOUS_SAMPLE_GAP)
        {
            self.push_point(UsagePoint {
                observed_at: observed_at.saturating_sub(SAMPLE_INTERVAL),
                value: None,
            });
        }
        self.push_point(UsagePoint { observed_at, value });
    }

    fn push_point(&mut self, point: UsagePoint) {
        if self.points.len() == USAGE_HISTORY_CAPACITY {
            self.points.pop_front();
        }
        self.points.push_back(point);
    }

    pub fn points(&self) -> &VecDeque<UsagePoint> {
        &self.points
    }

    fn clear(&mut self) {
        self.points.clear();
    }
}

impl SystemUsageModel {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_visible(&mut self, visible: bool) {
        if self.visible && !visible {
            self.gpu_history.clear();
            self.gpu_state = GpuReadingState::Collecting;
            self.processes.clear();
        }
        self.visible = visible;
    }

    pub fn record_memory(&mut self, observed_at: Duration, outcome: Result<MemoryReading, ()>) {
        match outcome {
            Ok(reading) => {
                self.ram_history.push(observed_at, Some(reading.percent));
                self.memory = Some(reading);
                self.memory_stale = false;
            }
            Err(()) => {
                self.ram_history.push(observed_at, None);
                self.memory_stale = self.memory.is_some();
            }
        }
    }

    pub fn record_gpu(&mut self, observed_at: Duration, outcome: GpuSampleOutcome) {
        if !self.visible {
            return;
        }
        let value = match outcome {
            GpuSampleOutcome::Available(reading) => {
                let value = Some(reading.utilization_percent);
                self.gpu_state = GpuReadingState::Available(reading);
                value
            }
            GpuSampleOutcome::Unavailable => {
                self.gpu_state = GpuReadingState::Unavailable;
                None
            }
            GpuSampleOutcome::Failed => {
                self.gpu_state = match &self.gpu_state {
                    GpuReadingState::Available(reading) | GpuReadingState::Stale(reading) => {
                        GpuReadingState::Stale(reading.clone())
                    }
                    GpuReadingState::Collecting
                    | GpuReadingState::Unavailable
                    | GpuReadingState::Failed => GpuReadingState::Failed,
                };
                None
            }
        };
        self.gpu_history.push(observed_at, value);
    }

    pub fn record_processes(&mut self, outcome: Result<Vec<ProcessMemory>, ()>) {
        if !self.visible {
            return;
        }
        if let Ok(mut processes) = outcome {
            processes.sort_by(|left, right| {
                right
                    .memory_bytes
                    .cmp(&left.memory_bytes)
                    .then_with(|| left.pid.cmp(&right.pid))
            });
            processes.truncate(20);
            self.processes = processes;
        }
    }

    pub fn gpu_history(&self) -> &UsageHistory {
        &self.gpu_history
    }

    pub fn view_model(&self, section: SystemUsageSection) -> SystemUsageViewModel {
        match section {
            SystemUsageSection::Ram => self.ram_view_model(),
            SystemUsageSection::Gpu => self.gpu_view_model(),
        }
    }

    fn ram_view_model(&self) -> SystemUsageViewModel {
        let Some(memory) = self.memory else {
            let mut view_model = empty_view_model(
                SystemUsageSection::Ram,
                "Coletando a primeira leitura…",
                &self.ram_history,
            );
            view_model.process_rows = self.process_rows();
            return view_model;
        };
        let (symbol, pressure) = match memory.pressure {
            MemoryPressure::Normal => ("✓", "Pressão normal"),
            MemoryPressure::Warning => ("△", "Pressão em atenção"),
            MemoryPressure::Critical => ("!", "Pressão crítica"),
        };
        let history = self.ram_history.points.iter().copied().collect::<Vec<_>>();
        SystemUsageViewModel {
            section: SystemUsageSection::Ram,
            primary_value: format!("{}% em uso", memory.percent.round() as u8),
            secondary_value: format!(
                "{} de {}",
                format_bytes(memory.used_bytes),
                format_bytes(memory.total_bytes)
            ),
            status: if self.memory_stale {
                format!("{symbol} {pressure} — atualização interrompida; última leitura preservada")
            } else {
                format!("{symbol} {pressure}")
            },
            details: vec![
                detail("Apps", memory.app_bytes),
                detail("Reservada pelo sistema", memory.wired_bytes),
                detail("Comprimida", memory.compressed_bytes),
                detail("Disponível", memory.available_bytes),
                detail("Cache recuperável", memory.cached_bytes),
                detail("Swap em uso", memory.swap_used_bytes),
            ],
            history_accessibility_label: history_summary(SystemUsageSection::Ram, &history),
            history,
            process_rows: self.process_rows(),
        }
    }

    fn process_rows(&self) -> Vec<ProcessRowViewModel> {
        self.processes
            .iter()
            .map(|process| {
                let memory = format_bytes(process.memory_bytes);
                ProcessRowViewModel {
                    pid: process.pid,
                    name: process.name.clone(),
                    accessibility_label: format!("{}, {memory}", process.name),
                    memory,
                }
            })
            .collect()
    }

    fn gpu_view_model(&self) -> SystemUsageViewModel {
        match &self.gpu_state {
            GpuReadingState::Collecting => empty_view_model(
                SystemUsageSection::Gpu,
                "Coletando a primeira leitura…",
                &self.gpu_history,
            ),
            GpuReadingState::Unavailable => empty_view_model(
                SystemUsageSection::Gpu,
                "O uso da GPU não está disponível neste Mac.",
                &self.gpu_history,
            ),
            GpuReadingState::Failed => empty_view_model(
                SystemUsageSection::Gpu,
                "Não foi possível ler esta métrica.",
                &self.gpu_history,
            ),
            GpuReadingState::Available(reading) => {
                self.available_gpu_view_model(reading, "Atualizado agora")
            }
            GpuReadingState::Stale(reading) => self.available_gpu_view_model(
                reading,
                "Atualização interrompida; última leitura preservada",
            ),
        }
    }

    fn available_gpu_view_model(&self, reading: &GpuReading, status: &str) -> SystemUsageViewModel {
        let mut details = vec![StatsDetailRow {
            label: "Uso atual",
            value: format!("{}%", reading.utilization_percent.round() as u8),
        }];
        if let Some(renderer) = reading.renderer_percent {
            details.push(StatsDetailRow {
                label: "Renderer",
                value: format!("{}%", renderer.round() as u8),
            });
        }
        if let Some(tiler) = reading.tiler_percent {
            details.push(StatsDetailRow {
                label: "Tiler",
                value: format!("{}%", tiler.round() as u8),
            });
        }
        if let Some(bytes) = reading.in_use_system_memory_bytes {
            details.push(detail("Memória compartilhada em uso", bytes));
        }
        let history = self.gpu_history.points.iter().copied().collect::<Vec<_>>();
        SystemUsageViewModel {
            section: SystemUsageSection::Gpu,
            primary_value: format!("{}% de uso", reading.utilization_percent.round() as u8),
            secondary_value: reading
                .device_name
                .clone()
                .unwrap_or_else(|| "GPU Apple".to_owned()),
            status: status.to_owned(),
            details,
            history_accessibility_label: history_summary(SystemUsageSection::Gpu, &history),
            history,
            process_rows: Vec::new(),
        }
    }
}

impl ProcessSamplingSchedule {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_visible(&mut self, visible: bool, now: Duration) {
        if visible == self.visible {
            return;
        }
        self.visible = visible;
        self.next_due = visible.then_some(now);
    }

    pub fn take_due(&mut self, now: Duration) -> bool {
        let Some(next_due) = self.next_due else {
            return false;
        };
        if now < next_due {
            return false;
        }
        self.next_due = Some(now.saturating_add(PROCESS_SAMPLE_INTERVAL));
        true
    }
}

impl Default for SystemUsageModel {
    fn default() -> Self {
        Self {
            visible: false,
            memory: None,
            memory_stale: false,
            ram_history: UsageHistory::new(),
            gpu_state: GpuReadingState::Collecting,
            gpu_history: UsageHistory::new(),
            processes: Vec::new(),
        }
    }
}

fn empty_view_model(
    section: SystemUsageSection,
    status: &str,
    history: &UsageHistory,
) -> SystemUsageViewModel {
    let points = history.points.iter().copied().collect::<Vec<_>>();
    SystemUsageViewModel {
        section,
        primary_value: "—".to_owned(),
        secondary_value: String::new(),
        status: status.to_owned(),
        details: Vec::new(),
        history_accessibility_label: history_summary(section, &points),
        history: points,
        process_rows: Vec::new(),
    }
}

fn history_summary(section: SystemUsageSection, history: &[UsagePoint]) -> String {
    let values = history
        .iter()
        .filter_map(|point| point.value)
        .collect::<Vec<_>>();
    if values.len() < 2 {
        return "O histórico aparecerá após duas leituras.".to_owned();
    }
    let current = values.last().copied().unwrap_or_default();
    let minimum = values.iter().copied().fold(f64::INFINITY, f64::min);
    let maximum = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let metric = match section {
        SystemUsageSection::Ram => "RAM",
        SystemUsageSection::Gpu => "GPU",
    };
    format!(
        "Histórico de uso de {metric}, últimos 5 minutos. Atual {}%, mínimo {}%, máximo {}%.",
        current.round() as u8,
        minimum.round() as u8,
        maximum.round() as u8
    )
}

fn detail(label: &'static str, bytes: u64) -> StatsDetailRow {
    StatsDetailRow {
        label,
        value: format_bytes(bytes),
    }
}

fn format_bytes(bytes: u64) -> String {
    const KB: f64 = 1_000.0;
    const MB: f64 = 1_000_000.0;
    const GB: f64 = 1_000_000_000.0;
    let (value, unit) = if bytes as f64 >= GB {
        (bytes as f64 / GB, "GB")
    } else if bytes as f64 >= MB {
        (bytes as f64 / MB, "MB")
    } else if bytes as f64 >= KB {
        (bytes as f64 / KB, "KB")
    } else {
        return format!("{bytes} B");
    };
    let formatted = if (value - value.round()).abs() < 0.05 {
        format!("{value:.0}")
    } else {
        format!("{value:.1}").replace('.', ",")
    };
    format!("{formatted} {unit}")
}
