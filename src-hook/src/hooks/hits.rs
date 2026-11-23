//! Recent hit positions, for the overlay's debug view.
//!
//! A damage instance holds, as homogeneous vec4s (game 2.0.2): the attacker's position at +0x90
//! (w = 1), the hit point at +0xB0 (w = 1) and the vector between them at +0x80 (w = 0). Found by
//! comparing hits: hit - attacker equals that vector. The hit point is kept relative to the
//! target: in its own frame (right, up, forward) when its facing can be read, so it follows the
//! target as it moves and turns. Enemies keep their feet at `+0x30` and a horizontal forward
//! vector at `+0x370` (with the matching yaw in radians, minus pi, at `+0x360`; game 2.0.2,
//! found on Ewiyar from two exports before and after it turned).

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use super::read_process_value;

const HIT_POINT_OFFSET: usize = 0xB0;
const ACTOR_POSITION: usize = 0x30;
const ACTOR_FORWARD: usize = 0x370;
const MAX_HITS: usize = 256;

#[derive(Clone, Copy, Debug)]
pub struct Hit {
    /// World position when it landed.
    pub position: [f32; 3],
    /// The target and the hit point in its frame, to follow it as it moves and turns.
    pub target: usize,
    pub local: Option<Local>,
    /// Actor index of the party member who dealt it.
    pub source_index: u32,
    pub damage: i32,
    pub at: Instant,
}

static HITS: Mutex<VecDeque<Hit>> = Mutex::new(VecDeque::new());

/// The hit point of a damage instance, if it is a valid point (w = 1).
fn hit_point(damage_instance: usize) -> Option<[f32; 3]> {
    let v =
        read_process_value::<[f32; 4]>((damage_instance + HIT_POINT_OFFSET) as *const [f32; 4])?;
    let valid = v.iter().all(|x| x.is_finite() && x.abs() < 1.0e6) && (v[3] - 1.0).abs() < 1.0e-3;
    valid.then_some([v[0], v[1], v[2]])
}

pub fn record(damage_instance: usize, target: usize, source_index: u32, damage: i32) {
    let Some(position) = hit_point(damage_instance) else {
        return;
    };
    let mut hits = HITS.lock().unwrap_or_else(|e| e.into_inner());
    if hits.len() >= MAX_HITS {
        hits.pop_front();
    }
    let local = feet(target).map(|feet| {
        let offset = [
            position[0] - feet[0],
            position[1] - feet[1],
            position[2] - feet[2],
        ];
        match facing(target) {
            Some(frame) => Local::Turning(frame.to_local(offset)),
            None => Local::Moving(offset),
        }
    });
    hits.push_back(Hit {
        position,
        target,
        local,
        source_index,
        damage,
        at: Instant::now(),
    });
}

/// A hit point relative to its target.
#[derive(Clone, Copy, Debug)]
pub enum Local {
    /// Right, up and forward components in the target's frame.
    Turning([f32; 3]),
    /// World-space offset from the feet, when the target's facing is unknown.
    Moving([f32; 3]),
}

/// A horizontal frame: right and forward on the ground plane, up along y.
struct Frame {
    right: [f32; 3],
    forward: [f32; 3],
}

impl Frame {
    fn from_forward(forward: [f32; 3]) -> Option<Self> {
        let length = (forward[0] * forward[0] + forward[2] * forward[2]).sqrt();
        if !(0.5..=1.5).contains(&length) {
            return None;
        }
        let forward = [forward[0] / length, 0.0, forward[2] / length];
        // right = up x forward with up = (0, 1, 0).
        let right = [forward[2], 0.0, -forward[0]];
        Some(Self { right, forward })
    }

    fn to_local(&self, offset: [f32; 3]) -> [f32; 3] {
        [
            offset[0] * self.right[0] + offset[2] * self.right[2],
            offset[1],
            offset[0] * self.forward[0] + offset[2] * self.forward[2],
        ]
    }

    fn to_world(&self, local: [f32; 3]) -> [f32; 3] {
        [
            local[0] * self.right[0] + local[2] * self.forward[0],
            local[1],
            local[0] * self.right[2] + local[2] * self.forward[2],
        ]
    }
}

/// An enemy's forward vector (horizontal, unit), for the debug view's facing arrow.
pub fn forward(instance: usize) -> Option<[f32; 3]> {
    facing(instance).map(|frame| frame.forward)
}

fn facing(instance: usize) -> Option<Frame> {
    let v = read_process_value::<[f32; 3]>((instance + ACTOR_FORWARD) as *const [f32; 3])?;
    v.iter().all(|x| x.is_finite()).then_some(())?;
    Frame::from_forward(v)
}

fn feet(instance: usize) -> Option<[f32; 3]> {
    let v = read_process_value::<[f32; 3]>((instance + ACTOR_POSITION) as *const [f32; 3])?;
    v.iter()
        .all(|x| x.is_finite() && x.abs() < 1.0e6)
        .then_some(v)
}

impl Hit {
    /// Where the hit is now: moved and turned along with its target if it can still be read.
    pub fn current_position(&self) -> [f32; 3] {
        let Some(feet) = feet(self.target) else {
            return self.position;
        };
        let offset = match self.local {
            Some(Local::Turning(local)) => match facing(self.target) {
                Some(frame) => frame.to_world(local),
                None => return self.position,
            },
            Some(Local::Moving(offset)) => offset,
            None => return self.position,
        };
        [
            feet[0] + offset[0],
            feet[1] + offset[1],
            feet[2] + offset[2],
        ]
    }
}

/// Hits no older than `max_age`, oldest first.
pub fn recent(max_age: Duration) -> Vec<Hit> {
    let hits = HITS.lock().unwrap_or_else(|e| e.into_inner());
    hits.iter()
        .filter(|hit| hit.at.elapsed() <= max_age)
        .copied()
        .collect()
}

pub fn reset() {
    HITS.lock().unwrap_or_else(|e| e.into_inner()).clear();
}

#[cfg(test)]
mod tests {
    use super::Frame;

    #[test]
    fn local_offsets_turn_with_the_target() {
        // Facing +z, a hit 1 to the right (+x) and 2 ahead.
        let before = Frame::from_forward([0.0, 0.0, 1.0]).unwrap();
        let local = before.to_local([1.0, 0.5, 2.0]);
        assert_eq!(local, [1.0, 0.5, 2.0]);
        // Turned to face +x: ahead is now +x, right is now -z.
        let after = Frame::from_forward([1.0, 0.0, 0.0]).unwrap();
        let world = after.to_world(local);
        assert!(
            (world[0] - 2.0).abs() < 1e-6
                && (world[1] - 0.5).abs() < 1e-6
                && (world[2] + 1.0).abs() < 1e-6
        );
        // Round trip.
        let back = after.to_local(world);
        assert!((back[0] - 1.0).abs() < 1e-6 && (back[2] - 2.0).abs() < 1e-6);
    }

    #[test]
    fn rejects_non_unit_forward() {
        assert!(Frame::from_forward([0.0, 0.0, 0.0]).is_none());
        assert!(Frame::from_forward([5.0, 0.0, 0.0]).is_none());
    }
}
