//! Experimental debug view: marks enemies (every damageable one, hit or not) and party members
//! in the 3D scene, with enemy hurtboxes and recent hit points.
//!
//! The game camera is the static `cCameraGame` at RVA 0x7C24860 (game 2.0.2): eye at +0x10,
//! look-at target at +0x20, up vector at +0x30 and the vertical field of view (radians) at
//! +0x178. Actors keep their feet position at +0x30. No matrices are read: the view basis is
//! built from eye/target/up and points are projected with the field of view and the screen's
//! aspect ratio, so a wrong guess shows up as markers drifting off their actors.

use hudhook::imgui::{DrawListMut, ImColor32, Ui};
use protocol::OverlayState;

use crate::hooks::hurtboxes::Shape;
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
const FACING_COLOR: ImColor32 = ImColor32::from_rgba(255, 220, 60, 230);
const HURTBOX_COLOR: ImColor32 = ImColor32::from_rgba(80, 255, 120, 200);
const HURTBOX_OFF_COLOR: ImColor32 = ImColor32::from_rgba(170, 170, 170, 70);
/// Damageable things that are neither enemies nor players: breakable objects and the like.
const NPC_COLOR: ImColor32 = ImColor32::from_rgba(230, 200, 255, 220);
const BODY_COLOR: ImColor32 = ImColor32::from_rgba(120, 200, 255, 150);
/// Items and treasure boxes: gold, with a filled diamond, to stand out.
const ITEM_COLOR: ImColor32 = ImColor32::from_rgba(255, 215, 0, 255);
/// Terrain wireframe colour; its alpha fades from `TERRAIN_ALPHA` near to a fifth far away.
const TERRAIN_RGB: [u8; 3] = [150, 255, 170];
const TERRAIN_ALPHA: f32 = 150.0;
/// Terrain is drawn this far around the camera target, at most `TERRAIN_MAX` triangles.
const TERRAIN_RADIUS: f32 = 15.0;
const TERRAIN_MAX: usize = 2500;
const OBJECT_COLOR: ImColor32 = ImColor32::from_rgba(255, 170, 60, 210);
/// Segments per full circle of a wireframe.
const RING_SEGMENTS: usize = 24;
/// Length of an enemy's facing arrow, in world units.
const FACING_LENGTH: f32 = 5.0;
/// How long a hit point stays on screen, fading out.
const HIT_LIFETIME: std::time::Duration = std::time::Duration::from_secs(3);
const HIT_RADIUS: f32 = 4.0;
/// Hits by someone not in the snapshot.
const HIT_DEFAULT_RGB: u32 = 0xFFFFFF;

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
    /// What the camera looks at, about the controlled character.
    target: Vec3,
    right: Vec3,
    up: Vec3,
    forward: Vec3,
    /// 1 / tan(fov / 2).
    focal: f32,
    /// The game's picture inside the window: top-left corner and size.
    origin: [f32; 2],
    screen: [f32; 2],
}

/// Where the game's picture sits in a window of `display` size: the whole window, or the
/// largest centred 16:9 rectangle when `fit_16_9` (the game letterboxes other window shapes).
pub fn viewport(display: [f32; 2], fit_16_9: bool) -> ([f32; 2], [f32; 2]) {
    const WIDE: f32 = 16.0 / 9.0;
    if !fit_16_9 || display[0] <= 0.0 || display[1] <= 0.0 {
        return ([0.0, 0.0], display);
    }
    if display[0] / display[1] > WIDE {
        let width = display[1] * WIDE;
        ([(display[0] - width) / 2.0, 0.0], [width, display[1]])
    } else {
        let height = display[0] / WIDE;
        ([0.0, (display[1] - height) / 2.0], [display[0], height])
    }
}

impl Camera {
    pub fn read(game_base: usize, (origin, screen): ([f32; 2], [f32; 2])) -> Option<Self> {
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
            target,
            right,
            up,
            forward,
            focal: 1.0 / (fov / 2.0).tan(),
            origin,
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
            self.origin[0] + (x + 1.0) * 0.5 * self.screen[0],
            self.origin[1] + (1.0 - y) * 0.5 * self.screen[1],
        ])
    }
}

