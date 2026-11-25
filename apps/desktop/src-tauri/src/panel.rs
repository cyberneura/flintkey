//! macOS: ランチャー窓を「アプリを前面にせずにキー入力を受け取るパネル」にする。
//!
//! macOS は、ホットキーをきっかけに自分を前面のアプリにすることを許さない。普通の窓のままだと、
//! 表示はされてもキー入力は元のアプリに届く。Spotlight 型のランチャーが使う NSPanel
//! (nonactivating) なら、アプリを前面にしないままキー入力を受け取れる。隠せばキー入力は
//! 元のアプリに自然に戻り、フルスクリーンのアプリの上にも出せる。
//!
//! Tauri は NSPanel を作れないので、作られた NSWindow のクラスを実行時に差し替える。

use objc2::ffi::{class_getInstanceSize, object_setClass};
use objc2::runtime::{AnyClass, AnyObject, Bool, ClassBuilder, Sel};
use objc2::{class, msg_send, sel};
use std::ffi::c_void;
use std::sync::OnceLock;

const STYLE_NONACTIVATING_PANEL: usize = 1 << 7;
/// すべての操作スペースに出す | Mission Control で動かさない | フルスクリーンのアプリの上にも出す。
const COLLECTION_BEHAVIOR: usize = (1 << 0) | (1 << 4) | (1 << 8);
/// メニューバーの 1 つ上 (`NSMainMenuWindowLevel + 1`)。Dock やフルスクリーンの窓より前。
const LEVEL: isize = 25;

extern "C-unwind" fn can_become_key_window(_: &AnyObject, _: Sel) -> Bool {
    Bool::YES
}

/// キー入力を受け取れる NSPanel のサブクラス。枠なしのパネルは既定ではキー入力を受け取らない。
fn panel_class() -> &'static AnyClass {
    static CLASS: OnceLock<&'static AnyClass> = OnceLock::new();
    CLASS.get_or_init(|| {
        let mut builder =
            ClassBuilder::new(c"FlintkeyLauncherPanel", class!(NSPanel)).expect("the class name is not taken");
        // SAFETY: `canBecomeKeyWindow` は引数なしで BOOL を返す。関数の型と一致している。
        unsafe {
            builder.add_method(
                sel!(canBecomeKeyWindow),
                can_become_key_window as extern "C-unwind" fn(_, _) -> _,
            );
        }
        builder.register()
    })
}

/// 窓をパネルに変える。変えられたら `true`。
///
/// # Safety
///
/// `ns_window` は生きている NSWindow で、メインスレッドから呼ぶこと。
pub unsafe fn convert(ns_window: *mut c_void) -> bool {
    let window = ns_window.cast::<AnyObject>();
    // 窓の実体は tao が NSWindow の後ろに自分のデータを足したもの。NSPanel が NSWindow より大きいと、
    // 差し替えたあとに NSPanel がそのデータの場所を自分のものとして使ってしまう。
    // SAFETY: どちらも登録済みのクラス。
    let same_layout = unsafe {
        class_getInstanceSize(std::ptr::from_ref(class!(NSPanel)).cast())
            == class_getInstanceSize(std::ptr::from_ref(class!(NSWindow)).cast())
    };
    if !same_layout {
        return false;
    }
    // SAFETY: 上で確かめた通りレイアウトは同じ。足したクラスはデータを持たず、メソッドを 1 つ上書きするだけ。
    unsafe {
        object_setClass(window.cast(), std::ptr::from_ref(panel_class()).cast());
        let style: usize = msg_send![window, styleMask];
        let _: () = msg_send![window, setStyleMask: style | STYLE_NONACTIVATING_PANEL];
        let _: () = msg_send![window, setCollectionBehavior: COLLECTION_BEHAVIOR];
        let _: () = msg_send![window, setLevel: LEVEL];
        // パネルは既定で、アプリが前面でなくなると隠れる。このアプリは前面にならないので、出た瞬間に消える。
        let _: () = msg_send![window, setHidesOnDeactivate: false];
    }
    true
}

/// パネルを出す。`take_key` ならキー入力の宛先にもする。どちらの場合もアプリは前面にしない。
///
/// # Safety
///
/// `ns_window` は `convert` 済みの生きている窓で、メインスレッドから呼ぶこと。
pub unsafe fn show(ns_window: *mut c_void, take_key: bool) {
    let window = ns_window.cast::<AnyObject>();
    // SAFETY: どちらも引数なしで何も返さない NSWindow のメソッド。
    unsafe {
        let _: () = msg_send![window, orderFrontRegardless];
        if take_key {
            let _: () = msg_send![window, makeKeyWindow];
        }
    }
}
