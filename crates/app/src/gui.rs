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
        if let Some(k) = v.get("edit_key").and_then(|x| x.as_str()) {
            s.edit_key = k.to_string();
        }
        if let Some(a) = v.get("autostart").and_then(|x| x.as_bool()) {
            s.autostart = a;
        }
        if let Some(e) = v.get("history_encrypt").and_then(|x| x.as_bool()) {
            s.history_encrypt = e;
        }
        if let Some(m) = v.get("paste_method").and_then(|x| x.as_str()) {
            if matches!(m, "clipboard" | "unicode") {
                s.paste_method = m.to_string();
            }
        }
        #[cfg(feature = "audio")]
        if let Some(gate) = v.get("noise_gate").and_then(|x| x.as_bool()) {
            s.noise_gate = gate;
        }
    }
    // Env wins over the file for the backend choice.
    if let Some(env_pref) = BackendPref::from_env() {
        s.backend = env_pref;
    }
    s
}

fn save_settings(s: &Settings) -> Result<(), String> {
    let mut map = serde_json::Map::new();
    let put =
        |map: &mut serde_json::Map<String, serde_json::Value>, k: &str, v: serde_json::Value| {
            map.insert(k.to_string(), v);
        };
    put(
        &mut map,
        "hotkey",
        serde_json::Value::String(s.hotkey.clone()),
    );
    put(&mut map, "lang", serde_json::Value::String(s.lang.clone()));
    put(
        &mut map,
        "groq_model",
        serde_json::Value::String(s.groq_model.clone()),
    );
    put(
        &mut map,
        "backend",
        serde_json::Value::String(s.backend.label().to_string()),
    );
    put(
        &mut map,
        "profile",
        serde_json::Value::String(s.profile.clone()),
    );
    put(&mut map, "sound", serde_json::Value::Bool(s.sound));
    put(
        &mut map,
        "edit_key",
        serde_json::Value::String(s.edit_key.clone()),
    );
    put(&mut map, "autostart", serde_json::Value::Bool(s.autostart));
    put(
        &mut map,
        "history_encrypt",
        serde_json::Value::Bool(s.history_encrypt),
    );
    put(
        &mut map,
        "paste_method",
        serde_json::Value::String(s.paste_method.clone()),
    );
    #[cfg(feature = "audio")]
    put(
        &mut map,
        "noise_gate",
        serde_json::Value::Bool(s.noise_gate),
    );
    write_json("config.json", &serde_json::Value::Object(map))
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

/// Human `DD.MM HH:MM:SS` for a history entry.
fn fmt_time(when: std::time::SystemTime) -> String {
    chrono::DateTime::<chrono::Local>::from(when)
        .format("%d.%m %H:%M:%S")
        .to_string()
}

/// 32x32 tray icon: dark rounded square, white mic capsule, status dot.
/// Dot: red = recording, gray = paused, green = ready (B-11: the icon
/// always mirrors the dictation state).
fn tray_icon(recording: bool, paused: bool) -> tray_icon::Icon {
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
    // Status dot: red while recording, gray while paused, green when idle.
    let (r, g) = if recording {
        (230, 60)
    } else if paused {
        (150, 150)
    } else {
        (80, 210)
    };
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
    tab: Tab,
    /// First launch ever (B-01): welcome card + Settings tab by default.
    first_run: bool,
    tray: Option<TrayIcon>,
    show_item: MenuItem,
    pause_item: MenuItem,
    quit_item: MenuItem,
    quit_requested: bool,
    icon_recording: bool,
    icon_paused: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    Status,
    History,
    Stats,
    Settings,
}

impl Tab {
    fn label(self) -> &'static str {
        match self {
            Self::Status => "Статус",
            Self::History => "История",
            Self::Stats => "Статистика",
            Self::Settings => "Настройки",
        }
    }
}

/// Product theme: dark indigo-accent UI, rounded cards.
fn apply_theme(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    let accent = egui::Color32::from_rgb(129, 140, 248);
    visuals.window_corner_radius = egui::CornerRadius::same(12);
    visuals.menu_corner_radius = egui::CornerRadius::same(8);
    visuals.selection.bg_fill = accent;
    visuals.hyperlink_color = accent;
    visuals.panel_fill = egui::Color32::from_rgb(20, 22, 30);
    visuals.window_fill = egui::Color32::from_rgb(24, 26, 36);
    visuals.faint_bg_color = egui::Color32::from_rgb(30, 33, 45);
    // Visible input fields on the dark background (like buttons, not pits).
    visuals.extreme_bg_color = egui::Color32::from_rgb(42, 46, 61);
    ctx.set_visuals(visuals);
}

