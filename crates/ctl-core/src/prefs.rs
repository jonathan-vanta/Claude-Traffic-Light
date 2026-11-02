use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tempfile::NamedTempFile;

pub const SIZE_CHOICES: &[u8] = &[1, 2, 3, 5, 10];
pub const TEXT_SIZE_CHOICES: &[u8] = &[1, 2, 3];
pub const POPUP_SIZE_CHOICES: &[u8] = &[100, 75, 50, 35];
pub const REPEAT_INTERVAL_CHOICES: &[u8] = &[1, 2, 5];
pub const TIMEOUT_CHOICES_MINUTES: &[u32] = &[15, 30, 60, 120, 240, 480];
pub const MAX_REPEAT_COUNT: u8 = 10;
pub const SOUND_NONE: &str = "none";
pub const SOUND_TONE_CHIME: &str = "builtin:chime";
pub const SOUND_TONE_ALERT: &str = "builtin:alert";
const FILE_PREFIX: &str = "file:";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Theme {
    FollowOs,
    Dark,
    Light,
    ContrastOs,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Orientation {
    #[default]
    Horizontal,
    Vertical,
}

impl Orientation {
    pub fn toggled(self) -> Self {
        match self {
            Self::Horizontal => Self::Vertical,
            Self::Vertical => Self::Horizontal,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct Preferences {
    pub orientation: Orientation,
    pub size: u8,
    pub text_size: u8,
    pub popup_size: u8,
    pub theme: Theme,
    pub yellow_sound: String,
    pub red_sound: String,
    /// Extra plays after the first while the light stays on the color. 0 turns repeat off.
    pub repeat_count: u8,
    pub repeat_interval_minutes: u8,
    pub show_idle_sessions: bool,
    pub session_timeout_minutes: u32,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            orientation: Orientation::Horizontal,
            size: 1,
            text_size: 1,
            popup_size: 100,
            theme: Theme::FollowOs,
            yellow_sound: SOUND_NONE.to_owned(),
            red_sound: SOUND_NONE.to_owned(),
            repeat_count: 0,
            repeat_interval_minutes: 1,
            show_idle_sessions: true,
            session_timeout_minutes: 120,
        }
    }
}

fn nearest<T: Copy + Ord>(value: T, choices: &[T], fallback: T) -> T {
    if choices.contains(&value) {
        value
    } else {
        fallback
    }
}

impl Preferences {
    /// Replace out-of-range values (hand-edited file) with defaults.
    pub fn sanitized(mut self) -> Self {
        let defaults = Self::default();
        self.size = nearest(self.size, SIZE_CHOICES, defaults.size);
        self.text_size = nearest(self.text_size, TEXT_SIZE_CHOICES, defaults.text_size);
        self.popup_size = nearest(self.popup_size, POPUP_SIZE_CHOICES, defaults.popup_size);
        self.repeat_count = self.repeat_count.min(MAX_REPEAT_COUNT);
        self.repeat_interval_minutes = nearest(
            self.repeat_interval_minutes,
            REPEAT_INTERVAL_CHOICES,
            defaults.repeat_interval_minutes,
        );
        self.session_timeout_minutes = self.session_timeout_minutes.clamp(15, 480);
        self
    }

    pub fn session_timeout_secs(&self) -> u64 {
        u64::from(self.session_timeout_minutes) * 60
    }

    pub fn repeat_interval_secs(&self) -> u64 {
        u64::from(self.repeat_interval_minutes) * 60
    }
}

pub fn file_sound_id(file_name: &str) -> String {
    format!("{FILE_PREFIX}{file_name}")
}

pub fn file_sound_name(sound_id: &str) -> Option<&str> {
    sound_id.strip_prefix(FILE_PREFIX)
}

fn home_dir() -> io::Result<PathBuf> {
    env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "home directory is unavailable"))
}

pub fn data_dir() -> io::Result<PathBuf> {
    if let Some(path) = env::var_os("CTL_DATA_DIR") {
        return Ok(PathBuf::from(path));
    }
    Ok(home_dir()?.join(".claude-traffic-light"))
}

