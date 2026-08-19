use objc2_foundation::{NSBundle, NSString};
use statlet::core::AppEvent;
use statlet::icon_assets::PreparedPngAsset;
use statlet::indicator_preferences::MetricKind;
use statlet::mole::MoleDetection;
use statlet::runtime_profile::BundleProfileMetadata;
use statlet::system_usage::{ProcessSampleFinished, SystemUsageSection};

pub mod environment;
pub mod fonts;
pub mod gpu;
pub mod notifications;
pub mod renderer;
pub mod sampler;
pub mod windows;

pub fn bundle_profile_metadata() -> BundleProfileMetadata {
    let bundle = NSBundle::mainBundle();
    BundleProfileMetadata {
        bundle_identifier: bundle.bundleIdentifier().map(|value| value.to_string()),
        runtime_profile: bundle_string(&bundle, "StatletRuntimeProfile"),
        dev_instance_id: bundle_string(&bundle, "StatletDevInstanceID"),
        dev_display_name: bundle_string(&bundle, "StatletDevDisplayName"),
        dev_short_marker: bundle_string(&bundle, "StatletDevShortMarker"),
    }
}

fn bundle_string(bundle: &NSBundle, key: &str) -> Option<String> {
    bundle
        .objectForInfoDictionaryKey(&NSString::from_str(key))?
        .downcast::<NSString>()
        .ok()
        .map(|value| value.to_string())
}

#[derive(Debug)]
pub enum RuntimeEvent {
    App(AppEvent),
    CloseKeyWindow,
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
    ProcessesSampled(ProcessSampleFinished),
    SystemUsageSurfaceChanged,
    SystemUsageSectionSelectedByUser(SystemUsageSection),
}
