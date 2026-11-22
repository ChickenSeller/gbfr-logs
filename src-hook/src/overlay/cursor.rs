//! Releases the game's mouse lock while the overlay menu has input focus.
//!
//! The game confines the cursor with `ClipCursor`, re-centres it with `SetCursorPos` and reads
//! camera movement through `GetCursorPos`. While the menu is open those user32 calls are
//! intercepted: the clip is lifted, re-centring is ignored and the game sees a frozen cursor so
//! the camera does not move. Closing the menu restores the clip the game last asked for.
//!
//! ImGui itself is fed from `WM_MOUSEMOVE`/`WM_INPUT` by hudhook and never calls these
//! functions, so it keeps tracking the real cursor.

use std::ffi::c_void;
use std::sync::Mutex;

use anyhow::{anyhow, Result};
use hudhook::windows::core::s;
use hudhook::windows::Win32::Foundation::{BOOL, POINT, RECT, TRUE};
use hudhook::windows::Win32::System::LibraryLoader::{GetModuleHandleA, GetProcAddress};
use hudhook::windows::Win32::UI::WindowsAndMessaging::GetClipCursor;
use log::{info, warn};

use retour::static_detour;

type ClipCursorFn = unsafe extern "system" fn(*const RECT) -> BOOL;
type SetCursorPosFn = unsafe extern "system" fn(i32, i32) -> BOOL;
type GetCursorPosFn = unsafe extern "system" fn(*mut POINT) -> BOOL;

static_detour! {
    static ClipCursorHook: unsafe extern "system" fn(*const RECT) -> BOOL;
    static SetCursorPosHook: unsafe extern "system" fn(i32, i32) -> BOOL;
    static GetCursorPosHook: unsafe extern "system" fn(*mut POINT) -> BOOL;
}

struct CursorState {
    captured: bool,
    /// Clip rectangle to restore on release; `None` means the game wants no clip.
    restore_clip: Option<RECT>,
    /// Cursor position reported to the game while captured.
    frozen_pos: POINT,
}

static STATE: Mutex<CursorState> = Mutex::new(CursorState {
    captured: false,
    restore_clip: None,
    frozen_pos: POINT { x: 0, y: 0 },
});

fn lock_state() -> std::sync::MutexGuard<'static, CursorState> {
    // A poisoned lock only means a panic happened mid-update; the plain data is still usable.
    STATE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

unsafe fn user32_export<T>(name: hudhook::windows::core::PCSTR) -> Result<T> {
    let module = GetModuleHandleA(s!("user32.dll"))?;
    let address = GetProcAddress(module, name).ok_or_else(|| anyhow!("export not found"))?;
    Ok(std::mem::transmute_copy::<_, T>(
        &(address as *const c_void),
    ))
}

pub fn install() -> Result<()> {
    unsafe {
        let clip: ClipCursorFn = user32_export(s!("ClipCursor"))?;
        let set_pos: SetCursorPosFn = user32_export(s!("SetCursorPos"))?;
        let get_pos: GetCursorPosFn = user32_export(s!("GetCursorPos"))?;

        ClipCursorHook
            .initialize(clip, clip_cursor_detour)?
            .enable()?;
        SetCursorPosHook
            .initialize(set_pos, set_cursor_pos_detour)?
            .enable()?;
        GetCursorPosHook
            .initialize(get_pos, get_cursor_pos_detour)?
            .enable()?;
    }
    info!("Overlay cursor hooks installed");
    Ok(())
}

fn hooks_ready() -> bool {
    ClipCursorHook.is_enabled() && SetCursorPosHook.is_enabled() && GetCursorPosHook.is_enabled()
}

fn clip_cursor_detour(rect: *const RECT) -> BOOL {
    let mut state = lock_state();
    if state.captured {
        state.restore_clip = unsafe { rect.as_ref() }.copied();
        return TRUE;
    }
    drop(state);
    unsafe { ClipCursorHook.call(rect) }
}

fn set_cursor_pos_detour(x: i32, y: i32) -> BOOL {
    if lock_state().captured {
        return TRUE;
    }
    unsafe { SetCursorPosHook.call(x, y) }
}

fn get_cursor_pos_detour(point: *mut POINT) -> BOOL {
    let state = lock_state();
    if state.captured && !point.is_null() {
        unsafe { point.write(state.frozen_pos) };
        return TRUE;
    }
    drop(state);
    unsafe { GetCursorPosHook.call(point) }
}

/// Switches between overlay input mode (`true`) and the game's own cursor handling.
/// Idempotent, so it can be called every frame with the current menu state.
pub fn set_captured(captured: bool) {
    if !hooks_ready() {
        return;
    }

    let mut state = lock_state();
    if state.captured == captured {
        return;
    }

    unsafe {
        if captured {
            let mut clip = RECT::default();
            state.restore_clip = GetClipCursor(&mut clip).is_ok().then_some(clip);

            let mut pos = POINT::default();
            if GetCursorPosHook.call(&mut pos).as_bool() {
                state.frozen_pos = pos;
            }

            if !ClipCursorHook.call(std::ptr::null()).as_bool() {
                warn!("Could not release cursor clip");
            }
        } else {
            let clip = state.restore_clip.take();
            let clip_ptr = clip
                .as_ref()
                .map_or(std::ptr::null(), |rect| rect as *const RECT);
            let _ = ClipCursorHook.call(clip_ptr);
            // Put the cursor back where the game last saw it so the camera does not jump.
            let _ = SetCursorPosHook.call(state.frozen_pos.x, state.frozen_pos.y);
        }
    }

    state.captured = captured;
}