pub fn prefs_path() -> io::Result<PathBuf> {
    Ok(data_dir()?.join("preferences.json"))
}

pub fn sounds_dir() -> io::Result<PathBuf> {
    Ok(data_dir()?.join("sounds"))
}

/// A missing file gives defaults. An unreadable or malformed file also gives
/// defaults, and is left untouched until the next explicit save.
pub fn load_prefs(path: &Path) -> Preferences {
    let Ok(bytes) = fs::read(path) else {
        return Preferences::default();
    };
    let bytes = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&bytes);
    serde_json::from_slice::<Preferences>(bytes)
        .map(Preferences::sanitized)
        .unwrap_or_default()
}

pub fn save_prefs(path: &Path, prefs: &Preferences) -> io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let mut temporary = NamedTempFile::new_in(parent)?;
    serde_json::to_writer_pretty(&mut temporary, prefs)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    temporary.write_all(b"\n")?;
    temporary.as_file().sync_all()?;
    temporary.persist(path).map_err(|error| error.error)?;
    Ok(())
}

/// File names of `.wav` and `.mp3` files in the sounds directory, sorted.
pub fn list_sound_files(dir: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| {
            let lower = name.to_ascii_lowercase();
            !name.starts_with('.') && (lower.ends_with(".wav") || lower.ends_with(".mp3"))
        })
        .collect();
    names.sort_by_key(|name| name.to_ascii_lowercase());
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_have_no_sound_and_no_repeat() {
        let prefs = Preferences::default();
        assert_eq!(prefs.yellow_sound, SOUND_NONE);
        assert_eq!(prefs.red_sound, SOUND_NONE);
        assert_eq!(prefs.repeat_count, 0);
    }

    #[test]
    fn missing_fields_take_defaults_and_bad_values_are_sanitized() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("preferences.json");
        fs::write(
            &path,
            br#"{"size": 7, "theme": "contrast_os", "repeat_count": 99, "session_timeout_minutes": 1}"#,
        )
        .unwrap();
        let prefs = load_prefs(&path);
        assert_eq!(prefs.size, 1);
        assert_eq!(prefs.theme, Theme::ContrastOs);
        assert_eq!(prefs.repeat_count, MAX_REPEAT_COUNT);
        assert_eq!(prefs.session_timeout_minutes, 15);
        assert_eq!(prefs.popup_size, 100);
    }

    #[test]
    fn malformed_file_yields_defaults_and_is_not_rewritten_on_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("preferences.json");
        fs::write(&path, b"{ nope").unwrap();
        assert_eq!(load_prefs(&path), Preferences::default());
        assert_eq!(fs::read(&path).unwrap(), b"{ nope");
    }

    #[test]
    fn orientation_defaults_to_horizontal_and_toggles() {
        assert_eq!(Preferences::default().orientation, Orientation::Horizontal);
        assert_eq!(Orientation::Horizontal.toggled(), Orientation::Vertical);
        assert_eq!(Orientation::Vertical.toggled(), Orientation::Horizontal);
        let parsed: Preferences = serde_json::from_str(r#"{"orientation": "vertical"}"#).unwrap();
        assert_eq!(parsed.orientation, Orientation::Vertical);
    }

    #[test]
    fn save_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("preferences.json");
        let prefs = Preferences {
            size: 3,
            theme: Theme::Light,
            red_sound: file_sound_id("bell.wav"),
            ..Preferences::default()
        };
        save_prefs(&path, &prefs).unwrap();
        assert_eq!(load_prefs(&path), prefs);
    }

    #[test]
    fn lists_only_wav_and_mp3_sorted() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["b.WAV", "a.mp3", "c.txt", ".hidden.wav"] {
            fs::write(dir.path().join(name), b"x").unwrap();
        }
        assert_eq!(list_sound_files(dir.path()), vec!["a.mp3", "b.WAV"]);
    }
}
