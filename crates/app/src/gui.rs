// SPDX-License-Identifier: MIT
//! Desktop GUI (eframe + system tray), launched with `--gui`.
//!
//! Windows-only v1: a status/settings/history window, a recording pill
//! overlay while the hotkey is held, and a tray icon (hide-on-close).
//! The low-level hotkey hook runs on a background thread; this module owns
//! the UI event loop and shares [`crate::state::AppState`] with it.

use std::sync::Arc;
use std::time::Duration;

use eframe::egui;
use tray_icon::{
    menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem},
    TrayIcon, TrayIconBuilder, TrayIconEvent,
};

use crate::state::app_dir;
use crate::state::{self, AppState, BackendPref, Settings};
use crate::win::{self, Hotkey};

fn read_json(name: &str) -> Option<serde_json::Value> {
    std::fs::read(app_dir().join(name))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
}

fn write_json(name: &str, value: &serde_json::Value) -> Result<(), String> {
    let dir = app_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {dir:?}: {e}"))?;
    std::fs::write(
        dir.join(name),
        serde_json::to_string_pretty(value).unwrap_or_default(),
    )
    .map_err(|e| format!("cannot write {name}: {e}"))
}

fn load_settings() -> Settings {
    let mut s = Settings::default();
    if let Some(v) = read_json("config.json") {
        if let Some(h) = v.get("hotkey").and_then(|x| x.as_str()) {
            if Hotkey::parse(h).is_some() {
                s.hotkey = h.to_ascii_uppercase();
            }
        }
        if let Some(l) = v.get("lang").and_then(|x| x.as_str()) {
            if matches!(l, "ru" | "en" | "auto") {
                s.lang = l.to_string();
            }
        }
        if let Some(m) = v.get("groq_model").and_then(|x| x.as_str()) {
            if !m.is_empty() {
                s.groq_model = m.to_string();
            }
        }
        if let Some(b) = v.get("backend").and_then(|x| x.as_str()) {
            s.backend = BackendPref::parse(b);
        }
        if let Some(p) = v.get("profile").and_then(|x| x.as_str()) {
            if matches!(p, "auto" | "chat" | "mail" | "code") {
                s.profile = p.to_string();
            }
        }
        if let Some(sound) = v.get("sound").and_then(|x| x.as_bool()) {
            s.sound = sound;
        }
    }
    // Env wins over the file for the backend choice.
    if let Some(env_pref) = BackendPref::from_env() {
        s.backend = env_pref;
    }
    s
}

fn save_settings(s: &Settings) -> Result<(), String> {
    write_json(
        "config.json",
        &serde_json::json!({
            "hotkey": s.hotkey,
            "lang": s.lang,
            "groq_model": s.groq_model,
            "backend": s.backend.label(),
            "profile": s.profile,
            "sound": s.sound,
        }),
    )
}

/// History export (M-11): JSONL with UTC timestamps + app, next to settings.
fn export_history_jsonl(state: &Arc<AppState>) -> Result<String, String> {
    let lines: Vec<String> = state
        .history
        .lock()
        .map(|h| {
            h.iter()
                .map(|e| {
                    let ts = chrono::DateTime::<chrono::Utc>::from(e.when).to_rfc3339();
                    serde_json::json!({"ts": ts, "app": e.app, "text": e.text}).to_string()
                })
                .collect()
        })
        .unwrap_or_default();
    let path = app_dir().join("history-export.jsonl");
    std::fs::write(&path, lines.join("\n")).map_err(|e| format!("экспорт: {e}"))?;
    Ok(path.display().to_string())
}

/// Human `HH:MM:SS` for a history entry.
fn fmt_time(when: std::time::SystemTime) -> String {
    chrono::DateTime::<chrono::Local>::from(when)
        .format("%H:%M:%S")
        .to_string()
}

