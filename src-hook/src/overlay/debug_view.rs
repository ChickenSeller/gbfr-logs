//! Experimental debug view: marks tracked enemies and party members in the 3D scene.
//!
//! The game camera is the static `cCameraGame` at RVA 0x7C24860 (game 2.0.2): eye at +0x10,
//! look-at target at +0x20, up vector at +0x30 and the vertical field of view (radians) at
//! +0x178. Actors keep their feet position at +0x30. No matrices are read: the view basis is
//! built from eye/target/up and points are projected with the field of view and the screen's
//! aspect ratio, so a wrong guess shows up as markers drifting off their actors.

use hudhook::imgui::{ImColor32, Ui};
use protocol::OverlayState;

use crate::hooks::read_value;

const CAMERA_RVA: usize = 0x7C2_4860;
const CAMERA_EYE: usize = 0x10;
const CAMERA_TARGET: usize = 0x20;
const CAMERA_UP: usize = 0x30;
const CAMERA_FOV: usize = 0x178;
const ACTOR_POSITION: usize = 0x30;
/// Height of the vertical marker line, in world units (the camera aims 2 above the feet).
const MARKER_HEIGHT: f32 = 2.0;
const NEAR: f32 = 0.1;

const ENEMY_COLOR: ImColor32 = ImColor32::from_rgba(255, 90, 80, 230);
const PARTY_COLOR: ImColor32 = ImColor32::from_rgba(90, 200, 255, 230);
const SHADOW_COLOR: ImColor32 = ImColor32::from_rgba(0, 0, 0, 200);

type Vec3 = [f32; 3];

fn sub(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: Vec3, b: Vec3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: Vec3, b: Vec3) -> Vec3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn normalize(v: Vec3) -> Option<Vec3> {
    let length = dot(v, v).sqrt();
    (length > 1.0e-6 && length.is_finite()).then(|| [v[0] / length, v[1] / length, v[2] / length])
}

fn read_vec3(address: usize) -> Option<Vec3> {
    let v = [
        read_value::<f32>(address as *const f32)?,
        read_value::<f32>((address + 4) as *const f32)?,
        read_value::<f32>((address + 8) as *const f32)?,
    ];
    v.iter()
        .all(|x| x.is_finite() && x.abs() < 1.0e7)
        .then_some(v)
}

/// Projects world points onto the screen for this frame.
pub struct Camera {
    eye: Vec3,
    right: Vec3,
    up: Vec3,
    forward: Vec3,
    /// 1 / tan(fov / 2).
    focal: f32,
    screen: [f32; 2],
}

impl Camera {
    pub fn read(game_base: usize, screen: [f32; 2]) -> Option<Self> {
        let camera = game_base + CAMERA_RVA;
        let eye = read_vec3(camera + CAMERA_EYE)?;
        let target = read_vec3(camera + CAMERA_TARGET)?;
        let up_hint = read_vec3(camera + CAMERA_UP)?;
        let fov = read_value::<f32>((camera + CAMERA_FOV) as *const f32)?;
        if !(0.1..3.0).contains(&fov) || screen[0] <= 0.0 || screen[1] <= 0.0 {
            return None;
        }
        let forward = normalize(sub(target, eye))?;
        let right = normalize(cross(forward, up_hint))?;
        let up = cross(right, forward);
        Some(Self {
            eye,
            right,
            up,
            forward,
            focal: 1.0 / (fov / 2.0).tan(),
            screen,
        })
    }

    /// Screen position of a world point, or `None` behind the camera.
    pub fn project(&self, point: Vec3) -> Option<[f32; 2]> {
        let d = sub(point, self.eye);
        let depth = dot(d, self.forward);
        if depth <= NEAR {
            return None;
        }
        let aspect = self.screen[0] / self.screen[1];
        let x = dot(d, self.right) / depth * self.focal / aspect;
        let y = dot(d, self.up) / depth * self.focal;
        Some([
            (x + 1.0) * 0.5 * self.screen[0],
            (1.0 - y) * 0.5 * self.screen[1],
        ])
    }
}

/// Draws a marker under every tracked enemy and party member.
pub fn render(ui: &Ui, game_base: usize, state: Option<&OverlayState>) {
    let Some(camera) = Camera::read(game_base, ui.io().display_size) else {
        return;
    };
    let draw = ui.get_background_draw_list();
    let mut mark = |instance: usize, label: &str, color: ImColor32| {
        let Some(feet) = read_vec3(instance + ACTOR_POSITION) else {
            return;
        };
        let head = [feet[0], feet[1] + MARKER_HEIGHT, feet[2]];
        let (Some(bottom), Some(top)) = (camera.project(feet), camera.project(head)) else {
            return;
        };
        let size = 8.0;
        draw.add_line(
            [bottom[0] - size, bottom[1]],
            [bottom[0] + size, bottom[1]],
            color,
        )
        .thickness(2.0)
        .build();
        draw.add_line(
            [bottom[0], bottom[1] - size],
            [bottom[0], bottom[1] + size],
            color,
        )
        .thickness(2.0)
        .build();
        draw.add_line(bottom, top, color).thickness(1.5).build();
        draw.add_text([top[0] + 5.0, top[1] + 1.0], SHADOW_COLOR, label);
        draw.add_text([top[0] + 4.0, top[1]], color, label);
    };

    for enemy in crate::hooks::enemy_hp::snapshot() {
        let name = state
            .and_then(|state| {
                state
                    .enemies
                    .iter()
                    .find(|e| e.type_hash == enemy.type_hash)
            })
            .map(|e| e.name.clone())
            .unwrap_or_else(|| format!("{:08x}", enemy.type_hash));
        mark(enemy.id, &name, ENEMY_COLOR);
    }
    for (actor_index, instance) in crate::hooks::statuses::party_instances() {
        let name = state
            .and_then(|state| state.players.iter().find(|p| p.index == actor_index))
            .map(|p| p.name.clone())
            .unwrap_or_else(|| format!("#{actor_index}"));
        mark(instance, &name, PARTY_COLOR);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projects_the_look_at_target_to_the_screen_centre() {
        let camera = Camera {
            eye: [0.0, 2.0, 10.0],
            forward: [0.0, 0.0, -1.0],
            right: [1.0, 0.0, 0.0],
            up: [0.0, 1.0, 0.0],
            focal: 1.0 / (0.8727f32 / 2.0).tan(),
            screen: [1920.0, 1080.0],
        };
        let centre = camera.project([0.0, 2.0, 0.0]).unwrap();
        assert!((centre[0] - 960.0).abs() < 0.01 && (centre[1] - 540.0).abs() < 0.01);
        // Up is up on screen, right is right.
        assert!(camera.project([0.0, 4.0, 0.0]).unwrap()[1] < 540.0);
        assert!(camera.project([2.0, 2.0, 0.0]).unwrap()[0] > 960.0);
        // Behind the camera.
        assert!(camera.project([0.0, 2.0, 20.0]).is_none());
    }
}
