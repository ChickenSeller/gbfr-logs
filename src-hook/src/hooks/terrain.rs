//! Map collision (terrain), loaded per stage area from `st{:x}/r{:x}{:02x}/r{:x}{:02x}.col`.
//!
//! Each loaded file becomes an `asset::misc::CollisionDataImpl` (game 2.0.2, loader at
//! 0x1426C6F80) holding the file's bytes at `+0x30` with their length at `+0x38`. All of them sit
//! in a global `std::unordered_map<u64 hash, CollisionDataImpl*>` whose list head (sentinel
//! node pointer, then size) is at RVA 0x73CF908; a node is `{next, prev, key, value}`.
//!
//! The files are `COL2` (version 0x20151201): a 0x38-byte header with the name count at `+0x0C`,
//! the mesh table offset at `+0x10` and the mesh count at `+0x14`; name offsets follow the header
//! (area and switch group names such as `rc01_church_area`). Each mesh table entry is 20 bytes,
//! `{attribute, group, type (2), offset, 1}`, and points at a mesh block `{flag, vertex offset,
//! vertex count, index offset, index count, 0}` with offsets from the file start, vertices as
//! world-space `(x, y, z, 1)` f32s and u16 indices, three per triangle. Worked out from the
//! files loaded in Folca and a battle area, where every decoded mesh passed these checks.

use std::sync::{Arc, Mutex};

use super::read_process_value;

const MAP_RVA: usize = 0x73C_F908;
const DATA: usize = 0x30;
const LENGTH: usize = 0x38;
const MAX_FILES: usize = 256;
/// Larger files are not plausible collision data (a stage area is a few MB at most).
const MAX_LENGTH: usize = 256 << 20;

#[derive(Clone, Copy, Debug)]
pub struct CollisionFile {
    pub hash: u64,
    pub data: usize,
    pub length: usize,
}

/// Every collision file currently loaded.
pub fn loaded_files() -> Vec<CollisionFile> {
    let Some(base) = super::rtti::game_base() else {
        return Vec::new();
    };
    let Some(sentinel) = read_process_value::<usize>((base + MAP_RVA) as *const usize) else {
        return Vec::new();
    };
    if sentinel == 0 {
        return Vec::new();
    }
    let mut files = Vec::new();
    let mut node = read_process_value::<usize>(sentinel as *const usize).unwrap_or(sentinel);
    while node != sentinel && node != 0 && files.len() < MAX_FILES {
        let Some([next, _prev, key, value]) =
            read_process_value::<[usize; 4]>(node as *const [usize; 4])
        else {
            break;
        };
        if value != 0 {
            let data = read_process_value::<usize>((value + DATA) as *const usize).unwrap_or(0);
            let length = read_process_value::<usize>((value + LENGTH) as *const usize).unwrap_or(0);
            if data != 0 && length <= MAX_LENGTH {
                files.push(CollisionFile {
                    hash: key as u64,
                    data,
                    length,
                });
            }
        }
        node = next;
    }
    files
}

pub type Triangle = [[f32; 3]; 3];

fn u32_at(bytes: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        bytes.get(offset..offset + 4)?.try_into().ok()?,
    ))
}

fn f32_at(bytes: &[u8], offset: usize) -> Option<f32> {
    Some(f32::from_le_bytes(
        bytes.get(offset..offset + 4)?.try_into().ok()?,
    ))
}

/// Every triangle of a `COL2` file in the file's own winding: clockwise seen from the front
/// (Direct3D's default), so `(b - a) x (c - a)` points away from the side that collides. Checked
/// on Folca's floor meshes, where nearly every triangle has that normal pointing down. Meshes
/// that fail a check are skipped.
pub fn parse(bytes: &[u8]) -> Vec<Triangle> {
    parse_raw(bytes).0
}

