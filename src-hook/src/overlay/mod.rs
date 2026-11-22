//! In-game overlay rendered with Dear ImGui through a DX11 `IDXGISwapChain::Present` hook.
//!
//! The game only links `d3d11.dll`, so the DX11 backend of `hudhook` is the only one enabled.
//! The overlay runs entirely on the game's render thread; anything it displays must be read
//! from shared state without blocking.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use hudhook::hooks::dx11::ImguiDx11Hooks;
use hudhook::imgui::{self, Condition, WindowFlags};
use hudhook::{Hudhook, ImguiRenderLoop, MessageFilter, RenderContext};
use log::{info, warn};
use protocol::{Message, OverlaySettings, OverlayUiState, OverlayWindow};

mod config;
mod cursor;
mod damage_taken;
mod dinput;
mod enemy_hp;
mod feed;
mod fonts;
mod hotkey;
mod keyboard;
mod message_loop;
mod meter;
mod settings_menu;

/// Presence of this file in the log directory disables the overlay at startup, so users can
/// recover from a renderer incompatibility without a new build.
const DISABLE_FILE_NAME: &str = "overlay-disabled";

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

/// In-game font selection. Changes are applied by rebuilding the atlas before a frame.
#[derive(Default)]
struct FontState {
    /// What the atlas currently uses, shown in the menu.
    description: String,
    /// Last problem with a chosen font.
    error: Option<String>,
    /// Rebuild the atlas before the next frame.
    reload: bool,
    /// Size shown by the slider; applied when the slider is released.
    size_draft: f32,
    /// Font files offered in the combo, listed when the menu first needs them.
    choices: Option<Vec<fonts::FontChoice>>,
    /// Pending "Open" dialog.
    browse: Option<fonts::FontBrowser>,
}

/// How often the overlay state is re-sent even without changes, so an app that connected later
/// still learns it.
const UI_STATE_RESEND: Duration = Duration::from_secs(2);

struct OverlayRenderLoop {
    menu_open: bool,
    /// Reports the overlay state to the app (tray check marks).
    tx: crate::event::Tx,
    last_ui_state: Option<(OverlayUiState, Instant)>,
    menu_hotkey: hotkey::HotkeyWatcher,
    /// Recording a new menu hotkey.
    key_capture: Option<hotkey::KeyCapture>,
    font: FontState,
    meter: meter::MeterWindow,
    enemy_hp: enemy_hp::EnemyHpWindow,
    damage_taken: damage_taken::DamageTakenWindow,
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
            // Open on start, so the menu (and its hotkey) is easy to discover.
            menu_open: true,
            tx: tx.clone(),
            last_ui_state: None,
            menu_hotkey: hotkey::HotkeyWatcher::default(),
            key_capture: None,
            font: FontState {
                size_draft: config.font.size,
                ..FontState::default()
            },
            meter: meter::MeterWindow::new(tx.clone()),
            enemy_hp: enemy_hp::EnemyHpWindow::default(),
            damage_taken: damage_taken::DamageTakenWindow::default(),
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
                self.render_hotkey_settings(ui);
                ui.separator();
                self.render_window_toggles(ui);
                ui.separator();
                ui.text("伤害统计");
                self.render_meter_settings(ui);
                ui.separator();
                ui.text("敌人血量");
                self.render_enemy_hp_settings(ui);
                ui.separator();
                ui.text("承受伤害");
                self.render_damage_taken_settings(ui);
                ui.separator();
                self.settings_editor.render(ui, settings);
                ui.separator();
                self.render_font_settings(ui);
                ui.separator();
                ui.text_disabled(format!(
                    "按 {} 关闭菜单，也可在托盘菜单中打开/关闭",
                    self.config.menu_hotkey.label()
                ));
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

    /// One checkbox per independent window; the tray mirrors them.
    fn render_window_toggles(&mut self, ui: &imgui::Ui) {
        ui.text("显示的窗口");
        for (i, window) in OverlayWindow::ALL.into_iter().enumerate() {
            if i > 0 {
                ui.same_line();
            }
            ui.checkbox(window_label(window), self.config.window_shown_mut(window));
        }
    }

    fn render_damage_taken_settings(&mut self, ui: &imgui::Ui) {
        let taken = &mut self.config.damage_taken;
        let (min_width, max_width) = config::ENEMY_HP_WIDTH_RANGE;
        ui.slider_config("宽度##damage_taken", min_width, max_width)
            .display_format("%.0f px")
            .build(&mut taken.width);
        let (min_scale, max_scale) = config::METER_SCALE_RANGE;
        ui.slider_config("缩放##damage_taken", min_scale, max_scale)
            .display_format("%.2fx")
            .build(&mut taken.scale);
        if ui.button("重置位置和大小##damage_taken") {
            *taken = config::DamageTakenConfig::default();
            self.damage_taken.reset_position();
        }
    }

