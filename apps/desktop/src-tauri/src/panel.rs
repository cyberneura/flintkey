//! macOS: turns the launcher window into a panel that receives keyboard input without activating the app.
//!
//! macOS does not let an app make itself frontmost in response to a hotkey. A normal window would appear,
//! but keyboard input would still go to the previous app. A (nonactivating) NSPanel, as used by
//! Spotlight-style launchers, receives keyboard input without activating the app. Hiding it naturally returns
//! keyboard input to the previous app, and it can appear above full-screen apps.
//!
//! Tauri cannot create an NSPanel, so the class of the created NSWindow is swapped at runtime.

use objc2::ffi::{class_getInstanceSize, object_setClass};
use objc2::runtime::{AnyClass, AnyObject, Bool, ClassBuilder, Sel};
use objc2::{class, msg_send, sel};
use std::ffi::c_void;
use std::sync::OnceLock;

const STYLE_NONACTIVATING_PANEL: usize = 1 << 7;
/// Appear on all Spaces | stay put in Mission Control | appear above full-screen apps.
const COLLECTION_BEHAVIOR: usize = (1 << 0) | (1 << 4) | (1 << 8);
/// One above the menu bar (`NSMainMenuWindowLevel + 1`). In front of the Dock and full-screen windows.
const LEVEL: isize = 25;

extern "C-unwind" fn can_become_key_window(_: &AnyObject, _: Sel) -> Bool {
    Bool::YES
}

/// Key events the panel has received, for the focus log.
pub static KEY_EVENTS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    fn IsSecureEventInputEnabled() -> u8;
}

/// The frontmost app and whether some app holds Secure Event Input, for the focus log.
///
/// # Safety
///
/// Must be called from the main thread.
pub unsafe fn system_state() -> String {
    // SAFETY: NSWorkspace methods without arguments; the bundle identifier is read while it is alive.
    unsafe {
        let workspace: *mut AnyObject = msg_send![class!(NSWorkspace), sharedWorkspace];
        let front: *mut AnyObject = msg_send![workspace, frontmostApplication];
        let bundle = if front.is_null() {
            "none".to_string()
        } else {
            let id: *mut AnyObject = msg_send![front, bundleIdentifier];
            if id.is_null() {
                "unknown".to_string()
            } else {
                let utf8: *const std::ffi::c_char = msg_send![id, UTF8String];
                if utf8.is_null() {
                    "unknown".to_string()
                } else {
                    std::ffi::CStr::from_ptr(utf8).to_string_lossy().into_owned()
                }
            }
        };
        format!(
            "frontmost={bundle} secure_input={} key_events={}",
            IsSecureEventInputEnabled() != 0,
            KEY_EVENTS.load(std::sync::atomic::Ordering::Relaxed)
        )
    }
}

/// NSEventTypeKeyDown, NSEventTypeKeyUp, NSEventTypeFlagsChanged.
const KEY_EVENT_TYPES: [usize; 3] = [10, 11, 12];

/// Notes key events reaching the panel in the focus log (their type only), then handles them as NSPanel does.
extern "C-unwind" fn send_event(this: &AnyObject, _: Sel, event: *mut AnyObject) {
    if crate::diag::enabled() && !event.is_null() {
        // SAFETY: `event` is the NSEvent being dispatched; `type` takes no arguments.
        let kind: usize = unsafe { msg_send![event, type] };
        if KEY_EVENT_TYPES.contains(&kind) {
            KEY_EVENTS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let key: Bool = unsafe { msg_send![this, isKeyWindow] };
            crate::diag::log(&format!("native event type={kind} key={}", key.as_bool()));
        }
    }
    // SAFETY: NSPanel implements `sendEvent:`, and `this` is an instance of a subclass of it.
    unsafe {
        let _: () = msg_send![super(this, class!(NSPanel)), sendEvent: event];
    }
}

/// An NSPanel subclass that can receive keyboard input. Borderless panels do not by default.
fn panel_class() -> &'static AnyClass {
    static CLASS: OnceLock<&'static AnyClass> = OnceLock::new();
    CLASS.get_or_init(|| {
        let mut builder =
            ClassBuilder::new(c"FlintkeyLauncherPanel", class!(NSPanel)).expect("the class name is not taken");
        // SAFETY: `canBecomeKeyWindow` takes no arguments and returns BOOL, matching the function's type.
        unsafe {
            builder.add_method(
                sel!(canBecomeKeyWindow),
                can_become_key_window as extern "C-unwind" fn(_, _) -> _,
            );
            builder.add_method(sel!(sendEvent:), send_event as extern "C-unwind" fn(_, _, _));
        }
        builder.register()
    })
}

