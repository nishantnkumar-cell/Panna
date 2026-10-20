//! Files opened from Finder: a double-click, Open With, or a drop on the Dock icon.
//!
//! macOS hands these to a running app as an "open documents" event, not on the command line,
//! and to an app it starts the same way, while it finishes launching. So a delegate that
//! receives them is set on the application at that moment; winit, under iced, sets none of its
//! own.

use std::path::PathBuf;
use std::sync::OnceLock;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{NSApplication, NSApplicationDelegate, NSApplicationWillFinishLaunchingNotification};
use objc2_foundation::{NSArray, NSNotification, NSNotificationCenter, NSURL};

type Open = Box<dyn Fn(PathBuf) + Send + Sync>;

/// What to do with each file, set once by [`on_open`].
static OPEN: OnceLock<Open> = OnceLock::new();

define_class!(
    // SAFETY: NSObject has no subclassing requirements, and the delegate keeps no state.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "PannaFinderDelegate"]
    struct Delegate;

    impl Delegate {
        // SAFETY: the signature matches a notification observer's: one notification argument.
        #[unsafe(method(willFinishLaunching:))]
        fn will_finish_launching(&self, _notification: &NSNotification) {
            NSApplication::sharedApplication(self.mtm()).setDelegate(Some(ProtocolObject::from_ref(self)));
        }
    }

    // SAFETY: NSObjectProtocol has no requirements beyond being an NSObject.
    unsafe impl NSObjectProtocol for Delegate {}

    // SAFETY: the one method implemented has the protocol's signature.
    unsafe impl NSApplicationDelegate for Delegate {
        #[unsafe(method(application:openURLs:))]
        fn application_open_urls(&self, _application: &NSApplication, urls: &NSArray<NSURL>) {
            let Some(open) = OPEN.get() else { return };
            for path in urls.iter().filter_map(|url| url.to_file_path()) {
                open(path);
            }
        }
    }
);

impl Delegate {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(());
        // SAFETY: NSObject's designated initialiser, called once on a fresh allocation.
        unsafe { msg_send![super(this), init] }
    }
}

/// Calls `open` with each file Finder asks the app to open, from the main thread. Call it on
/// the main thread before the app starts; later calls do nothing.
pub fn on_open(open: impl Fn(PathBuf) + Send + Sync + 'static) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    if OPEN.set(Box::new(open)).is_err() {
        return;
    }
    let delegate = Delegate::new(mtm);
    let observer: &AnyObject = &delegate;
    // SAFETY: the observer is a live object whose `willFinishLaunching:` method takes the
    // notification, and the name is AppKit's own constant, valid for the whole run.
    unsafe {
        NSNotificationCenter::defaultCenter().addObserver_selector_name_object(
            observer,
            sel!(willFinishLaunching:),
            Some(NSApplicationWillFinishLaunchingNotification),
            None,
        );
    }
    // The application holds its delegate weakly, and this one lasts as long as the app.
    std::mem::forget(delegate);
}
