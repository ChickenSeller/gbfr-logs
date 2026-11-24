//! Hides DirectInput mouse and keyboard input from the game while the overlay menu is open.
//!
//! The game reads camera movement and attacks through DirectInput 8, which bypasses the window
//! messages that hudhook's `MessageFilter` blocks. Every DirectInput device of an interface
//! flavour shares one static vtable inside dinput8.dll, so the `GetDeviceState` and
//! `GetDeviceData` slots are swapped in that table, found through throw-away system mouse
//! devices. Inline-patching the functions themselves stopped the game's damage hook from
//! reporting, so no dinput8 code is modified. Calls are filtered by device type; joysticks are
//! left alone because zeroing their state would read as axes pinned to one side.
//!
//! The original functions always run first so DirectInput keeps draining its buffers; only the
//! results handed back to the game are cleared.

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use anyhow::{anyhow, Context, Result};
use hudhook::windows::core::{Interface, HRESULT};
use hudhook::windows::Win32::Devices::HumanInterfaceDevice::{
    DirectInput8Create, GUID_SysMouse, IDirectInput8A, IDirectInput8W, IDirectInputDevice8A,
    IDirectInputDevice8W, DI8DEVTYPE_KEYBOARD, DI8DEVTYPE_MOUSE, DIDEVCAPS, DIDEVICEOBJECTDATA,
    DIRECTINPUT_VERSION,
};
use hudhook::windows::Win32::System::LibraryLoader::GetModuleHandleA;
use hudhook::windows::Win32::System::Memory::{
    VirtualProtect, PAGE_PROTECTION_FLAGS, PAGE_READWRITE,
};
use log::{info, warn};

type GetDeviceStateFn = unsafe extern "system" fn(*mut c_void, u32, *mut c_void) -> HRESULT;
type GetDeviceDataFn =
    unsafe extern "system" fn(*mut c_void, u32, *mut DIDEVICEOBJECTDATA, *mut u32, u32) -> HRESULT;
type GetCapabilitiesFn = unsafe extern "system" fn(*mut c_void, *mut DIDEVCAPS) -> HRESULT;

/// Vtable slots of `IDirectInputDevice8{A,W}`: IUnknown (0-2), GetCapabilities (3),
/// EnumObjects, GetProperty, SetProperty, Acquire, Unacquire, GetDeviceState (9),
/// GetDeviceData (10).
const SLOT_GET_CAPABILITIES: usize = 3;
const SLOT_GET_DEVICE_STATE: usize = 9;
const SLOT_GET_DEVICE_DATA: usize = 10;

static BLOCKED: AtomicBool = AtomicBool::new(false);

/// Original function pointers taken out of the vtables. The wide and ANSI tables may point at
/// different implementations, so each keeps its own originals.
static ORIGINAL_STATE_W: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_DATA_W: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_STATE_A: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_DATA_A: AtomicUsize = AtomicUsize::new(0);
/// The patched vtables, to put the originals back when unloading.
static VTABLE_W: AtomicUsize = AtomicUsize::new(0);
static VTABLE_A: AtomicUsize = AtomicUsize::new(0);

/// While `true`, the game receives empty mouse and keyboard input.
pub fn set_blocked(blocked: bool) {
    BLOCKED.store(blocked, Ordering::Relaxed);
}

/// Returns the vtable of a throw-away wide system mouse device.
unsafe fn wide_vtable() -> Result<*mut usize> {
    let instance = GetModuleHandleA(None)?;
    let mut raw = std::ptr::null_mut();
    DirectInput8Create(
        instance,
        DIRECTINPUT_VERSION,
        &IDirectInput8W::IID,
        &mut raw,
        None,
    )?;
    let input = IDirectInput8W::from_raw(raw);
    let mut device: Option<IDirectInputDevice8W> = None;
    input.CreateDevice(&GUID_SysMouse, &mut device, None)?;
    let device = device.context("CreateDevice returned no device")?;
    // The vtable is static data inside dinput8.dll and outlives the dummy device.
    Ok(*(device.as_raw() as *const *mut usize))
}

