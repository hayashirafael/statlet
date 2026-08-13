use statlet::core::AppEvent;
use statlet::icon_assets::PreparedPngAsset;
use statlet::indicator_preferences::MetricKind;
use statlet::mole::MoleDetection;

pub mod environment;
pub mod fonts;
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
}