/// The class of the window macOS 27 puts up inside an app for Siri's text input (a Siri mark at the text field).
/// While it is the key window, every key press goes to it, so the launcher looks frozen until it is dismissed.
const SIRI_INPUT_WINDOW: &str = "CampoLightweightUIHost";

/// The app's visible Siri input windows.
///
/// # Safety
///
/// Must be called from the main thread.
unsafe fn siri_input_windows() -> Vec<*mut AnyObject> {
    // SAFETY: NSApplication / NSWindow methods without arguments, on the main thread.
    unsafe {
        let app: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
        let windows: *mut AnyObject = msg_send![app, windows];
        let count: usize = msg_send![windows, count];
        (0..count)
            .map(|index| -> *mut AnyObject { msg_send![windows, objectAtIndex: index] })
            .filter(|&window| {
                let visible: Bool = msg_send![window, isVisible];
                visible.as_bool() && (*window).class().name().to_string_lossy().contains(SIRI_INPUT_WINDOW)
            })
            .collect()
    }
}

/// Hides the app's visible Siri input windows. One that is up takes the keys even while the panel reports itself as
/// key, and it can already be up when the panel is shown.
///
/// # Safety
///
/// Must be called from the main thread.
unsafe fn hide_siri_input_windows() {
    // SAFETY: the caller's guarantees.
    for window in unsafe { siri_input_windows() } {
        crate::diag::log("hid a Siri input window");
        // SAFETY: a live NSWindow from the app's window list, on the main thread.
        let _: () = unsafe { msg_send![window, orderOut: std::ptr::null::<AnyObject>()] };
    }
}

/// Turns the window into a panel. Returns `true` on success.
///
/// # Safety
///
/// `ns_window` must be a live NSWindow, and this must be called from the main thread.
pub unsafe fn convert(ns_window: *mut c_void) -> bool {
    let window = ns_window.cast::<AnyObject>();
    // The window object is an NSWindow with tao's own data appended. If NSPanel were larger than NSWindow,
    // after the swap NSPanel would use that data's memory as its own.
    // SAFETY: both are registered classes.
    let same_layout = unsafe {
        class_getInstanceSize(std::ptr::from_ref(class!(NSPanel)).cast())
            == class_getInstanceSize(std::ptr::from_ref(class!(NSWindow)).cast())
    };
    if !same_layout {
        return false;
    }
    // SAFETY: the layout is the same, as checked above. The added class has no data and overrides one method.
    unsafe {
        object_setClass(window.cast(), std::ptr::from_ref(panel_class()).cast());
        let style: usize = msg_send![window, styleMask];
        let _: () = msg_send![window, setStyleMask: style | STYLE_NONACTIVATING_PANEL];
        let _: () = msg_send![window, setCollectionBehavior: COLLECTION_BEHAVIOR];
        let _: () = msg_send![window, setLevel: LEVEL];
        // By default a panel hides when its app deactivates. This app never activates, so it would vanish at once.
        let _: () = msg_send![window, setHidesOnDeactivate: false];
    }
    true
}

/// Shows the panel. With `take_key` it also becomes the keyboard target. The app is never activated.
///
/// # Safety
///
/// `ns_window` must be a live window already passed to `convert`, and this must be called from the main thread.
pub unsafe fn show(ns_window: *mut c_void, take_key: bool) {
    let window = ns_window.cast::<AnyObject>();
    // SAFETY: both are NSWindow methods that take no arguments and return nothing.
    unsafe {
        let _: () = msg_send![window, orderFrontRegardless];
        if take_key {
            hide_siri_input_windows();
            let _: () = msg_send![window, makeKeyWindow];
        }
    }
    if take_key {
        // A key window whose first responder is not the web view drops every key press; the page cannot notice,
        // because nothing reaches it.
        // SAFETY: the caller's guarantees; `web_view_in` only reads the view tree.
        unsafe {
            let content: *mut AnyObject = msg_send![window, contentView];
            if let Some(web_view) = web_view_in(content) {
                let _: Bool = msg_send![window, makeFirstResponder: web_view];
            }
        }
    }
}