    fn render_enemy_hp_settings(&mut self, ui: &imgui::Ui) {
        let enemy = &mut self.config.enemy_hp;
        let (min_width, max_width) = config::ENEMY_HP_WIDTH_RANGE;
        ui.slider_config("宽度##enemy_hp", min_width, max_width)
            .display_format("%.0f px")
            .build(&mut enemy.width);
        let (min_scale, max_scale) = config::METER_SCALE_RANGE;
        ui.slider_config("缩放##enemy_hp", min_scale, max_scale)
            .display_format("%.2fx")
            .build(&mut enemy.scale);
        let (min_count, max_count) = config::ENEMY_HP_COUNT_RANGE;
        ui.slider_config("显示数量##enemy_hp", min_count, max_count)
            .build(&mut enemy.max_enemies);
        if ui.button("重置位置和大小##enemy_hp") {
            *enemy = config::EnemyHpConfig::default();
            self.enemy_hp.reset_position();
        }
    }

    /// Shows the menu hotkey and lets the user record a new one.
    fn render_hotkey_settings(&mut self, ui: &imgui::Ui) {
        ui.text(format!("菜单快捷键: {}", self.config.menu_hotkey.label()));
        ui.same_line();
        if self.key_capture.is_some() {
            if ui.button("取消") {
                self.key_capture = None;
            }
            ui.text_colored(
                [1.0, 0.85, 0.4, 1.0],
                "请按下新的快捷键（可加 Ctrl/Shift/Alt，Esc 取消）",
            );
        } else if ui.button("更改") {
            self.key_capture = Some(hotkey::KeyCapture::start());
        }
    }

    /// Font picker: system/user font files, a file browser and the glyph size.
    fn render_font_settings(&mut self, ui: &imgui::Ui) {
        ui.text("字体");
        ui.text_disabled(format!("当前: {}", self.font.description));

        let choices = self.font.choices.get_or_insert_with(fonts::discover_fonts);
        let current = self.config.font.path.as_deref();
        let preview = match current {
            None => "系统默认 (微软雅黑)".to_string(),
            Some(path) => Path::new(path).file_name().map_or_else(
                || path.to_string(),
                |name| name.to_string_lossy().into_owned(),
            ),
        };
        let mut picked: Option<Option<PathBuf>> = None;
        if let Some(_combo) = ui.begin_combo("字体文件", &preview) {
            if ui
                .selectable_config("系统默认 (微软雅黑)")
                .selected(current.is_none())
                .build()
            {
                picked = Some(None);
            }
            for choice in choices.iter() {
                let selected = current.is_some_and(|path| Path::new(path) == choice.path);
                if ui
                    .selectable_config(&choice.label)
                    .selected(selected)
                    .build()
                {
                    picked = Some(Some(choice.path.clone()));
                }
            }
        }
        ui.same_line();
        if self.font.browse.is_none() && ui.button("浏览…") {
            self.font.browse = Some(fonts::FontBrowser::open());
        }
        if let Some(browser) = &self.font.browse {
            match browser.poll() {
                Some(result) => {
                    self.font.browse = None;
                    if let Some(path) = result {
                        picked = Some(Some(path));
                    }
                }
                None => ui.text_disabled("请在弹出的对话框中选择字体文件…"),
            }
        }
        if let Some(path) = picked {
            self.choose_font(path);
        }

        let (min_size, max_size) = fonts::FONT_SIZE_RANGE;
        ui.slider_config("字号", min_size, max_size)
            .display_format("%.0f px")
            .build(&mut self.font.size_draft);
        // Rebuilding the atlas takes a moment, so only apply once the slider is released.
        if ui.is_item_deactivated_after_edit() && self.font.size_draft != self.config.font.size {
            self.config.font.size = self.font.size_draft;
            self.font.reload = true;
        }

        if let Some(error) = &self.font.error {
            ui.text_colored([1.0, 0.45, 0.45, 1.0], error);
        }
    }

    /// Validates a chosen font before using it: stb_truetype aborts the game on fonts it
    /// cannot read, so unsupported files never reach the atlas.
    fn choose_font(&mut self, path: Option<PathBuf>) {
        if let Some(path) = &path {
            match std::fs::read(path) {
                Ok(bytes) if fonts::is_supported_font(&bytes) => {}
                Ok(_) => {
                    self.font.error = Some(format!(
                        "无法使用 {}: 仅支持 TrueType 轮廓的 .ttf / .ttc 字体",
                        path.display()
                    ));
                    return;
                }
                Err(error) => {
                    self.font.error = Some(format!("无法读取 {}: {error}", path.display()));
                    return;
                }
            }
        }
        self.font.error = None;
        self.config.font.path = path.map(|path| path.to_string_lossy().into_owned());
        self.font.reload = true;
    }

