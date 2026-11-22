//! In-game overlay rendered with Dear ImGui through a DX11 `IDXGISwapChain::Present` hook.
//!
//! The game only links `d3d11.dll`, so the DX11 backend of `hudhook` is the only one enabled.
//! The overlay runs entirely on the game's render thread; anything it displays must be read
//! from shared state without blocking.

use std::path::PathBuf;

use hudhook::hooks::dx11::ImguiDx11Hooks;
use hudhook::imgui::{self, Condition, FontConfig, FontGlyphRanges, FontSource, WindowFlags};
use hudhook::windows::Win32::UI::Input::KeyboardAndMouse::VK_INSERT;
use hudhook::{Hudhook, ImguiRenderLoop, MessageFilter, RenderContext};
use log::{info, warn};
use protocol::OverlaySettings;

mod config;
mod cursor;
mod dinput;
mod feed;
mod keyboard;
mod message_loop;
mod meter;
mod settings_menu;

/// Presence of this file in the log directory disables the overlay at startup, so users can
/// recover from a renderer incompatibility without a new build.
const DISABLE_FILE_NAME: &str = "overlay-disabled";

const FONT_SIZE: f32 = 18.0;

/// System CJK fonts tried in order. Player names and translated skill names need CJK glyphs,
/// which the ImGui built-in font does not contain. Index 0 of `msyh.ttc` is Microsoft YaHei.
const FONT_CANDIDATES: &[&str] = &[
    "msyh.ttc",
    "msyh.ttf",
    "simhei.ttf",
    "YuGothM.ttc",
    "meiryo.ttc",
];

fn is_disabled_by_user() -> bool {
    config::data_dir().is_some_and(|dir| dir.join(DISABLE_FILE_NAME).exists())
}

/// Installs the Present hook. Errors are logged and never propagate: damage tracking must keep
/// working even when the overlay cannot attach.
pub fn setup(tx: crate::event::Tx) {
    if is_disabled_by_user() {
        info!("Overlay disabled by {DISABLE_FILE_NAME} marker file");
        return;
    }

    // Each group releases one part of the game's input while the menu is open. A failure only
    // costs that part; the overlay and damage tracking keep working.
    let input_hooks: [(&str, fn() -> anyhow::Result<()>); 4] = [
        ("cursor", cursor::install),
        ("keyboard", keyboard::install),
        ("message", message_loop::install),
        ("DirectInput", dinput::install),
    ];
    for (name, install) in input_hooks {
        if let Err(error) = install() {
            warn!("Overlay {name} hooks unavailable: {error:#}");
        }
    }

    // Meter snapshots from the desktop app. `setup` runs inside the hook's tokio runtime.
    feed::spawn();

    // `get_target_addrs` inside hudhook creates a dummy device and panics if that fails.
    // Isolate it on its own thread so a panic can only take the overlay down.
    let result = std::thread::Builder::new()
        .name("overlay-setup".into())
        .spawn(|| {
            Hudhook::builder()
                .with::<ImguiDx11Hooks>(OverlayRenderLoop::new(tx))
                .build()
                .apply()
        })
        .map(|handle| handle.join());

    match result {
        Ok(Ok(Ok(()))) => info!("Overlay Present hook installed"),
        Ok(Ok(Err(status))) => warn!("Overlay hook could not be applied: {status:?}"),
        Ok(Err(_)) => warn!("Overlay setup panicked; overlay disabled"),
        Err(error) => warn!("Could not spawn overlay setup thread: {error}"),
    }
}

/// Merged into the CJK font for Hangul, which Microsoft YaHei lacks.
const KOREAN_FONT: &str = "malgun.ttf";

fn fonts_dir() -> PathBuf {
    std::env::var_os("WINDIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"))
        .join("Fonts")
}

