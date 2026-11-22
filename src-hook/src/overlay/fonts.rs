//! Overlay fonts: the system default, user-chosen font files and live atlas rebuilds.
//!
//! Dear ImGui rasterises with stb_truetype, which only understands TrueType outlines
//! (`glyf`). Feeding it anything else (CFF-based `.otf`, a corrupt file) makes the atlas
//! build fail and the C++ assert aborts the game, so every file is checked first.
//!
//! The chosen font is the base; system CJK fonts are merged behind it so glyphs it lacks
//! (Chinese, Japanese, Korean player names) still render. ImGui merging never replaces a
//! glyph the base font already has.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use hudhook::imgui::{FontConfig, FontGlyphRanges, FontSource};
use log::{info, warn};

pub const DEFAULT_FONT_SIZE: f32 = 18.0;
pub const FONT_SIZE_RANGE: (f32, f32) = (12.0, 32.0);

/// System fallbacks merged behind the base font: CJK first, then Hangul (Microsoft YaHei has
/// no Hangul).
const CJK_FALLBACKS: &[&str] = &[
    "msyh.ttc",
    "msyh.ttf",
    "simhei.ttf",
    "YuGothM.ttc",
    "meiryo.ttc",
];
const KOREAN_FALLBACK: &str = "malgun.ttf";

pub fn system_fonts_dir() -> PathBuf {
    std::env::var_os("WINDIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"))
        .join("Fonts")
}

/// Fonts installed for the current user only (Windows 10 1809+).
fn user_fonts_dir() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(|dir| PathBuf::from(dir).join(r"Microsoft\Windows\Fonts"))
}

/// Checks that `bytes` is a TrueType font (or a collection whose first face is one) with
/// `glyf` outlines, which is what stb_truetype can rasterise.
pub fn is_supported_font(bytes: &[u8]) -> bool {
    let read_u32 = |at: usize| -> Option<u32> {
        bytes
            .get(at..at + 4)
            .map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    };
    let read_u16 = |at: usize| -> Option<u16> {
        bytes
            .get(at..at + 2)
            .map(|b| u16::from_be_bytes([b[0], b[1]]))
    };

    let Some(tag) = read_u32(0) else {
        return false;
    };
    // A collection: check its first face.
    let face = if tag == u32::from_be_bytes(*b"ttcf") {
        match read_u32(12) {
            Some(offset) => offset as usize,
            None => return false,
        }
    } else {
        0
    };
    let Some(version) = read_u32(face) else {
        return false;
    };
    // 0x00010000 or 'true' are TrueType outlines; 'OTTO' is CFF and unsupported.
    if version != 0x0001_0000 && version != u32::from_be_bytes(*b"true") {
        return false;
    }
    let Some(tables) = read_u16(face + 4) else {
        return false;
    };
    let has = |wanted: &[u8; 4]| {
        (0..tables as usize).any(|index| {
            bytes.get(face + 12 + index * 16..face + 16 + index * 16) == Some(wanted.as_slice())
        })
    };
    has(b"glyf") && has(b"loca") && has(b"cmap")
}

/// A font file the user can pick.
#[derive(Clone)]
pub struct FontChoice {
    pub path: PathBuf,
    pub label: String,
}

/// TrueType fonts in the system and per-user font folders, sorted by file name. Only the
/// start of each file is read for the format check.
pub fn discover_fonts() -> Vec<FontChoice> {
    let mut choices = Vec::new();
    for dir in std::iter::once(system_fonts_dir()).chain(user_fonts_dir()) {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let extension = path
                .extension()
                .and_then(|e| e.to_str())
                .map(str::to_ascii_lowercase);
            if !matches!(extension.as_deref(), Some("ttf" | "ttc")) {
                continue;
            }
            if read_header(&path).is_some_and(|header| is_supported_font(&header)) {
                let label = path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default();
                choices.push(FontChoice { path, label });
            }
        }
    }
    choices.sort_by_key(|choice| choice.label.to_lowercase());
    choices.dedup_by(|a, b| a.label.eq_ignore_ascii_case(&b.label));
    choices
}

