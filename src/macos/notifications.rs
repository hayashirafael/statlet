use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{Bool, ProtocolObject};
use objc2::{define_class, msg_send, DefinedClass, MainThreadOnly};
use objc2_foundation::{MainThreadMarker, NSBundle, NSError, NSObject, NSObjectProtocol, NSString};
use objc2_user_notifications::{
    UNAuthorizationOptions, UNMutableNotificationContent, UNNotification,
    UNNotificationPresentationOptions, UNNotificationRequest, UNNotificationResponse,
    UNNotificationSound, UNUserNotificationCenter, UNUserNotificationCenterDelegate,
};
use statlet::core::AppEvent;
use statlet::disk::{format_decimal_gigabytes, DiskObservation};
use tao::event_loop::EventLoopProxy;

use super::RuntimeEvent;

struct NotificationDelegateIvars {
    proxy: EventLoopProxy<RuntimeEvent>,
}

define_class!(
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[ivars = NotificationDelegateIvars]
    struct NotificationDelegate;

    unsafe impl NSObjectProtocol for NotificationDelegate {}

    unsafe impl UNUserNotificationCenterDelegate for NotificationDelegate {
        #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
        fn will_present(
            &self,
            _center: &UNUserNotificationCenter,
            _notification: &UNNotification,
            completion_handler: &block2::DynBlock<dyn Fn(UNNotificationPresentationOptions)>,
        ) {
            completion_handler.call((UNNotificationPresentationOptions::Banner
                | UNNotificationPresentationOptions::List
                | UNNotificationPresentationOptions::Sound,));
        }

        #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
        fn did_receive(
            &self,
            _center: &UNUserNotificationCenter,
            _response: &UNNotificationResponse,
            completion_handler: &block2::DynBlock<dyn Fn()>,
        ) {
            let _ = self
                .ivars()
                .proxy
                .send_event(RuntimeEvent::App(AppEvent::NotificationActivated));
            completion_handler.call(());
        }
    }
);

impl NotificationDelegate {
    fn new(mtm: MainThreadMarker, proxy: EventLoopProxy<RuntimeEvent>) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(NotificationDelegateIvars { proxy });
        unsafe { msg_send![super(this), init] }
    }
}

pub struct NotificationManager {
    center: Retained<UNUserNotificationCenter>,
    _delegate: Retained<NotificationDelegate>,
}

impl NotificationManager {
    pub fn new(mtm: MainThreadMarker, proxy: EventLoopProxy<RuntimeEvent>) -> Option<Self> {
        if !current_process_supports_notifications() {
            return None;
        }
        let center = UNUserNotificationCenter::currentNotificationCenter();
        let delegate = NotificationDelegate::new(mtm, proxy);
        center.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        Some(Self {
            center,
            _delegate: delegate,
        })
    }

    pub fn request_authorization(&self) {
        let completion = RcBlock::new(|_granted: Bool, _error: *mut NSError| {});
        self.center
            .requestAuthorizationWithOptions_completionHandler(
                UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound,
                &completion,
            );
    }

    pub fn deliver_disk_pressure(&self, observation: DiskObservation) {
        let content = UNMutableNotificationContent::new();
        content.setTitle(&NSString::from_str("O disco continua acima do limite"));
        content.setBody(&NSString::from_str(&format!(
            "{:.1}% ocupado, com {} disponível. Revise o espaço sem remover nada automaticamente.",
            observation.occupied_percent(),
            format_decimal_gigabytes(observation.available_bytes())
        )));
        content.setSound(Some(&UNNotificationSound::defaultSound()));
        let request = UNNotificationRequest::requestWithIdentifier_content_trigger(
            &NSString::from_str(&format!(
                "statlet.disk.{}",
                observation.observed_at().as_nanos()
            )),
            &content,
            None,
        );
        self.center
            .addNotificationRequest_withCompletionHandler(&request, None);
    }
}

fn current_process_supports_notifications() -> bool {
    NSBundle::mainBundle().bundleIdentifier().is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unbundled_cargo_binary_does_not_initialize_user_notifications() {
        assert!(!current_process_supports_notifications());
    }
}