fn load_cjk_font() -> Option<(Vec<u8>, String)> {
    let fonts_dir = fonts_dir();
    FONT_CANDIDATES.iter().find_map(|name| {
        let path = fonts_dir.join(name);
        // imgui-rs copies TTF data into atlas-owned memory, so this buffer can be dropped.
        let bytes = std::fs::read(&path).ok()?;
        Some((bytes, path.display().to_string()))
    })
}

/// Edge-triggered hotkey polled every frame. Polling avoids depending on the game's window
/// messages, which may be consumed by its own input layer.
struct Hotkey {
    vk: i32,
    was_down: bool,
}

impl Hotkey {
    const fn new(vk: i32) -> Self {
        Self {
            vk,
            was_down: false,
        }
    }

    fn pressed(&mut self) -> bool {
        // Bypass the overlay's own keyboard blocking, which is active while the menu is open.
        let down = keyboard::real_async_key_state(self.vk) as u16 & 0x8000 != 0;
        let pressed = down && !self.was_down;
        self.was_down = down;
        pressed
    }
}

struct OverlayRenderLoop {
    menu_open: bool,
    menu_hotkey: Hotkey,
    font_source: Option<String>,
    meter: meter::MeterWindow,
    settings_editor: settings_menu::SettingsEditor,
    /// Latest snapshot from the app; `None` until it connects.
    snapshot: Option<feed::Snapshot>,
    config: config::OverlayConfig,
    /// What is on disk; compared against `config` to save only real changes.
    saved_config: config::OverlayConfig,
}

impl OverlayRenderLoop {
    fn new(tx: crate::event::Tx) -> Self {
        let config = config::load();
        Self {
            menu_open: false,
            menu_hotkey: Hotkey::new(VK_INSERT.0 as i32),
            font_source: None,
            meter: meter::MeterWindow::new(tx.clone()),
            settings_editor: settings_menu::SettingsEditor::new(tx),
            snapshot: None,
            saved_config: config.clone(),
            config,
        }
    }

    fn apply_style(ctx: &mut imgui::Context) {
        let style = ctx.style_mut();
        style.use_dark_colors();
        style.window_rounding = 6.0;
        style.frame_rounding = 4.0;
        style.grab_rounding = 4.0;
        style.window_border_size = 0.0;
        style.window_padding = [8.0, 6.0];
        style.item_spacing = [6.0, 4.0];
    }

    fn render_menu(&mut self, ui: &imgui::Ui, settings: Option<&OverlaySettings>) {
        let mut open = self.menu_open;
        ui.window("GBFR Logs 设置")
            .position([40.0, 120.0], Condition::FirstUseEver)
            .flags(WindowFlags::ALWAYS_AUTO_RESIZE)
            .opened(&mut open)
            .build(|| {
                ui.checkbox("显示伤害统计", &mut self.config.show_meter);
                self.render_meter_settings(ui);
                ui.separator();
                self.settings_editor.render(ui, settings);
                ui.separator();
                ui.text_disabled(match &self.font_source {
                    Some(path) => format!("字体: {path}"),
                    None => "字体: ImGui 默认 (未找到中文字体)".to_string(),
                });
                ui.text_disabled("按 Insert 关闭菜单");
                ui.separator();
                ui.text_disabled("GBFR Logs, maintained and modified by ChickenSeller.");
                ui.text_disabled(
                    "Based on GBFR Logs Awa Edition by onelittlechildawa and GBFR Logs by False Spring.",
                );
            });
        self.menu_open = open;
    }

    /// Sliders apply on the next frame, so the meter resizes while they are dragged.
    fn render_meter_settings(&mut self, ui: &imgui::Ui) {
        let meter = &mut self.config.meter;
        let (min_width, max_width) = config::METER_WIDTH_RANGE;
        ui.slider_config("宽度", min_width, max_width)
            .display_format("%.0f px")
            .build(&mut meter.width);
        let (min_scale, max_scale) = config::METER_SCALE_RANGE;
        ui.slider_config("缩放", min_scale, max_scale)
            .display_format("%.2fx")
            .build(&mut meter.scale);
        if ui.button("重置位置和大小") {
            *meter = config::MeterConfig {
                sort: meter.sort,
                ..config::MeterConfig::default()
            };
            self.meter.reset_position();
        }
        ui.text_disabled("菜单打开时可直接拖动伤害统计窗口");
    }