/// Enough of a font to see its table directory (and a collection's first face).
fn read_header(path: &Path) -> Option<Vec<u8>> {
    use std::io::Read;
    let mut file = std::fs::File::open(path).ok()?;
    let mut header = vec![0; 4096];
    let read = file.read(&mut header).ok()?;
    header.truncate(read);
    Some(header)
}

/// Font data to put into the atlas: the base face and the fallbacks merged behind it.
pub struct FontSet {
    base: Option<Vec<u8>>,
    cjk: Option<Vec<u8>>,
    korean: Option<Vec<u8>>,
    size: f32,
    /// Shown in the menu.
    pub description: String,
}

impl FontSet {
    /// Loads `custom` (if any and valid) plus the system fallbacks. An unusable custom font
    /// is reported in `error` and the system default is used instead.
    pub fn load(custom: Option<&Path>, size: f32) -> (Self, Option<String>) {
        let fonts_dir = system_fonts_dir();
        let cjk = CJK_FALLBACKS.iter().find_map(|name| {
            let bytes = std::fs::read(fonts_dir.join(name)).ok()?;
            is_supported_font(&bytes).then_some(bytes)
        });
        let korean = std::fs::read(fonts_dir.join(KOREAN_FALLBACK))
            .ok()
            .filter(|bytes| is_supported_font(bytes));

        let mut error = None;
        let base = custom.and_then(|path| match std::fs::read(path) {
            Ok(bytes) if is_supported_font(&bytes) => Some(bytes),
            Ok(_) => {
                error = Some(format!("不支持的字体格式: {}", path.display()));
                None
            }
            Err(read_error) => {
                error = Some(format!("无法读取字体: {} ({read_error})", path.display()));
                None
            }
        });

        let description = match (&base, custom) {
            (Some(_), Some(path)) => path.display().to_string(),
            _ if cjk.is_some() => "系统默认 (微软雅黑)".to_string(),
            _ => "ImGui 默认 (未找到中文字体)".to_string(),
        };
        if let Some(message) = &error {
            warn!("Overlay font: {message}");
        }
        info!("Overlay font: {description}, {size}px");

        (
            Self {
                base,
                cjk,
                korean,
                size,
                description,
            },
            error,
        )
    }

    /// Sources for `FontAtlas::add_font`; every source after the first is merged into it.
    pub fn sources(&self) -> Vec<FontSource<'_>> {
        let config = |ranges: FontGlyphRanges| {
            Some(FontConfig {
                glyph_ranges: ranges,
                oversample_h: 1,
                oversample_v: 1,
                ..FontConfig::default()
            })
        };
        let mut sources = Vec::new();
        if let Some(base) = &self.base {
            sources.push(FontSource::TtfData {
                data: base,
                size_pixels: self.size,
                config: config(FontGlyphRanges::chinese_full()),
            });
        }
        if let Some(cjk) = &self.cjk {
            sources.push(FontSource::TtfData {
                data: cjk,
                size_pixels: self.size,
                config: config(FontGlyphRanges::chinese_full()),
            });
        }
        if let Some(korean) = &self.korean {
            sources.push(FontSource::TtfData {
                data: korean,
                size_pixels: self.size,
                config: config(FontGlyphRanges::korean()),
            });
        }
        if sources.is_empty() {
            sources.push(FontSource::DefaultFontData {
                config: Some(FontConfig {
                    size_pixels: self.size,
                    ..FontConfig::default()
                }),
            });
        }
        sources
    }
}

/// A Windows "Open" dialog running on its own thread, so the game keeps rendering while it
/// is open. A shared slot instead of a channel keeps the render loop `Sync`, as hudhook
/// requires.
pub struct FontBrowser(Arc<Mutex<Option<Option<PathBuf>>>>);

