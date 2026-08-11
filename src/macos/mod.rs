use statlet::core::AppEvent;
use statlet::mole::MoleDetection;

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
}