/// Returns the vtable of a throw-away ANSI system mouse device.
unsafe fn ansi_vtable() -> Result<*mut usize> {
    let instance = GetModuleHandleA(None)?;
    let mut raw = std::ptr::null_mut();
    DirectInput8Create(
        instance,
        DIRECTINPUT_VERSION,
        &IDirectInput8A::IID,
        &mut raw,
        None,
    )?;
    let input = IDirectInput8A::from_raw(raw);
    let mut device: Option<IDirectInputDevice8A> = None;
    input.CreateDevice(&GUID_SysMouse, &mut device, None)?;
    let device = device.context("CreateDevice returned no device")?;
    Ok(*(device.as_raw() as *const *mut usize))
}

/// Swaps one vtable entry. Pointer-sized aligned stores are atomic on x64, so concurrent
/// callers see either the old or the new entry.
unsafe fn swap_slot(vtable: *mut usize, slot: usize, replacement: usize) -> Result<()> {
    let entry = vtable.add(slot);
    let mut old_protect = PAGE_PROTECTION_FLAGS(0);
    VirtualProtect(
        entry as *const c_void,
        std::mem::size_of::<usize>(),
        PAGE_READWRITE,
        &mut old_protect,
    )?;
    entry.write_volatile(replacement);
    let mut ignored = PAGE_PROTECTION_FLAGS(0);
    VirtualProtect(
        entry as *const c_void,
        std::mem::size_of::<usize>(),
        old_protect,
        &mut ignored,
    )?;
    Ok(())
}

fn is_replacement(entry: usize) -> bool {
    [
        state_w as usize,
        state_a as usize,
        data_w as usize,
        data_a as usize,
    ]
    .contains(&entry)
}

/// Patches the state/data slots of one vtable and records the originals. A table that was
/// already patched (the wide and ANSI tables can be the same object) is left as is.
unsafe fn patch_vtable(
    vtable: *mut usize,
    state_replacement: GetDeviceStateFn,
    data_replacement: GetDeviceDataFn,
    original_state: &AtomicUsize,
    original_data: &AtomicUsize,
    patched: &AtomicUsize,
) -> Result<()> {
    if vtable.is_null() {
        return Err(anyhow!("null DirectInput vtable"));
    }
    let state = vtable.add(SLOT_GET_DEVICE_STATE).read_volatile();
    let data = vtable.add(SLOT_GET_DEVICE_DATA).read_volatile();
    if is_replacement(state) || is_replacement(data) {
        return Ok(());
    }

    // Publish the originals before the replacements can be reached.
    original_state.store(state, Ordering::SeqCst);
    original_data.store(data, Ordering::SeqCst);
    swap_slot(vtable, SLOT_GET_DEVICE_STATE, state_replacement as usize)?;
    swap_slot(vtable, SLOT_GET_DEVICE_DATA, data_replacement as usize)?;
    patched.store(vtable as usize, Ordering::SeqCst);
    Ok(())
}

/// Puts the original methods back in a vtable this module patched.
unsafe fn restore_vtable(
    patched: &AtomicUsize,
    original_state: &AtomicUsize,
    original_data: &AtomicUsize,
) {
    let vtable = patched.swap(0, Ordering::SeqCst) as *mut usize;
    if vtable.is_null() {
        return;
    }
    for (slot, original) in [
        (SLOT_GET_DEVICE_STATE, original_state),
        (SLOT_GET_DEVICE_DATA, original_data),
    ] {
        let original = original.load(Ordering::SeqCst);
        if original != 0 && is_replacement(vtable.add(slot).read_volatile()) {
            if let Err(error) = swap_slot(vtable, slot, original) {
                warn!("Could not restore a DirectInput method: {error:#}");
            }
        }
    }
}

/// Undoes [`install`], for unloading.
pub fn uninstall() {
    unsafe {
        restore_vtable(&VTABLE_W, &ORIGINAL_STATE_W, &ORIGINAL_DATA_W);
        restore_vtable(&VTABLE_A, &ORIGINAL_STATE_A, &ORIGINAL_DATA_A);
    }
}

