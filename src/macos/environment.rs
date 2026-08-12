use std::ffi::c_void;

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{
    define_class, msg_send, ClassType, DefinedClass, MainThreadMarker, MainThreadOnly, Message,
};
use objc2_app_kit::{
    NSAppearanceCustomization, NSApplication, NSApplicationDidChangeScreenParametersNotification,
    NSFontSetChangedNotification, NSStatusBarButton, NSSystemColorsDidChangeNotification,
    NSWorkspace, NSWorkspaceAccessibilityDisplayOptionsDidChangeNotification,
};
use objc2_foundation::{
    ns_string, NSDictionary, NSKeyValueChangeKey, NSKeyValueObservingOptions, NSNotification,
    NSNotificationCenter, NSObject, NSObjectNSKeyValueObserverRegistration, NSObjectProtocol,
    NSString,
};
use statlet::indicator_preferences::IndicatorAppearance;
use tao::event_loop::EventLoopProxy;

use super::RuntimeEvent;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VisualEnvironment {
    pub appearance: IndicatorAppearance,
    pub increase_contrast: bool,
    pub differentiate_without_color: bool,
    pub reduce_transparency: bool,
}

impl VisualEnvironment {
    pub fn current(button: Option<&NSStatusBarButton>, marker: MainThreadMarker) -> Self {
        let appearance = button.map_or_else(
            || NSApplication::sharedApplication(marker).effectiveAppearance(),
            |button| button.effectiveAppearance(),
        );
        let appearance = if appearance.name().to_string().contains("Dark") {
            IndicatorAppearance::Dark
        } else {
            IndicatorAppearance::Light
        };
        let workspace = NSWorkspace::sharedWorkspace();
        Self {
            appearance,
            increase_contrast: workspace.accessibilityDisplayShouldIncreaseContrast(),
            differentiate_without_color: workspace
                .accessibilityDisplayShouldDifferentiateWithoutColor(),
            reduce_transparency: workspace.accessibilityDisplayShouldReduceTransparency(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum VisualEnvironmentSignal {
    Appearance,
    Accessibility,
    SystemColors,
    Fonts,
    Screens,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NotificationCenterKind {
    Default,
    Workspace,
}

fn notification_center_for(signal: VisualEnvironmentSignal) -> NotificationCenterKind {
    match signal {
        VisualEnvironmentSignal::Accessibility => NotificationCenterKind::Workspace,
        VisualEnvironmentSignal::Appearance
        | VisualEnvironmentSignal::SystemColors
        | VisualEnvironmentSignal::Fonts
        | VisualEnvironmentSignal::Screens => NotificationCenterKind::Default,
    }
}

trait VisualEventSink {
    fn enqueue(&self, event: RuntimeEvent);
}

impl VisualEventSink for EventLoopProxy<RuntimeEvent> {
    fn enqueue(&self, event: RuntimeEvent) {
        let _ = self.send_event(event);
    }
}

fn enqueue_visual_event(sink: &impl VisualEventSink, signal: VisualEnvironmentSignal) {
    let event = match signal {
        VisualEnvironmentSignal::Appearance
        | VisualEnvironmentSignal::Accessibility
        | VisualEnvironmentSignal::SystemColors => RuntimeEvent::VisualEnvironmentChanged,
        VisualEnvironmentSignal::Fonts => RuntimeEvent::FontSetChanged,
        VisualEnvironmentSignal::Screens => RuntimeEvent::ScreenParametersChanged,
    };
    sink.enqueue(event);
}

trait ObservationBackend {
    type Button: ?Sized;
    type ButtonToken;
    type NotificationToken;

    fn observe_notification(&mut self, signal: VisualEnvironmentSignal) -> Self::NotificationToken;
    fn observe_button(&mut self, button: &Self::Button) -> Self::ButtonToken;
}

struct ObserverTokens<B: ObservationBackend> {
    backend: B,
    _notification_tokens: Vec<B::NotificationToken>,
    button_observer: Option<B::ButtonToken>,
}

impl<B: ObservationBackend> ObserverTokens<B> {
    fn new(mut backend: B) -> Self {
        let notification_tokens = [
            VisualEnvironmentSignal::Accessibility,
            VisualEnvironmentSignal::SystemColors,
            VisualEnvironmentSignal::Fonts,
            VisualEnvironmentSignal::Screens,
        ]
        .into_iter()
        .map(|signal| backend.observe_notification(signal))
        .collect();
        Self {
            backend,
            _notification_tokens: notification_tokens,
            button_observer: None,
        }
    }

    fn rebind_button(&mut self, button: Option<&B::Button>) {
        drop(self.button_observer.take());
        self.button_observer = button.map(|button| self.backend.observe_button(button));
    }
}

struct EnvironmentEventTargetIvars {
    proxy: EventLoopProxy<RuntimeEvent>,
    signal: VisualEnvironmentSignal,
}

define_class!(
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[ivars = EnvironmentEventTargetIvars]
    struct EnvironmentEventTarget;

    unsafe impl NSObjectProtocol for EnvironmentEventTarget {}

    impl EnvironmentEventTarget {
        #[unsafe(method(environmentChanged:))]
        fn environment_changed(&self, _notification: &NSNotification) {
            enqueue_visual_event(&self.ivars().proxy, self.ivars().signal);
        }

        #[unsafe(method(observeValueForKeyPath:ofObject:change:context:))]
        unsafe fn observe_value(
            &self,
            _key_path: Option<&NSString>,
            _object: Option<&AnyObject>,
            _change: Option<&NSDictionary<NSKeyValueChangeKey, AnyObject>>,
            _context: *mut c_void,
        ) {
            enqueue_visual_event(&self.ivars().proxy, self.ivars().signal);
        }
    }
);

impl EnvironmentEventTarget {
    fn new(
        marker: MainThreadMarker,
        proxy: EventLoopProxy<RuntimeEvent>,
        signal: VisualEnvironmentSignal,
    ) -> Retained<Self> {
        let this = Self::alloc(marker).set_ivars(EnvironmentEventTargetIvars { proxy, signal });
        unsafe { msg_send![super(this), init] }
    }
}

struct NativeNotificationToken {
    center: Retained<NSNotificationCenter>,
    target: Retained<EnvironmentEventTarget>,
}

impl Drop for NativeNotificationToken {
    fn drop(&mut self) {
        unsafe { self.center.removeObserver(&self.target) };
    }
}

struct NativeButtonToken {
    button: Retained<NSStatusBarButton>,
    target: Retained<EnvironmentEventTarget>,
}

fn button_object(button: &NSStatusBarButton) -> &NSObject {
    button
        .as_super()
        .as_super()
        .as_super()
        .as_super()
        .as_super()
}

impl Drop for NativeButtonToken {
    fn drop(&mut self) {
        unsafe {
            button_object(&self.button).removeObserver_forKeyPath(
                self.target.as_super(),
                ns_string!("effectiveAppearance"),
            )
        };
    }
}

struct NativeObservationBackend {
    marker: MainThreadMarker,
    proxy: EventLoopProxy<RuntimeEvent>,
}

impl ObservationBackend for NativeObservationBackend {
    type Button = NSStatusBarButton;
    type ButtonToken = NativeButtonToken;
    type NotificationToken = NativeNotificationToken;

    fn observe_notification(&mut self, signal: VisualEnvironmentSignal) -> Self::NotificationToken {
        let name = match signal {
            VisualEnvironmentSignal::Accessibility => unsafe {
                NSWorkspaceAccessibilityDisplayOptionsDidChangeNotification
            },
            VisualEnvironmentSignal::SystemColors => unsafe { NSSystemColorsDidChangeNotification },
            VisualEnvironmentSignal::Fonts => unsafe { NSFontSetChangedNotification },
            VisualEnvironmentSignal::Screens => unsafe {
                NSApplicationDidChangeScreenParametersNotification
            },
            VisualEnvironmentSignal::Appearance => {
                unreachable!("appearance changes use button KVO")
            }
        };
        let center = match notification_center_for(signal) {
            NotificationCenterKind::Default => NSNotificationCenter::defaultCenter(),
            NotificationCenterKind::Workspace => {
                NSWorkspace::sharedWorkspace().notificationCenter()
            }
        };
        let target = EnvironmentEventTarget::new(self.marker, self.proxy.clone(), signal);
        unsafe {
            center.addObserver_selector_name_object(
                &target,
                objc2::sel!(environmentChanged:),
                Some(name),
                None,
            )
        };
        NativeNotificationToken { center, target }
    }

    fn observe_button(&mut self, button: &Self::Button) -> Self::ButtonToken {
        let target = EnvironmentEventTarget::new(
            self.marker,
            self.proxy.clone(),
            VisualEnvironmentSignal::Appearance,
        );
        unsafe {
            button_object(button).addObserver_forKeyPath_options_context(
                target.as_super(),
                ns_string!("effectiveAppearance"),
                NSKeyValueObservingOptions::New,
                std::ptr::null_mut(),
            )
        };
        NativeButtonToken {
            button: button.retain(),
            target,
        }
    }
}

pub struct VisualEnvironmentObserver {
    tokens: ObserverTokens<NativeObservationBackend>,
}

impl VisualEnvironmentObserver {
    pub fn new(marker: MainThreadMarker, proxy: EventLoopProxy<RuntimeEvent>) -> Self {
        Self {
            tokens: ObserverTokens::new(NativeObservationBackend { marker, proxy }),
        }
    }

    pub fn rebind_status_button(&mut self, button: Option<&NSStatusBarButton>) {
        self.tokens.rebind_button(button);
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use super::{
        enqueue_visual_event, notification_center_for, NotificationCenterKind, ObservationBackend,
        ObserverTokens, VisualEnvironmentSignal, VisualEventSink,
    };
    use crate::macos::RuntimeEvent;

    #[derive(Default)]
    struct FakeState {
        notification_registrations: Vec<VisualEnvironmentSignal>,
        button_registrations: Vec<u8>,
        button_operations: Vec<String>,
        notification_drops: usize,
        button_drops: usize,
    }

    struct FakeToken {
        state: Rc<RefCell<FakeState>>,
        button: Option<u8>,
    }

    impl Drop for FakeToken {
        fn drop(&mut self) {
            let mut state = self.state.borrow_mut();
            if let Some(button) = self.button {
                state.button_drops += 1;
                state.button_operations.push(format!("drop {button}"));
            } else {
                state.notification_drops += 1;
            }
        }
    }

    struct FakeBackend {
        state: Rc<RefCell<FakeState>>,
    }

    impl ObservationBackend for FakeBackend {
        type Button = u8;
        type ButtonToken = FakeToken;
        type NotificationToken = FakeToken;

        fn observe_notification(
            &mut self,
            signal: VisualEnvironmentSignal,
        ) -> Self::NotificationToken {
            self.state
                .borrow_mut()
                .notification_registrations
                .push(signal);
            FakeToken {
                state: Rc::clone(&self.state),
                button: None,
            }
        }

        fn observe_button(&mut self, button: &Self::Button) -> Self::ButtonToken {
            let mut state = self.state.borrow_mut();
            state.button_registrations.push(*button);
            state.button_operations.push(format!("bind {button}"));
            FakeToken {
                state: Rc::clone(&self.state),
                button: Some(*button),
            }
        }
    }

    #[derive(Default)]
    struct FakeSink(RefCell<Vec<RuntimeEvent>>);

    impl VisualEventSink for FakeSink {
        fn enqueue(&self, event: RuntimeEvent) {
            self.0.borrow_mut().push(event);
        }
    }

    #[test]
    fn callbacks_enqueue_only_their_typed_runtime_event() {
        let sink = FakeSink::default();

        for signal in [
            VisualEnvironmentSignal::Appearance,
            VisualEnvironmentSignal::Accessibility,
            VisualEnvironmentSignal::SystemColors,
            VisualEnvironmentSignal::Fonts,
            VisualEnvironmentSignal::Screens,
        ] {
            enqueue_visual_event(&sink, signal);
        }

        let events = sink.0.into_inner();
        assert_eq!(events.len(), 5);
        assert!(matches!(events[0], RuntimeEvent::VisualEnvironmentChanged));
        assert!(matches!(events[1], RuntimeEvent::VisualEnvironmentChanged));
        assert!(matches!(events[2], RuntimeEvent::VisualEnvironmentChanged));
        assert!(matches!(events[3], RuntimeEvent::FontSetChanged));
        assert!(matches!(events[4], RuntimeEvent::ScreenParametersChanged));
    }

    #[test]
    fn accessibility_uses_the_workspace_notification_center() {
        assert_eq!(
            notification_center_for(VisualEnvironmentSignal::Accessibility),
            NotificationCenterKind::Workspace
        );
        for signal in [
            VisualEnvironmentSignal::SystemColors,
            VisualEnvironmentSignal::Fonts,
            VisualEnvironmentSignal::Screens,
        ] {
            assert_eq!(
                notification_center_for(signal),
                NotificationCenterKind::Default
            );
        }
    }

    #[test]
    fn notification_tokens_live_with_the_observer_and_release_on_drop() {
        let state = Rc::new(RefCell::new(FakeState::default()));
        let observer = ObserverTokens::new(FakeBackend {
            state: Rc::clone(&state),
        });

        assert_eq!(state.borrow().notification_registrations.len(), 4);
        assert_eq!(state.borrow().notification_drops, 0);

        drop(observer);

        assert_eq!(state.borrow().notification_drops, 4);
    }

    #[test]
    fn changing_the_status_button_releases_and_rebinds_appearance_observation() {
        let state = Rc::new(RefCell::new(FakeState::default()));
        let mut observer = ObserverTokens::new(FakeBackend {
            state: Rc::clone(&state),
        });

        observer.rebind_button(Some(&1));
        assert_eq!(state.borrow().button_registrations, vec![1]);
        assert_eq!(state.borrow().button_drops, 0);

        observer.rebind_button(Some(&2));
        assert_eq!(state.borrow().button_registrations, vec![1, 2]);
        assert_eq!(state.borrow().button_drops, 1);
        assert_eq!(
            state.borrow().button_operations,
            vec!["bind 1", "drop 1", "bind 2"]
        );

        observer.rebind_button(None);
        assert_eq!(state.borrow().button_drops, 2);
    }
}