/// 32x32 tray icon: dark rounded square, white mic capsule, status dot.
fn tray_icon(recording: bool) -> tray_icon::Icon {
    const W: usize = 32;
    let mut rgba = vec![0u8; W * W * 4];
    let mut px = |x: i32, y: i32, r: u8, g: u8, b: u8, a: u8| {
        if (0..W as i32).contains(&x) && (0..W as i32).contains(&y) {
            let i = (y as usize * W + x as usize) * 4;
            rgba[i..i + 4].copy_from_slice(&[r, g, b, a]);
        }
    };
    for y in 0..W as i32 {
        for x in 0..W as i32 {
            // Rounded-square background.
            let cx = (x - 15).clamp(-15, 16).abs();
            let cy = (y - 15).clamp(-15, 16).abs();
            if cx + cy < 26 {
                px(x, y, 24, 24, 30, 255);
            }
        }
    }
    // Mic capsule.
    for y in 7..=19 {
        for x in 13..=18 {
            px(x, y, 235, 235, 240, 255);
        }
    }
    for x in 11..=20 {
        px(x, 21, 235, 235, 240, 255);
        px(x, 22, 235, 235, 240, 255);
    }
    px(15, 24, 235, 235, 240, 255);
    px(16, 24, 235, 235, 240, 255);
    // Status dot: red while recording, green when idle.
    let (r, g) = if recording { (230, 60) } else { (80, 210) };
    for y in 4..=8 {
        for x in 23..=27 {
            let dx = x - 25;
            let dy = y - 6;
            if dx * dx + dy * dy <= 5 {
                px(x, y, r, g, 90, 255);
            }
        }
    }
    tray_icon::Icon::from_rgba(rgba, W as u32, W as u32).expect("tray icon rgba")
}

struct GuiApp {
    state: Arc<AppState>,
    settings: Settings,
    status_line: String,
    history_search: String,
    history_app_filter: String,
    tray: Option<TrayIcon>,
    show_item: MenuItem,
    pause_item: MenuItem,
    quit_item: MenuItem,
    quit_requested: bool,
    icon_recording: bool,
}

impl GuiApp {
    fn apply_settings(&self) {
        if let Some(hk) = Hotkey::parse(&self.settings.hotkey) {
            win::set_hotkey_vk(hk.to_vk());
        }
        if let Ok(mut pref) = self.state.backend_pref.lock() {
            *pref = self.settings.backend;
        }
        std::env::set_var("FLOWVOICE_LANG", &self.settings.lang);
        std::env::set_var("FLOWVOICE_GROQ_MODEL", &self.settings.groq_model);
        std::env::set_var("FLOWVOICE_PROFILE", &self.settings.profile);
        std::env::set_var(
            "FLOWVOICE_SOUND",
            if self.settings.sound { "1" } else { "0" },
        );
    }