/// The triangles, and whether every mesh of the file decoded (a file still being loaded, or
/// read short, does not).
fn parse_raw(bytes: &[u8]) -> (Vec<Triangle>, bool) {
    let mut triangles = Vec::new();
    if bytes.get(..4) != Some(b"COL2".as_slice()) {
        return (triangles, false);
    }
    let (Some(table), Some(meshes)) = (u32_at(bytes, 0x10), u32_at(bytes, 0x14)) else {
        return (triangles, false);
    };
    let mut complete = true;
    for mesh in 0..meshes.min(100_000) as usize {
        let entry = table as usize + mesh * 20;
        let Some(block) = u32_at(bytes, entry + 12) else {
            complete = false;
            break;
        };
        let block = block as usize;
        let (Some(vertex_offset), Some(vertex_count), Some(index_offset), Some(index_count)) = (
            u32_at(bytes, block + 4),
            u32_at(bytes, block + 8),
            u32_at(bytes, block + 12),
            u32_at(bytes, block + 16),
        ) else {
            complete = false;
            continue;
        };
        let (vertex_offset, vertex_count) = (vertex_offset as usize, vertex_count as usize);
        let (index_offset, index_count) = (index_offset as usize, index_count as usize);
        if index_count % 3 != 0
            || vertex_offset + vertex_count * 16 > bytes.len()
            || index_offset + index_count * 2 > bytes.len()
        {
            complete = false;
            continue;
        }
        let vertex = |index: usize| -> Option<[f32; 3]> {
            let at = vertex_offset + index * 16;
            let v = [
                f32_at(bytes, at)?,
                f32_at(bytes, at + 4)?,
                f32_at(bytes, at + 8)?,
            ];
            v.iter()
                .all(|x| x.is_finite() && x.abs() < 1.0e6)
                .then_some(v)
        };
        let mut mesh_triangles = Vec::with_capacity(index_count / 3);
        let mut valid = true;
        'corners: for corner in (0..index_count).step_by(3) {
            let mut triangle = [[0.0; 3]; 3];
            for (k, slot) in triangle.iter_mut().enumerate() {
                let at = index_offset + (corner + k) * 2;
                let index = u16::from_le_bytes([bytes[at], bytes[at + 1]]) as usize;
                match (index < vertex_count).then(|| vertex(index)).flatten() {
                    Some(v) => *slot = v,
                    None => {
                        valid = false;
                        break 'corners;
                    }
                }
            }
            mesh_triangles.push(triangle);
        }
        if valid {
            triangles.extend(mesh_triangles);
        } else {
            complete = false;
        }
    }
    (triangles, complete)
}

struct Parsed {
    data: usize,
    length: usize,
    triangles: Arc<Vec<Triangle>>,
    /// Every mesh decoded; otherwise the file is parsed again after `RETRY_AFTER`.
    complete: bool,
    at: std::time::Instant,
}

/// A file that did not fully decode (still loading, or read short) is tried again this often.
const RETRY_AFTER: std::time::Duration = std::time::Duration::from_secs(2);

/// Parsed files, kept while they stay loaded at the same address.
static PARSED: Mutex<Vec<Parsed>> = Mutex::new(Vec::new());

/// Triangles of every loaded file, each file parsed once.
fn all_triangles() -> Vec<Arc<Vec<Triangle>>> {
    let files = loaded_files();
    let mut parsed = PARSED.lock().unwrap_or_else(|e| e.into_inner());
    parsed.retain(|p| {
        files
            .iter()
            .any(|f| f.data == p.data && f.length == p.length)
    });
    // Drop incomplete results that are due a retry.
    parsed.retain(|p| p.complete || p.at.elapsed() < RETRY_AFTER);
    for file in &files {
        if !parsed
            .iter()
            .any(|p| p.data == file.data && p.length == file.length)
        {
            let (triangles, complete) = parse_raw(&bytes(file, file.length));
            if !complete {
                log::info!(
                    "Terrain {:#x}: {} bytes decoded partly ({} triangles), retrying",
                    file.hash,
                    file.length,
                    triangles.len()
                );
            }
            parsed.push(Parsed {
                data: file.data,
                length: file.length,
                triangles: Arc::new(triangles),
                complete,
                at: std::time::Instant::now(),
            });
        }
    }
    parsed.iter().map(|p| p.triangles.clone()).collect()
}

/// Squared distance from `point` to the bounding box of `triangle`: 0 inside it. Large floor
/// triangles whose corners are all far away still count as near when you stand on them.
pub fn box_distance2(triangle: &Triangle, point: [f32; 3]) -> f32 {
    (0..3)
        .map(|k| {
            let low = triangle[0][k].min(triangle[1][k]).min(triangle[2][k]);
            let high = triangle[0][k].max(triangle[1][k]).max(triangle[2][k]);
            let d = (low - point[k]).max(point[k] - high).max(0.0);
            d * d
        })
        .sum()
}

