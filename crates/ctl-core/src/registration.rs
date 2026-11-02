use std::env;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use fs2::FileExt;
use serde_json::{Value, json};
use tempfile::NamedTempFile;

const HOOK_EVENTS: &[&str] = &[
    "SessionStart",
    "SessionEnd",
    "UserPromptSubmit",
    "PreToolUse",
    "PostToolUse",
    "PermissionRequest",
    "Stop",
    "StopFailure",
    "PostModelSwitch",
    "SubagentStart",
    "SubagentStop",
];

pub fn user_settings_path() -> io::Result<PathBuf> {
    let home = env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "home directory is unavailable"))?;
    Ok(PathBuf::from(home).join(".claude").join("settings.json"))
}

pub fn register_user_hooks(settings_path: &Path, hook_path: &Path) -> io::Result<()> {
    let parent = settings_path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;

    let lock_path = settings_path.with_extension("json.ctl-lock");
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(lock_path)?;
    lock.lock_exclusive()?;

    let original = match fs::read(settings_path) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };

    let mut settings = match original.as_deref() {
        Some(bytes) => {
            let bytes = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes);
            serde_json::from_slice::<Value>(bytes)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?
        }
        None => json!({}),
    };

    let settings_object = settings.as_object_mut().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "Claude settings must be a JSON object",
        )
    })?;
    let hooks = settings_object
        .entry("hooks")
        .or_insert_with(|| Value::Object(Default::default()))
        .as_object_mut()
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "Claude hooks must be a JSON object",
            )
        })?;

    let hook_command = hook_path.to_string_lossy();
    let mut changed = false;
    for event_name in HOOK_EVENTS {
        let groups = hooks
            .entry(*event_name)
            .or_insert_with(|| Value::Array(Vec::new()))
            .as_array_mut()
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("Claude hook event {event_name} must be an array"),
                )
            })?;

        let mut found = false;
        for group in groups.iter_mut() {
            let Some(handlers) = group.get_mut("hooks") else {
                continue;
            };
            let handlers = handlers.as_array_mut().ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("Claude hook handlers for {event_name} must be an array"),
                )
            })?;

            handlers.retain_mut(|handler| {
                if handler.get("command").and_then(Value::as_str) != Some(hook_command.as_ref()) {
                    return true;
                }

                if found {
                    changed = true;
                    return false;
                }
                found = true;

                let Some(handler) = handler.as_object_mut() else {
                    return true;
                };
                for (key, value) in [
                    ("type", Value::String("command".to_owned())),
                    ("args", Value::Array(Vec::new())),
                    ("async", Value::Bool(true)),
                ] {
                    if handler.get(key) != Some(&value) {
                        handler.insert(key.to_owned(), value);
                        changed = true;
                    }
                }
                true
            });
        }

        if !found {
            groups.push(json!({
                "hooks": [{
                    "type": "command",
                    "command": hook_command,
                    "args": [],
                    "async": true
                }]
            }));
            changed = true;
        }
    }

    if !changed {
        FileExt::unlock(&lock)?;
        return Ok(());
    }

    if let Some(original) = original.as_deref() {
        backup_once(settings_path, original)?;
    }
    write_json_atomic(settings_path, &settings)?;
    FileExt::unlock(&lock)?;
    Ok(())
}

fn backup_once(settings_path: &Path, original: &[u8]) -> io::Result<()> {
    let backup_path = settings_path.with_extension("json.ctl-backup");
    if backup_path.exists() {
        return Ok(());
    }

    let parent = settings_path.parent().unwrap_or_else(|| Path::new("."));
    let mut temporary = NamedTempFile::new_in(parent)?;
    temporary.write_all(original)?;
    temporary.as_file().sync_all()?;

    match temporary.persist_noclobber(backup_path) {
        Ok(_) => Ok(()),
        Err(error) if error.error.kind() == io::ErrorKind::AlreadyExists => Ok(()),
        Err(error) => Err(error.error),
    }
}

fn write_json_atomic(path: &Path, settings: &Value) -> io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let mut temporary = NamedTempFile::new_in(parent)?;
    serde_json::to_writer_pretty(&mut temporary, settings)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    temporary.write_all(b"\n")?;
    if let Ok(metadata) = fs::metadata(path) {
        temporary
            .as_file()
            .set_permissions(metadata.permissions())?;
    }
    temporary.as_file().sync_all()?;
    temporary.persist(path).map_err(|error| error.error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registration_is_idempotent_repairs_entries_and_keeps_backup_once() {
        let directory = tempfile::tempdir().unwrap();
        let settings_path = directory.path().join("settings.json");
        let hook_path = directory.path().join("bin/ctl-hook");
        let original = br#"{"permissions":{"allow":[]},"hooks":{"SessionStart":[{"hooks":[{"type":"command","command":"/tmp/test/bin/ctl-hook","async":false}]}]},"custom":true}"#;
        fs::write(&settings_path, original).unwrap();

        register_user_hooks(&settings_path, &hook_path).unwrap();
        let first_install = fs::read(&settings_path).unwrap();
        register_user_hooks(&settings_path, &hook_path).unwrap();
        let settings_text = String::from_utf8(fs::read(&settings_path).unwrap()).unwrap();
        let settings: Value = serde_json::from_str(&settings_text).unwrap();

        for event in HOOK_EVENTS {
            let handlers = settings["hooks"][*event]
                .as_array()
                .unwrap()
                .iter()
                .flat_map(|group| group["hooks"].as_array().into_iter().flatten())
                .filter(|handler| handler["command"] == hook_path.to_string_lossy().as_ref())
                .collect::<Vec<_>>();
            assert_eq!(handlers.len(), 1, "duplicate handler for {event}");
            assert_eq!(handlers[0]["args"], json!([]));
            assert_eq!(handlers[0]["async"], true);
        }

        assert_eq!(
            fs::read(settings_path.with_extension("json.ctl-backup")).unwrap(),
            original
        );
        assert!(settings_text.find("permissions").unwrap() < settings_text.find("hooks").unwrap());
        assert!(settings_text.find("hooks").unwrap() < settings_text.find("custom").unwrap());
        assert_eq!(fs::read(settings_path).unwrap(), first_install);
    }

    #[test]
    fn malformed_settings_are_refused_without_backup_or_modification() {
        let directory = tempfile::tempdir().unwrap();
        let settings_path = directory.path().join("settings.json");
        let malformed = b"{ definitely not json";
        fs::write(&settings_path, malformed).unwrap();

        assert!(register_user_hooks(&settings_path, Path::new("/tmp/ctl-hook")).is_err());
        assert_eq!(fs::read(&settings_path).unwrap(), malformed);
        assert!(!settings_path.with_extension("json.ctl-backup").exists());
    }

    #[test]
    fn wrong_hooks_shape_is_refused() {
        let directory = tempfile::tempdir().unwrap();
        let settings_path = directory.path().join("settings.json");
        fs::write(&settings_path, br#"{"hooks":false}"#).unwrap();

        assert!(register_user_hooks(&settings_path, Path::new("/tmp/ctl-hook")).is_err());
    }
}
