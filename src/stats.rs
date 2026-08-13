use std::collections::VecDeque;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;

use crate::core::MemoryPressure;
use crate::metrics::MemoryReading;

pub const USAGE_HISTORY_CAPACITY: usize = 150;
pub const USAGE_HISTORY_WINDOW: Duration = Duration::from_secs(5 * 60);
const MAX_CONTINUOUS_SAMPLE_GAP: Duration = Duration::from_secs(6);
const SAMPLE_INTERVAL: Duration = Duration::from_secs(2);
const PROCESS_SAMPLE_INTERVAL: Duration = Duration::from_secs(6);
const MAX_PROCESS_REORDER_DEFERRAL: Duration = Duration::from_secs(15);

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UsagePoint {
    pub observed_at: Duration,
    pub value: Option<f64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GraphNavigationCommand {
    Previous,
    Next,
    First,
    Last,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GraphSampleSelection {
    pub index: usize,
    pub accessibility_value: String,
    pub should_notify_accessibility: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GraphNavigation {
    selected_observed_at: Option<Duration>,
    follows_latest: bool,
}

impl Default for GraphNavigation {
    fn default() -> Self {
        Self {
            selected_observed_at: None,
            follows_latest: true,
        }
    }
}

impl GraphNavigation {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn update_points(&mut self, points: &[UsagePoint]) -> Option<GraphSampleSelection> {
        let valid_indices = valid_graph_indices(points);
        let index = if self.follows_latest {
            valid_indices.last().copied()
        } else {
            self.selected_observed_at
                .and_then(|observed_at| {
                    valid_indices
                        .iter()
                        .copied()
                        .find(|&index| points[index].observed_at == observed_at)
                })
                .or_else(|| valid_indices.first().copied())
        }?;
        self.selected_observed_at = Some(points[index].observed_at);
        Some(graph_selection(points, index, false))
    }

    pub fn move_selection(
        &mut self,
        points: &[UsagePoint],
        command: GraphNavigationCommand,
    ) -> Option<GraphSampleSelection> {
        let valid_indices = valid_graph_indices(points);
        let current = self
            .selected_observed_at
            .and_then(|observed_at| {
                valid_indices
                    .iter()
                    .position(|&index| points[index].observed_at == observed_at)
            })
            .unwrap_or_else(|| valid_indices.len().saturating_sub(1));
        let position = match command {
            GraphNavigationCommand::Previous => current.saturating_sub(1),
            GraphNavigationCommand::Next => {
                (current + 1).min(valid_indices.len().saturating_sub(1))
            }
            GraphNavigationCommand::First => 0,
            GraphNavigationCommand::Last => valid_indices.len().saturating_sub(1),
        };
        let index = *valid_indices.get(position)?;
        let should_notify_accessibility =
            self.selected_observed_at != Some(points[index].observed_at);
        self.follows_latest = index == *valid_indices.last()?;
        self.selected_observed_at = Some(points[index].observed_at);
        Some(graph_selection(points, index, should_notify_accessibility))
    }
}

fn valid_graph_indices(points: &[UsagePoint]) -> Vec<usize> {
    points
        .iter()
        .enumerate()
        .filter_map(|(index, point)| point.value.filter(|value| value.is_finite()).map(|_| index))
        .collect()
}

fn graph_selection(
    points: &[UsagePoint],
    index: usize,
    should_notify_accessibility: bool,
) -> GraphSampleSelection {
    let point = points[index];
    let window_end = points
        .last()
        .map_or(point.observed_at, |last| last.observed_at);
    let age = window_end.saturating_sub(point.observed_at).as_secs();
    let time = match age {
        0 => "agora".to_owned(),
        1 => "há 1 segundo".to_owned(),
        seconds => format!("há {seconds} segundos"),
    };
    GraphSampleSelection {
        index,
        should_notify_accessibility,
        accessibility_value: format!(
            "{}%, {time}",
            point
                .value
                .expect("selection only contains valid points")
                .round() as u8
        ),
    }
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SystemUsageAccessibilityState {
    Collecting,
    Available,
    MemoryPressure(MemoryPressure),
    Unavailable,
    Failed,
    Stale,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SystemUsageAccessibilityUpdate {
    pub announcement: Option<String>,
    pub focus_summary: bool,
}

impl SystemUsageAccessibilityUpdate {
    pub fn include_announcement(&mut self, announcement: impl Into<String>) {
        let announcement = announcement.into();
        self.announcement = Some(match self.announcement.take() {
            Some(existing) => format!("{existing} {announcement}"),
            None => announcement,
        });
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SystemUsageAccessibilityCoordinator {
    ram_state: Option<SystemUsageAccessibilityState>,
    gpu_state: Option<SystemUsageAccessibilityState>,
    last_ram_pressure: Option<MemoryPressure>,
    pending_summary_focus: Option<SystemUsageSection>,
}

impl SystemUsageAccessibilityCoordinator {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn request_summary_focus_after_user_switch(&mut self, section: SystemUsageSection) {
        self.pending_summary_focus = Some(section);
    }

    pub fn observe(
        &mut self,
        section: SystemUsageSection,
        state: SystemUsageAccessibilityState,
    ) -> SystemUsageAccessibilityUpdate {
        let previous_ram_pressure = self.last_ram_pressure;
        if let (SystemUsageSection::Ram, SystemUsageAccessibilityState::MemoryPressure(pressure)) =
            (section, state)
        {
            self.last_ram_pressure = Some(pressure);
        }
        let previous = match section {
            SystemUsageSection::Ram => self.ram_state.replace(state),
            SystemUsageSection::Gpu => self.gpu_state.replace(state),
        };
        let focus_summary = self.pending_summary_focus == Some(section);
        if focus_summary {
            self.pending_summary_focus = None;
        }
        SystemUsageAccessibilityUpdate {
            announcement: previous.and_then(|previous| {
                accessibility_transition_announcement(
                    section,
                    previous,
                    state,
                    previous_ram_pressure,
                )
            }),
            focus_summary,
        }
    }
}

fn accessibility_transition_announcement(
    section: SystemUsageSection,
    previous: SystemUsageAccessibilityState,
    current: SystemUsageAccessibilityState,
    previous_ram_pressure: Option<MemoryPressure>,
) -> Option<String> {
    if previous == current {
        return None;
    }
    let was_interrupted = accessibility_state_is_interrupted(previous);
    let is_interrupted = accessibility_state_is_interrupted(current);
    if is_interrupted && !was_interrupted {
        return Some(match section {
            SystemUsageSection::Ram => "Atualização da RAM interrompida.".to_owned(),
            SystemUsageSection::Gpu => "Atualização da GPU interrompida.".to_owned(),
        });
    }
    if was_interrupted && !is_interrupted && accessibility_state_is_available(current) {
        let recovery = match section {
            SystemUsageSection::Ram => "Leitura de RAM restabelecida.".to_owned(),
            SystemUsageSection::Gpu => "Leitura de GPU restabelecida.".to_owned(),
        };
        return Some(match (section, current) {
            (SystemUsageSection::Ram, SystemUsageAccessibilityState::MemoryPressure(pressure))
                if previous_ram_pressure.is_some_and(|previous| previous != pressure) =>
            {
                format!("{recovery} {}", memory_pressure_announcement(pressure))
            }
            _ => recovery,
        });
    }
    match (section, current) {
        (SystemUsageSection::Ram, SystemUsageAccessibilityState::MemoryPressure(pressure)) => {
            Some(memory_pressure_announcement(pressure).to_owned())
        }
        _ => None,
    }
}

fn memory_pressure_announcement(pressure: MemoryPressure) -> &'static str {
    match pressure {
        MemoryPressure::Normal => "Pressão da memória normal.",
        MemoryPressure::Warning => "Pressão da memória em atenção.",
        MemoryPressure::Critical => "Pressão da memória crítica.",
    }
}

fn accessibility_state_is_interrupted(state: SystemUsageAccessibilityState) -> bool {
    matches!(
        state,
        SystemUsageAccessibilityState::Unavailable
            | SystemUsageAccessibilityState::Failed
            | SystemUsageAccessibilityState::Stale
    )
}

fn accessibility_state_is_available(state: SystemUsageAccessibilityState) -> bool {
    matches!(
        state,
        SystemUsageAccessibilityState::Available | SystemUsageAccessibilityState::MemoryPressure(_)
    )
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StatsDetailRow {
    pub label: &'static str,
    pub value: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MemoryCompositionSegment {
    pub label: &'static str,
    pub fraction: f64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProcessMemory {
    pub pid: u32,
    pub name: String,
    pub memory_bytes: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ProcessListStatus {
    #[default]
    Collecting,
    Available,
    Unavailable,
    Failed,
    Stale,
}

impl ProcessListStatus {
    pub fn message(self) -> &'static str {
        match self {
            Self::Collecting => "Coletando detalhes por processo…",
            Self::Available => "",
            Self::Unavailable => "Detalhes por processo não estão disponíveis nesta leitura.",
            Self::Failed => "Não foi possível ler os detalhes por processo.",
            Self::Stale => "Detalhes por processo desatualizados; última leitura preservada.",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProcessSampleOutcome {
    Available(Vec<ProcessMemory>),
    Unavailable,
    Failed,
    Cancelled,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProcessSampleCompletion {
    pub observed_at: Duration,
    pub live_visible: bool,
    pub interaction_active: bool,
    pub request_visibility_generation: u64,
    pub live_visibility_generation: u64,
    pub generation: u64,
    pub outcome: ProcessSampleOutcome,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProcessRowViewModel {
    pub pid: u32,
    pub name: String,
    pub memory: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SystemUsageViewModel {
    pub section: SystemUsageSection,
    pub accessibility_state: SystemUsageAccessibilityState,
    pub primary_value: String,
    pub secondary_value: String,
    pub status: String,
    pub details: Vec<StatsDetailRow>,
    pub memory_composition: Vec<MemoryCompositionSegment>,
    pub memory_composition_accessibility_label: String,
    pub history: Vec<UsagePoint>,
    pub history_accessibility_label: String,
    pub process_rows: Vec<ProcessRowViewModel>,
    pub process_status: ProcessListStatus,
}

#[derive(Clone, Debug, Default)]
pub struct SystemUsageRenderCoalescer {
    last_rendered: Option<SystemUsageViewModel>,
}

impl SystemUsageRenderCoalescer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn take_changed(&mut self, view_model: &SystemUsageViewModel) -> bool {
        if self.last_rendered.as_ref() == Some(view_model) {
            return false;
        }
        self.last_rendered = Some(view_model.clone());
        true
    }

    pub fn reset(&mut self) {
        self.last_rendered = None;
    }
}

#[derive(Clone, Debug, PartialEq)]
enum GpuReadingState {
    Collecting,
    Available(GpuReading, Duration),
    Stale(GpuReading, Duration),
    Unavailable,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum MemoryReadingState {
    Collecting,
    Available(MemoryReading, Duration),
    Stale(MemoryReading, Duration),
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

    fn human_device_name(&self) -> Option<&str> {
        self.device_name.as_deref().filter(|name| {
            let normalized = name.trim().to_ascii_lowercase();
            !normalized.is_empty()
                && !normalized.starts_with("agx")
                && !normalized.contains("accelerator")
        })
    }
}

fn normalize_required_percent(value: f64) -> Option<f64> {
    value.is_finite().then(|| value.clamp(0.0, 100.0))
}

fn normalize_optional_percent(value: f64) -> Option<f64> {
    normalize_required_percent(value)
}

pub fn history_x_position(observed_at: Duration, window_end: Duration) -> f64 {
    let age = window_end.saturating_sub(observed_at);
    (1.0 - age.as_secs_f64() / USAGE_HISTORY_WINDOW.as_secs_f64()).clamp(0.0, 1.0)
}

pub fn graph_pointer_selection(
    points: &[UsagePoint],
    normalized_x: f64,
) -> Option<GraphSampleSelection> {
    let window_end = points.last()?.observed_at;
    let index = points
        .iter()
        .enumerate()
        .min_by(|(_, left), (_, right)| {
            let left_distance =
                (history_x_position(left.observed_at, window_end) - normalized_x).abs();
            let right_distance =
                (history_x_position(right.observed_at, window_end) - normalized_x).abs();
            left_distance.total_cmp(&right_distance)
        })?
        .0;
    points[index].value.filter(|value| value.is_finite())?;
    Some(graph_selection(points, index, false))
}

#[derive(Clone, Debug, PartialEq)]
pub struct SystemUsageModel {
    visible: bool,
    memory_state: MemoryReadingState,
    ram_history: UsageHistory,
    gpu_state: GpuReadingState,
    gpu_history: UsageHistory,
    processes: Vec<ProcessMemory>,
    process_status: ProcessListStatus,
    deferred_processes: Option<DeferredProcessRows>,
    latest_observed_at: Duration,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct DeferredProcessRows {
    started_at: Duration,
    rows: Vec<ProcessMemory>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ProcessSamplingSchedule {
    visible: bool,
    next_due: Option<Duration>,
}

#[derive(Clone, Debug)]
pub struct ProcessSampleCancellation(Arc<AtomicBool>);

impl ProcessSampleCancellation {
    fn new() -> Self {
        Self(Arc::new(AtomicBool::new(false)))
    }

    fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

pub trait SystemUsageSamplingPorts {
    fn sample_gpu(&mut self) -> GpuSampleOutcome;
    fn request_process_sample(
        &mut self,
        generation: u64,
        cancellation: ProcessSampleCancellation,
    ) -> bool;
}

#[derive(Clone, Debug, Default)]
pub struct SystemUsageSamplingCoordinator {
    visible: bool,
    generation: u64,
    process_in_flight: Option<u64>,
    process_cancellation: Option<ProcessSampleCancellation>,
    process_schedule: ProcessSamplingSchedule,
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
        self.expire_before(observed_at.saturating_sub(USAGE_HISTORY_WINDOW));
    }

    fn push_point(&mut self, point: UsagePoint) {
        if self.points.len() == USAGE_HISTORY_CAPACITY {
            self.points.pop_front();
        }
        self.points.push_back(point);
    }

    fn expire_before(&mut self, cutoff: Duration) {
        while self
            .points
            .front()
            .is_some_and(|point| point.observed_at < cutoff)
        {
            self.points.pop_front();
        }
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
            self.process_status = ProcessListStatus::Collecting;
            self.deferred_processes = None;
        }
        self.visible = visible;
    }

    pub fn record_memory(&mut self, observed_at: Duration, outcome: Result<MemoryReading, ()>) {
        self.latest_observed_at = self.latest_observed_at.max(observed_at);
        match outcome {
            Ok(reading) => {
                self.ram_history.push(observed_at, Some(reading.percent));
                self.memory_state = MemoryReadingState::Available(reading, observed_at);
            }
            Err(()) => {
                self.ram_history.push(observed_at, None);
                self.memory_state = match self.memory_state {
                    MemoryReadingState::Available(reading, valid_at)
                    | MemoryReadingState::Stale(reading, valid_at) => {
                        MemoryReadingState::Stale(reading, valid_at)
                    }
                    MemoryReadingState::Collecting | MemoryReadingState::Failed => {
                        MemoryReadingState::Failed
                    }
                };
            }
        }
    }

    pub fn record_gpu(&mut self, observed_at: Duration, outcome: GpuSampleOutcome) {
        if !self.visible {
            return;
        }
        self.latest_observed_at = self.latest_observed_at.max(observed_at);
        let value = match outcome {
            GpuSampleOutcome::Available(reading) => {
                let value = Some(reading.utilization_percent);
                self.gpu_state = GpuReadingState::Available(reading, observed_at);
                value
            }
            GpuSampleOutcome::Unavailable => {
                self.gpu_state = GpuReadingState::Unavailable;
                None
            }
            GpuSampleOutcome::Failed => {
                self.gpu_state = match &self.gpu_state {
                    GpuReadingState::Available(reading, valid_at)
                    | GpuReadingState::Stale(reading, valid_at) => {
                        GpuReadingState::Stale(reading.clone(), *valid_at)
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

    pub fn record_process_sample(
        &mut self,
        observed_at: Duration,
        outcome: ProcessSampleOutcome,
        interaction_active: bool,
    ) {
        if !self.visible {
            return;
        }
        match outcome {
            ProcessSampleOutcome::Available(mut processes) => {
                normalize_process_rows(&mut processes);
                if interaction_active && !self.processes.is_empty() {
                    let started_at = self
                        .deferred_processes
                        .as_ref()
                        .map_or(observed_at, |pending| pending.started_at);
                    self.deferred_processes = Some(DeferredProcessRows {
                        started_at,
                        rows: processes,
                    });
                } else {
                    self.processes = processes;
                    self.process_status = ProcessListStatus::Available;
                    self.deferred_processes = None;
                }
            }
            ProcessSampleOutcome::Unavailable => {
                self.processes.clear();
                self.process_status = ProcessListStatus::Unavailable;
                self.deferred_processes = None;
            }
            ProcessSampleOutcome::Failed => {
                self.process_status = if self.processes.is_empty() {
                    ProcessListStatus::Failed
                } else {
                    ProcessListStatus::Stale
                };
                self.deferred_processes = None;
            }
            ProcessSampleOutcome::Cancelled => {}
        }
        self.apply_deferred_processes(observed_at, interaction_active);
    }

    pub fn apply_deferred_processes(&mut self, now: Duration, interaction_active: bool) {
        let should_apply = self.deferred_processes.as_ref().is_some_and(|pending| {
            !interaction_active
                || now.saturating_sub(pending.started_at) >= MAX_PROCESS_REORDER_DEFERRAL
        });
        if should_apply {
            let pending = self
                .deferred_processes
                .take()
                .expect("deferred rows were checked above");
            self.processes = pending.rows;
            self.process_status = ProcessListStatus::Available;
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
        let (memory, valid_at) = match self.memory_state {
            MemoryReadingState::Collecting => {
                let mut view_model = empty_view_model(
                    SystemUsageSection::Ram,
                    SystemUsageAccessibilityState::Collecting,
                    "Coletando a primeira leitura…",
                    &self.ram_history,
                );
                view_model.process_rows = self.process_rows();
                view_model.process_status = self.process_status;
                return view_model;
            }
            MemoryReadingState::Failed => {
                let mut view_model = empty_view_model(
                    SystemUsageSection::Ram,
                    SystemUsageAccessibilityState::Failed,
                    "Não foi possível ler a RAM.",
                    &self.ram_history,
                );
                view_model.process_rows = self.process_rows();
                view_model.process_status = self.process_status;
                return view_model;
            }
            MemoryReadingState::Available(reading, _) => (reading, None),
            MemoryReadingState::Stale(reading, valid_at) => (reading, Some(valid_at)),
        };
        let memory_stale = valid_at.is_some();
        let (symbol, pressure) = match memory.pressure {
            MemoryPressure::Normal => ("✓", "Pressão normal"),
            MemoryPressure::Warning => ("△", "Pressão em atenção"),
            MemoryPressure::Critical => ("!", "Pressão crítica"),
        };
        let history = self.ram_history.points.iter().copied().collect::<Vec<_>>();
        let memory_composition = memory_composition(memory);
        let memory_composition_accessibility_label = format!(
            "Composição da memória física: Apps {}; Reservada {}; Comprimida {}; Disponível {}.",
            format_bytes(memory.app_bytes),
            format_bytes(memory.wired_bytes),
            format_bytes(memory.compressed_bytes),
            format_bytes(memory.available_bytes),
        );
        SystemUsageViewModel {
            section: SystemUsageSection::Ram,
            accessibility_state: if memory_stale {
                SystemUsageAccessibilityState::Stale
            } else {
                SystemUsageAccessibilityState::MemoryPressure(memory.pressure)
            },
            primary_value: format!("{}% em uso", memory.percent.round() as u8),
            secondary_value: format!(
                "{} de {}",
                format_bytes(memory.used_bytes),
                format_bytes(memory.total_bytes)
            ),
            status: if memory_stale {
                format!(
                    "{symbol} {pressure} — atualização interrompida; última leitura {}",
                    elapsed_age(
                        self.latest_observed_at,
                        valid_at.expect("stale reading has time")
                    )
                )
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
            memory_composition,
            memory_composition_accessibility_label,
            history_accessibility_label: history_summary(SystemUsageSection::Ram, &history),
            history,
            process_rows: self.process_rows(),
            process_status: self.process_status,
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
                    memory,
                }
            })
            .collect()
    }

    fn gpu_view_model(&self) -> SystemUsageViewModel {
        match &self.gpu_state {
            GpuReadingState::Collecting => empty_view_model(
                SystemUsageSection::Gpu,
                SystemUsageAccessibilityState::Collecting,
                "Coletando a primeira leitura…",
                &self.gpu_history,
            ),
            GpuReadingState::Unavailable => empty_view_model(
                SystemUsageSection::Gpu,
                SystemUsageAccessibilityState::Unavailable,
                "O uso da GPU não está disponível neste Mac.",
                &self.gpu_history,
            ),
            GpuReadingState::Failed => empty_view_model(
                SystemUsageSection::Gpu,
                SystemUsageAccessibilityState::Failed,
                "Não foi possível ler esta métrica.",
                &self.gpu_history,
            ),
            GpuReadingState::Available(reading, _) => self.available_gpu_view_model(
                reading,
                "Atualizado agora",
                SystemUsageAccessibilityState::Available,
            ),
            GpuReadingState::Stale(reading, valid_at) => self.available_gpu_view_model(
                reading,
                &format!(
                    "Atualização interrompida; última leitura {}",
                    elapsed_age(self.latest_observed_at, *valid_at)
                ),
                SystemUsageAccessibilityState::Stale,
            ),
        }
    }

    fn available_gpu_view_model(
        &self,
        reading: &GpuReading,
        status: &str,
        accessibility_state: SystemUsageAccessibilityState,
    ) -> SystemUsageViewModel {
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
            accessibility_state,
            primary_value: format!("{}% de uso", reading.utilization_percent.round() as u8),
            secondary_value: reading.human_device_name().unwrap_or_default().to_owned(),
            status: status.to_owned(),
            details,
            memory_composition: Vec::new(),
            memory_composition_accessibility_label: String::new(),
            history_accessibility_label: history_summary(SystemUsageSection::Gpu, &history),
            history,
            process_rows: Vec::new(),
            process_status: ProcessListStatus::Collecting,
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

impl SystemUsageSamplingCoordinator {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn collect_if_visible<P: SystemUsageSamplingPorts>(
        &mut self,
        now: Duration,
        visible: bool,
        ports: &mut P,
    ) -> Option<GpuSampleOutcome> {
        self.set_visible(now, visible);
        if !visible {
            return None;
        }

        let gpu = ports.sample_gpu();
        if self.process_in_flight.is_none() && self.process_schedule.take_due(now) {
            let cancellation = ProcessSampleCancellation::new();
            if ports.request_process_sample(self.generation, cancellation.clone()) {
                self.process_in_flight = Some(self.generation);
                self.process_cancellation = Some(cancellation);
            }
        }
        Some(gpu)
    }

    pub fn set_visible(&mut self, now: Duration, visible: bool) {
        if visible != self.visible {
            self.visible = visible;
            self.generation = self.generation.wrapping_add(1);
            self.process_schedule.set_visible(visible, now);
            if !visible {
                if let Some(cancellation) = &self.process_cancellation {
                    cancellation.cancel();
                }
            }
        }
    }

    pub fn accept_process_sample(&mut self, generation: u64) -> bool {
        if self.process_in_flight != Some(generation) {
            return false;
        }
        self.process_in_flight = None;
        self.process_cancellation = None;
        self.visible && generation == self.generation
    }

    pub fn record_processes_if_current(
        &mut self,
        completion: ProcessSampleCompletion,
        model: &mut SystemUsageModel,
    ) -> bool {
        self.set_visible(completion.observed_at, completion.live_visible);
        if !self.accept_process_sample(completion.generation)
            || completion.request_visibility_generation != completion.live_visibility_generation
        {
            return false;
        }
        model.record_process_sample(
            completion.observed_at,
            completion.outcome,
            completion.interaction_active,
        );
        true
    }
}

impl Default for SystemUsageModel {
    fn default() -> Self {
        Self {
            visible: false,
            memory_state: MemoryReadingState::Collecting,
            ram_history: UsageHistory::new(),
            gpu_state: GpuReadingState::Collecting,
            gpu_history: UsageHistory::new(),
            processes: Vec::new(),
            process_status: ProcessListStatus::Collecting,
            deferred_processes: None,
            latest_observed_at: Duration::ZERO,
        }
    }
}

fn empty_view_model(
    section: SystemUsageSection,
    accessibility_state: SystemUsageAccessibilityState,
    status: &str,
    history: &UsageHistory,
) -> SystemUsageViewModel {
    let points = history.points.iter().copied().collect::<Vec<_>>();
    SystemUsageViewModel {
        section,
        accessibility_state,
        primary_value: "—".to_owned(),
        secondary_value: String::new(),
        status: status.to_owned(),
        details: Vec::new(),
        memory_composition: Vec::new(),
        memory_composition_accessibility_label: String::new(),
        history_accessibility_label: history_summary(section, &points),
        history: points,
        process_rows: Vec::new(),
        process_status: ProcessListStatus::Collecting,
    }
}

fn history_summary(section: SystemUsageSection, history: &[UsagePoint]) -> String {
    let metric = match section {
        SystemUsageSection::Ram => "RAM",
        SystemUsageSection::Gpu => "GPU",
    };
    let values = history
        .iter()
        .filter_map(|point| point.value)
        .collect::<Vec<_>>();
    let gaps = history.iter().filter(|point| point.value.is_none()).count();
    let gap_summary = match gaps {
        0 => String::new(),
        1 => "; 1 lacuna".to_owned(),
        count => format!("; {count} lacunas"),
    };
    if values.is_empty() && gaps > 0 {
        return format!(
            "Histórico de uso de {metric}, últimos 5 minutos. Leitura atual indisponível; nenhuma leitura válida{gap_summary}."
        );
    }
    if values.len() < 2 && gaps == 0 {
        return "O histórico aparecerá após duas leituras.".to_owned();
    }
    let last_valid = values.last().copied().unwrap_or_default();
    let minimum = values.iter().copied().fold(f64::INFINITY, f64::min);
    let maximum = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    if history.last().is_some_and(|point| point.value.is_some()) {
        format!(
            "Histórico de uso de {metric}, últimos 5 minutos. Atual {}%, mínimo {}%, máximo {}%{gap_summary}.",
            last_valid.round() as u8,
            minimum.round() as u8,
            maximum.round() as u8
        )
    } else {
        format!(
            "Histórico de uso de {metric}, últimos 5 minutos. Leitura atual indisponível; última válida {}%, mínimo {}%, máximo {}%{gap_summary}.",
            last_valid.round() as u8,
            minimum.round() as u8,
            maximum.round() as u8
        )
    }
}

fn detail(label: &'static str, bytes: u64) -> StatsDetailRow {
    StatsDetailRow {
        label,
        value: format_bytes(bytes),
    }
}

fn memory_composition(memory: MemoryReading) -> Vec<MemoryCompositionSegment> {
    let denominator = memory.total_bytes.max(1) as f64;
    [
        ("Apps", memory.app_bytes),
        ("Reservada", memory.wired_bytes),
        ("Comprimida", memory.compressed_bytes),
        ("Disponível", memory.available_bytes),
    ]
    .into_iter()
    .map(|(label, bytes)| MemoryCompositionSegment {
        label,
        fraction: bytes as f64 / denominator,
    })
    .collect()
}

fn normalize_process_rows(processes: &mut Vec<ProcessMemory>) {
    processes.sort_by(|left, right| {
        right
            .memory_bytes
            .cmp(&left.memory_bytes)
            .then_with(|| left.pid.cmp(&right.pid))
    });
    processes.truncate(20);
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

fn elapsed_age(now: Duration, observed_at: Duration) -> String {
    let seconds = now.saturating_sub(observed_at).as_secs();
    match seconds {
        1 => "há 1 s".to_owned(),
        seconds => format!("há {seconds} s"),
    }
}
