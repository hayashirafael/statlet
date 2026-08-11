//! Two-line status-item renderer.
//!
//! Derived and modified from featherbar commit 90ab504, Apache-2.0:
//! https://github.com/nim444/featherbar/tree/90ab504b025db15665ce5d97b8ae4d4cdeb47dc3

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{AnyThread, MainThreadMarker, Message};
use objc2_app_kit::{
    NSAccessibility, NSApplication, NSAttributedStringNSStringDrawing, NSColor, NSFont,
    NSFontAttributeName, NSForegroundColorAttributeName, NSImage, NSStatusBarButton, NSView,
};
use objc2_foundation::{NSDictionary, NSMutableAttributedString, NSPoint, NSSize, NSString};

use statlet::core::{MetricPresentation, MetricSeverity, StatusPresentation};

const FONT_SIZE: f64 = 12.0;
const LINE_GAP: f64 = 2.0;
const HEIGHT: f64 = 22.0;

#[derive(Clone, Copy)]
enum Level {
    Neutral,
    Good,
    Warning,
    Critical,
}

struct Segment {
    text: String,
    level: Level,
}

pub struct Renderer {
    attributes: [Retained<NSDictionary<NSString, AnyObject>>; 4],
    top_y: f64,
    bottom_y: f64,
}

impl Renderer {
    pub fn new() -> Self {
        let font = NSFont::monospacedSystemFontOfSize_weight(FONT_SIZE, unsafe {
            objc2_app_kit::NSFontWeightMedium
        });
        let attributes =
            [Level::Neutral, Level::Good, Level::Warning, Level::Critical].map(|level| {
                NSDictionary::from_retained_objects(
                    &[unsafe { NSFontAttributeName }, unsafe {
                        NSForegroundColorAttributeName
                    }],
                    &[
                        Retained::into_super(Retained::into_super(font.retain())),
                        Retained::into_super(Retained::into_super(color(level))),
                    ],
                )
            });

        let cap_height = font.capHeight();
        let descent = -font.descender();
        let margin = (HEIGHT - 2.0 * cap_height - LINE_GAP) / 2.0;
        Self {
            attributes,
            bottom_y: margin - descent,
            top_y: margin + cap_height + LINE_GAP - descent,
        }
    }

    pub fn set_status(&self, button: &NSStatusBarButton, status: &StatusPresentation) {
        let top = self.attributed_line(&segments(&status.top));
        let bottom = self.attributed_line(&segments(&status.bottom));
        let width = top.size().width.max(bottom.size().width).ceil();
        let image = NSImage::initWithSize(
            NSImage::alloc(),
            NSSize {
                width,
                height: HEIGHT,
            },
        );

        #[allow(deprecated)]
        {
            image.lockFocus();
            bottom.drawAtPoint(NSPoint {
                x: 0.0,
                y: self.bottom_y,
            });
            top.drawAtPoint(NSPoint {
                x: 0.0,
                y: self.top_y,
            });
            image.unlockFocus();
        }

        button.setImage(Some(&image));
        button.setAccessibilityLabel(Some(&NSString::from_str(&status.accessibility_label)));
        button.setToolTip(Some(&NSString::from_str(&status.accessibility_label)));
    }

    fn attributed_line(&self, segments: &[Segment]) -> Retained<NSMutableAttributedString> {
        let line = NSMutableAttributedString::new();
        for segment in segments {
            let run = unsafe {
                objc2_foundation::NSAttributedString::new_with_attributes(
                    &NSString::from_str(&segment.text),
                    &self.attributes[segment.level as usize],
                )
            };
            line.appendAttributedString(&run);
        }
        line
    }
}

pub fn status_button(marker: MainThreadMarker) -> Option<Retained<NSStatusBarButton>> {
    let app = NSApplication::sharedApplication(marker);
    for window in app.windows() {
        if window.class().name().to_bytes() == b"NSStatusBarWindow" {
            if let Some(view) = window.contentView() {
                if let Some(button) = find_button(&view) {
                    return Some(button);
                }
            }
        }
    }
    None
}

fn find_button(view: &NSView) -> Option<Retained<NSStatusBarButton>> {
    if let Ok(button) = view.retain().downcast::<NSStatusBarButton>() {
        return Some(button);
    }
    for subview in view.subviews() {
        if let Some(button) = find_button(&subview) {
            return Some(button);
        }
    }
    None
}

fn segments(metric: &MetricPresentation) -> [Segment; 2] {
    [
        Segment {
            text: metric.label.to_owned(),
            level: Level::Neutral,
        },
        Segment {
            text: metric.value.clone(),
            level: match metric.severity {
                MetricSeverity::Good => Level::Good,
                MetricSeverity::Warning => Level::Warning,
                MetricSeverity::Critical => Level::Critical,
            },
        },
    ]
}

fn color(level: Level) -> Retained<NSColor> {
    match level {
        Level::Neutral => NSColor::labelColor(),
        Level::Good => NSColor::systemGreenColor(),
        Level::Warning => NSColor::systemOrangeColor(),
        Level::Critical => NSColor::systemRedColor(),
    }
}
