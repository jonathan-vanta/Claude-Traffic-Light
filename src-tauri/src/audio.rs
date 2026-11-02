//! Native sound playback. A dedicated thread owns the output stream because
//! the stream handle is not `Send` on every platform.

use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant};

use ctl_core::prefs::{SOUND_NONE, SOUND_TONE_ALERT, SOUND_TONE_CHIME, file_sound_name};
use rodio::source::{SineWave, Source};
use rodio::{Decoder, DeviceSinkBuilder, MixerDeviceSink, Player};

/// A stream that has been idle this long is reopened before the next play, which
/// recovers from a stalled device or a changed default output (sleep/wake, unplugged headphones).
const STREAM_MAX_IDLE: Duration = Duration::from_secs(30);

enum Command {
    Play(String),
}

#[derive(Clone)]
pub struct Audio {
    sender: Sender<Command>,
}

impl Audio {
    pub fn start(sounds_dir: Option<PathBuf>) -> Self {
        let (sender, receiver) = mpsc::channel::<Command>();
        thread::Builder::new()
            .name("ctl-audio".to_owned())
            .spawn(move || {
                let mut stream: Option<MixerDeviceSink> = None;
                let mut last_used = Instant::now();
                let mut players: Vec<Player> = Vec::new();
                loop {
                    match receiver.recv_timeout(Duration::from_secs(5)) {
                        Ok(Command::Play(sound_id)) => {
                            players.retain(|player| !player.empty());
                            if last_used.elapsed() > STREAM_MAX_IDLE {
                                players.clear();
                                stream = None;
                            }
                            if stream.is_none() {
                                stream = open_stream();
                            }
                            let Some(sink) = stream.as_ref() else { continue };
                            match start_sound(sink, &sound_id, sounds_dir.as_deref()) {
                                Some(player) => players.push(player),
                                None => {
                                    // Playback failed: drop the stream so the next play reopens it.
                                    players.clear();
                                    stream = None;
                                }
                            }
                            last_used = Instant::now();
                        }
                        Err(RecvTimeoutError::Timeout) => {
                            players.retain(|player| !player.empty());
                            if players.is_empty() && last_used.elapsed() > STREAM_MAX_IDLE {
                                stream = None;
                            }
                        }
                        Err(RecvTimeoutError::Disconnected) => return,
                    }
                }
            })
            .expect("failed to start audio thread");
        Self { sender }
    }

    pub fn play(&self, sound_id: &str) {
        if sound_id != SOUND_NONE {
            let _ = self.sender.send(Command::Play(sound_id.to_owned()));
        }
    }
}

fn open_stream() -> Option<MixerDeviceSink> {
    let mut sink = DeviceSinkBuilder::open_default_sink().ok()?;
    sink.log_on_drop(false);
    Some(sink)
}

fn start_sound(
    sink: &MixerDeviceSink,
    sound_id: &str,
    sounds_dir: Option<&std::path::Path>,
) -> Option<Player> {
    let player = Player::connect_new(sink.mixer());
    match sound_id {
        SOUND_TONE_CHIME => {
            // Two rising notes.
            let first = SineWave::new(660.0)
                .take_duration(Duration::from_millis(180))
                .fade_out(Duration::from_millis(60))
                .amplify(0.35);
            let second = SineWave::new(880.0)
                .take_duration(Duration::from_millis(320))
                .fade_out(Duration::from_millis(160))
                .amplify(0.35);
            player.append(first);
            player.append(second);
        }
        SOUND_TONE_ALERT => {
            // Three short high beeps.
            for _ in 0..3 {
                let beep = SineWave::new(1_000.0)
                    .take_duration(Duration::from_millis(110))
                    .fade_in(Duration::from_millis(10))
                    .fade_out(Duration::from_millis(30))
                    .amplify(0.4);
                let gap = SineWave::new(1_000.0)
                    .take_duration(Duration::from_millis(80))
                    .amplify(0.0);
                player.append(beep);
                player.append(gap);
            }
        }
        other => {
            let name = file_sound_name(other)?;
            // Reject path components so a preference value cannot escape the sounds directory.
            if name.contains(['/', '\\']) || name == ".." {
                return None;
            }
            let file = File::open(sounds_dir?.join(name)).ok()?;
            let decoder = Decoder::try_from(BufReader::new(file)).ok()?;
            player.append(decoder);
        }
    }
    Some(player)
}
