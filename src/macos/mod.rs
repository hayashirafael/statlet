use statlet::core::AppEvent;
use statlet::mole::MoleDetection;

pub mod environment;
pub mod fonts;
pub mod notifications;
pub mod renderer;
pub mod sampler;
pub mod windows;

#[derive(Clone, Debug)]
pub enum RuntimeEvent {
    App(AppEvent),
    VisualEnvironmentChanged,
    FontSetChanged,
    ScreenParametersChanged,
    MoleDetected {
        generation: u64,
        detection: MoleDetection,
    },
}
