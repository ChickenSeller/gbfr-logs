//! Where the overlay's windows go, at any resolution.
//!
//! Saved positions are pixels on the display they were saved on (`OverlayConfig::layout_display`);
//! when the game runs at another size they are scaled to it (`OverlayConfig::adapt_to_display`).
//! Default positions are given for `REFERENCE_DISPLAY` and scaled the same way. A window is then
//! kept wholly on screen once its size is known, and placed again whenever the display changes.

use hudhook::imgui::Condition;

/// The display the default positions were laid out on.
pub const REFERENCE_DISPLAY: [f32; 2] = [2560.0, 1600.0];

/// A position given for `REFERENCE_DISPLAY`, on `display`.
pub fn scaled_default(position: [f32; 2], display: [f32; 2]) -> [f32; 2] {
    if display[0] <= 0.0 || display[1] <= 0.0 {
        return position;
    }
    [
        position[0] * display[0] / REFERENCE_DISPLAY[0],
        position[1] * display[1] / REFERENCE_DISPLAY[1],
    ]
}

/// `position` moved so a window of `size` stays on screen; a window larger than the screen keeps
/// its top-left corner on it. With no size yet, a corner of `fallback` must stay visible.
pub fn on_screen(position: [f32; 2], size: [f32; 2], display: [f32; 2]) -> [f32; 2] {
    const FALLBACK: [f32; 2] = [80.0, 40.0];
    if display[0] <= 0.0 || display[1] <= 0.0 {
        return position;
    }
    let extent = |axis: usize| {
        let size = if size[axis] > 0.0 {
            size[axis]
        } else {
            FALLBACK[axis]
        };
        (display[axis] - size).max(0.0)
    };
    [
        position[0].clamp(0.0, extent(0)),
        position[1].clamp(0.0, extent(1)),
    ]
}

/// Per window: its last size and the display it was placed on.
#[derive(Default)]
pub struct Placement {
    display: [f32; 2],
    size: [f32; 2],
}

impl Placement {
    /// Where to put a window saved at `saved`, and whether to force it there: when the display
    /// changed since the last frame, or when (part of) the window lies off screen. Checked every
    /// frame, as auto-sized windows only reach their real size after a few frames.
    pub fn place(&mut self, saved: [f32; 2], display: [f32; 2]) -> ([f32; 2], Condition) {
        let changed = self.display != [0.0, 0.0] && self.display != display;
        self.display = display;
        let position = on_screen(saved, self.size, display);
        let off_screen =
            (position[0] - saved[0]).abs() > 0.5 || (position[1] - saved[1]).abs() > 0.5;
        let condition = if changed || off_screen {
            Condition::Always
        } else {
            Condition::FirstUseEver
        };
        (position, condition)
    }

    pub fn set_size(&mut self, size: [f32; 2]) {
        self.size = size;
    }

    /// Placed again on the next frame (the saved position was moved).
    pub fn replace(&mut self) {
        self.display = [-1.0, -1.0];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_scale_with_the_display() {
        assert_eq!(
            scaled_default([1280.0, 800.0], [1920.0, 1080.0]),
            [960.0, 540.0]
        );
    }

    #[test]
    fn windows_stay_wholly_on_screen() {
        // A meter saved low on a 1600-line display, shown at 1080 lines.
        assert_eq!(
            on_screen([1065.0, 1449.0], [540.0, 200.0], [1920.0, 1080.0]),
            [1065.0, 880.0]
        );
        // Wider than the screen: its left edge stays on it.
        assert_eq!(
            on_screen([300.0, 10.0], [2000.0, 100.0], [1920.0, 1080.0]),
            [0.0, 10.0]
        );
    }

    #[test]
    fn placed_again_when_off_screen_or_the_display_changes() {
        let mut placement = Placement::default();
        let display = [1920.0, 1080.0];
        // Size not known yet: a corner on screen is enough.
        assert_eq!(
            placement.place([1864.0, 9.0], display),
            ([1840.0, 9.0], Condition::Always)
        );
        assert_eq!(
            placement.place([400.0, 9.0], display),
            ([400.0, 9.0], Condition::FirstUseEver)
        );
        // Grown to its real size: moved wholly on screen.
        placement.set_size([300.0, 120.0]);
        assert_eq!(
            placement.place([1840.0, 9.0], display),
            ([1620.0, 9.0], Condition::Always)
        );
        assert_eq!(
            placement.place([400.0, 9.0], [2560.0, 1600.0]).1,
            Condition::Always
        );
    }
}