    /// Fills the atlas with the configured font (or the system default) and its fallbacks.
    fn fill_font_atlas(&mut self, ctx: &mut imgui::Context) {
        let custom = self.config.font.path.as_deref().map(Path::new);
        let (set, error) = fonts::FontSet::load(custom, self.config.font.size);
        if error.is_some() {
            self.font.error = error;
        }
        self.font.description = set.description.clone();
        let atlas = ctx.fonts();
        atlas.clear();
        atlas.add_font(&set.sources());
    }

    /// Rebuilds the atlas with the current font settings and uploads it as a new texture.
    fn rebuild_fonts(&mut self, ctx: &mut imgui::Context, render_context: &mut dyn RenderContext) {
        self.fill_font_atlas(ctx);
        let atlas = ctx.fonts();
        let texture = atlas.build_rgba32_texture();
        match render_context.load_texture(texture.data, texture.width, texture.height) {
            Ok(id) => ctx.fonts().tex_id = id,
            Err(error) => {
                warn!("Could not upload the rebuilt font atlas: {error:?}");
                self.font.error = Some("字体纹理上传失败".to_string());
            }
        }
    }

    /// Sends the visible state to the app when it changes, and periodically so a newly
    /// connected app catches up.
    fn report_ui_state(&mut self) {
        let state = OverlayUiState {
            visible_windows: OverlayWindow::ALL
                .into_iter()
                .filter(|window| *self.config.window_shown(*window))
                .collect(),
            menu_open: self.menu_open,
            menu_hotkey: self.config.menu_hotkey.label(),
        };
        let due = self.last_ui_state.as_ref().map_or(true, |(last, sent)| {
            *last != state || sent.elapsed() >= UI_STATE_RESEND
        });
        if due {
            // Fails only while no app is connected.
            let _ = self.tx.send(Message::OverlayUiState(state.clone()));
            self.last_ui_state = Some((state, Instant::now()));
        }
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

        // hudhook builds and uploads the atlas right after `initialize`.
        self.fill_font_atlas(ctx);

        info!("Overlay render loop initialized");
    }

    fn before_render<'a>(
        &'a mut self,
        ctx: &mut imgui::Context,
        render_context: &'a mut dyn RenderContext,
    ) {
        // Fonts can only change between frames.
        if std::mem::take(&mut self.font.reload) {
            self.rebuild_fonts(ctx, render_context);
        }
        if let Some(capture) = &mut self.key_capture {
            match capture.poll() {
                Some(hotkey::CaptureResult::Captured(combo)) => {
                    self.config.menu_hotkey = combo;
                    // The new key is still down; don't let it close the menu right away.
                    self.menu_hotkey.suppress_until_released();
                    self.key_capture = None;
                }
                Some(hotkey::CaptureResult::Cancelled) => self.key_capture = None,
                None => {}
            }
        } else if self.menu_hotkey.pressed(&self.config.menu_hotkey) {
            self.menu_open = !self.menu_open;
        }
        // The tray menu can toggle both, e.g. for keyboards without the hotkey.
        if feed::take_menu_toggle() {
            self.menu_open = !self.menu_open;
            self.key_capture = None;
        }
        for window in feed::take_window_toggles() {
            let shown = self.config.window_shown_mut(window);
            *shown = !*shown;
        }
        if !self.menu_open {
            self.key_capture = None;
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
        if self.config.show_damage_taken {
            let state = self.snapshot.as_ref().map(|snapshot| &snapshot.state);
            let transparency = state.map_or(0.35, |state| {
                self.settings_editor.effective(&state.settings).transparency
            });
            self.damage_taken.render(
                ui,
                self.menu_open,
                &mut self.config.damage_taken,
                state,
                transparency,
            );
        }
        if self.config.show_enemy_hp {
            let state = self.snapshot.as_ref().map(|snapshot| &snapshot.state);
            let names = state.map_or(&[][..], |state| state.enemies.as_slice());
            let transparency = state.map_or(0.35, |state| {
                self.settings_editor.effective(&state.settings).transparency
            });
            self.enemy_hp.render(
                ui,
                self.menu_open,
                &mut self.config.enemy_hp,
                names,
                transparency,
            );
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
        self.report_ui_state();
    }

    fn message_filter(&self, _io: &imgui::Io) -> MessageFilter {
        if self.menu_open {
            MessageFilter::InputAll
        } else {
            MessageFilter::empty()
        }
    }
}

/// Menu label of a window's visibility checkbox.
fn window_label(window: OverlayWindow) -> &'static str {
    match window {
        OverlayWindow::Meter => "伤害统计",
        OverlayWindow::EnemyHp => "敌人血量",
        OverlayWindow::DamageTaken => "承受伤害",
    }
}