pub fn install() -> Result<()> {
    unsafe {
        let wide = wide_vtable().context("wide DirectInput device")?;
        patch_vtable(
            wide,
            state_w,
            data_w,
            &ORIGINAL_STATE_W,
            &ORIGINAL_DATA_W,
            &VTABLE_W,
        )
        .context("wide DirectInput vtable")?;

        match ansi_vtable() {
            Ok(ansi) => patch_vtable(
                ansi,
                state_a,
                data_a,
                &ORIGINAL_STATE_A,
                &ORIGINAL_DATA_A,
                &VTABLE_A,
            )
            .context("ANSI DirectInput vtable")?,
            Err(error) => info!("ANSI DirectInput vtable unavailable: {error:#}"),
        }
    }

    info!("Overlay DirectInput vtable hooks installed");
    Ok(())
}

unsafe fn call_state(
    original: &AtomicUsize,
    device: *mut c_void,
    size: u32,
    data: *mut c_void,
) -> HRESULT {
    let original: GetDeviceStateFn = std::mem::transmute(original.load(Ordering::Acquire));
    filter_state(device, size, data, original(device, size, data))
}

unsafe fn call_data(
    original: &AtomicUsize,
    device: *mut c_void,
    size: u32,
    data: *mut DIDEVICEOBJECTDATA,
    count: *mut u32,
    flags: u32,
) -> HRESULT {
    let original: GetDeviceDataFn = std::mem::transmute(original.load(Ordering::Acquire));
    filter_data(device, count, original(device, size, data, count, flags))
}

unsafe extern "system" fn state_w(device: *mut c_void, size: u32, data: *mut c_void) -> HRESULT {
    call_state(&ORIGINAL_STATE_W, device, size, data)
}

unsafe extern "system" fn state_a(device: *mut c_void, size: u32, data: *mut c_void) -> HRESULT {
    call_state(&ORIGINAL_STATE_A, device, size, data)
}

unsafe extern "system" fn data_w(
    device: *mut c_void,
    size: u32,
    data: *mut DIDEVICEOBJECTDATA,
    count: *mut u32,
    flags: u32,
) -> HRESULT {
    call_data(&ORIGINAL_DATA_W, device, size, data, count, flags)
}

unsafe extern "system" fn data_a(
    device: *mut c_void,
    size: u32,
    data: *mut DIDEVICEOBJECTDATA,
    count: *mut u32,
    flags: u32,
) -> HRESULT {
    call_data(&ORIGINAL_DATA_A, device, size, data, count, flags)
}

#[derive(PartialEq)]
enum DeviceKind {
    Mouse,
    Keyboard,
    Other,
}

/// Reads the device type through the device's own vtable. `GetCapabilities` sits at the same
/// slot (right after IUnknown) in both the ANSI and wide interfaces.
unsafe fn device_kind(device: *mut c_void) -> DeviceKind {
    if device.is_null() {
        return DeviceKind::Other;
    }
    let vtable = *(device as *const *const usize);
    let get_capabilities: GetCapabilitiesFn =
        std::mem::transmute(*vtable.add(SLOT_GET_CAPABILITIES));

    let mut caps = DIDEVCAPS {
        dwSize: std::mem::size_of::<DIDEVCAPS>() as u32,
        ..Default::default()
    };
    if get_capabilities(device, &mut caps).is_err() {
        return DeviceKind::Other;
    }
    match caps.dwDevType & 0xFF {
        DI8DEVTYPE_MOUSE => DeviceKind::Mouse,
        DI8DEVTYPE_KEYBOARD => DeviceKind::Keyboard,
        _ => DeviceKind::Other,
    }
}

fn filter_state(device: *mut c_void, size: u32, data: *mut c_void, result: HRESULT) -> HRESULT {
    if !BLOCKED.load(Ordering::Relaxed) || result.is_err() || data.is_null() {
        return result;
    }
    let kind = unsafe { device_kind(device) };
    if kind != DeviceKind::Other {
        unsafe { std::ptr::write_bytes(data as *mut u8, 0, size as usize) };
    }
    result
}

fn filter_data(device: *mut c_void, count: *mut u32, result: HRESULT) -> HRESULT {
    if !BLOCKED.load(Ordering::Relaxed) || result.is_err() || count.is_null() {
        return result;
    }
    let kind = unsafe { device_kind(device) };
    if kind != DeviceKind::Other {
        // The events were consumed by the original call; report none to the game.
        unsafe { *count = 0 };
    }
    result
}
