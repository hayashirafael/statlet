use statlet::core::AppEvent;
use statlet::mole::MoleDetection;
use statlet::stats::ProcessSampleOutcome;

pub mod gpu;
pub mod notifications;
pub mod renderer;
pub mod sampler;
pub mod windows;

#[derive(Clone, Debug)]
pub enum RuntimeEvent {
    App(AppEvent),
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
}