    fn poll_tray(&mut self, ctx: &egui::Context) {
        for event in MenuEvent::receiver().try_iter() {
            if event.id == self.show_item.id() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            } else if event.id == self.pause_item.id() {
                let paused = win::is_enabled();
                win::set_enabled(!paused);
                self.pause_item.set_text(if paused {
                    "Слушать"
                } else {
                    "Пауза"
                });
                self.state.push_log(if paused {
                    "[gui] dictation paused (tray)".to_string()
                } else {
                    "[gui] dictation resumed (tray)".to_string()
                });
            } else if event.id == self.quit_item.id() {
                self.quit_requested = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
        for event in TrayIconEvent::receiver().try_iter() {
            if matches!(
                event,
                TrayIconEvent::Click {
                    button: tray_icon::MouseButton::Left,
                    ..
                }
            ) {
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            }
        }
    }

    fn refresh_tray_icon(&mut self) {
        let recording = self
            .state
            .recording
            .load(std::sync::atomic::Ordering::SeqCst);
        if recording != self.icon_recording {
            self.icon_recording = recording;
            if let Some(tray) = &self.tray {
                let _ = tray.set_icon(Some(tray_icon(recording)));
            }
        }
    }

    fn save_history(&self) {
        let entries: Vec<serde_json::Value> = self
            .state
            .history
            .lock()
            .map(|h| {
                h.iter()
                    .map(|e| {
                        serde_json::json!({
                            "when": e.when.duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
                            "text": e.text,
                            "app": e.app,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        let _ = write_json("history.json", &serde_json::Value::Array(entries));
        self.state
            .history_dirty
            .store(false, std::sync::atomic::Ordering::SeqCst);
    }

    fn show_status(&self, ui: &mut egui::Ui) {
        ui.heading("Статус");
        let recording = self
            .state
            .recording
            .load(std::sync::atomic::Ordering::SeqCst);
        ui.horizontal(|ui| {
            if !win::is_enabled() {
                ui.label(
                    egui::RichText::new("[пауза]")
                        .color(egui::Color32::from_rgb(150, 150, 150))
                        .strong(),
                );
            } else if recording {
                let secs = self.state.recording_secs();
                ui.label(
                    egui::RichText::new(format!("[ЗАПИСЬ {secs} c]"))
                        .color(egui::Color32::from_rgb(230, 60, 60))
                        .strong(),
                );
            } else {
                ui.label(
                    egui::RichText::new("[готов]")
                        .color(egui::Color32::from_rgb(80, 200, 120))
                        .strong(),
                );
            }
            let backend = self
                .state
                .backend_label
                .lock()
                .map(|b| b.clone())
                .unwrap_or_default();
            ui.label(format!("бэкенд: {backend}"));
            // Master switch: pause/resume dictation, window stays open.
            let paused = !win::is_enabled();
            let label = if paused {
                "Слушать (вкл)"
            } else {
                "Пауза (выкл)"
            };
            if ui.button(label).clicked() {
                win::set_enabled(paused);
                self.state.push_log(if paused {
                    "[gui] dictation resumed".to_string()
                } else {
                    "[gui] dictation paused".to_string()
                });
            }
        });
        let last = self
            .state
            .last_text
            .lock()
            .map(|t| t.clone())
            .unwrap_or_default();
        if !last.is_empty() {
            ui.label("Последняя вставка:");
            ui.label(egui::RichText::new(last).italics());
        }
        ui.separator();
        ui.label(format!(
            "Groq: {}",
            if std::env::var_os("GROQ_API_KEY").is_some_and(|k| !k.is_empty()) {
                "ключ задан"
            } else {
                "нет ключа"
            }
        ));
        ui.label(format!(
            "Локальный whisper: {}",
            if whisper_available() {
                "файлы на месте"
            } else {
                "не установлен"
            }
        ));
    }

    fn show_settings(&mut self, ui: &mut egui::Ui) {
        ui.heading("Настройки");
        ui.horizontal(|ui| {
            ui.label("Хоткей:");
            egui::ComboBox::from_id_salt("hotkey")
                .selected_text(self.settings.hotkey.clone())
                .show_ui(ui, |ui| {
                    for name in ["RCONTROL", "F7", "F8", "F9"] {
                        ui.selectable_value(&mut self.settings.hotkey, name.to_string(), name);
                    }
                });
        });
        ui.horizontal(|ui| {
            ui.label("Язык:");
            egui::ComboBox::from_id_salt("lang")
                .selected_text(self.settings.lang.clone())
                .show_ui(ui, |ui| {
                    for name in ["ru", "en", "auto"] {
                        ui.selectable_value(&mut self.settings.lang, name.to_string(), name);
                    }
                });
            ui.label(
                egui::RichText::new("auto — определять автоматически")
                    .weak()
                    .small(),
            );
        });
        ui.horizontal(|ui| {
            ui.label("Groq-модель:");
            ui.add_sized(
                [280.0, 22.0],
                egui::TextEdit::singleline(&mut self.settings.groq_model),
            );
        });
        ui.horizontal(|ui| {
            ui.label("Бэкенд:");
            egui::ComboBox::from_id_salt("backend")
                .selected_text(self.settings.backend.label())
                .show_ui(ui, |ui| {
                    for pref in BackendPref::all() {
                        ui.selectable_value(&mut self.settings.backend, *pref, pref.label());
                    }
                });
        });
        ui.horizontal(|ui| {
            ui.label("Профиль:");
            egui::ComboBox::from_id_salt("profile")
                .selected_text(self.settings.profile.clone())
                .show_ui(ui, |ui| {
                    for name in ["auto", "chat", "mail", "code"] {
                        ui.selectable_value(&mut self.settings.profile, name.to_string(), name);
                    }
                });
            ui.checkbox(&mut self.settings.sound, "Звуки");
        });
        if ui.button("Сохранить и применить").clicked() {
            match save_settings(&self.settings) {
                Ok(()) => {
                    self.apply_settings();
                    self.status_line = "настройки сохранены".to_string();
                    self.state.push_log("[gui] settings saved".to_string());
                }
                Err(e) => self.status_line = e,
            }
        }
        if !self.status_line.is_empty() {
            ui.label(&self.status_line);
        }
    }

    fn show_history(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading("История");
            if ui.button("Очистить").clicked() {
                if let Ok(mut h) = self.state.history.lock() {
                    h.clear();
                }
                self.state
                    .history_dirty
                    .store(true, std::sync::atomic::Ordering::SeqCst);
            }
            if ui.button("Экспорт JSONL").clicked() {
                match export_history_jsonl(&self.state) {
                    Ok(path) => self.status_line = format!("история: {path}"),
                    Err(e) => self.status_line = e,
                }
            }
        });
        ui.horizontal(|ui| {
            ui.label("Поиск:");
            ui.add_sized(
                [180.0, 22.0],
                egui::TextEdit::singleline(&mut self.history_search),
            );
            ui.label("Приложение:");
            ui.add_sized(
                [140.0, 22.0],
                egui::TextEdit::singleline(&mut self.history_app_filter),
            );
        });
        let query = self.history_search.to_lowercase();
        let app_q = self.history_app_filter.to_lowercase();
        // Newest first, with source indices for deletion.
        let entries: Vec<(usize, String, String, String)> = self
            .state
            .history
            .lock()
            .map(|h| {
                h.iter()
                    .enumerate()
                    .rev()
                    .filter(|(_, e)| {
                        (query.is_empty() || e.text.to_lowercase().contains(&query))
                            && (app_q.is_empty() || e.app.to_lowercase().contains(&app_q))
                    })
                    .map(|(i, e)| (i, fmt_time(e.when), e.app.clone(), e.text.clone()))
                    .collect()
            })
            .unwrap_or_default();
        let mut delete_idx: Option<usize> = None;
        egui::ScrollArea::vertical()
            .id_salt("history")
            .max_height(220.0)
            .show(ui, |ui| {
                if entries.is_empty() {
                    ui.label("пока пусто — зажмите хоткей и надиктуйте");
                }
                for (idx, when, app, text) in entries {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(when).weak().small());
                        if !app.is_empty() {
                            ui.label(egui::RichText::new(app).weak().small());
                        }
                        if ui.small_button("Копия").clicked() {
                            if let Ok(mut cb) = arboard::Clipboard::new() {
                                let _ = cb.set_text(text.clone());
                            }
                        }
                        if ui.small_button("Вставить").clicked() {
                            crate::win::paste(&text);
                        }
                        if ui.small_button("Удал.").clicked() {
                            delete_idx = Some(idx);
                        }
                        ui.label(text);
                    });
                    ui.separator();
                }
            });
        if let Some(idx) = delete_idx {
            if let Ok(mut h) = self.state.history.lock() {
                if idx < h.len() {
                    h.remove(idx);
                    self.state
                        .history_dirty
                        .store(true, std::sync::atomic::Ordering::SeqCst);
                }
            }
        }
    }

    fn show_stats(&mut self, ui: &mut egui::Ui) {
        ui.heading("Статистика");
        let entries = crate::journal::read_all(&crate::state::journal_path());
        let day_start = chrono::Local::now()
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .and_then(|d| d.and_local_timezone(chrono::Local).single())
            .map(|d| d.timestamp() as u64)
            .unwrap_or(0);
        let today = crate::journal::stats_since(&entries, day_start);
        let total = crate::journal::stats_since(&entries, 0);
        ui.label(format!(
            "Сегодня: реплик {}, слов {}, средняя задержка {:.1} c, средний темп {:.0} сл/мин, рекорд {:.0}",
            today.replicas, today.words, today.avg_secs, today.avg_wpm, today.best_wpm
        ));
        ui.label(format!(
            "Всего: реплик {}, слов {}, средняя задержка {:.1} c",
            total.replicas, total.words, total.avg_secs
        ));
        ui.horizontal(|ui| {
            if ui.button("Экспорт CSV").clicked() {
                let path = crate::state::app_dir().join("stats.csv");
                match std::fs::write(&path, crate::journal::to_csv(&entries)) {
                    Ok(()) => self.status_line = format!("статистика: {}", path.display()),
                    Err(e) => self.status_line = format!("CSV: {e}"),
                }
            }
            if ui.button("Сбросить статистику").clicked() {
                // Journal only: visible history stays (AL-13).
                let _ = std::fs::remove_file(crate::state::journal_path());
                self.status_line = "статистика сброшена, история цела".to_string();
            }
        });
    }

    fn show_log(&self, ui: &mut egui::Ui) {
        ui.heading("Лог");
        let lines = self.state.recent_log();
        egui::ScrollArea::vertical()
            .id_salt("log")
            .max_height(140.0)
            .show(ui, |ui| {
                for line in lines.iter().rev().take(60) {
                    ui.label(egui::RichText::new(line).small().monospace());
                }
            });
    }

    fn show_overlay(&self, ctx: &egui::Context) {
        let recording = self
            .state
            .recording
            .load(std::sync::atomic::Ordering::SeqCst);
        if !recording {
            return;
        }
        ctx.show_viewport_immediate(
            egui::ViewportId::from_hash_of("flowvoice-rec"),
            egui::ViewportBuilder::default()
                .with_title("flowvoice — запись")
                .with_decorations(false)
                .with_transparent(true)
                .with_resizable(false)
                .with_always_on_top()
                .with_inner_size([280.0, 72.0]),
            |ui, _| {
                ui.vertical_centered(|ui| {
                    let secs = self.state.recording_secs();
                    ui.label(
                        egui::RichText::new(format!("[{secs} c] говорите…"))
                            .color(egui::Color32::from_rgb(230, 70, 70))
                            .size(22.0)
                            .strong(),
                    );
                    ui.label(egui::RichText::new("отпустите хоткей для вставки").weak());
                });
                ui.ctx().request_repaint_after(Duration::from_millis(250));
            },
        );
    }
}

impl eframe::App for GuiApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if ctx.input(|i| i.viewport().close_requested()) && !self.quit_requested {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            self.state.push_log("[gui] hidden to tray".to_string());
        }

        self.poll_tray(&ctx);
        self.refresh_tray_icon();
        if self
            .state
            .history_dirty
            .load(std::sync::atomic::Ordering::SeqCst)
        {
            self.save_history();
        }

        egui::ScrollArea::vertical().id_salt("main").show(ui, |ui| {
            self.show_status(ui);
            ui.add_space(8.0);
            self.show_settings(ui);
            ui.add_space(8.0);
            self.show_history(ui);
            ui.add_space(8.0);
            self.show_stats(ui);
            ui.add_space(8.0);
            self.show_log(ui);
        });

        let ctx = ui.ctx().clone();
        self.show_overlay(&ctx);
    }
}

