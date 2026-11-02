//! Tray / menu bar settings. Every choice applies immediately and persists to preferences.json.

use ctl_core::prefs::{
    MAX_REPEAT_COUNT, Orientation, POPUP_SIZE_CHOICES, Preferences, REPEAT_INTERVAL_CHOICES, SIZE_CHOICES,
    SOUND_NONE, SOUND_TONE_ALERT, SOUND_TONE_CHIME, TEXT_SIZE_CHOICES, TIMEOUT_CHOICES_MINUTES,
    Theme, file_sound_id, file_sound_name, prefs_path, save_prefs,
};
use tauri::menu::{CheckMenuItem, IsMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager, Runtime};

use crate::{Shared, main_window};

const TRAY_ID: &str = "main";

pub fn install(app: &AppHandle) -> tauri::Result<()> {
    let menu = build_menu(app)?;
    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("Claude Traffic Light")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| handle_menu_event(app, event.id().as_ref()));
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    Ok(())
}

fn check<R: Runtime>(
    app: &AppHandle<R>,
    id: String,
    text: &str,
    checked: bool,
) -> tauri::Result<CheckMenuItem<R>> {
    CheckMenuItem::with_id(app, id, text, true, checked, None::<&str>)
}

fn submenu<R: Runtime>(
    app: &AppHandle<R>,
    text: &str,
    items: &[CheckMenuItem<R>],
) -> tauri::Result<Submenu<R>> {
    let refs: Vec<&dyn IsMenuItem<R>> = items.iter().map(|i| i as &dyn IsMenuItem<R>).collect();
    Submenu::with_items(app, text, true, &refs)
}

fn sound_label(sound_id: &str) -> String {
    match sound_id {
        SOUND_NONE => "None".to_owned(),
        SOUND_TONE_CHIME => "Chime (tone)".to_owned(),
        SOUND_TONE_ALERT => "Alert (tone)".to_owned(),
        other => file_sound_name(other).unwrap_or(other).to_owned(),
    }
}

fn sound_submenu<R: Runtime>(
    app: &AppHandle<R>,
    title: &str,
    prefix: &str,
    selected: &str,
    files: &[String],
) -> tauri::Result<Submenu<R>> {
    let mut ids: Vec<String> = vec![
        SOUND_NONE.to_owned(),
        SOUND_TONE_CHIME.to_owned(),
        SOUND_TONE_ALERT.to_owned(),
    ];
    ids.extend(files.iter().map(|name| file_sound_id(name)));
    if !ids.iter().any(|id| id == selected) {
        // Selected file was removed from the folder: keep it visible so it can be seen and replaced.
        ids.push(selected.to_owned());
    }
    let items = ids
        .iter()
        .map(|id| {
            let label = if ids.iter().position(|x| x == id) >= Some(3 + files.len()) {
                format!("{} (missing)", sound_label(id))
            } else {
                sound_label(id)
            };
            check(app, format!("{prefix}:{id}"), &label, id == selected)
        })
        .collect::<tauri::Result<Vec<_>>>()?;
    submenu(app, title, &items)
}

