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
}
