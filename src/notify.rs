//! Native notifications.
//!
//! Only macOS needs this: WKWebView has no web Notification API, so the page
//! shim forwards notifications here. Windows and Linux webviews show them
//! natively once the permission handler grants access.

#[cfg(target_os = "macos")]
mod imp {
    use std::sync::OnceLock;
    use std::sync::atomic::{AtomicU64, Ordering};

    use block2::RcBlock;
    use objc2::runtime::Bool;
    use objc2_foundation::{NSBundle, NSError, NSString};
    use objc2_user_notifications::{
        UNAuthorizationOptions, UNMutableNotificationContent, UNNotificationRequest,
        UNUserNotificationCenter,
    };

    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    static SEQ: AtomicU64 = AtomicU64::new(0);

    /// The notification center throws for a bare binary; it needs a bundle.
    fn available() -> bool {
        *AVAILABLE.get_or_init(|| {
            let bundled = NSBundle::mainBundle().bundleIdentifier().is_some();
            if !bundled {
                eprintln!("[oxicord] notifications need the .app bundle (scripts/bundle-macos.sh)");
            }
            bundled
        })
    }

    pub fn init() {
        if !available() {
            return;
        }
        let center = UNUserNotificationCenter::currentNotificationCenter();
        let done = RcBlock::new(|granted: Bool, _err: *mut NSError| {
            if !granted.as_bool() {
                eprintln!("[oxicord] notification permission was not granted");
            }
        });
        center.requestAuthorizationWithOptions_completionHandler(
            UNAuthorizationOptions::Alert | UNAuthorizationOptions::Badge,
            &done,
        );
    }

    pub fn show(title: &str, body: &str) {
        if !available() {
            return;
        }
        let content = UNMutableNotificationContent::new();
        content.setTitle(&NSString::from_str(title));
        content.setBody(&NSString::from_str(body));
        // No sound: Discord plays its own.
        let id = format!("oxicord-{}", SEQ.fetch_add(1, Ordering::Relaxed));
        let request = UNNotificationRequest::requestWithIdentifier_content_trigger(
            &NSString::from_str(&id),
            &content,
            None,
        );
        UNUserNotificationCenter::currentNotificationCenter()
            .addNotificationRequest_withCompletionHandler(&request, None);
    }
}

#[cfg(not(target_os = "macos"))]
mod imp {
    pub fn init() {}
    pub fn show(_title: &str, _body: &str) {}
}

pub use imp::{init, show};