/// Whether the panel is the key window and its web view the first responder: whether key presses reach the page.
///
/// # Safety
///
/// `ns_window` must be a live window already passed to `convert`, and this must be called from the main thread.
pub unsafe fn receives_keys(ns_window: *mut c_void) -> bool {
    let window = ns_window.cast::<AnyObject>();
    // SAFETY: NSWindow methods without arguments; `web_view_in` only reads the view tree.
    unsafe {
        let key: Bool = msg_send![window, isKeyWindow];
        // A Siri input window takes the keys while the panel still reports itself as key.
        if !key.as_bool() || !siri_input_windows().is_empty() {
            return false;
        }
        let content: *mut AnyObject = msg_send![window, contentView];
        let responder: *mut AnyObject = msg_send![window, firstResponder];
        match web_view_in(content) {
            // WKWebView hands first responder to its own subviews, so any view inside it counts.
            Some(web_view) => {
                !responder.is_null() && {
                    let is_view: Bool = msg_send![responder, isKindOfClass: class!(NSView)];
                    is_view.as_bool() && {
                        let inside: Bool = msg_send![responder, isDescendantOf: web_view];
                        inside.as_bool()
                    }
                }
            }
            None => true,
        }
    }
}

/// The first WKWebView in the view tree under `view`.
///
/// # Safety
///
/// `view` must be null or a live NSView, on the main thread.
unsafe fn web_view_in(view: *mut AnyObject) -> Option<*mut AnyObject> {
    if view.is_null() {
        return None;
    }
    let class = AnyClass::get(c"WKWebView")?;
    // SAFETY: NSView methods; the subviews array is only read while it is alive.
    unsafe {
        let matches: Bool = msg_send![view, isKindOfClass: class];
        if matches.as_bool() {
            return Some(view);
        }
        let subviews: *mut AnyObject = msg_send![view, subviews];
        let count: usize = msg_send![subviews, count];
        (0..count).find_map(|index| {
            let child: *mut AnyObject = msg_send![subviews, objectAtIndex: index];
            web_view_in(child)
        })
    }
}

/// Key window status, the app's activation and the first responder's class, for the focus log.
///
/// # Safety
///
/// `ns_window` must be a live window already passed to `convert`, and this must be called from the main thread.
pub unsafe fn describe(ns_window: *mut c_void) -> String {
    let window = ns_window.cast::<AnyObject>();
    // SAFETY: NSWindow / NSApplication methods without arguments; class names are static strings.
    unsafe {
        let key: Bool = msg_send![window, isKeyWindow];
        let visible: Bool = msg_send![window, isVisible];
        let app: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
        let active: Bool = msg_send![app, isActive];
        let responder: *mut AnyObject = msg_send![window, firstResponder];
        let responder = if responder.is_null() {
            "none".to_string()
        } else {
            (*responder).class().name().to_string_lossy().into_owned()
        };
        format!(
            "key={} visible={} app_active={} first_responder={responder} receives_keys={}",
            key.as_bool(),
            visible.as_bool(),
            active.as_bool(),
            receives_keys(ns_window)
        )
    }
}

/// The app's own windows: class, level, frame and whether each is key or visible. For the focus log.
///
/// # Safety
///
/// Must be called from the main thread.
pub unsafe fn app_windows() -> String {
    // SAFETY: NSApplication / NSWindow methods without arguments, read on the main thread.
    unsafe {
        let app: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
        let windows: *mut AnyObject = msg_send![app, windows];
        let count: usize = msg_send![windows, count];
        let mut out = Vec::new();
        for index in 0..count {
            let window: *mut AnyObject = msg_send![windows, objectAtIndex: index];
            let visible: Bool = msg_send![window, isVisible];
            if !visible.as_bool() {
                continue;
            }
            let key: Bool = msg_send![window, isKeyWindow];
            let level: isize = msg_send![window, level];
            let frame: objc2_foundation::NSRect = msg_send![window, frame];
            out.push(format!(
                "{}@{level}{}[{:.0},{:.0} {:.0}x{:.0}]",
                (*window).class().name().to_string_lossy(),
                if key.as_bool() { "*key" } else { "" },
                frame.origin.x,
                frame.origin.y,
                frame.size.width,
                frame.size.height
            ));
        }
        format!("app_windows={}", out.join(" "))
    }
}

/// Whether a Siri input window of the app is visible (and so holds the keyboard).
///
/// # Safety
///
/// Must be called from the main thread.
pub unsafe fn siri_input_window_up() -> bool {
    // SAFETY: the caller's guarantees.
    !unsafe { siri_input_windows() }.is_empty()
}