impl FontBrowser {
    pub fn open() -> Self {
        let slot = Arc::new(Mutex::new(None));
        let result = Arc::clone(&slot);
        std::thread::spawn(move || {
            let chosen = open_font_dialog();
            *result.lock().unwrap_or_else(|e| e.into_inner()) = Some(chosen);
        });
        Self(slot)
    }

    /// `None` while the dialog is open; then the chosen path, or `Some(None)` if cancelled.
    pub fn poll(&self) -> Option<Option<PathBuf>> {
        self.0.try_lock().ok()?.take()
    }
}

fn open_font_dialog() -> Option<PathBuf> {
    use windows::core::PCWSTR;
    use windows::Win32::UI::Controls::Dialogs::{
        GetOpenFileNameW, OFN_FILEMUSTEXIST, OFN_NOCHANGEDIR, OFN_PATHMUSTEXIST, OPENFILENAMEW,
    };

    let wide = |text: &str| text.encode_utf16().collect::<Vec<u16>>();
    // Pairs of "label\0pattern\0", terminated by an extra \0.
    let filter = wide("字体文件 (*.ttf;*.ttc)\0*.ttf;*.ttc\0所有文件 (*.*)\0*.*\0\0");
    let title: Vec<u16> = wide("选择字体文件\0");
    let initial_dir: Vec<u16> = system_fonts_dir()
        .as_os_str()
        .to_string_lossy()
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let mut buffer = vec![0u16; 1024];

    let mut dialog = OPENFILENAMEW {
        lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
        lpstrFilter: PCWSTR(filter.as_ptr()),
        lpstrFile: windows::core::PWSTR(buffer.as_mut_ptr()),
        nMaxFile: buffer.len() as u32,
        lpstrTitle: PCWSTR(title.as_ptr()),
        lpstrInitialDir: PCWSTR(initial_dir.as_ptr()),
        Flags: OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR,
        ..Default::default()
    };

    // No owner window: the game's window belongs to another thread, and tying a modal
    // dialog to it from here could stall the game.
    if unsafe { GetOpenFileNameW(&mut dialog) }.as_bool() {
        let length = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
        Some(PathBuf::from(String::from_utf16_lossy(&buffer[..length])))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn font_with_tables(version: &[u8; 4], tables: &[&[u8; 4]]) -> Vec<u8> {
        let mut bytes = version.to_vec();
        bytes.extend_from_slice(&(tables.len() as u16).to_be_bytes());
        bytes.extend_from_slice(&[0; 6]);
        for tag in tables {
            bytes.extend_from_slice(*tag);
            bytes.extend_from_slice(&[0; 12]);
        }
        bytes
    }

    #[test]
    fn accepts_truetype_outlines() {
        let font = font_with_tables(&[0, 1, 0, 0], &[b"cmap", b"glyf", b"loca"]);
        assert!(is_supported_font(&font));
    }

    #[test]
    fn rejects_cff_and_garbage() {
        assert!(!is_supported_font(&font_with_tables(
            b"OTTO",
            &[b"cmap", b"CFF "]
        )));
        assert!(!is_supported_font(&font_with_tables(
            &[0, 1, 0, 0],
            &[b"cmap"]
        )));
        assert!(!is_supported_font(b"not a font"));
        assert!(!is_supported_font(&[]));
    }

    #[test]
    fn checks_the_first_face_of_a_collection() {
        let face = font_with_tables(&[0, 1, 0, 0], &[b"cmap", b"glyf", b"loca"]);
        let mut collection = b"ttcf".to_vec();
        collection.extend_from_slice(&[0, 1, 0, 0, 0, 0, 0, 1]);
        collection.extend_from_slice(&16u32.to_be_bytes());
        collection.extend_from_slice(&face);
        assert!(is_supported_font(&collection));
    }
}
