use std::cell::Cell;
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc,
};

use objc2::rc::Retained;
use objc2::{define_class, msg_send, DefinedClass, MainThreadOnly};
use objc2_app_kit::{
    NSAlert, NSAlertFirstButtonReturn, NSAlertStyle, NSBackingStoreType, NSButton,
    NSControlStateValueOn, NSEvent, NSEventModifierFlags, NSPopUpButton, NSSegmentedControl,
    NSTextField, NSWindow, NSWindowCollectionBehavior, NSWindowDelegate, NSWindowStyleMask,
};
use objc2_foundation::{
    ns_string, MainThreadMarker, NSNotification, NSObject, NSObjectProtocol, NSPoint, NSRect,
    NSSize,
};
use statlet::core::{AppEvent, WarningThreshold};
use statlet::system_usage::SystemUsageSection;
use tao::event_loop::EventLoopProxy;

use super::super::RuntimeEvent;

pub(super) struct ControlTargetIvars {
    proxy: EventLoopProxy<RuntimeEvent>,
    system_usage_visible: Arc<AtomicBool>,
    system_usage_generation: Arc<AtomicU64>,
}

define_class!(
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[ivars = ControlTargetIvars]
    pub(super) struct ControlTarget;

    unsafe impl NSObjectProtocol for ControlTarget {}

    unsafe impl NSWindowDelegate for ControlTarget {
        #[unsafe(method(windowDidBecomeKey:))]
        fn system_usage_became_key(&self, _notification: &NSNotification) {
            self.update_system_usage_visibility(true);
        }

        #[unsafe(method(windowWillClose:))]
        fn system_usage_will_close(&self, _notification: &NSNotification) {
            self.update_system_usage_visibility(false);
        }

        #[unsafe(method(windowDidMiniaturize:))]
        fn system_usage_did_miniaturize(&self, _notification: &NSNotification) {
            self.update_system_usage_visibility(false);
        }

        #[unsafe(method(windowDidDeminiaturize:))]
        fn system_usage_did_deminiaturize(&self, _notification: &NSNotification) {
            self.update_system_usage_visibility(true);
        }
    }

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

        #[unsafe(method(changeSystemUsageSection:))]
        fn change_system_usage_section(&self, sender: &NSSegmentedControl) {
            let section = if sender.selectedSegment() == 1 {
                SystemUsageSection::Gpu
            } else {
                SystemUsageSection::Ram
            };
            let _ = self
                .ivars()
                .proxy
                .send_event(RuntimeEvent::SystemUsageSectionSelectedByUser(section));
        }

        #[unsafe(method(selectSystemUsageRam:))]
        fn select_system_usage_ram(&self, _sender: &NSButton) {
            let _ = self.ivars().proxy.send_event(
                RuntimeEvent::SystemUsageSectionSelectedByUser(SystemUsageSection::Ram),
            );
        }

        #[unsafe(method(selectSystemUsageGpu:))]
        fn select_system_usage_gpu(&self, _sender: &NSButton) {
            let _ = self.ivars().proxy.send_event(
                RuntimeEvent::SystemUsageSectionSelectedByUser(SystemUsageSection::Gpu),
            );
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

        #[unsafe(method(resetIndicator:))]
        fn reset_indicator(&self, _sender: &NSButton) {
            let mtm = MainThreadMarker::new().expect("preferences actions run on the main thread");
            let alert = NSAlert::new(mtm);
            alert.setAlertStyle(NSAlertStyle::Warning);
            alert.setMessageText(ns_string!("Restaurar o indicador aos padrões?"));
            alert.setInformativeText(&objc2_foundation::NSString::from_str(
                indicator_reset_confirmation(),
            ));
            let destructive = alert.addButtonWithTitle(ns_string!("Restaurar indicador"));
            destructive.setHasDestructiveAction(true);
            alert.addButtonWithTitle(ns_string!("Cancelar"));
            if alert.runModal() == NSAlertFirstButtonReturn {
                self.send_app_event(AppEvent::ResetIndicatorConfirmed);
            }
        }

        #[unsafe(method(undoIndicatorReset:))]
        fn undo_indicator_reset(&self, _sender: &NSButton) {
            self.send_app_event(AppEvent::UndoIndicatorReset);
        }

        #[unsafe(method(retrySavePreferences:))]
        fn retry_save_preferences(&self, _sender: &NSButton) {
            self.send_app_event(AppEvent::RetrySavePreferences);
        }
    }
);