/// Draws a marker under every tracked enemy and party member.
pub fn render(
    ui: &Ui,
    game_base: usize,
    state: Option<&OverlayState>,
    show: &super::config::DebugViewConfig,
) {
    let camera = Camera::read(game_base, viewport(ui.io().display_size, show.fit_16_9));
    let damageable = crate::hooks::hurtboxes::actors();
    let party = crate::hooks::statuses::party_instances();
    let (entities, characters) = crate::hooks::entities::characters();
    // Body collision of everyone known, each actor once.
    let mut bodies: Vec<usize> = party.iter().map(|(_, instance)| *instance).collect();
    bodies.extend(damageable.iter().map(|a| a.instance));
    bodies.extend(characters.iter().map(|c| c.instance));
    bodies.sort_unstable();
    bodies.dedup();
    let mut body_shapes = Vec::new();
    let mut undecoded = 0;
    if !show.body_collision {
        bodies.clear();
    }
    for instance in bodies {
        let (shapes, other) = crate::hooks::body_collision::of(instance);
        body_shapes.extend(shapes);
        undecoded += other;
    }
    let (terrain, terrain_total) = match (&camera, show.terrain) {
        (Some(camera), true) => {
            crate::hooks::terrain::triangles_near(camera.target, TERRAIN_RADIUS, TERRAIN_MAX)
        }
        _ => (Vec::new(), 0),
    };
    if show.status_line {
        draw_status(
            ui,
            camera.is_some(),
            &damageable,
            party.len(),
            entities,
            characters.len(),
            (body_shapes.len(), undecoded),
            (terrain.len(), terrain_total),
        );
    }
    let Some(camera) = camera else {
        return;
    };
    let draw = ui.get_background_draw_list();
    for (triangle, alpha) in visible_terrain_cached(&camera, &terrain) {
        let [r, g, b] = TERRAIN_RGB;
        polyline(
            &draw,
            &camera,
            &[triangle[0], triangle[1], triangle[2], triangle[0]],
            ImColor32::from_rgba(r, g, b, (TERRAIN_ALPHA * alpha) as u8),
        );
    }
    for shape in body_shapes.iter().filter(|s| s.is_body_sized()) {
        draw_shape(&draw, &camera, shape, BODY_COLOR);
    }
    drop(draw);
    if show.hit_points {
        draw_hits(ui, &camera, state);
    }
    let draw = ui.get_background_draw_list();
    let mark = |instance: usize, label: &str, color: ImColor32| {
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

    // Every enemy that can take damage, hit or not; names come from the meter once one was hit.
    let tracked = crate::hooks::enemy_hp::snapshot();
    for object in damageable
        .iter()
        .filter(|a| show.objects && !a.enemy && !a.player && a.is_field_object())
    {
        let mut any = false;
        for hurtbox in crate::hooks::hurtboxes::of(object.instance)
            .into_iter()
            .filter(|h| h.shape.is_body_sized())
        {
            let color = if hurtbox.enabled {
                OBJECT_COLOR
            } else {
                HURTBOX_OFF_COLOR
            };
            draw_shape(&draw, &camera, &hurtbox.shape, color);
            any = true;
        }
        if any {
            mark(object.instance, &object.class, OBJECT_COLOR);
        }
    }
    for actor in damageable
        .iter()
        .filter(|actor| actor.enemy && (show.enemies || show.hurtboxes || show.facing))
    {
        let instance = actor.instance;
        let name = tracked
            .iter()
            .find(|enemy| enemy.id == instance)
            .and_then(|enemy| {
                state?
                    .enemies
                    .iter()
                    .find(|e| e.type_hash == enemy.type_hash)
                    .map(|e| e.name.clone())
            })
            .unwrap_or_else(|| actor.class.clone());
        let label = match hp_percent(instance) {
            Some(percent) => format!("{name} {percent:.0}%"),
            None => name,
        };
        for hurtbox in crate::hooks::hurtboxes::of(instance)
            .into_iter()
            .filter(|h| show.hurtboxes && h.shape.is_body_sized())
        {
            let color = if hurtbox.enabled {
                HURTBOX_COLOR
            } else {
                HURTBOX_OFF_COLOR
            };
            draw_shape(&draw, &camera, &hurtbox.shape, color);
        }
        if show.enemies {
            mark(instance, &label, ENEMY_COLOR);
        }
        // The facing the hit points turn with, to check it against the body.
        if !show.facing {
            continue;
        }
        if let (Some(feet), Some(forward)) = (
            read_vec3(instance + ACTOR_POSITION),
            crate::hooks::hits::forward(instance),
        ) {
            let lift = 0.5;
            let from = [feet[0], feet[1] + lift, feet[2]];
            let to = [
                feet[0] + forward[0] * FACING_LENGTH,
                feet[1] + lift,
                feet[2] + forward[2] * FACING_LENGTH,
            ];
            if let (Some(a), Some(b)) = (camera.project(from), camera.project(to)) {
                draw.add_line(a, b, FACING_COLOR).thickness(3.0).build();
                draw.add_circle(b, 5.0, FACING_COLOR).filled(true).build();
            }
        }
    }
    for &(actor_index, instance) in party.iter().filter(|_| show.party) {
        let name = state
            .and_then(|state| state.players.iter().find(|p| p.index == actor_index))
            .map(|p| p.name.clone())
            .unwrap_or_else(|| format!("#{actor_index}"));
        mark(instance, &name, PARTY_COLOR);
    }
    // Player characters the party list does not know, e.g. outside quests.
    for player in damageable.iter().filter(|a| {
        show.party && a.player && !party.iter().any(|(_, instance)| *instance == a.instance)
    }) {
        mark(player.instance, &player.class, PARTY_COLOR);
    }
    // Everyone else on the field: NPCs, and characters that cannot take damage right now.
    use crate::hooks::entities::Kind;
    // One townsperson can be several entities (body and behaviour) at one spot: mark it once.
    let mut marked: Vec<Vec3> = Vec::new();
    for character in &characters {
        let wanted = match character.kind {
            Kind::Player => show.party,
            Kind::Npc => show.npcs,
            Kind::Enemy => show.enemies,
            Kind::Item => show.items,
            Kind::Object => show.objects,
        };
        if !wanted {
            continue;
        }
        let instance = character.instance;
        if party.iter().any(|(_, i)| *i == instance)
            || damageable
                .iter()
                .any(|a| a.instance == instance && (a.enemy || a.player))
        {
            continue;
        }
        let Some(feet) = read_vec3(instance + ACTOR_POSITION) else {
            continue;
        };
        // Pooled or not yet placed objects sit at the origin.
        if feet == [0.0, 0.0, 0.0] {
            continue;
        }
        if marked
            .iter()
            .any(|p| dot(sub(*p, feet), sub(*p, feet)) < 0.3 * 0.3)
        {
            continue;
        }
        marked.push(feet);
        if character.kind == Kind::Item {
            if let Some(at) = camera.project([feet[0], feet[1] + 0.3, feet[2]]) {
                let size = 9.0;
                let diamond = [
                    [at[0], at[1] - size],
                    [at[0] + size, at[1]],
                    [at[0], at[1] + size],
                    [at[0] - size, at[1]],
                ];
                draw.add_triangle(diamond[0], diamond[1], diamond[2], ITEM_COLOR)
                    .filled(true)
                    .build();
                draw.add_triangle(diamond[0], diamond[2], diamond[3], ITEM_COLOR)
                    .filled(true)
                    .build();
                draw.add_polyline(
                    vec![diamond[0], diamond[1], diamond[2], diamond[3], diamond[0]],
                    SHADOW_COLOR,
                )
                .thickness(1.5)
                .build();
                let label = [at[0] + size + 4.0, at[1] - 8.0];
                draw.add_text(
                    [label[0] + 1.0, label[1] + 1.0],
                    SHADOW_COLOR,
                    &character.class,
                );
                draw.add_text(label, ITEM_COLOR, &character.class);
            }
            continue;
        }
        let color = match character.kind {
            Kind::Player => PARTY_COLOR,
            Kind::Npc => NPC_COLOR,
            Kind::Enemy => ENEMY_COLOR,
            Kind::Object => OBJECT_COLOR,
            Kind::Item => ITEM_COLOR,
        };
        mark(instance, &character.class, color);
    }
}

/// One line in the top-left corner saying what the view can see, so an empty scene can be told
/// apart from a camera it cannot read.
fn draw_status(
    ui: &Ui,
    camera_ok: bool,
    damageable: &[crate::hooks::hurtboxes::DamageableActor],
    party: usize,
    entities: usize,
    characters: usize,
    (capsules, undecoded): (usize, usize),
    (terrain, terrain_total): (usize, usize),
) {
    let enemies = damageable.iter().filter(|a| a.enemy).count();
    let players = damageable.iter().filter(|a| a.player).count();
    let others = damageable.len() - enemies - players;
    let text = format!(
        "调试视图  相机：{}  可受击：敌人 {enemies} / 玩家 {players} / 其他 {others}  队员：{party}  实体表：{entities}（角色/物件 {characters}）  身体碰撞：胶囊 {capsules} / 未解码 {undecoded}  地形：{terrain} / {terrain_total} 三角形",
        if camera_ok { "正常" } else { "读取失败" },
    );
    let draw = ui.get_background_draw_list();
    draw.add_text([11.0, 9.0], SHADOW_COLOR, &text);
    draw.add_text([10.0, 8.0], ImColor32::from_rgba(255, 255, 255, 220), &text);
}

/// Occlusion tests per frame above which hidden terrain is not culled, to keep the frame rate.
const OCCLUSION_BUDGET: usize = 3_000_000;

/// Whether the segment from `from` to `to` passes through `triangle` before reaching `to`
/// (Möller–Trumbore, both faces).
fn blocks(from: Vec3, to: Vec3, triangle: &crate::hooks::terrain::Triangle) -> bool {
    let [a, b, c] = *triangle;
    let direction = sub(to, from);
    let edge1 = sub(b, a);
    let edge2 = sub(c, a);
    let p = cross(direction, edge2);
    let det = dot(edge1, p);
    if det.abs() < 1.0e-8 {
        return false;
    }
    let inv = 1.0 / det;
    let s = sub(from, a);
    let u = dot(s, p) * inv;
    if !(0.0..=1.0).contains(&u) {
        return false;
    }
    let q = cross(s, edge1);
    let v = dot(direction, q) * inv;
    if v < 0.0 || u + v > 1.0 {
        return false;
    }
    let t = dot(edge2, q) * inv;
    // Hits right at the end are the target itself or what it touches.
    t > 1.0e-4 && t < 0.995
}

type VisibleTerrain = Vec<(crate::hooks::terrain::Triangle, f32)>;

/// How often the occlusion test is redone; frames in between draw the last result.
const OCCLUSION_EVERY: std::time::Duration = std::time::Duration::from_millis(66);
static VISIBLE_TERRAIN: std::sync::Mutex<Option<(std::time::Instant, VisibleTerrain)>> =
    std::sync::Mutex::new(None);

/// `visible_terrain`, recomputed at most every `OCCLUSION_EVERY`.
fn visible_terrain_cached(
    camera: &Camera,
    terrain: &[crate::hooks::terrain::Triangle],
) -> VisibleTerrain {
    let mut cache = VISIBLE_TERRAIN.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((at, visible)) = cache.as_ref() {
        if at.elapsed() < OCCLUSION_EVERY {
            return visible.clone();
        }
    }
    let visible = visible_terrain(camera, terrain);
    *cache = Some((std::time::Instant::now(), visible.clone()));
    visible
}

/// Terrain triangles to draw with their opacity: facing the camera, on screen, and not hidden
/// behind other terrain. A triangle shows if its centre or a point near one of its corners can be
/// seen from the camera; every nearby triangle, either face, can hide it.
fn visible_terrain(
    camera: &Camera,
    terrain: &[crate::hooks::terrain::Triangle],
) -> Vec<(crate::hooks::terrain::Triangle, f32)> {
    let candidates: Vec<(usize, f32)> = terrain
        .iter()
        .enumerate()
        .filter_map(|(i, t)| terrain_visibility(camera, t).map(|alpha| (i, alpha)))
        .collect();
    if candidates.len() * terrain.len() * 4 > OCCLUSION_BUDGET {
        return candidates
            .into_iter()
            .map(|(i, a)| (terrain[i], a))
            .collect();
    }
    candidates
        .into_iter()
        .filter(|&(index, _)| {
            let [a, b, c] = terrain[index];
            let centre = scale(add(add(a, b), c), 1.0 / 3.0);
            let towards = |corner: Vec3| add(centre, scale(sub(corner, centre), 0.8));
            [centre, towards(a), towards(b), towards(c)]
                .into_iter()
                .any(|point| {
                    !terrain
                        .iter()
                        .enumerate()
                        .any(|(other, t)| other != index && blocks(camera.eye, point, t))
                })
        })
        .map(|(i, alpha)| (terrain[i], alpha))
        .collect()
}

/// Whether a terrain triangle is worth drawing, as an opacity factor: it must face the camera
/// (back faces of walls and the underside of floors are culled) and lie on screen, and it fades
/// with distance from the camera target.
fn terrain_visibility(camera: &Camera, triangle: &crate::hooks::terrain::Triangle) -> Option<f32> {
    let [a, b, c] = *triangle;
    // Clockwise front faces (see `hooks::terrain::parse`): this normal points behind the face.
    let normal = cross(sub(b, a), sub(c, a));
    if dot(normal, sub(camera.eye, a)) >= 0.0 {
        return None;
    }
    let margin = 0.1;
    let on_screen = triangle.iter().any(|corner| {
        camera.project(*corner).is_some_and(|p| {
            let x = p[0] - camera.origin[0];
            let y = p[1] - camera.origin[1];
            x >= -camera.screen[0] * margin
                && x <= camera.screen[0] * (1.0 + margin)
                && y >= -camera.screen[1] * margin
                && y <= camera.screen[1] * (1.0 + margin)
        })
    });
    if !on_screen {
        return None;
    }
    let distance = crate::hooks::terrain::box_distance2(triangle, camera.target).sqrt();
    Some((1.0 - distance / TERRAIN_RADIUS).clamp(0.2, 1.0))
}

/// Actor HP (i64 at +0x160, max at +0x168) as a percentage.
fn hp_percent(instance: usize) -> Option<f64> {
    let hp = read_value::<i64>((instance + 0x160) as *const i64)?;
    let max = read_value::<i64>((instance + 0x168) as *const i64)?;
    (max > 0 && (0..=max).contains(&hp)).then(|| hp as f64 * 100.0 / max as f64)
}

fn add(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn scale(v: Vec3, s: f32) -> Vec3 {
    [v[0] * s, v[1] * s, v[2] * s]
}

/// Two unit vectors perpendicular to `axis` and to each other.
fn perpendiculars(axis: Vec3) -> (Vec3, Vec3) {
    let hint = if axis[1].abs() < 0.9 {
        [0.0, 1.0, 0.0]
    } else {
        [1.0, 0.0, 0.0]
    };
    let u = normalize(cross(axis, hint)).unwrap_or([1.0, 0.0, 0.0]);
    let v = cross(axis, u);
    (u, v)
}

/// A polyline through world points; segments with an end behind the camera are skipped.
/// Wireframe points closer to the camera than this are not drawn: they project far off screen.
const WIRE_NEAR: f32 = 1.0;

fn polyline(draw: &DrawListMut, camera: &Camera, points: &[Vec3], color: ImColor32) {
    let projected: Vec<Option<[f32; 2]>> = points
        .iter()
        .map(|p| {
            (dot(sub(*p, camera.eye), camera.forward) > WIRE_NEAR)
                .then(|| camera.project(*p))
                .flatten()
        })
        .collect();
    let diagonal =
        (camera.screen[0] * camera.screen[0] + camera.screen[1] * camera.screen[1]).sqrt();
    for pair in projected.windows(2) {
        if let (Some(a), Some(b)) = (pair[0], pair[1]) {
            let length = ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt();
            if length <= diagonal {
                draw.add_line(a, b, color).thickness(1.5).build();
            }
        }
    }
}

/// An arc of radius `r` around `center` in the plane of `u` and `v`, from angle `from` to `to`.
fn arc(
    draw: &DrawListMut,
    camera: &Camera,
    center: Vec3,
    (u, v): (Vec3, Vec3),
    r: f32,
    (from, to): (f32, f32),
    color: ImColor32,
) {
    let steps =
        ((RING_SEGMENTS as f32) * (to - from).abs() / std::f32::consts::TAU).ceil() as usize;
    let steps = steps.max(2);
    let points: Vec<Vec3> = (0..=steps)
        .map(|i| {
            let t = from + (to - from) * i as f32 / steps as f32;
            add(center, add(scale(u, r * t.cos()), scale(v, r * t.sin())))
        })
        .collect();
    polyline(draw, camera, &points, color);
}

fn draw_shape(draw: &DrawListMut, camera: &Camera, shape: &Shape, color: ImColor32) {
    use std::f32::consts::{PI, TAU};
    const X: Vec3 = [1.0, 0.0, 0.0];
    const Y: Vec3 = [0.0, 1.0, 0.0];
    const Z: Vec3 = [0.0, 0.0, 1.0];
    match *shape {
        Shape::Sphere { center, radius } => {
            for plane in [(X, Y), (Y, Z), (Z, X)] {
                arc(draw, camera, center, plane, radius, (0.0, TAU), color);
            }
        }
        Shape::Capsule { a, b, radius } => {
            let Some(axis) = normalize(sub(b, a)) else {
                for plane in [(X, Y), (Y, Z), (Z, X)] {
                    arc(draw, camera, a, plane, radius, (0.0, TAU), color);
                }
                return;
            };
            let (u, v) = perpendiculars(axis);
            // Rings at both ends, four lines along the side, half circles over each cap.
            arc(draw, camera, a, (u, v), radius, (0.0, TAU), color);
            arc(draw, camera, b, (u, v), radius, (0.0, TAU), color);
            for side in [u, v, scale(u, -1.0), scale(v, -1.0)] {
                let offset = scale(side, radius);
                polyline(draw, camera, &[add(a, offset), add(b, offset)], color);
            }
            let back = scale(axis, -1.0);
            for side in [u, v] {
                arc(draw, camera, a, (side, back), radius, (0.0, PI), color);
                arc(draw, camera, b, (side, axis), radius, (0.0, PI), color);
            }
        }
        Shape::Obb { center, axes, half } => {
            let corner = |sx: f32, sy: f32, sz: f32| {
                add(
                    center,
                    add(
                        scale(axes[0], half[0] * sx),
                        add(scale(axes[1], half[1] * sy), scale(axes[2], half[2] * sz)),
                    ),
                )
            };
            let signs = [-1.0, 1.0];
            for &s1 in &signs {
                for &s2 in &signs {
                    polyline(
                        draw,
                        camera,
                        &[corner(-1.0, s1, s2), corner(1.0, s1, s2)],
                        color,
                    );
                    polyline(
                        draw,
                        camera,
                        &[corner(s1, -1.0, s2), corner(s1, 1.0, s2)],
                        color,
                    );
                    polyline(
                        draw,
                        camera,
                        &[corner(s1, s2, -1.0), corner(s1, s2, 1.0)],
                        color,
                    );
                }
            }
        }
    }
}

/// Recent hit points as dots in the attacker's meter colour, fading over `HIT_LIFETIME`.
fn draw_hits(ui: &Ui, camera: &Camera, state: Option<&OverlayState>) {
    let hits = crate::hooks::hits::recent(HIT_LIFETIME);
    if hits.is_empty() {
        return;
    }
    let draw = ui.get_background_draw_list();
    for hit in hits {
        let Some(point) = camera.project(hit.current_position()) else {
            continue;
        };
        let rgb = state
            .and_then(|state| state.players.iter().find(|p| p.index == hit.source_index))
            .map_or(HIT_DEFAULT_RGB, |p| p.color);
        let life = 1.0 - hit.at.elapsed().as_secs_f32() / HIT_LIFETIME.as_secs_f32();
        let alpha = (life.clamp(0.0, 1.0) * 230.0) as u8;
        let [_, r, g, b] = rgb.to_be_bytes();
        draw.add_circle(point, HIT_RADIUS, ImColor32::from_rgba(r, g, b, alpha))
            .filled(true)
            .build();
        draw.add_circle(
            point,
            HIT_RADIUS + 1.0,
            ImColor32::from_rgba(0, 0, 0, alpha / 2),
        )
        .build();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wall_hides_what_is_behind_it() {
        let wall = [[-1.0, -1.0, 5.0], [1.0, -1.0, 5.0], [0.0, 2.0, 5.0]];
        assert!(blocks([0.0, 0.0, 0.0], [0.0, 0.0, 10.0], &wall));
        assert!(!blocks([0.0, 0.0, 0.0], [0.0, 0.0, 4.0], &wall));
        assert!(!blocks([0.0, 0.0, 0.0], [5.0, 0.0, 10.0], &wall));
    }

    #[test]
    fn letterboxes_to_16_9() {
        // A 16:10 window: bars above and below.
        let (origin, size) = viewport([2420.0, 1512.0], true);
        assert!((size[0] - 2420.0).abs() < 1e-3 && (size[1] - 1361.25).abs() < 1e-2);
        assert!((origin[1] - 75.375).abs() < 1e-2 && origin[0] == 0.0);
        // Ultrawide: bars left and right, unless the game fills the window.
        let (origin, size) = viewport([3440.0, 1440.0], true);
        assert!((size[0] - 2560.0).abs() < 1e-3 && (origin[0] - 440.0).abs() < 1e-3);
        assert_eq!(
            viewport([3440.0, 1440.0], false),
            ([0.0, 0.0], [3440.0, 1440.0])
        );
    }

    #[test]
    fn projects_the_look_at_target_to_the_screen_centre() {
        let camera = Camera {
            eye: [0.0, 2.0, 10.0],
            target: [0.0, 2.0, 0.0],
            forward: [0.0, 0.0, -1.0],
            right: [1.0, 0.0, 0.0],
            up: [0.0, 1.0, 0.0],
            focal: 1.0 / (0.8727f32 / 2.0).tan(),
            origin: [0.0, 0.0],
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
