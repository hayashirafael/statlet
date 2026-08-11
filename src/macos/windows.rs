use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{define_class, msg_send, sel, DefinedClass, MainThreadOnly};
use objc2_app_kit::{
    NSApplication, NSBackingStoreType, NSButton, NSControlStateValueOn, NSPopUpButton, NSTextField,
    NSWindow, NSWindowStyleMask,
};
use objc2_foundation::{
    ns_string, MainThreadMarker, NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize,
};
use statlet::core::{AppEvent, Preferences, WarningThreshold, WindowKind};
use tao::event_loop::EventLoopProxy;

#[derive(Clone, Copy, Debug)]
pub struct RuntimeEvent(pub AppEvent);

struct ControlTargetIvars {
    proxy: EventLoopProxy<RuntimeEvent>,
}

define_class!(
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[ivars = ControlTargetIvars]
    struct ControlTarget;

    unsafe impl NSObjectProtocol for ControlTarget {}

    impl ControlTarget {
        #[unsafe(method(toggleMoleIntegration:))]
        fn toggle_mole_integration(&self, sender: &NSButton) {
            let enabled = sender.state() == NSControlStateValueOn;
            let _ = self
                .ivars()
                .proxy
                .send_event(RuntimeEvent(AppEvent::SetMoleIntegrationEnabled(enabled)));
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
                .send_event(RuntimeEvent(AppEvent::SetWarningThreshold(threshold)));
        }
    }
);

impl ControlTarget {
    fn new(mtm: MainThreadMarker, proxy: EventLoopProxy<RuntimeEvent>) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(ControlTargetIvars { proxy });
        unsafe { msg_send![super(this), init] }
    }
}

pub struct WindowManager {
    control_target: Retained<ControlTarget>,
    preferences: Option<PreferencesWindow>,
    history: Option<Retained<NSWindow>>,
}

struct PreferencesWindow {
    window: Retained<NSWindow>,
    mole_checkbox: Retained<NSButton>,
    warning_threshold: Retained<NSPopUpButton>,
}

impl WindowManager {
    pub fn new(mtm: MainThreadMarker, proxy: EventLoopProxy<RuntimeEvent>) -> Self {
        let control_target = ControlTarget::new(mtm, proxy);
        Self {
            control_target,
            preferences: None,
            history: None,
        }
    }

    pub fn show(&mut self, kind: WindowKind, preferences: Preferences) {
        let mtm = MainThreadMarker::new().expect("native window actions run on the main thread");
        let window = match kind {
            WindowKind::Preferences => {
                if self.preferences.is_none() {
                    self.preferences = Some(create_preferences_window(mtm, &self.control_target));
                }
                self.update_preferences(preferences);
                &self
                    .preferences
                    .as_ref()
                    .expect("preferences window was created")
                    .window
            }
            WindowKind::History => self
                .history
                .get_or_insert_with(|| create_history_window(mtm)),
        };

        window.makeKeyAndOrderFront(None);
        NSApplication::sharedApplication(mtm).activate();
    }

    pub fn update_preferences(&self, preferences: Preferences) {
        let Some(window) = &self.preferences else {
            return;
        };
        window.apply(preferences);
    }
}

impl PreferencesWindow {
    fn apply(&self, preferences: Preferences) {
        self.mole_checkbox
            .setState(if preferences.mole_integration_enabled {
                NSControlStateValueOn
            } else {
                0
            });
        self.warning_threshold
            .selectItemWithTitle(&threshold_title(preferences.warning_threshold));
        self.warning_threshold
            .setEnabled(preferences.mole_integration_enabled);
    }
}