/// Whether the local whisper files are present (status panel).
/// Compiles with and without the `audio` feature.
#[cfg(feature = "audio")]
pub(crate) fn whisper_available() -> bool {
    crate::whisper::available()
}

#[cfg(not(feature = "audio"))]
pub(crate) fn whisper_available() -> bool {
    false
}

/// Build the tray icon + menu. The returned `GuiApp` owns the tray handle.
fn build_tray() -> (Option<TrayIcon>, MenuItem, MenuItem, MenuItem) {
    let show_item = MenuItem::new("Показать", true, None);
    let pause_item = MenuItem::new("Пауза", true, None);
    let quit_item = MenuItem::new("Выход", true, None);
    let menu = Menu::new();
    let tray = (|| -> Option<TrayIcon> {
        menu.append_items(&[
            &show_item,
            &pause_item,
            &PredefinedMenuItem::separator(),
            &quit_item,
        ])
        .ok()?;
        TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip("flowvoice — диктовка")
            .with_icon(tray_icon(false))
            .build()
            .ok()
    })();
    if tray.is_none() {
        crate::state::emit("[gui] tray unavailable, window-only mode");
    }
    (tray, show_item, pause_item, quit_item)
}

/// Entry point for `--gui`: hook thread + preload + tray + event loop.
pub fn run() {
    if !win::ensure_single_instance() {
        win::fatal_popup("flowvoice is already running (single instance)");
        std::process::exit(1);
    }
    let state = AppState::new();
    state::attach(state.clone());
    if let Some(msg) = state::take_pending_notice() {
        state::emit(&msg);
    }

    // Restore persisted history.
    if let Some(v) = read_json("history.json") {
        if let Some(arr) = v.as_array() {
            if let Ok(mut history) = state.history.lock() {
                for item in arr.iter().take(100) {
                    let secs = item.get("when").and_then(|w| w.as_u64()).unwrap_or(0);
                    let text = item
                        .get("text")
                        .and_then(|t| t.as_str())
                        .unwrap_or_default();
                    if text.is_empty() {
                        continue;
                    }
                    history.push(crate::state::HistoryEntry {
                        when: std::time::UNIX_EPOCH + Duration::from_secs(secs),
                        text: text.to_string(),
                        app: item
                            .get("app")
                            .and_then(|a| a.as_str())
                            .unwrap_or_default()
                            .to_string(),
                    });
                }
            }
        }
    }

    let mut settings = load_settings();
    // Env overrides the file for these two (same rule as console mode).
    if let Ok(lang) = std::env::var("FLOWVOICE_LANG") {
        if matches!(lang.as_str(), "ru" | "en" | "auto") {
            settings.lang = lang;
        }
    }
    if let Ok(m) = std::env::var("FLOWVOICE_GROQ_MODEL") {
        if !m.is_empty() {
            settings.groq_model = m;
        }
    }

    let hotkey = Hotkey::parse(&settings.hotkey).unwrap_or_default();
    win::spawn_pump(hotkey);

    #[cfg(feature = "audio")]
    crate::audio::preload();
    #[cfg(not(feature = "audio"))]
    state.push_log("[gui] audio disabled: rebuild with --features audio".to_string());

    // Apply (hotkey already set via spawn_pump; env for STT modules).
    std::env::set_var("FLOWVOICE_LANG", &settings.lang);
    std::env::set_var("FLOWVOICE_GROQ_MODEL", &settings.groq_model);
    std::env::set_var("FLOWVOICE_PROFILE", &settings.profile);
    std::env::set_var("FLOWVOICE_SOUND", if settings.sound { "1" } else { "0" });
    if let Ok(mut pref) = state.backend_pref.lock() {
        *pref = settings.backend;
    }

    let (tray, show_item, pause_item, quit_item) = build_tray();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("flowvoice")
            .with_inner_size([600.0, 700.0])
            .with_min_inner_size([480.0, 560.0]),
        ..Default::default()
    };
    let app_state = state.clone();
    if let Err(e) = eframe::run_native(
        "flowvoice",
        options,
        Box::new(move |cc| {
            app_state
                .egui_ctx
                .set(cc.egui_ctx.clone())
                .map_err(|_| "egui context already set")?;
            Ok(Box::new(GuiApp {
                state: app_state.clone(),
                settings,
                status_line: String::new(),
                history_search: String::new(),
                history_app_filter: String::new(),
                tray,
                show_item,
                pause_item,
                quit_item,
                quit_requested: false,
                icon_recording: false,
            }) as Box<dyn eframe::App>)
        }),
    ) {
        // Popup first: stderr may be invalid in windowed mode, so a print
        // here could panic and hide the message.
        crate::win::fatal_popup(&format!("[gui] event loop failed: {e}"));
        std::process::exit(1);
    }
}