    /// Settings only change while the menu is open, so saving once it is closed writes each
    /// round of edits a single time.
    fn save_if_changed(&mut self) {
        if !self.menu_open && self.config != self.saved_config {
            config::save(&self.config);
            self.saved_config = self.config.clone();
        }
    }
}

impl ImguiRenderLoop for OverlayRenderLoop {
    fn initialize<'a>(
        &'a mut self,
        ctx: &mut imgui::Context,
        _render_context: &'a mut dyn RenderContext,
    ) {
        // Never write imgui.ini into the game directory.
        ctx.set_ini_filename(None::<PathBuf>);
        Self::apply_style(ctx);

        match load_cjk_font() {
            Some((data, path)) => {
                let korean = std::fs::read(fonts_dir().join(KOREAN_FONT)).ok();
                let mut sources = vec![FontSource::TtfData {
                    data: &data,
                    size_pixels: FONT_SIZE,
                    config: Some(FontConfig {
                        glyph_ranges: FontGlyphRanges::chinese_full(),
                        oversample_h: 1,
                        oversample_v: 1,
                        name: Some("CJK".into()),
                        ..FontConfig::default()
                    }),
                }];
                // imgui-rs merges every source after the first into it.
                if let Some(korean) = &korean {
                    sources.push(FontSource::TtfData {
                        data: korean,
                        size_pixels: FONT_SIZE,
                        config: Some(FontConfig {
                            glyph_ranges: FontGlyphRanges::korean(),
                            oversample_h: 1,
                            oversample_v: 1,
                            ..FontConfig::default()
                        }),
                    });
                }
                ctx.fonts().add_font(&sources);
                info!("Overlay font loaded from {path}");
                self.font_source = Some(path);
            }
            None => {
                warn!("No CJK system font found; overlay falls back to the ImGui default font");
                ctx.fonts()
                    .add_font(&[FontSource::DefaultFontData { config: None }]);
            }
        }

        info!("Overlay render loop initialized");
    }

    fn before_render<'a>(
        &'a mut self,
        ctx: &mut imgui::Context,
        _render_context: &'a mut dyn RenderContext,
    ) {
        if self.menu_hotkey.pressed() {
            self.menu_open = !self.menu_open;
        }
        // Also covers the menu being closed through its title-bar button during `render`.
        cursor::set_captured(self.menu_open);
        dinput::set_blocked(self.menu_open);
        keyboard::set_blocked(self.menu_open);
        message_loop::set_blocked(self.menu_open);
        keyboard::sync_modifiers(ctx.io_mut());
        // The game hides the OS cursor; let ImGui draw one while the menu takes input.
        ctx.io_mut().mouse_draw_cursor = self.menu_open;
    }

    fn render(&mut self, ui: &mut imgui::Ui) {
        feed::refresh(&mut self.snapshot);

        if self.config.show_meter {
            let state = self.snapshot.as_ref().map(|snapshot| &snapshot.state);
            let values = state.map(|state| self.settings_editor.effective(&state.settings).clone());
            let view = state
                .zip(values.as_ref())
                .map(|(state, values)| meter::MeterView { state, values });
            self.meter
                .render(ui, self.menu_open, &mut self.config.meter, view);
        }
        if self.menu_open {
            // A copy, so the menu can borrow the loop mutably while editing.
            let settings = self
                .snapshot
                .as_ref()
                .map(|snapshot| snapshot.state.settings.clone());
            self.render_menu(ui, settings.as_ref());
        }
        self.save_if_changed();
    }

    fn message_filter(&self, _io: &imgui::Io) -> MessageFilter {
        if self.menu_open {
            MessageFilter::InputAll
        } else {
            MessageFilter::empty()
        }
    }
}