fn create_preferences_window(mtm: MainThreadMarker, target: &ControlTarget) -> PreferencesWindow {
    let window = create_window(mtm, "Preferências do Statlet", NSSize::new(480.0, 238.0));
    let content = window
        .contentView()
        .expect("preferences window content view");

    let heading = NSTextField::labelWithString(ns_string!("Disco e Mole"), mtm);
    heading.setFont(Some(&objc2_app_kit::NSFont::boldSystemFontOfSize(15.0)));
    heading.setFrame(NSRect::new(
        NSPoint::new(24.0, 184.0),
        NSSize::new(420.0, 24.0),
    ));

    let checkbox = unsafe {
        NSButton::checkboxWithTitle_target_action(
            ns_string!("Monitorar o disco com a integração do Mole"),
            Some(target as &AnyObject),
            Some(sel!(toggleMoleIntegration:)),
            mtm,
        )
    };
    checkbox.setFrame(NSRect::new(
        NSPoint::new(24.0, 144.0),
        NSSize::new(410.0, 24.0),
    ));

    let explanation = NSTextField::labelWithString(
        ns_string!(
            "O Statlet apenas avisa quando o limite for mantido.\nA limpeza é feita fora do app."
        ),
        mtm,
    );
    explanation.setTextColor(Some(&objc2_app_kit::NSColor::secondaryLabelColor()));
    explanation.setMaximumNumberOfLines(2);
    explanation.setFrame(NSRect::new(
        NSPoint::new(44.0, 102.0),
        NSSize::new(410.0, 38.0),
    ));

    let threshold_label = NSTextField::labelWithString(ns_string!("Avisar a partir de"), mtm);
    threshold_label.setFrame(NSRect::new(
        NSPoint::new(44.0, 70.0),
        NSSize::new(180.0, 24.0),
    ));

    let threshold = unsafe {
        let popup = NSPopUpButton::initWithFrame_pullsDown(
            NSPopUpButton::alloc(mtm),
            NSRect::new(NSPoint::new(245.0, 66.0), NSSize::new(110.0, 30.0)),
            false,
        );
        popup.setTarget(Some(target as &AnyObject));
        popup.setAction(Some(sel!(changeWarningThreshold:)));
        popup
    };
    for value in [70, 75, 80, 85, 90, 95] {
        threshold.addItemWithTitle(&threshold_title(
            WarningThreshold::try_from(value).expect("known threshold"),
        ));
    }

    content.addSubview(&heading);
    content.addSubview(&checkbox);
    content.addSubview(&explanation);
    content.addSubview(&threshold_label);
    content.addSubview(&threshold);

    PreferencesWindow {
        window,
        mole_checkbox: checkbox,
        warning_threshold: threshold,
    }
}

fn create_history_window(mtm: MainThreadMarker) -> Retained<NSWindow> {
    let window = create_window(mtm, "Histórico do Statlet", NSSize::new(520.0, 310.0));
    let content = window.contentView().expect("history window content view");

    let heading = NSTextField::labelWithString(ns_string!("Nenhum alerta ainda"), mtm);
    heading.setFont(Some(&objc2_app_kit::NSFont::boldSystemFontOfSize(17.0)));
    heading.setFrame(NSRect::new(
        NSPoint::new(24.0, 238.0),
        NSSize::new(460.0, 28.0),
    ));

    let explanation = NSTextField::labelWithString(
        ns_string!("Os alertas de uso sustentado do disco aparecerão aqui quando a integração estiver ativa."),
        mtm,
    );
    explanation.setTextColor(Some(&objc2_app_kit::NSColor::secondaryLabelColor()));
    explanation.setFrame(NSRect::new(
        NSPoint::new(24.0, 204.0),
        NSSize::new(470.0, 24.0),
    ));

    content.addSubview(&heading);
    content.addSubview(&explanation);
    window
}

fn create_window(mtm: MainThreadMarker, title: &str, size: NSSize) -> Retained<NSWindow> {
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
    window.setTitle(&objc2_foundation::NSString::from_str(title));
    window.center();
    window
}

fn threshold_title(threshold: WarningThreshold) -> Retained<objc2_foundation::NSString> {
    objc2_foundation::NSString::from_str(&format!("{}%", threshold.get()))
}
