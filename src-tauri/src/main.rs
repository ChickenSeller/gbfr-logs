// Prevents additional console window on Windows in release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::{
    collections::HashMap,
    fs::File,
    io::Write,
    path::Path,
};

use anyhow::Context;
use db::logs::LogEntry;
use dll_syringe::{process::OwnedProcess, Syringe};
use interprocess::os::windows::named_pipe::tokio::RecvPipeStream;
use log::{info, warn, LevelFilter};
use parser::{
    constants::{CharacterType, EnemyType},
    v1::{self, PlayerData},
};
use protocol::Message;
use rusqlite::params_from_iter;
use serde::{Deserialize, Serialize};
use tauri::{
    api::dialog::blocking::FileDialogBuilder, AppHandle, CustomMenuItem, LogicalSize, Manager,
    Size, State, SystemTray, SystemTrayEvent, SystemTrayMenu, SystemTrayMenuItem,
};
use tauri_plugin_log::LogTarget;
use tauri_plugin_window_state::{AppHandleExt, StateFlags};
use tokio_stream::StreamExt;
use tokio_util::codec::FramedRead;

mod db;
mod diagnostics;
mod overlay;
mod parser;

#[tauri::command]
async fn delete_all_logs() -> Result<(), String> {
    let conn = db::connect_to_db().map_err(|e| e.to_string())?;
    conn.execute("DELETE FROM logs", [])
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
fn export_damage_log_to_file(id: u32, options: ParseOptions) -> Result<(), String> {
    let file_path = FileDialogBuilder::new()
        .add_filter("csv", &["csv"])
        .set_file_name(&format!("{id}_damage_log.csv"))
        .set_title("Export Damage Log")
        .save_file()
        .ok_or("No file selected!")?;

    let conn = db::connect_to_db().map_err(|e| e.to_string())?;

    let mut stmt = conn
        .prepare("SELECT data, version FROM logs WHERE id = ?")
        .map_err(|e| e.to_string())?;

    let (blob, version): (Vec<u8>, u8) = stmt
        .query_row([id], |row| Ok((row.get(0)?, row.get(1)?)))
        .context("Failed to fetch log from database")
        .map_err(|e| e.to_string())?;

    let parser = parser::deserialize_version(&blob, version).map_err(|e| e.to_string())?;

    let file = File::create(file_path).map_err(|e| e.to_string())?;

    // @TODO(false): Split formatting into a separate function.
    let mut writer = std::io::BufWriter::new(file);

    writeln!(
        writer,
        "timestamp,source_type,child_source_type,source_index,target_type,target_index,action_id,flags,damage"
    )
    .map_err(|e| e.to_string())?;

    for (event_ts, event) in parser.encounter.event_log() {
        if let Message::DamageEvent(damage_event) = event {
            let timestamp = event_ts - parser.start_time();
            let target_type = EnemyType::from_hash(damage_event.target.parent_actor_type);
            let parent_character_type =
                CharacterType::from_hash(damage_event.source.parent_actor_type);
            let child_character_type = CharacterType::from_hash(damage_event.source.actor_type);

            if options.targets.is_empty() || options.targets.contains(&target_type) {
                writeln!(
                    writer,
                    "{},{},{},{},{},{},{},{},{}",
                    timestamp,
                    parent_character_type,
                    child_character_type,
                    damage_event.source.parent_index,
                    target_type,
                    damage_event.target.parent_index,
                    damage_event.action_id,
                    damage_event.flags,
                    damage_event.damage
                )
                .map_err(|e| e.to_string())?;
            }
        }
    }

    writer.flush().map_err(|e| e.to_string())?;

    Ok(())
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SearchResult {
    logs: Vec<LogEntry>,
    page: u32,
    page_count: u32,
    log_count: i32,
    /// IDs of the enemies that can be filtered by.
    enemy_ids: Vec<u32>,
    /// IDs of the quests that can be filtered by.
    quest_ids: Vec<u32>,
    /// Names of the Players that can be filtered by.
    player_ids: Vec<String>,
    /// Names of the Characters that can be filtered by.
    player_types: Vec<String>,
}

#[tauri::command]
fn fetch_logs(
    page: Option<u32>,
    filter_by_enemy_id: Option<u32>,
    filter_by_quest_id: Option<u32>,
    sort_direction: Option<String>,
    sort_type: Option<String>,
    quest_completed: Option<bool>,
    filter_by_player_id: Option<String>,
    filter_by_player_character: Option<String>,
) -> Result<SearchResult, String> {
    let conn = db::connect_to_db().map_err(|e| e.to_string())?;
    let page = page.unwrap_or(1);
    let per_page = 10;
    let offset = page.saturating_sub(1) * per_page;

    let sort_type_param = sort_type
        .map(|s| match s.as_str() {
            "time" => db::logs::SortType::Time,
            "duration" => db::logs::SortType::Duration,
            "quest-elapsed-time" => db::logs::SortType::QuestElapsedTime,
            _ => db::logs::SortType::Time,
        })
        .unwrap_or(db::logs::SortType::Time);

    let sort_direction_param = sort_direction
        .map(|s| match s.as_str() {
            "asc" => db::logs::SortDirection::Ascending,
            _ => db::logs::SortDirection::Descending,
        })
        .unwrap_or(db::logs::SortDirection::Descending);

    let logs = db::logs::get_logs(
        &conn,
        filter_by_enemy_id,
        filter_by_quest_id,
        per_page,
        offset,
        &sort_type_param,
        &sort_direction_param,
        quest_completed,
        &filter_by_player_id,
        &filter_by_player_character,
    )
    .map_err(|e| e.to_string())?;

    let log_count = db::logs::get_logs_count(
        &conn,
        filter_by_enemy_id,
        filter_by_quest_id,
        quest_completed,
        &filter_by_player_id,
        &filter_by_player_character,
    )
    .map_err(|e| e.to_string())?;

    let page_count = (log_count as f64 / per_page as f64).ceil() as u32;

    let mut enemy_ids = Vec::new();
    let mut quest_ids = Vec::new();
    let mut player_ids = Vec::new();
    let mut player_types = Vec::new();

    let mut query = conn
        .prepare("SELECT primary_target, quest_id, p1_name, p1_type, p2_name, p2_type, p3_name, p3_type, p4_name, p4_type from logs")
        .map_err(|e| e.to_string())?;

    let rows = query
        .query_map([], |row| {
            Ok((
                row.get::<usize, Option<u32>>(0)?,    // primary_target
                row.get::<usize, Option<u32>>(1)?,    // quest_id
                row.get::<usize, Option<String>>(2)?, // p1_name
                row.get::<usize, Option<String>>(3)?, // p1_type
                row.get::<usize, Option<String>>(4)?, // p2_name
                row.get::<usize, Option<String>>(5)?, // p2_type
                row.get::<usize, Option<String>>(6)?, // p3_name
                row.get::<usize, Option<String>>(7)?, // p3_type
                row.get::<usize, Option<String>>(8)?, // p4_name
                row.get::<usize, Option<String>>(9)?, // p4_type
            ))
        })
        .map_err(|e| e.to_string())?;

    for row in rows {
        let (
            primary_target,
            quest_id,
            p1_name,
            p1_type,
            p2_name,
            p2_type,
            p3_name,
            p3_type,
            p4_name,
            p4_type,
        ) = row.map_err(|e| e.to_string())?;

        if let Some(primary_target) = primary_target {
            if !enemy_ids.contains(&primary_target) {
                enemy_ids.push(primary_target);
            }
        }

        if let Some(quest_id) = quest_id {
            if !quest_ids.contains(&quest_id) {
                quest_ids.push(quest_id);
            }
        }

        for p_name in [p1_name, p2_name, p3_name, p4_name] {
            if let Some(p_name) = p_name {
                if !player_ids.contains(&p_name) {
                    player_ids.push(p_name)
                }
            }
        }

        for p_type in [p1_type, p2_type, p3_type, p4_type] {
            if let Some(p_type) = p_type {
                if !player_types.contains(&p_type) {
                    player_types.push(p_type)
                }
            }
        }
    }

    Ok(SearchResult {
        logs,
        page,
        page_count,
        log_count,
        enemy_ids,
        quest_ids,
        player_ids,
        player_types,
    })
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct EncounterStateResponse {
    encounter_state: v1::DerivedEncounterState,
    players: [Option<PlayerData>; 4],
    quest_id: Option<u32>,
    quest_timer: Option<u32>,
    quest_completed: bool,
    targets: Vec<EnemyType>,
    dps_chart: HashMap<u32, Vec<i32>>,
    sba_chart: HashMap<u32, Vec<f32>>,
    sba_events: Vec<(i64, protocol::Message)>,
    death_events: Vec<(i64, protocol::Message)>,
    chart_len: usize,
    sba_chart_len: usize,
}

#[derive(Debug, Deserialize)]
struct ParseOptions {
    targets: Vec<EnemyType>,
}

/// The full combat log of a saved encounter, for the log page's "Combat Log" tab.
#[tauri::command]
fn fetch_combat_log(id: u64) -> Result<Vec<parser::v1::combat_log::CombatLogEntry>, String> {
    let conn = db::connect_to_db().map_err(|e| e.to_string())?;
    let (blob, version): (Vec<u8>, u8) = conn
        .query_row("SELECT data, version FROM logs WHERE id = ?", [id], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .map_err(|e| e.to_string())?;
    let parser = parser::deserialize_version(&blob, version).map_err(|e| e.to_string())?;
    Ok(parser.combat_log())
}

#[tauri::command]
fn fetch_encounter_state(id: u64, options: ParseOptions) -> Result<EncounterStateResponse, String> {
    let conn = db::connect_to_db().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare("SELECT data, version FROM logs WHERE id = ?")
        .map_err(|e| e.to_string())?;

    let (blob, version): (Vec<u8>, u8) = stmt
        .query_row([id], |row| Ok((row.get(0)?, row.get(1)?)))
        .map_err(|e| e.to_string())?;

    // @TODO(false): If we deserialize from an older version, we should save it back into the DB as the newer format.
    let mut parser = parser::deserialize_version(&blob, version).map_err(|e| e.to_string())?;

    parser.reparse_with_options(&options.targets);

    let duration = parser.derived_state.duration();

    let mut player_dps: HashMap<u32, Vec<i32>> = HashMap::new();

    const DPS_INTERVAL: i64 = 3 * 1_000;
    const SBA_INTERVAL: i64 = 1_000;

    for player in parser.derived_state.party.values() {
        player_dps.insert(
            player.index,
            vec![0; (duration / DPS_INTERVAL) as usize + 1],
        );
    }

    let mut targets = Vec::new();
    let start_time = parser.start_time();

    for (timestamp, event) in parser.encounter.event_log() {
        match event {
            Message::DamageEvent(damage_event) => {
                let index = ((timestamp - start_time) / DPS_INTERVAL) as usize;
                let target_type = EnemyType::from_hash(damage_event.target.parent_actor_type);

                if !targets.contains(&target_type) {
                    targets.push(target_type);
                }

                if let Some(chart) = player_dps.get_mut(&damage_event.source.parent_index) {
                    // Check to see if the target is in the list of targets to filter by.
                    if options.targets.is_empty() || options.targets.contains(&target_type) {
                        chart[index] += damage_event.damage;
                    }
                }
            }
            _ => continue,
        }
    }

    let sba_chart = parser.generate_sba_chart(SBA_INTERVAL);

    let sba_events = parser
        .encounter
        .event_log()
        .filter(|(_, e)| {
            matches!(
                e,
                Message::OnContinueSBAChain(_)
                    | Message::OnAttemptSBA(_)
                    | Message::OnPerformSBA(_)
            )
        })
        .map(|(ts, e)| (*ts - start_time, e.clone()))
        .collect();

    let death_events = parser
        .encounter
        .event_log()
        .filter(|(_, e)| matches!(e, Message::OnDeathEvent(_)))
        .map(|(ts, e)| (*ts - start_time, e.clone()))
        .collect();

    Ok(EncounterStateResponse {
        encounter_state: parser.derived_state,
        players: parser.encounter.player_data,
        quest_id: parser.encounter.quest_id,
        quest_timer: parser.encounter.quest_timer,
        quest_completed: parser.encounter.quest_completed,
        dps_chart: player_dps,
        chart_len: (duration / DPS_INTERVAL) as usize + 1,
        sba_chart_len: (duration / SBA_INTERVAL) as usize + 1,
        sba_chart,
        sba_events,
        death_events,
        targets,
    })
}

#[tauri::command]
fn delete_logs(ids: Vec<u64>) -> Result<(), String> {
    let conn = db::connect_to_db().map_err(|e| e.to_string())?;

    let id_params: Vec<String> = ids.iter().map(|_| "?".to_string()).collect();
    let param = id_params.join(",");

    let sql = format!("DELETE FROM logs WHERE id IN ({})", param);
    let mut statement = conn.prepare_cached(&sql).map_err(|e| e.to_string())?;
    statement
        .execute(params_from_iter(ids))
        .map_err(|e| e.to_string())?;

    Ok(())
}

// Continuously check for the game process and inject the DLL when found.
async fn check_and_perform_hook(app: AppHandle) {
    loop {
        match OwnedProcess::find_first_by_name("granblue_fantasy_relink.exe") {
            Some(target) => {
                let syringe = Syringe::for_process(target);
                let debug_dll_path = Path::new("hook-dbg.dll");
                let mut dll_path = Path::new("hook.dll");

                // Only development builds may opt into the debug hook. A
                // stale debug DLL beside an installed release must never take
                // precedence over the packaged production hook.
                if cfg!(debug_assertions) && debug_dll_path.exists() {
                    dll_path = debug_dll_path;
                }

                info!("Found game process, injecting DLL: {:?}", dll_path);

                match syringe.inject(dll_path) {
                    Ok(_) => {
                        let _ = app.emit_all("success-alert", "Found game..");
                    }
                    Err(error) => {
                        // An older Hook may already be loaded in a game that
                        // stayed open during an app upgrade. Still try its pipe;
                        // the compatibility decoder below can consume it.
                        warn!("Could not inject Hook {:?}: {:?}", dll_path, error);
                        let _ = app.emit_all(
                            "error-alert",
                            "Hook injection failed; trying the existing game connection.",
                        );
                    }
                }

                connect_and_run_parser(app);

                break;
            }
            None => {
                tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
            }
        }
    }
}

// Connect to the game hook event channel and listen for damage events.
fn connect_and_run_parser(app: AppHandle) {
    // The logs window hosts the in-game overlay bridge, so it receives the meter events.
    let window = app.get_window("logs").expect("Logs window not found");

    let database = db::connect_to_db().expect("Could not connect to database");
    let mut state = v1::Parser::new(app.clone(), window.clone(), database);

    tauri::async_runtime::spawn(async move {
        loop {
            match RecvPipeStream::connect_by_path(protocol::PIPE_NAME).await {
                Ok(stream) => {
                    info!("Connected to game!");

                    let _ = app.emit_all("success-alert", "Connnected to game!");

                    let decoder = tokio_util::codec::LengthDelimitedCodec::new();
                    let mut reader = FramedRead::new(stream, decoder);
                    let mut inactivity_check =
                        tokio::time::interval(std::time::Duration::from_secs(1));
                    inactivity_check
                        .set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

                    loop {
                        tokio::select! {
                            message = reader.next() => {
                                let Some(Ok(msg)) = message else {
                                    break;
                                };

                                // Handle EOF when the game closes.
                                if msg.is_empty() {
                                    break;
                                }

                                match protocol::deserialize_message(&msg) {
                                    Ok(msg) => {
                                        match msg {
                                        // The hook sends taken damage as a plain DamageEvent too.
                                        protocol::Message::DamageEvent(event)
                                        | protocol::Message::DamageTaken(event) => {
                                            state.on_damage_event(event);
                                        }
                                        protocol::Message::DiagnosticsReport(report) => {
                                            diagnostics::deliver_probe(report);
                                        }
                                        protocol::Message::OverlayClearRequest => {
                                            state.clear_live_encounter();
                                        }
                                        protocol::Message::PartBroken(event) => {
                                            state.on_part_broken(event);
                                        }
                                        protocol::Message::StatusApplied(event) => {
                                            state.on_status_applied(event);
                                        }
                                        protocol::Message::StatusRemoved(event) => {
                                            state.on_status_removed(event);
                                        }
                                        protocol::Message::OnAreaEnter(event) => {
                                            state.on_area_enter_event(event);
                                        }
                                        protocol::Message::PlayerLoadEvent(event) => {
                                            state.on_player_load_event(event);
                                        }
                                        protocol::Message::PlayerIdentityEvent(event) => {
                                            state.on_player_identity_event(event);
                                        }
                                        protocol::Message::OnQuestComplete(event) => {
                                            state.on_quest_complete_event(event);
                                        }
                                        protocol::Message::OnUpdateSBA(event) => {
                                            state.on_sba_update(event);
                                        }
                                        protocol::Message::OnAttemptSBA(event) => {
                                            state.on_sba_attempt(event);
                                        }
                                        protocol::Message::OnPerformSBA(event) => {
                                            state.on_sba_perform(event);
                                        }
                                        protocol::Message::OnContinueSBAChain(event) => {
                                            state.on_continue_sba_chain(event);
                                        }
                                        protocol::Message::OnDeathEvent(event) => {
                                            state.on_death_event(event);
                                        }
                                        protocol::Message::OnBattleEnd => {
                                            state.on_battle_end_event();
                                        }
                                        // The meter window owns the export formats; this is
                                        // UI input, so it never reaches the parser or logs.
                                        protocol::Message::OverlayCopyRequest(request) => {
                                            let _ = window.emit("overlay-copy-request", request);
                                        }
                                        // In-game settings edits go to the settings store.
                                        protocol::Message::OverlaySettingsChange(values) => {
                                            let _ = window.emit("overlay-settings-change", values);
                                        }
                                        protocol::Message::OverlayUiState(ui_state) => {
                                            apply_overlay_ui_state(&app, ui_state);
                                        }
                                        }
                                    }
                                    Err(error) => {
                                        warn!(
                                            "Could not decode Hook message ({} bytes): {:?}",
                                            msg.len(),
                                            error
                                        );
                                    }
                                }
                            }
                            _ = inactivity_check.tick() => {
                                state.auto_save_if_inactive(chrono::Utc::now().timestamp_millis());
                            }
                        }
                    }

                    info!("Game has closed.");

                    // The game has closed, so we should go back to waiting for the game to reopen.
                    let _ = app.emit_all("error-alert", "Game has closed!");
                    break;
                }
                Err(_) => {
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                }
            }
        }

        // Check for the game process again.
        tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
        tauri::async_runtime::spawn(check_and_perform_hook(app));
    });
}

fn system_tray_with_menu() -> SystemTray {
    let logs = CustomMenuItem::new("open_logs", "Open Logs");
    let overlay_menu =
        CustomMenuItem::new("toggle_overlay_menu", TOGGLE_OVERLAY_MENU_DEFAULT).selected();
    let reset_windows = CustomMenuItem::new("reset_windows", "Reset Windows");
    let export_diagnostics = CustomMenuItem::new("export_diagnostics", "Export Diagnostics");
    let about = CustomMenuItem::new("about", "About");
    let quit = CustomMenuItem::new("quit", "Quit");

    let mut menu = SystemTrayMenu::new().add_item(logs);
    // One check item per in-game window, checked if it is shown by default.
    for window in protocol::OverlayWindow::ALL {
        let mut item = CustomMenuItem::new(window.tray_id(), window.default_label());
        if window.default_visible() {
            item = item.selected();
        }
        menu = menu.add_item(item);
    }
    let menu = menu
        .add_item(overlay_menu)
        .add_item(reset_windows)
        .add_item(export_diagnostics)
        .add_item(about)
        .add_native_item(SystemTrayMenuItem::Separator)
        .add_item(quit);

    SystemTray::new().with_menu(menu)
}

/// Translated labels for what the app draws natively (tray menu, window titles), sent by the
/// logs window whenever its language is set.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct NativeLabels {
    open_logs: String,
    /// Labels of the window check items, keyed by `OverlayWindow::tray_id`.
    overlay_windows: std::collections::HashMap<String, String>,
    toggle_overlay_menu: String,
    reset_windows: String,
    export_diagnostics: String,
    about: String,
    quit: String,
    logs_title: String,
    about_title: String,
}

/// Title for the About window, which is created on first use.
#[derive(Default)]
struct AboutTitle(std::sync::Mutex<Option<String>>);

/// Pieces of the "show in-game settings" tray label, which combines the translated text with
/// the hotkey reported by the overlay; either can change on its own.
struct OverlayMenuLabel(std::sync::Mutex<(String, Option<String>)>);

impl Default for OverlayMenuLabel {
    fn default() -> Self {
        Self(std::sync::Mutex::new((
            TOGGLE_OVERLAY_MENU_DEFAULT.to_string(),
            None,
        )))
    }
}

const TOGGLE_OVERLAY_MENU_DEFAULT: &str = "Show In-game Settings";

/// Updates the overlay menu tray item from its stored text and hotkey.
fn refresh_overlay_menu_label(app: &AppHandle) {
    let state = app.state::<OverlayMenuLabel>();
    let (text, hotkey) = state.0.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let title = match hotkey {
        Some(hotkey) => format!("{text} ({hotkey})"),
        None => text,
    };
    let _ = app
        .tray_handle()
        .get_item("toggle_overlay_menu")
        .set_title(title);
}

/// Mirrors the in-game overlay in the tray: check marks and the menu hotkey.
fn apply_overlay_ui_state(app: &AppHandle, ui_state: protocol::OverlayUiState) {
    let tray = app.tray_handle();
    for window in protocol::OverlayWindow::ALL {
        let _ = tray
            .get_item(window.tray_id())
            .set_selected(ui_state.visible_windows.contains(&window));
    }
    let _ = tray
        .get_item("toggle_overlay_menu")
        .set_selected(ui_state.menu_open);

    let changed = {
        let label = app.state::<OverlayMenuLabel>();
        let mut label = label.0.lock().unwrap_or_else(|e| e.into_inner());
        let changed = label.1.as_deref() != Some(ui_state.menu_hotkey.as_str());
        label.1 = Some(ui_state.menu_hotkey);
        changed
    };
    if changed {
        refresh_overlay_menu_label(app);
    }
}

/// The tray menu and window titles exist before any translation is loaded, so they start in
/// English and are relabelled from the front-end, which owns the language setting.
#[tauri::command]
fn set_native_labels(app: AppHandle, about_title: State<AboutTitle>, labels: NativeLabels) {
    let tray = app.tray_handle();
    app.state::<OverlayMenuLabel>()
        .0
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .0 = labels.toggle_overlay_menu;
    refresh_overlay_menu_label(&app);

    for (id, title) in [
        ("open_logs", labels.open_logs),
        ("reset_windows", labels.reset_windows),
        ("export_diagnostics", labels.export_diagnostics),
        ("about", labels.about),
        ("quit", labels.quit),
    ] {
        let _ = tray.get_item(id).set_title(title);
    }
    for window in protocol::OverlayWindow::ALL {
        if let Some(title) = labels.overlay_windows.get(window.tray_id()) {
            let _ = tray.get_item(window.tray_id()).set_title(title);
        }
    }

    if let Some(window) = app.get_window("logs") {
        let _ = window.set_title(&labels.logs_title);
    }
    if let Some(window) = app.get_window("about") {
        let _ = window.set_title(&labels.about_title);
    }
    *about_title.0.lock().unwrap_or_else(|e| e.into_inner()) = Some(labels.about_title);
}

fn toggle_window_visibility(handle: &AppHandle, id: &str, focus: Option<bool>) {
    if let Some(window) = handle.get_window(id) {
        if let Some(focus_value) = focus {
            if focus_value {
                window.set_focus().unwrap();
            }
        }

        match window.is_visible().unwrap() {
            true => window.hide().unwrap(),
            false => window.show().unwrap(),
        }
    }
}

fn menu_tray_handler(handle: &AppHandle, event: SystemTrayEvent) {
    let should_focus = true;
    match event {
        SystemTrayEvent::LeftClick { .. } => {
            toggle_window_visibility(handle, "logs", Some(should_focus))
        }
        SystemTrayEvent::MenuItemClick { id, .. } => match id.as_str() {
            "open_logs" => toggle_window_visibility(handle, "logs", Some(should_focus)),
            "about" => show_about_window(handle),
            "export_diagnostics" => {
                let app = handle.clone();
                tauri::async_runtime::spawn(async move {
                    let result = diagnostics::export(app.clone()).await;
                    let window = app.get_window("logs");
                    match result {
                        Ok(path) => {
                            diagnostics::reveal(&path);
                            if let Some(window) = window {
                                let _ = window.emit("diagnostics-exported", path.display().to_string());
                            }
                        }
                        Err(error) => {
                            log::error!("Diagnostics export failed: {error:#}");
                            if let Some(window) = window {
                                let _ = window.emit("diagnostics-export-failed", format!("{error:#}"));
                            }
                        }
                    }
                });
            }
            "toggle_overlay_menu" => {
                let feed = handle.state::<overlay::OverlayFeed>().inner().clone();
                tauri::async_runtime::spawn(async move { feed.toggle_menu().await });
            }
            id if protocol::OverlayWindow::from_tray_id(id).is_some() => {
                let window = protocol::OverlayWindow::from_tray_id(id).unwrap();
                let feed = handle.state::<overlay::OverlayFeed>().inner().clone();
                tauri::async_runtime::spawn(async move { feed.toggle_window(window).await });
            }
            "reset_windows" => {
                if let Some(window) = handle.get_window("logs") {
                    let _ = window.show();
                    let _ = window.unminimize();
                    let _ = window.set_size(Size::Logical(LogicalSize {
                        width: 800.0,
                        height: 600.0,
                    }));
                }
            }
            "quit" => {
                let _ = handle.save_window_state(StateFlags::all());
                handle.exit(0)
            }
            _ => {}
        },
        _ => {} // Ignore rest of the events.
    }
}

/// Brings the logs window back when the app is launched again. Only the logs window: the
/// About window stays closed unless it is asked for.
fn show_window(app: &AppHandle) {
    if let Some(window) = app.get_window("logs") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

/// Shows the About window (version, credits and licence), creating it on first use.
fn show_about_window(handle: &AppHandle) {
    if let Some(window) = handle.get_window("about") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
        return;
    }

    // All windows share one WebView2 environment, which cannot be created again with other
    // browser arguments, so the About window reuses the logs window's.
    let browser_args = handle
        .config()
        .tauri
        .windows
        .iter()
        .find(|window| window.label == "logs")
        .and_then(|window| window.additional_browser_args.clone());

    // Built off the tray handler: creating a WebView2 window on the event loop thread from an
    // event callback can deadlock on Windows.
    let title = handle
        .state::<AboutTitle>()
        .0
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
        .unwrap_or_else(|| "About GBFR Logs".to_string());

    let handle = handle.clone();
    tauri::async_runtime::spawn(async move {
        let mut builder =
            tauri::WindowBuilder::new(&handle, "about", tauri::WindowUrl::App("about".into()))
                .title(title)
                .inner_size(560.0, 640.0)
                .min_inner_size(420.0, 400.0)
                .center();
        if let Some(args) = &browser_args {
            builder = builder.additional_browser_args(args);
        }
        if let Err(error) = builder.build() {
            warn!("Could not open the About window: {error}");
        }
    });
}

fn main() {
    info!("Starting application..");

    // Kaguya Edition renamed the data folder (gbfr-logs-awa -> gbfr-logs-kaguya). Done before
    // the hook is injected, so both find the moved settings and logs.
    match protocol::paths::migrate_legacy_data_dir() {
        Ok(true) => info!("Moved the data folder to {}", protocol::paths::DATA_DIR_NAME),
        Ok(false) => {}
        Err(error) => log::warn!("Could not move the old data folder: {error}"),
    }

    // Setup the database.
    db::setup_db().expect("Failed to setup database");

    info!("Database setup complete, launching application..");

    let overlay_feed = overlay::OverlayFeed::default();

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_window(app);
        }))
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .plugin(
            tauri_plugin_log::Builder::default()
                .targets([LogTarget::Folder("logs".into()), LogTarget::Stdout])
                .level(LevelFilter::Warn)
                .level_for("tao", LevelFilter::Error)
                .build(),
        )
        .manage(overlay_feed.clone())
        .manage(AboutTitle::default())
        .manage(OverlayMenuLabel::default())
        .system_tray(system_tray_with_menu())
        .on_system_tray_event(menu_tray_handler)
        .on_window_event(|event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event.event() {
                event.window().hide().unwrap();
                api.prevent_close();
            }
        })
        .invoke_handler(tauri::generate_handler![
            fetch_encounter_state,
            fetch_combat_log,
            fetch_logs,
            delete_logs,
            delete_all_logs,
            export_damage_log_to_file,
            overlay::push_overlay_state,
            overlay::push_overlay_notice,
            set_native_labels,
        ])
        .setup(move |app| {
            // Perform the game hook check in a separate thread.
            tauri::async_runtime::spawn(check_and_perform_hook(app.handle()));
            // Pace meter snapshots to the in-game overlay, independent of the meter window.
            tauri::async_runtime::spawn(overlay_feed.run());

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
