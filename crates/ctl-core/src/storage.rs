use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use fs2::FileExt;
use tempfile::NamedTempFile;

use crate::{SessionEvent, SessionStore};

const LOCK_TIMEOUT: Duration = Duration::from_secs(2);
const LOCK_RETRY: Duration = Duration::from_millis(20);

pub fn state_path() -> io::Result<PathBuf> {
    if let Some(path) = env::var_os("CTL_STATE_PATH") {
        return Ok(PathBuf::from(path));
    }

    let home = env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "home directory is unavailable"))?;

    Ok(PathBuf::from(home)
        .join(".claude-traffic-light")
        .join("state.json"))
}

pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub fn read_store(path: &Path) -> io::Result<SessionStore> {
    match fs::read(path) {
        Ok(bytes) => {
            let bytes = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&bytes);
            serde_json::from_slice(bytes)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(SessionStore::default()),
        Err(error) => Err(error),
    }
}

pub fn apply_event(path: &Path, event: SessionEvent, timeout_secs: u64) -> io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;

    let lock_path = path.with_extension("json.lock");
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(lock_path)?;
    acquire_lock(&lock)?;

    let mut store = read_store(path)?;
    let timestamp_secs = event.timestamp_secs;
    store.apply(event);
    store.prune_stale(timestamp_secs, timeout_secs);
    write_store_atomic(path, &store)?;
    FileExt::unlock(&lock)?;
    Ok(())
}

pub fn write_store_atomic(path: &Path, store: &SessionStore) -> io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;

    let mut temporary = NamedTempFile::new_in(parent)?;
    serde_json::to_writer(&mut temporary, store)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    temporary.write_all(b"\n")?;
    temporary.as_file().sync_all()?;
    temporary.persist(path).map_err(|error| error.error)?;
    Ok(())
}

fn acquire_lock(lock: &File) -> io::Result<()> {
    let deadline = std::time::Instant::now() + LOCK_TIMEOUT;
    loop {
        match lock.try_lock_exclusive() {
            Ok(()) => return Ok(()),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                if std::time::Instant::now() >= deadline {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "timed out waiting for the state file lock",
                    ));
                }
                thread::sleep(LOCK_RETRY);
            }
            Err(error) => return Err(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::{SessionEventKind, SessionStatus};

    fn start_event(session_id: &str, timestamp_secs: u64) -> SessionEvent {
        SessionEvent {
            session_id: session_id.to_owned(),
            timestamp_secs,
            kind: SessionEventKind::SessionStart {
                cwd: "/work/project".to_owned(),
            },
        }
    }

    #[test]
    fn reads_state_with_utf8_bom() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("state.json");
        let bytes = serde_json::to_vec(&SessionStore::default()).unwrap();
        let mut bom_bytes = vec![0xef, 0xbb, 0xbf];
        bom_bytes.extend(bytes);
        fs::write(&path, bom_bytes).unwrap();

        assert_eq!(read_store(&path).unwrap(), SessionStore::default());
    }

    #[test]
    fn apply_event_persists_atomically_and_reads_back() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("state.json");
        apply_event(&path, start_event("session-a", 100), 7_200).unwrap();

        let store = read_store(&path).unwrap();
        assert_eq!(store.sessions["session-a"].status, SessionStatus::Idle);
    }

    #[test]
    fn malformed_existing_state_is_not_overwritten() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("state.json");
        fs::write(&path, b"not json").unwrap();

        assert!(apply_event(&path, start_event("session-a", 100), 7_200).is_err());
        assert_eq!(fs::read(path).unwrap(), b"not json");
    }

    #[test]
    fn concurrent_updates_keep_both_sessions() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("state.json");
        let first_path = path.clone();
        let second_path = path.clone();
        let first = thread::spawn(move || {
            apply_event(&first_path, start_event("one", 100), 7_200).unwrap();
        });
        let second = thread::spawn(move || {
            apply_event(&second_path, start_event("two", 100), 7_200).unwrap();
        });
        first.join().unwrap();
        second.join().unwrap();

        let store = read_store(&path).unwrap();
        assert!(store.sessions.contains_key("one"));
        assert!(store.sessions.contains_key("two"));
    }
}
