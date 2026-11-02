//! Watches the aggregate light and plays the yellow/red alert sounds natively.

use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use ctl_core::Light;
use ctl_core::prefs::Preferences;
use ctl_core::storage::{now_secs, read_store, state_path};

use crate::Shared;

const POLL: Duration = Duration::from_millis(400);

pub fn sound_for(prefs: &Preferences, light: Light) -> Option<&str> {
    match light {
        Light::Yellow => Some(prefs.yellow_sound.as_str()),
        Light::Red => Some(prefs.red_sound.as_str()),
        Light::Dark | Light::Green => None,
    }
}

pub fn spawn(shared: Arc<Shared>) {
    thread::Builder::new()
        .name("ctl-watcher".to_owned())
        .spawn(move || run(&shared))
        .expect("failed to start watcher thread");
}

fn run(shared: &Shared) {
    // The first observation is a baseline: starting the app while already red is not a change.
    let mut current: Option<Light> = None;
    let mut repeats_done: u8 = 0;
    let mut repeating = false;
    let mut next_repeat_at = Instant::now();

    loop {
        thread::sleep(POLL);
        let prefs = shared.prefs.lock().unwrap().clone();
        let Ok(path) = state_path() else { continue };
        let Ok(store) = read_store(&path) else { continue };
        let light = store.aggregate_light(now_secs(), prefs.session_timeout_secs());

        if current != Some(light) {
            let is_baseline = current.is_none();
            current = Some(light);
            repeats_done = 0;
            repeating = false;
            if !is_baseline
                && let Some(sound) = sound_for(&prefs, light)
                && sound != ctl_core::prefs::SOUND_NONE
            {
                shared.audio.play(sound);
                repeating = true;
                next_repeat_at = Instant::now() + Duration::from_secs(prefs.repeat_interval_secs());
            }
            continue;
        }

        if repeating
            && repeats_done < prefs.repeat_count
            && Instant::now() >= next_repeat_at
            && let Some(sound) = sound_for(&prefs, light)
        {
            shared.audio.play(sound);
            repeats_done += 1;
            next_repeat_at = Instant::now() + Duration::from_secs(prefs.repeat_interval_secs());
        }
    }
}
