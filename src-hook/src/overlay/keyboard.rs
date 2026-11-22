//! Hides polled keyboard state from the game while the overlay menu is open.
//!
//! The game reads movement and actions by polling `GetAsyncKeyState`, `GetKeyState` and
//! `GetKeyboardState` instead of DirectInput or window messages. While blocked, those calls
//! report every key as released. The overlay's own code reads the real state through the
//! original functions (`real_*` below).

use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{anyhow, Result};
use hudhook::imgui::{Io, Key};
use hudhook::windows::core::{s, PCSTR};
use hudhook::windows::Win32::Foundation::{BOOL, TRUE};
use hudhook::windows::Win32::System::LibraryLoader::{GetModuleHandleA, GetProcAddress};
use hudhook::windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, GetKeyState, VIRTUAL_KEY, VK_APPS, VK_CONTROL, VK_LCONTROL, VK_LMENU,
    VK_LSHIFT, VK_MENU, VK_RCONTROL, VK_RMENU, VK_RSHIFT, VK_SHIFT,
};
use log::info;

use retour::static_detour;

type KeyStateFn = unsafe extern "system" fn(i32) -> i16;
type KeyboardStateFn = unsafe extern "system" fn(*mut u8) -> BOOL;

static_detour! {
    static GetAsyncKeyStateHook: unsafe extern "system" fn(i32) -> i16;
    static GetKeyStateHook: unsafe extern "system" fn(i32) -> i16;
    static GetKeyboardStateHook: unsafe extern "system" fn(*mut u8) -> BOOL;
}

static BLOCKED: AtomicBool = AtomicBool::new(false);

pub fn set_blocked(blocked: bool) {
    BLOCKED.store(blocked, Ordering::Relaxed);
}

fn is_blocked() -> bool {
    BLOCKED.load(Ordering::Relaxed)
}

unsafe fn user32_export<T>(name: PCSTR) -> Result<T> {
    let module = GetModuleHandleA(s!("user32.dll"))?;
    let address = GetProcAddress(module, name).ok_or_else(|| anyhow!("export not found"))?;
    Ok(std::mem::transmute_copy(&address))
}

pub fn install() -> Result<()> {
    unsafe {
        let async_state: KeyStateFn = user32_export(s!("GetAsyncKeyState"))?;
        let key_state: KeyStateFn = user32_export(s!("GetKeyState"))?;
        let keyboard_state: KeyboardStateFn = user32_export(s!("GetKeyboardState"))?;

        GetAsyncKeyStateHook
            .initialize(async_state, |vk| {
                if is_blocked() {
                    0
                } else {
                    GetAsyncKeyStateHook.call(vk)
                }
            })?
            .enable()?;
        GetKeyStateHook
            .initialize(key_state, |vk| {
                if is_blocked() {
                    0
                } else {
                    GetKeyStateHook.call(vk)
                }
            })?
            .enable()?;
        GetKeyboardStateHook
            .initialize(keyboard_state, |keys| {
                // Let the call run so Windows keeps its per-thread state in sync.
                let result = GetKeyboardStateHook.call(keys);
                if is_blocked() && !keys.is_null() {
                    std::ptr::write_bytes(keys, 0, 256);
                    return TRUE;
                }
                result
            })?
            .enable()?;
    }

    info!("Overlay keyboard hooks installed");
    Ok(())
}

/// `GetAsyncKeyState` that ignores the overlay's blocking.
pub fn real_async_key_state(vk: i32) -> i16 {
    unsafe {
        if GetAsyncKeyStateHook.is_enabled() {
            GetAsyncKeyStateHook.call(vk)
        } else {
            GetAsyncKeyState(vk)
        }
    }
}

fn real_key_down(vk: VIRTUAL_KEY) -> bool {
    let state = unsafe {
        if GetKeyStateHook.is_enabled() {
            GetKeyStateHook.call(vk.0 as i32)
        } else {
            GetKeyState(vk.0 as i32)
        }
    };
    state < 0
}

/// hudhook reads modifier keys with `GetKeyState`, which reports nothing while blocked.
/// Re-queue the real modifier state after its message processing so ImGui shortcuts keep
/// working; later events in the queue win.
pub fn sync_modifiers(io: &mut Io) {
    if !is_blocked() {
        return;
    }
    for (key, vk) in [
        (Key::ModCtrl, VK_CONTROL),
        (Key::ModShift, VK_SHIFT),
        (Key::ModAlt, VK_MENU),
        (Key::ModSuper, VK_APPS),
        (Key::LeftCtrl, VK_LCONTROL),
        (Key::RightCtrl, VK_RCONTROL),
        (Key::LeftShift, VK_LSHIFT),
        (Key::RightShift, VK_RSHIFT),
        (Key::LeftAlt, VK_LMENU),
        (Key::RightAlt, VK_RMENU),
    ] {
        io.add_key_event(key, real_key_down(vk));
    }
}
