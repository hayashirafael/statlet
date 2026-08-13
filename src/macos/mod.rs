use statlet::core::AppEvent;
use statlet::icon_assets::PreparedPngAsset;
use statlet::indicator_preferences::MetricKind;
use statlet::mole::MoleDetection;
use statlet::stats::{ProcessSampleOutcome, SystemUsageSection};

pub mod environment;
pub mod fonts;
pub mod gpu;
pub mod notifications;
pub mod renderer;
pub mod sampler;
pub mod windows;

#[derive(Debug)]
pub enum RuntimeEvent {
    App(AppEvent),
    MetricPngPrepared {
        metric: MetricKind,
        generation: u64,
        result: Result<PreparedPngAsset, String>,
    },
    VisualEnvironmentChanged,
    FontSetChanged,
    ScreenParametersChanged,
    MoleDetected {
        generation: u64,
        detection: MoleDetection,
    },
    ProcessesSampled {
        generation: u64,
        visibility_generation: u64,
        outcome: ProcessSampleOutcome,
    },
    SystemUsageVisibilityChanged(bool),
    SystemUsageSectionSelectedByUser(SystemUsageSection),
}
