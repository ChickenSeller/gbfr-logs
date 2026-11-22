//! Keeps keyboard input that arrives as window messages away from the game while the menu is
//! open.
//!
//! - The game pumps its queue with `PeekMessageA`. Keyboard messages it removes while blocked
//!   are translated and dispatched here, so hudhook's window procedure still feeds them to
//!   ImGui, and then handed back to the game as `WM_NULL`.
//! - The game reads movement and actions through raw input (`GetRawInputData` on a window that
//!   hudhook does not subclass). While blocked, key-down records are rewritten to the
//!   documented "overrun" key, which the game ignores. Key-up records pass through so keys
//!   held when the menu opened are released normally instead of getting stuck.

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{anyhow, Result};
use hudhook::windows::core::{s, PCSTR};
use hudhook::windows::Win32::Foundation::{BOOL, HWND};
use hudhook::windows::Win32::System::LibraryLoader::{GetModuleHandleA, GetProcAddress};
use hudhook::windows::Win32::UI::Input::{
    RAWINPUT, RAWINPUTHEADER, RAWKEYBOARD, RID_INPUT, RIM_TYPEKEYBOARD,
};
use hudhook::windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageA, TranslateMessage, MSG, PM_REMOVE, RI_KEY_BREAK, WM_KEYFIRST, WM_KEYLAST,
    WM_NULL,
};
use log::info;
use retour::static_detour;

type PeekMessageFn = unsafe extern "system" fn(*mut MSG, HWND, u32, u32, u32) -> BOOL;
type GetRawInputDataFn = unsafe extern "system" fn(isize, u32, *mut c_void, *mut u32, u32) -> u32;

static_detour! {
    static PeekMessageAHook: unsafe extern "system" fn(*mut MSG, HWND, u32, u32, u32) -> BOOL;
    static GetRawInputDataHook: unsafe extern "system" fn(isize, u32, *mut c_void, *mut u32, u32) -> u32;
}

/// Same value as `KEYBOARD_OVERRUN_MAKE_CODE`; Windows uses it for keys that carry no input.
const OVERRUN_KEY: u16 = 0xFF;

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
        let peek: PeekMessageFn = user32_export(s!("PeekMessageA"))?;
        PeekMessageAHook
            .initialize(peek, |msg, hwnd, min, max, remove| {
                let result = PeekMessageAHook.call(msg, hwnd, min, max, remove);
                if result.as_bool() && is_blocked() && remove & PM_REMOVE.0 != 0 {
                    swallow_keyboard_message(msg);
                }
                result
            })?
            .enable()?;

        let raw_input: GetRawInputDataFn = user32_export(s!("GetRawInputData"))?;
        GetRawInputDataHook
            .initialize(raw_input, |handle, command, buffer, size, header_size| {
                let written = GetRawInputDataHook.call(handle, command, buffer, size, header_size);
                if is_blocked() {
                    mask_raw_key_down(command, buffer, written);
                }
                written
            })?
            .enable()?;
    }

    info!("Overlay message and raw input hooks installed");
    Ok(())
}

/// Routes a keyboard message to the window procedure (and so to ImGui), then hides it from
/// the game's own loop.
unsafe fn swallow_keyboard_message(msg: *mut MSG) {
    if msg.is_null() || !(WM_KEYFIRST..=WM_KEYLAST).contains(&(*msg).message) {
        return;
    }
    let _ = TranslateMessage(msg);
    DispatchMessageA(msg);
    (*msg).message = WM_NULL;
}

/// Rewrites a key-down record filled in by `GetRawInputData(RID_INPUT)` into an overrun key.
unsafe fn mask_raw_key_down(command: u32, buffer: *mut c_void, written: u32) {
    let needed = std::mem::size_of::<RAWINPUTHEADER>() + std::mem::size_of::<RAWKEYBOARD>();
    if command != RID_INPUT.0
        || buffer.is_null()
        || written == u32::MAX
        || (written as usize) < needed
    {
        return;
    }
    let raw = buffer as *mut RAWINPUT;
    if (*raw).header.dwType != RIM_TYPEKEYBOARD.0 {
        return;
    }
    let keyboard = &mut (*raw).data.keyboard;
    if u32::from(keyboard.Flags) & RI_KEY_BREAK != 0 {
        return;
    }
    keyboard.MakeCode = OVERRUN_KEY;
    keyboard.VKey = OVERRUN_KEY;
    keyboard.Message = WM_NULL;
}
