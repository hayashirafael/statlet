//! PROTOTYPE — two-line image renderer.
//!
//! Derived from featherbar commit 90ab504, Apache-2.0.

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{AnyThread, MainThreadMarker, Message};
use objc2_app_kit::{
    NSApplication, NSAttributedStringNSStringDrawing, NSColor, NSFont, NSFontAttributeName,
    NSForegroundColorAttributeName, NSImage, NSStatusBarButton, NSView,
};
use objc2_foundation::{NSDictionary, NSMutableAttributedString, NSPoint, NSSize, NSString};

const FONT_SIZE: f64 = 12.0;
const LINE_GAP: f64 = 2.0;
const HEIGHT: f64 = 22.0;

#[derive(Clone, Copy)]
pub enum Level {
    Neutral,
    Good,
    Warn,
    Crit,
}

pub struct Seg {
    pub text: String,
    pub level: Level,
}

impl Seg {
    pub fn new(text: impl Into<String>, level: Level) -> Self {
        Self {
            text: text.into(),
            level,
        }
    }
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

fn color(level: Level) -> Retained<NSColor> {
    match level {
        Level::Neutral => NSColor::labelColor(),
        Level::Good => NSColor::systemGreenColor(),
        Level::Warn => NSColor::systemOrangeColor(),
        Level::Crit => NSColor::systemRedColor(),
    }
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
        let attributes = [Level::Neutral, Level::Good, Level::Warn, Level::Crit].map(|level| {
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

    fn attributed_line(&self, segments: &[Seg]) -> Retained<NSMutableAttributedString> {
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

    pub fn set_title(&self, button: &NSStatusBarButton, top: &[Seg], bottom: &[Seg]) {
        let top = self.attributed_line(top);
        let bottom = self.attributed_line(bottom);
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
    }
}