impl ControlTarget {
    pub(super) fn new(
        mtm: MainThreadMarker,
        proxy: EventLoopProxy<RuntimeEvent>,
        system_usage_visible: Arc<AtomicBool>,
        system_usage_generation: Arc<AtomicU64>,
    ) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(ControlTargetIvars {
            proxy,
            system_usage_visible,
            system_usage_generation,
        });
        unsafe { msg_send![super(this), init] }
    }

    pub(super) fn event_proxy(&self) -> EventLoopProxy<RuntimeEvent> {
        self.ivars().proxy.clone()
    }

    fn send_app_event(&self, event: AppEvent) {
        let _ = self.ivars().proxy.send_event(RuntimeEvent::App(event));
    }

    pub(super) fn update_system_usage_visibility(&self, visible: bool) {
        let changed = self
            .ivars()
            .system_usage_visible
            .swap(visible, Ordering::AcqRel)
            != visible;
        if changed {
            self.ivars()
                .system_usage_generation
                .fetch_add(1, Ordering::AcqRel);
        }
        let _ = self
            .ivars()
            .proxy
            .send_event(RuntimeEvent::SystemUsageSurfaceChanged);
    }
}

pub(super) struct PreferencesWindowHostIvars {
    proxy: EventLoopProxy<RuntimeEvent>,
    can_undo_indicator_reset: Cell<bool>,
}

define_class!(
    #[unsafe(super = NSWindow)]
    #[thread_kind = MainThreadOnly]
    #[ivars = PreferencesWindowHostIvars]
    pub(super) struct PreferencesWindowHost;

    unsafe impl NSObjectProtocol for PreferencesWindowHost {}

    impl PreferencesWindowHost {
        #[unsafe(method(performKeyEquivalent:))]
        fn perform_key_equivalent(&self, event: &NSEvent) -> bool {
            let modifiers = event.modifierFlags() & NSEventModifierFlags::DeviceIndependentFlagsMask;
            let command_only = modifiers == NSEventModifierFlags::Command;
            let characters = event
                .charactersIgnoringModifiers()
                .map(|characters| characters.to_string())
                .unwrap_or_default();
            if should_intercept_indicator_undo(
                &characters,
                command_only,
                self.ivars().can_undo_indicator_reset.get(),
            ) {
                let _ = self
                    .ivars()
                    .proxy
                    .send_event(RuntimeEvent::App(AppEvent::UndoIndicatorReset));
                true
            } else {
                unsafe { msg_send![super(self), performKeyEquivalent: event] }
            }
        }
    }
);

impl PreferencesWindowHost {
    pub(super) fn new(
        mtm: MainThreadMarker,
        title: &str,
        size: NSSize,
        proxy: EventLoopProxy<RuntimeEvent>,
    ) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(PreferencesWindowHostIvars {
            proxy,
            can_undo_indicator_reset: Cell::new(false),
        });
        let window: Retained<Self> = unsafe {
            msg_send![
                super(this),
                initWithContentRect: NSRect::new(NSPoint::new(0.0, 0.0), size),
                styleMask: NSWindowStyleMask::Titled | NSWindowStyleMask::Closable,
                backing: NSBackingStoreType::Buffered,
                defer: false
            ]
        };
        configure_window(&window, title);
        window
    }

    pub(super) fn set_can_undo_indicator_reset(&self, can_undo: bool) {
        self.ivars().can_undo_indicator_reset.set(can_undo);
    }
}

pub(super) fn should_intercept_indicator_undo(
    characters_ignoring_modifiers: &str,
    command_only: bool,
    can_undo_indicator_reset: bool,
) -> bool {
    can_undo_indicator_reset && command_only && characters_ignoring_modifiers == "z"
}

fn indicator_reset_confirmation() -> &'static str {
    "As cores de CPU/RAM, os identificadores e os PNGs associados, os rótulos, a tipografia e o intervalo serão restaurados aos padrões. Disco e Mole não serão alterados."
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
    configure_window(&window, title);
    window
}

fn configure_window(window: &NSWindow, title: &str) {
    unsafe { window.setReleasedWhenClosed(false) };
    window.setCollectionBehavior(NSWindowCollectionBehavior::MoveToActiveSpace);
    window.setTitle(&objc2_foundation::NSString::from_str(title));
    window.center();
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

#[cfg(test)]
mod tests {
    use super::indicator_reset_confirmation;

    #[test]
    fn global_reset_confirmation_summarizes_every_indicator_group_and_safety_boundary() {
        let summary = indicator_reset_confirmation();

        for group in [
            "cores de CPU/RAM",
            "identificadores",
            "PNGs",
            "rótulos",
            "tipografia",
            "intervalo",
        ] {
            assert!(summary.contains(group), "missing {group} in {summary}");
        }
        assert!(summary.contains("Disco e Mole não serão alterados"));
    }
}
