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
        }
        builder.register()
    })
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
        if !key.as_bool() {
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