impl GuiApp {
    fn apply_settings(&self) {
        if let Some(hk) = Hotkey::parse(&self.settings.hotkey) {
            win::set_hotkey_vk(hk.to_vk());
        }
        win::set_edit_hotkey_vk(win::parse_edit_key(&self.settings.edit_key));
        if let Err(e) = win::set_autostart(self.settings.autostart) {
            self.state.push_log(format!("[gui] autostart failed: {e}"));
        }
        std::env::set_var("FLOWVOICE_PASTE_METHOD", &self.settings.paste_method);
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
        #[cfg(feature = "audio")]
        std::env::set_var(
            "FLOWVOICE_NOISE_GATE",
            if self.settings.noise_gate { "1" } else { "0" },
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
                // Graceful viewport Close is unreliable (the loop may
                // survive it): flush state and terminate deterministically.
                self.quit_requested = true;
                self.save_history();
                std::process::exit(0);
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
        let paused = !win::is_enabled();
        // Redraw on any state change (record/paused/ready all differ).
        let key = (recording, paused);
        let changed = key.0 != self.icon_recording || key.1 != self.icon_paused;
        if changed {
            self.icon_recording = key.0;
            self.icon_paused = key.1;
            if let Some(tray) = &self.tray {
                let _ = tray.set_icon(Some(tray_icon(recording, paused)));
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
        // Encrypted store (M-17): DPAPI blob instead of plain JSON, and
        // vice versa — only one store file exists at a time.
        let saved = if self.settings.history_encrypt {
            match win::dpapi_protect(serde_json::Value::Array(entries).to_string().as_bytes()) {
                Ok(blob) => {
                    let _ = std::fs::remove_file(app_dir().join("history.json"));
                    std::fs::write(app_dir().join("history.enc"), &blob)
                        .map_err(|e| format!("history save failed: {e}"))
                }
                Err(e) => Err(format!("history encrypt failed: {e}")),
            }
        } else {
            let _ = std::fs::remove_file(app_dir().join("history.enc"));
            write_json("history.json", &serde_json::Value::Array(entries))
        };
        if let Err(e) = saved {
            self.state.push_log(format!("[gui] {e}"));
            return;
        }
        self.state
            .history_dirty
            .store(false, std::sync::atomic::Ordering::SeqCst);
    }

    fn show_header(&mut self, ui: &mut egui::Ui) {
        egui::Panel::top("header").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("flowvoice").size(20.0).strong());
                let recording = self
                    .state
                    .recording
                    .load(std::sync::atomic::Ordering::SeqCst);
                if !win::is_enabled() {
                    ui.label(
                        egui::RichText::new("[пауза]")
                            .color(egui::Color32::from_rgb(150, 150, 150))
                            .strong(),
                    );
                } else if recording {
                    ui.label(
                        egui::RichText::new("[запись]")
                            .color(egui::Color32::from_rgb(230, 60, 60))
                            .strong(),
                    );
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let paused = !win::is_enabled();
                    if ui
                        .button(if paused {
                            "Слушать"
                        } else {
                            "Пауза"
                        })
                        .clicked()
                    {
                        win::set_enabled(paused);
                        self.pause_item.set_text(if paused {
                            "Слушать"
                        } else {
                            "Пауза"
                        });
                        self.state.push_log(if paused {
                            "[gui] dictation resumed".to_string()
                        } else {
                            "[gui] dictation paused".to_string()
                        });
                    }
                });
            });
            ui.horizontal(|ui| {
                for tab in [Tab::Status, Tab::History, Tab::Stats, Tab::Settings] {
                    ui.selectable_value(&mut self.tab, tab, tab.label());
                }
            });
        });
    }

    fn show_status(&self, ui: &mut egui::Ui) {
        ui.heading("Статус");
        if self.first_run {
            ui.label(egui::RichText::new("Добро пожаловать! Три шага до диктовки:").strong());
            ui.label(
                "1. Держите хоткей (по умолчанию Right Ctrl) и говорите, отпустите для вставки.",
            );
            ui.label("2. Для облачного распознавания задайте GROQ_API_KEY, иначе работает локальный движок.");
            ui.label("3. Проверьте звук и язык во вкладке «Настройки», затем диктуйте в любое поле ввода.");
            ui.separator();
        }
        let backend = self
            .state
            .backend_label
            .lock()
            .map(|b| b.clone())
            .unwrap_or_default();
        ui.label(format!("бэкенд: {backend}"));
        #[cfg(feature = "audio")]
        {
            // Live input level (C-07): follows the mic during recording.
            let level = crate::audio::MIC_LEVEL.load(std::sync::atomic::Ordering::SeqCst);
            ui.horizontal(|ui| {
                ui.label("Микрофон:");
                ui.add_sized([160.0, 18.0], egui::ProgressBar::new(level as f32 / 100.0));
            });
        }
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
            ui.label("Правка:");
            egui::ComboBox::from_id_salt("editkey")
                .selected_text(self.settings.edit_key.clone())
                .show_ui(ui, |ui| {
                    for name in ["выкл", "RCONTROL", "F7", "F8", "F9"] {
                        ui.selectable_value(&mut self.settings.edit_key, name.to_string(), name);
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
            #[cfg(feature = "audio")]
            ui.checkbox(&mut self.settings.noise_gate, "Шумоподавление");
            ui.checkbox(&mut self.settings.autostart, "Автозапуск");
            ui.checkbox(&mut self.settings.history_encrypt, "Шифровать историю");
            ui.label("Вставка:");
            egui::ComboBox::from_id_salt("pastemethod")
                .selected_text(self.settings.paste_method.clone())
                .show_ui(ui, |ui| {
                    for name in ["clipboard", "unicode"] {
                        ui.selectable_value(
                            &mut self.settings.paste_method,
                            name.to_string(),
                            name,
                        );
                    }
                });
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
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if entries.is_empty() {
                    ui.label("пока пусто — зажмите хоткей и надиктуйте");
                }
                for (idx, when, app, text) in entries {
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new(when).weak().small());
                            if !app.is_empty() {
                                ui.label(egui::RichText::new(app).weak().small());
                            }
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui.small_button("Удал.").clicked() {
                                        delete_idx = Some(idx);
                                    }
                                    if ui.small_button("Вставить").clicked() {
                                        crate::win::paste(&text);
                                    }
                                    if ui.small_button("Копия").clicked() {
                                        if let Ok(mut cb) = arboard::Clipboard::new() {
                                            let _ = cb.set_text(text.clone());
                                        }
                                    }
                                },
                            );
                        });
                        ui.label(egui::RichText::new(text).size(14.0));
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
                .with_inner_size([180.0, 52.0])
                .with_position([12.0, 12.0]),
            |ui, _| {
                ui.vertical_centered(|ui| {
                    let secs = self.state.recording_secs();
                    ui.label(
                        egui::RichText::new(format!("[{secs} c]"))
                            .color(egui::Color32::from_rgb(230, 70, 70))
                            .size(15.0)
                            .strong(),
                    );
                    ui.label(egui::RichText::new("отпустите хоткей").size(11.0).weak());
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

        apply_theme(&ctx);

        self.poll_tray(&ctx);
        self.refresh_tray_icon();
        if self
            .state
            .history_dirty
            .load(std::sync::atomic::Ordering::SeqCst)
        {
            self.save_history();
        }

        self.show_header(ui);
        match self.tab {
            Tab::Status => {
                egui::ScrollArea::vertical().id_salt("tab").show(ui, |ui| {
                    self.show_status(ui);
                    ui.add_space(8.0);
                    self.show_log(ui);
                });
            }
            Tab::History => self.show_history(ui),
            Tab::Stats => {
                egui::ScrollArea::vertical().id_salt("tab").show(ui, |ui| {
                    self.show_stats(ui);
                });
            }
            Tab::Settings => {
                egui::ScrollArea::vertical().id_salt("tab").show(ui, |ui| {
                    self.show_settings(ui);
                });
            }
        }

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
            .with_icon(tray_icon(false, false))
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

    // Restore persisted history: encrypted store first, plain fallback.
    let stored: Option<Vec<serde_json::Value>> = std::fs::read(app_dir().join("history.enc"))
        .ok()
        .and_then(|blob| win::dpapi_unprotect(&blob).ok())
        .and_then(|plain| serde_json::from_slice(&plain).ok())
        .and_then(|v: serde_json::Value| v.as_array().cloned())
        .or_else(|| read_json("history.json").and_then(|v| v.as_array().cloned()));
    if let Some(arr) = stored {
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
    win::set_edit_hotkey_vk(win::parse_edit_key(&settings.edit_key));
    if let Ok(k) = std::env::var("FLOWVOICE_EDIT_KEY") {
        win::set_edit_hotkey_vk(win::parse_edit_key(&k));
    }
    if let Err(e) = win::set_autostart(settings.autostart) {
        state.push_log(format!("[gui] autostart failed: {e}"));
    }
    std::env::set_var("FLOWVOICE_PASTE_METHOD", &settings.paste_method);
    // First-run marker (B-01): welcome card + Settings tab on debut.
    let seen = app_dir().join(".seen");
    let first_run = !seen.exists();
    if first_run {
        let _ = std::fs::create_dir_all(app_dir());
        let _ = std::fs::write(&seen, "1");
    }
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
    #[cfg(feature = "audio")]
    std::env::set_var(
        "FLOWVOICE_NOISE_GATE",
        if settings.noise_gate { "1" } else { "0" },
    );
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
    let first_run_tab = if first_run {
        Tab::Settings
    } else {
        Tab::Status
    };
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
                tab: first_run_tab,
                first_run,
                tray,
                show_item,
                pause_item,
                quit_item,
                quit_requested: false,
                icon_recording: false,
                icon_paused: false,
            }) as Box<dyn eframe::App>)
        }),
    ) {
        // Popup first: stderr may be invalid in windowed mode, so a print
        // here could panic and hide the message.
        crate::win::fatal_popup(&format!("[gui] event loop failed: {e}"));
        std::process::exit(1);
    }
}