fn build_menu<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Menu<R>> {
    let shared = app.state::<std::sync::Arc<Shared>>();
    let prefs = shared.prefs.lock().unwrap().clone();
    let files = shared.sound_files.lock().unwrap().clone();

    let orientation = submenu(
        app,
        "Orientation",
        &[
            check(app, "orient:horizontal".to_owned(), "Horizontal", prefs.orientation == Orientation::Horizontal)?,
            check(app, "orient:vertical".to_owned(), "Vertical", prefs.orientation == Orientation::Vertical)?,
        ],
    )?;
    let size = submenu(
        app,
        "Size",
        &SIZE_CHOICES
            .iter()
            .map(|&n| check(app, format!("size:{n}"), &format!("{n}x"), n == prefs.size))
            .collect::<tauri::Result<Vec<_>>>()?,
    )?;
    let text_size = submenu(
        app,
        "Text Size",
        &TEXT_SIZE_CHOICES
            .iter()
            .map(|&n| check(app, format!("text:{n}"), &format!("{n}x"), n == prefs.text_size))
            .collect::<tauri::Result<Vec<_>>>()?,
    )?;
    let popup_size = submenu(
        app,
        "Popup Size",
        &POPUP_SIZE_CHOICES
            .iter()
            .map(|&n| check(app, format!("popup:{n}"), &format!("{n}%"), n == prefs.popup_size))
            .collect::<tauri::Result<Vec<_>>>()?,
    )?;
    let themes = [
        ("follow_os", "Follow OS", Theme::FollowOs),
        ("dark", "Dark", Theme::Dark),
        ("light", "Light", Theme::Light),
        ("contrast_os", "Contrast OS", Theme::ContrastOs),
    ];
    let theme = submenu(
        app,
        "Theme",
        &themes
            .iter()
            .map(|(key, label, value)| check(app, format!("theme:{key}"), label, *value == prefs.theme))
            .collect::<tauri::Result<Vec<_>>>()?,
    )?;

    let yellow = sound_submenu(app, "Yellow Sound", "ysound", &prefs.yellow_sound, &files)?;
    let red = sound_submenu(app, "Red Sound", "rsound", &prefs.red_sound, &files)?;
    let repeat = submenu(
        app,
        "Repeat",
        &(0..=MAX_REPEAT_COUNT)
            .map(|n| {
                let label = match n {
                    0 => "Off".to_owned(),
                    1 => "1 time".to_owned(),
                    n => format!("{n} times"),
                };
                check(app, format!("repeat:{n}"), &label, n == prefs.repeat_count)
            })
            .collect::<tauri::Result<Vec<_>>>()?,
    )?;
    let interval = submenu(
        app,
        "Repeat Every",
        &REPEAT_INTERVAL_CHOICES
            .iter()
            .map(|&n| {
                let label = if n == 1 { "1 minute".to_owned() } else { format!("{n} minutes") };
                check(app, format!("interval:{n}"), &label, n == prefs.repeat_interval_minutes)
            })
            .collect::<tauri::Result<Vec<_>>>()?,
    )?;
    let rescan = MenuItem::with_id(app, "rescan", "Rescan Sounds", true, None::<&str>)?;
    let sound = Submenu::with_items(
        app,
        "Sound",
        true,
        &[&yellow, &red, &repeat, &interval, &PredefinedMenuItem::separator(app)?, &rescan],
    )?;

    let idle = check(app, "idle".to_owned(), "Idle Sessions", prefs.show_idle_sessions)?;
    let timeout = submenu(
        app,
        "Active Session Timeout",
        &TIMEOUT_CHOICES_MINUTES
            .iter()
            .map(|&m| {
                let label = if m < 60 { format!("{m} minutes") } else if m == 60 { "1 hour".to_owned() } else { format!("{} hours", m / 60) };
                check(app, format!("timeout:{m}"), &label, m == prefs.session_timeout_minutes)
            })
            .collect::<tauri::Result<Vec<_>>>()?,
    )?;

    let toggle = MenuItem::with_id(app, "toggle", "Show/Hide", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let sep1 = PredefinedMenuItem::separator(app)?;
    let sep2 = PredefinedMenuItem::separator(app)?;
    Menu::with_items(
        app,
        &[
            &toggle, &sep1, &orientation, &size, &text_size, &popup_size, &theme, &sound, &idle, &timeout,
            &sep2, &quit,
        ],
    )
}

fn handle_menu_event(app: &AppHandle, id: &str) {
    let shared = app.state::<std::sync::Arc<Shared>>();
    match id {
        "quit" => {
            app.exit(0);
            return;
        }
        "toggle" => {
            if let Some(window) = main_window(app) {
                if window.is_visible().unwrap_or(true) {
                    let _ = window.hide();
                } else {
                    let _ = window.show();
                }
            }
            return;
        }
        "rescan" => {
            shared.rescan_sounds();
            refresh_menu(app);
            return;
        }
        _ => {}
    }

    let (key, value) = id.split_once(':').unwrap_or((id, ""));
    let mut preview: Option<String> = None;
    {
        let mut prefs = shared.prefs.lock().unwrap();
        let number = value.parse::<u32>().ok();
        match key {
            "size" => set_u8(number, SIZE_CHOICES, &mut prefs.size),
            "text" => set_u8(number, TEXT_SIZE_CHOICES, &mut prefs.text_size),
            "popup" => set_u8(number, POPUP_SIZE_CHOICES, &mut prefs.popup_size),
            "interval" => set_u8(number, REPEAT_INTERVAL_CHOICES, &mut prefs.repeat_interval_minutes),
            "repeat" => {
                if let Some(n) = number.filter(|n| *n <= u32::from(MAX_REPEAT_COUNT)) {
                    prefs.repeat_count = n as u8;
                }
            }
            "timeout" => {
                if let Some(m) = number.filter(|m| TIMEOUT_CHOICES_MINUTES.contains(m)) {
                    prefs.session_timeout_minutes = m;
                }
            }
            "theme" => {
                prefs.theme = match value {
                    "dark" => Theme::Dark,
                    "light" => Theme::Light,
                    "contrast_os" => Theme::ContrastOs,
                    _ => Theme::FollowOs,
                }
            }
            "orient" => {
                prefs.orientation = if value == "vertical" {
                    Orientation::Vertical
                } else {
                    Orientation::Horizontal
                }
            }
            "idle" => prefs.show_idle_sessions = !prefs.show_idle_sessions,
            "ysound" => {
                prefs.yellow_sound = value.to_owned();
                preview = Some(value.to_owned());
            }
            "rsound" => {
                prefs.red_sound = value.to_owned();
                preview = Some(value.to_owned());
            }
            _ => return,
        }
        let snapshot: Preferences = prefs.clone();
        drop(prefs);
        if let Ok(path) = prefs_path()
            && let Err(error) = save_prefs(&path, &snapshot)
        {
            eprintln!("could not save preferences: {error}");
        }
    }
    if let Some(sound) = preview {
        shared.audio.play(&sound);
    }
    refresh_menu(app);
}

fn set_u8(number: Option<u32>, choices: &[u8], target: &mut u8) {
    if let Some(n) = number.and_then(|n| u8::try_from(n).ok())
        && choices.contains(&n)
    {
        *target = n;
    }
}

fn refresh_menu(app: &AppHandle) {
    if let (Some(tray), Ok(menu)) = (app.tray_by_id(TRAY_ID), build_menu(app)) {
        let _ = tray.set_menu(Some(menu));
    }
}
