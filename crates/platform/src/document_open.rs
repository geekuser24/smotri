//! Finder Open Documents events. Owner-approved platform boundary, AGENTS §0.3.
//! No project access from Cocoa callbacks; paths are queued for the UI.

use objc2::rc::Retained;
use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_foundation::{
    NSAppleEventDescriptor, NSAppleEventManager, NSAppleEventManagerWillProcessFirstEventNotification, NSNotification, NSNotificationCenter, NSObject,
    NSObjectProtocol,
};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex, OnceLock};

#[derive(Default)]
struct Pending {
    paths: VecDeque<String>,
    wake: Option<Arc<dyn Fn() + Send + Sync>>,
}
fn pending() -> &'static Mutex<Pending> {
    static QUEUE: OnceLock<Mutex<Pending>> = OnceLock::new();
    QUEUE.get_or_init(Default::default)
}
pub fn next_path() -> Option<String> {
    pending().lock().unwrap_or_else(std::sync::PoisonError::into_inner).paths.pop_front()
}
pub fn set_waker(wake: Arc<dyn Fn() + Send + Sync>) {
    pending().lock().unwrap_or_else(std::sync::PoisonError::into_inner).wake = Some(wake);
}
fn enqueue(path: String) {
    let wake = {
        let mut q = pending().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if path.is_empty() || path.contains('\0') || path.len() > 32768 || q.paths.len() >= 64 {
            log::warn!("Finder open queue limit reached");
            return;
        }
        if !q.paths.contains(&path) {
            q.paths.push_back(path);
        }
        q.wake.clone()
    };
    if let Some(wake) = wake {
        wake();
    }
}

define_class!(
    // SAFETY: NSObject permits subclasses; the handler has no custom ivars or Drop.
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    struct DocumentHandler;
    // SAFETY: NSObjectProtocol imposes no additional requirements.
    unsafe impl NSObjectProtocol for DocumentHandler {}
    impl DocumentHandler {
        // SAFETY: NSNotificationCenter sends precisely this notification signature.
        #[unsafe(method(smotriFirstEvent:))]
        fn first_event(&self, _notification: &NSNotification) {
            if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.register())).is_err() {
                log::error!("Finder document-open registration failed");
            }
        }
        // SAFETY: NSAppleEventManager invokes handlers with event and reply descriptors.
        #[unsafe(method(smotriOpenDocuments:withReplyEvent:))]
        fn open_documents(&self, event: &NSAppleEventDescriptor, _reply: &NSAppleEventDescriptor) {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let Some(files) = event.paramDescriptorForKeyword(u32::from_be_bytes(*b"----")) else { return };
                for index in 1..=files.numberOfItems().clamp(0, 64) {
                    if let Some(url) = files.descriptorAtIndex(index).and_then(|d| d.fileURLValue())
                        && url.isFileURL()
                        && let Some(path) = url.path() {
                            enqueue(path.to_string());
                        }
                }
            }));
            if result.is_err() { log::error!("Finder document-open callback failed"); }
        }
    }
);
impl DocumentHandler {
    fn register(&self) {
        // SAFETY: handler outlives the app; selector is implemented above with the documented
        // two-descriptor signature. 'aevt'/'odoc' is the standard Open Documents event.
        unsafe {
            NSAppleEventManager::sharedAppleEventManager().setEventHandler_andSelector_forEventClass_andEventID(
                self,
                sel!(smotriOpenDocuments:withReplyEvent:),
                u32::from_be_bytes(*b"aevt"),
                u32::from_be_bytes(*b"odoc"),
            );
        }
    }
}

/// Before the event loop starts. Re-register before the first event, after AppKit initialized
/// its default handlers. This does not replace winit's application delegate.
pub fn install() -> Result<(), String> {
    let mtm = MainThreadMarker::new().ok_or("Finder handler requires the main thread")?;
    // Only main-thread callers can get here, so registration cannot race.
    static INSTALLED: OnceLock<()> = OnceLock::new();
    if INSTALLED.get().is_some() {
        return Ok(());
    }
    // SAFETY: NSObject init on a freshly allocated NSObject subclass with no ivars.
    let handler: Retained<DocumentHandler> = unsafe { msg_send![DocumentHandler::alloc(mtm), init] };
    handler.register();
    // SAFETY: the selector accepts one NSNotification, and the retained handler lives for the
    // entire process. The notification constant is supplied by Foundation.
    unsafe {
        NSNotificationCenter::defaultCenter().addObserver_selector_name_object(
            &handler,
            sel!(smotriFirstEvent:),
            Some(NSAppleEventManagerWillProcessFirstEventNotification),
            None,
        );
    }
    // AppKit does not retain notification observers. Exactly one startup handler is retained.
    std::mem::forget(handler);
    let _ = INSTALLED.set(());
    Ok(())
}
