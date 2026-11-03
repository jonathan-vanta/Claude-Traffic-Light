# Claude Traffic Light

A small always-on-top desktop widget that shows what your Claude Code sessions are doing:

| Light  | Meaning                                              |
| ------ | ---------------------------------------------------- |
| Red    | A session is waiting for you to approve a permission |
| Yellow | A session is working                                 |
| Green  | All sessions are idle                                |
| Off    | No active sessions                                   |

When several sessions are running, the most urgent one wins (red, then yellow, then green). Click the ⓘ button to see each session, its status, current tool and model.

It works through Claude Code hooks: a small `ctl-hook` binary runs on session events and writes the state to a file, and the widget reads that file.

## Requirements

- macOS 11 or later (the build script and app bundle are macOS-only)
- Rust 1.85 or later, for the 2024 edition (install with [rustup](https://rustup.rs) or `brew install rust`)
- Xcode Command Line Tools: `xcode-select --install`
- Claude Code

## Setup

Clone the repo, then run these two steps from its root.

### 1. Install the hook

```sh
cargo build --release -p ctl-hook
./target/release/ctl-hook install
```

This does two things:

- Copies the hook binary to `~/.claude-traffic-light/bin/ctl-hook`.
- Adds that binary as an async hook for the session events it needs in `~/.claude/settings.json`. Your existing settings are kept, and a one-time backup of the original file is saved as `~/.claude/settings.json.ctl-backup`.

Running `install` again is safe; it won't add duplicate entries.

Restart any Claude Code sessions that are already open so they pick up the new hooks.

### 2. Build and launch the app

```sh
scripts/make-app.sh
open ~/Applications/"Claude Traffic Light.app"
```

The script builds a release binary and installs it as `~/Applications/Claude Traffic Light.app`. Pass a different folder to install it elsewhere, for example `scripts/make-app.sh /Applications`.

To start it at login, add it under System Settings → General → Login Items.

## Using it

- **Move it:** drag the widget.
- **Switch between horizontal and vertical:** double-click the widget.
- **Session list:** click ⓘ. Click outside the list or press Esc to close it.
- **Settings:** use the menu bar icon. It has size, text size, popup size, theme, orientation, sounds, whether idle sessions are listed, the session timeout, Show/Hide and Quit.

### Custom sounds

Put `.wav` or `.mp3` files in `~/.claude-traffic-light/sounds/`, then choose **Sound → Rescan Sounds** from the menu bar icon. They will appear under Yellow Sound and Red Sound.

## Development

The repo is a Cargo workspace:

| Path               | What it is                                                          |
| ------------------ | ------------------------------------------------------------------- |
| `crates/ctl-core`  | Shared state model, preferences, and hook registration              |
| `crates/ctl-hook`  | The hook binary Claude Code calls                                   |
| `src-tauri`        | The Tauri 2 desktop app (window, tray menu, sounds, file watcher)   |
| `ui`               | The widget's HTML, CSS and JavaScript                               |
| `scripts`          | `make-app.sh` build-and-install script                              |

Common commands:

```sh
cargo test                     # tests for ctl-core and ctl-hook
cargo check -p ctl-app         # type-check the desktop app
scripts/make-app.sh            # rebuild and reinstall the app
```

After rebuilding, quit the running app from its menu bar icon and open it again to see your changes. If you change `crates/ctl-hook`, run `./target/release/ctl-hook install` again so the copy in `~/.claude-traffic-light/bin` is updated.

Some values must match between files. Each is marked with a comment: `TAIL_GAP` and `SHADOW_PAD` in `src-tauri/src/main.rs` correspond to `GAP` and `PAD` in `ui/main.js`.

### Where data lives

| File                                         | Contents                         |
| -------------------------------------------- | -------------------------------- |
| `~/.claude-traffic-light/state.json`         | Current session state            |
| `~/.claude-traffic-light/preferences.json`   | Settings chosen in the tray menu |
| `~/.claude-traffic-light/sounds/`            | Custom alert sounds              |

For testing, set `CTL_STATE_PATH` to use a different state file. The hook and the app both need to see the same value. `CTL_DATA_DIR` moves preferences and sounds (not the state file) and only affects the app.

## Uninstalling

1. Quit the app from its menu bar icon and delete `~/Applications/Claude Traffic Light.app`.
2. In `~/.claude/settings.json`, remove the hook entries whose `command` points to `~/.claude-traffic-light/bin/ctl-hook`. You can also restore `~/.claude/settings.json.ctl-backup`, but only if you haven't changed your settings since installing, since the backup won't include those changes.
3. Delete `~/.claude-traffic-light`.