/// Up to `max` triangles whose bounding box comes within `radius` of `center`, and how many
/// triangles are loaded in all.
pub fn triangles_near(center: [f32; 3], radius: f32, max: usize) -> (Vec<Triangle>, usize) {
    let radius2 = radius * radius;
    let near = |triangle: &Triangle| box_distance2(triangle, center) <= radius2;
    let mut out = Vec::new();
    let mut total = 0;
    for file in all_triangles() {
        total += file.len();
        for triangle in file.iter() {
            if out.len() < max && near(triangle) {
                out.push(*triangle);
            }
        }
    }
    (out, total)
}

/// Up to `max` bytes of a loaded file, read in chunks.
pub fn bytes(file: &CollisionFile, max: usize) -> Vec<u8> {
    const CHUNK: usize = 4096;
    let length = file.length.min(max);
    let mut out = Vec::with_capacity(length);
    let mut offset = 0;
    while offset + CHUNK <= length {
        let Some(chunk) = read_process_value::<[u8; CHUNK]>((file.data + offset) as *const _)
        else {
            return out;
        };
        out.extend_from_slice(&chunk);
        offset += CHUNK;
    }
    // The tail: a whole chunk could run past the end of the buffer into unmapped memory.
    while offset + 64 <= length {
        let Some(chunk) = read_process_value::<[u8; 64]>((file.data + offset) as *const _) else {
            return out;
        };
        out.extend_from_slice(&chunk);
        offset += 64;
    }
    while offset < length {
        let Some(byte) = read_process_value::<u8>((file.data + offset) as *const u8) else {
            return out;
        };
        out.push(byte);
        offset += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A file with one mesh: a single triangle.
    fn one_triangle() -> Vec<u8> {
        let mut b = vec![0u8; 0x100];
        b[..4].copy_from_slice(b"COL2");
        b[0x10..0x14].copy_from_slice(&0x40u32.to_le_bytes());
        b[0x14..0x18].copy_from_slice(&1u32.to_le_bytes());
        // Mesh entry at 0x40 -> block at 0x60.
        b[0x4C..0x50].copy_from_slice(&0x60u32.to_le_bytes());
        for (i, v) in [0xFFFF_FFFFu32, 0x80, 3, 0xB0, 3, 0].iter().enumerate() {
            b[0x60 + i * 4..0x64 + i * 4].copy_from_slice(&v.to_le_bytes());
        }
        let vertices = [
            [1.0f32, 2.0, 3.0, 1.0],
            [4.0, 5.0, 6.0, 1.0],
            [7.0, 8.0, 9.0, 1.0],
        ];
        for (i, v) in vertices.iter().enumerate() {
            for (k, x) in v.iter().enumerate() {
                let at = 0x80 + i * 16 + k * 4;
                b[at..at + 4].copy_from_slice(&x.to_le_bytes());
            }
        }
        for (i, index) in [0u16, 1, 2].iter().enumerate() {
            b[0xB0 + i * 2..0xB2 + i * 2].copy_from_slice(&index.to_le_bytes());
        }
        b
    }

    #[test]
    fn big_triangles_under_you_are_near() {
        let floor = [
            [-100.0, 0.0, -100.0],
            [100.0, 0.0, -100.0],
            [0.0, 0.0, 100.0],
        ];
        assert_eq!(box_distance2(&floor, [0.0, 1.0, 0.0]), 1.0);
        assert!(box_distance2(&floor, [0.0, 50.0, 0.0]) > 15.0 * 15.0);
    }

    #[test]
    fn parses_a_mesh() {
        assert_eq!(
            parse_raw(&one_triangle()).0,
            vec![[[1.0, 2.0, 3.0], [4.0, 5.0, 6.0], [7.0, 8.0, 9.0]]]
        );
    }

    #[test]
    fn skips_out_of_range_indices() {
        let mut b = one_triangle();
        b[0xB0..0xB2].copy_from_slice(&9u16.to_le_bytes());
        assert!(!parse_raw(&b).1);
        assert!(parse_raw(&b).0.is_empty());
        assert!(parse(b"NOPE").is_empty());
    }
}
