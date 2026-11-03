mod audio;
mod tray;
mod watcher;

use std::sync::{Arc, Mutex};

use audio::Audio;
use ctl_core::prefs::{Preferences, list_sound_files, load_prefs, prefs_path, save_prefs, sounds_dir};
use ctl_core::storage::{now_secs, read_store, state_path};
use ctl_core::{SessionSummary, StateSnapshot};
use serde::Serialize;
use tauri::{LogicalPosition, LogicalSize, Manager, State, WebviewWindow};

/// Space between the widget and the speech bubble, including the tail.
const TAIL_GAP: f64 = 10.0;
/// Transparent margin around the content so its drop shadow isn't clipped by the window edge.
const SHADOW_PAD: f64 = 16.0;

pub struct Shared {
    pub prefs: Mutex<Preferences>,
    pub sound_files: Mutex<Vec<String>>,
    /// Logical offset of the widget inside the window (`SHADOW_PAD`, plus more while a popup is open).
    widget_offset: Mutex<(f64, f64)>,
    pub audio: Audio,
}

impl Shared {
    pub fn rescan_sounds(&self) {
        let files = sounds_dir()
            .map(|dir| list_sound_files(&dir))
            .unwrap_or_default();
        *self.sound_files.lock().unwrap() = files;
    }
}

#[derive(Serialize)]
struct Placement {
    offset_x: f64,
    offset_y: f64,
    popup_above: bool,
}

#[tauri::command]
fn get_light_state(shared: State<'_, Arc<Shared>>) -> Result<StateSnapshot, String> {
    let timeout = shared.prefs.lock().unwrap().session_timeout_secs();
    let path = state_path().map_err(|error| error.to_string())?;
    let store = read_store(&path).map_err(|error| error.to_string())?;
    Ok(store.snapshot(now_secs(), timeout))
}

#[tauri::command]
fn get_sessions(shared: State<'_, Arc<Shared>>) -> Result<Vec<SessionSummary>, String> {
    let prefs = shared.prefs.lock().unwrap().clone();
    let path = state_path().map_err(|error| error.to_string())?;
    let store = read_store(&path).map_err(|error| error.to_string())?;
    Ok(store.summaries(
        now_secs(),
        prefs.session_timeout_secs(),
        prefs.show_idle_sessions,
    ))
}

#[tauri::command]
fn get_prefs(shared: State<'_, Arc<Shared>>) -> Preferences {
    shared.prefs.lock().unwrap().clone()
}

#[tauri::command]
fn toggle_orientation(shared: State<'_, Arc<Shared>>) -> Result<Preferences, String> {
    let mut prefs = shared.prefs.lock().unwrap();
    prefs.orientation = prefs.orientation.toggled();
    let path = prefs_path().map_err(|error| error.to_string())?;
    save_prefs(&path, &prefs).map_err(|error| error.to_string())?;
    Ok(prefs.clone())
}

/// Size the window to the widget (plus the popup when `popup_w > 0`) without moving the
/// widget on screen. The popup opens below the widget, or above it when there is no room,
/// and the window shifts left when the popup would run off the right screen edge.
#[tauri::command]
fn set_layout(
    window: WebviewWindow,
    shared: State<'_, Arc<Shared>>,
    widget_w: f64,
    widget_h: f64,
    popup_w: f64,
    popup_h: f64,
) -> Result<Placement, String> {
    let scale = window.scale_factor().map_err(|e| e.to_string())?;
    let position = window.outer_position().map_err(|e| e.to_string())?;
    let mut offset = shared.widget_offset.lock().unwrap();

    let origin_x = f64::from(position.x) / scale + offset.0;
    let origin_y = f64::from(position.y) / scale + offset.1;

    let has_popup = popup_w > 0.0 && popup_h > 0.0;
    // `width`/`height` and `new_offset` describe the visible content; SHADOW_PAD is added around it.
    let width = if has_popup { widget_w.max(popup_w) } else { widget_w };
    let mut height = widget_h;
    let mut new_offset = (0.0, 0.0);
    let mut popup_above = false;

    let monitor = window.current_monitor().map_err(|e| e.to_string())?;
    let (left, right, bottom) = match &monitor {
        Some(m) => {
            let s = m.scale_factor();
            (
                f64::from(m.position().x) / s,
                (f64::from(m.position().x) + f64::from(m.size().width)) / s,
                (f64::from(m.position().y) + f64::from(m.size().height)) / s,
            )
        }
        None => (f64::MIN, f64::MAX, f64::MAX),
    };

    if has_popup {
        height = widget_h + TAIL_GAP + popup_h;
        let overflow = origin_x + width - right;
        new_offset.0 = overflow.max(0.0).min(width - widget_w);
        // Never push the window past the left edge either.
        new_offset.0 = new_offset.0.min((origin_x - left).max(0.0));
        if origin_y + widget_h + TAIL_GAP + popup_h > bottom {
            popup_above = true;
            new_offset.1 = popup_h + TAIL_GAP;
        }
    }

    let new_offset = (new_offset.0 + SHADOW_PAD, new_offset.1 + SHADOW_PAD);
    let new_x = origin_x - new_offset.0;
    let new_y = origin_y - new_offset.1;
    window
        .set_position(LogicalPosition::new(new_x.round(), new_y.round()))
        .map_err(|e| e.to_string())?;
    window
        .set_size(LogicalSize::new(
            (width + 2.0 * SHADOW_PAD).ceil(),
            (height + 2.0 * SHADOW_PAD).ceil(),
        ))
        .map_err(|e| e.to_string())?;
    *offset = new_offset;

    Ok(Placement {
        offset_x: new_offset.0,
        offset_y: new_offset.1,
        popup_above,
    })
}

fn main() {
    let prefs = prefs_path().map(|path| load_prefs(&path)).unwrap_or_default();
    let audio = Audio::start(sounds_dir().ok());
    let shared = Arc::new(Shared {
        prefs: Mutex::new(prefs),
        sound_files: Mutex::new(Vec::new()),
        widget_offset: Mutex::new((SHADOW_PAD, SHADOW_PAD)),
        audio,
    });
    shared.rescan_sounds();
    watcher::spawn(shared.clone());

    tauri::Builder::default()
        .manage(shared)
        .invoke_handler(tauri::generate_handler![
            get_light_state,
            get_sessions,
            get_prefs,
            toggle_orientation,
            set_layout
        ])
        .setup(|app| {
            tray::install(app.handle())?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("failed to launch Claude Traffic Light");
}

pub fn main_window(app: &tauri::AppHandle) -> Option<WebviewWindow> {
    app.get_webview_window("main")
}
