use objc2::rc::Retained;
use objc2::{define_class, msg_send, DefinedClass, MainThreadOnly};
use objc2_app_kit::{
    NSAlert, NSAlertFirstButtonReturn, NSAlertStyle, NSBackingStoreType, NSButton,
    NSControlStateValueOn, NSPopUpButton, NSTextField, NSWindow, NSWindowCollectionBehavior,
    NSWindowStyleMask,
};
use objc2_foundation::{
    ns_string, MainThreadMarker, NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize,
};
use statlet::core::{AppEvent, WarningThreshold};
use tao::event_loop::EventLoopProxy;

use super::super::RuntimeEvent;

pub(super) struct ControlTargetIvars {
    proxy: EventLoopProxy<RuntimeEvent>,
}

define_class!(
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[ivars = ControlTargetIvars]
    pub(super) struct ControlTarget;

    unsafe impl NSObjectProtocol for ControlTarget {}

    impl ControlTarget {
        #[unsafe(method(toggleMoleIntegration:))]
        fn toggle_mole_integration(&self, sender: &NSButton) {
            let enabled = sender.state() == NSControlStateValueOn;
            let _ = self
                .ivars()
                .proxy
                .send_event(RuntimeEvent::App(AppEvent::SetMoleIntegrationEnabled(
                    enabled,
                )));
        }

        #[unsafe(method(changeWarningThreshold:))]
        fn change_warning_threshold(&self, sender: &NSPopUpButton) {
            let Some(title) = sender.titleOfSelectedItem() else {
                return;
            };
            let Ok(value) = title.to_string().trim_end_matches('%').parse::<u8>() else {
                return;
            };
            let Ok(threshold) = WarningThreshold::try_from(value) else {
                return;
            };
            let _ = self
                .ivars()
                .proxy
                .send_event(RuntimeEvent::App(AppEvent::SetWarningThreshold(threshold)));
        }

        #[unsafe(method(openMoleInTerminal:))]
        fn open_mole_in_terminal(&self, _sender: &NSButton) {
            let _ = self
                .ivars()
                .proxy
                .send_event(RuntimeEvent::App(AppEvent::OpenMoleInTerminal));
        }

        #[unsafe(method(clearHistory:))]
        fn clear_history(&self, _sender: &NSButton) {
            let mtm = MainThreadMarker::new().expect("history actions run on the main thread");
            let alert = NSAlert::new(mtm);
            alert.setAlertStyle(NSAlertStyle::Warning);
            alert.setMessageText(ns_string!("Apagar todo o histórico?"));
            alert.setInformativeText(ns_string!(
                "Esta ação remove os registros locais do Statlet e não pode ser desfeita."
            ));
            alert.addButtonWithTitle(ns_string!("Apagar histórico"));
            alert.addButtonWithTitle(ns_string!("Cancelar"));
            if alert.runModal() == NSAlertFirstButtonReturn {
                let _ = self
                    .ivars()
                    .proxy
                    .send_event(RuntimeEvent::App(AppEvent::ClearHistoryConfirmed));
            }
        }
    }
);

impl ControlTarget {
    pub(super) fn new(
        mtm: MainThreadMarker,
        proxy: EventLoopProxy<RuntimeEvent>,
    ) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(ControlTargetIvars { proxy });
        unsafe { msg_send![super(this), init] }
    }
}

pub(super) fn create_window(
    mtm: MainThreadMarker,
    title: &str,
    size: NSSize,
) -> Retained<NSWindow> {
    let window = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            NSRect::new(NSPoint::new(0.0, 0.0), size),
            NSWindowStyleMask::Titled | NSWindowStyleMask::Closable,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    unsafe { window.setReleasedWhenClosed(false) };
    window.setCollectionBehavior(NSWindowCollectionBehavior::MoveToActiveSpace);
    window.setTitle(&objc2_foundation::NSString::from_str(title));
    window.center();
    window
}

pub(super) fn text_label(
    mtm: MainThreadMarker,
    text: &str,
    frame: NSRect,
) -> Retained<NSTextField> {
    let label = NSTextField::labelWithString(&objc2_foundation::NSString::from_str(text), mtm);
    label.setFrame(frame);
    label
}

pub(super) fn threshold_title(threshold: WarningThreshold) -> Retained<objc2_foundation::NSString> {
    objc2_foundation::NSString::from_str(&format!("{}%", threshold.get()))
}
