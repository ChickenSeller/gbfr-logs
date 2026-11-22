//! Configurable menu hotkey: a key plus optional Ctrl/Shift/Alt, polled every frame.
//!
//! Keys are read with the real `GetAsyncKeyState` (bypassing the overlay's own keyboard
//! blocking) so the hotkey also works while the menu is open. Polling avoids depending on the
//! game's window messages, which may be consumed by its own input layer.

use hudhook::windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyNameTextW, MapVirtualKeyW, MAPVK_VK_TO_VSC,
};
use serde::{Deserialize, Serialize};

use super::keyboard::real_async_key_state;

const VK_SHIFT: u16 = 0x10;
const VK_CONTROL: u16 = 0x11;
const VK_MENU: u16 = 0x12;
const VK_ESCAPE: u16 = 0x1B;
const VK_INSERT: u16 = 0x2D;

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(default)]
pub struct KeyCombo {
    /// Windows virtual-key code of the main key.
    pub key: u16,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

impl Default for KeyCombo {
    fn default() -> Self {
        Self {
            key: VK_INSERT,
            ctrl: false,
            shift: false,
            alt: false,
        }
    }
}

fn is_down(vk: u16) -> bool {
    real_async_key_state(vk as i32) as u16 & 0x8000 != 0
}

/// Modifier keys and mouse buttons cannot be the main key: modifiers are part of the combo,
/// and a mouse button would trigger on every click.
fn can_be_main_key(vk: u16) -> bool {
    !matches!(vk, 0x01..=0x06 | 0x10..=0x12 | 0x5B | 0x5C | 0xA0..=0xA5) && vk < 0xFF
}

impl KeyCombo {
    pub fn is_valid(&self) -> bool {
        self.key != 0 && can_be_main_key(self.key)
    }

    fn modifiers_match(&self) -> bool {
        is_down(VK_CONTROL) == self.ctrl
            && is_down(VK_SHIFT) == self.shift
            && is_down(VK_MENU) == self.alt
    }

    /// Readable form such as `Ctrl+Shift+F8`, using the keyboard layout's key names.
    pub fn label(&self) -> String {
        let mut parts = Vec::new();
        if self.ctrl {
            parts.push("Ctrl".to_string());
        }
        if self.shift {
            parts.push("Shift".to_string());
        }
        if self.alt {
            parts.push("Alt".to_string());
        }
        parts.push(key_name(self.key));
        parts.join("+")
    }
}

/// Navigation keys share scan codes with the numeric keypad; the extended bit picks them.
fn is_extended(vk: u16) -> bool {
    matches!(vk, 0x21..=0x28 | 0x2D | 0x2E | 0x6F | 0x90 | 0x5D)
}

fn key_name(vk: u16) -> String {
    let scan = unsafe { MapVirtualKeyW(vk as u32, MAPVK_VK_TO_VSC) };
    let mut lparam = (scan as i32) << 16;
    if is_extended(vk) {
        lparam |= 1 << 24;
    }
    let mut buffer = [0u16; 64];
    let length = unsafe { GetKeyNameTextW(lparam, &mut buffer) };
    if length > 0 {
        String::from_utf16_lossy(&buffer[..length as usize])
    } else {
        format!("0x{vk:02X}")
    }
}

/// Edge detection: fires once when the combo goes down.
#[derive(Default)]
pub struct HotkeyWatcher {
    was_down: bool,
}

impl HotkeyWatcher {
    /// Ignores the combo until its key is released, e.g. right after recording it.
    pub fn suppress_until_released(&mut self) {
        self.was_down = true;
    }

    pub fn pressed(&mut self, combo: &KeyCombo) -> bool {
        let down = combo.is_valid() && is_down(combo.key) && combo.modifiers_match();
        let pressed = down && !self.was_down;
        self.was_down = down;
        pressed
    }
}

pub enum CaptureResult {
    Captured(KeyCombo),
    Cancelled,
}

/// Records the next key press as a new hotkey. Keys already held when recording starts (the
/// mouse button that clicked "change", a held modifier) are ignored until released.
pub struct KeyCapture {
    held: Vec<bool>,
}

impl KeyCapture {
    pub fn start() -> Self {
        Self {
            held: (0..0x100u16).map(is_down).collect(),
        }
    }

    pub fn poll(&mut self) -> Option<CaptureResult> {
        for vk in 0x07..0xFFu16 {
            let down = is_down(vk);
            let was_held = std::mem::replace(&mut self.held[vk as usize], down);
            if !down || was_held || !can_be_main_key(vk) {
                continue;
            }
            if vk == VK_ESCAPE {
                return Some(CaptureResult::Cancelled);
            }
            return Some(CaptureResult::Captured(KeyCombo {
                key: vk,
                ctrl: is_down(VK_CONTROL),
                shift: is_down(VK_SHIFT),
                alt: is_down(VK_MENU),
            }));
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modifiers_and_mouse_buttons_are_not_main_keys() {
        for vk in [0x01, 0x02, 0x10, 0x11, 0x12, 0x5B, 0xA0, 0xA5] {
            assert!(!can_be_main_key(vk), "{vk:#x}");
        }
        for vk in [VK_INSERT, 0x41, 0x70, 0x77, 0x24] {
            assert!(can_be_main_key(vk), "{vk:#x}");
        }
    }

    #[test]
    fn default_is_insert() {
        assert_eq!(KeyCombo::default().key, VK_INSERT);
        assert!(KeyCombo::default().is_valid());
        assert!(!KeyCombo {
            key: 0,
            ..KeyCombo::default()
        }
        .is_valid());
    }
}
