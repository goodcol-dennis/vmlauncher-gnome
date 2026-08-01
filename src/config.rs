//! Persistent window state (size, fullscreen) across sessions.

use std::fs;
use std::path::PathBuf;

/// Saved window state.
pub struct WindowState {
    pub width: i32,
    pub height: i32,
    pub fullscreen: bool,
}

impl Default for WindowState {
    fn default() -> Self {
        Self { width: 1280, height: 800, fullscreen: false }
    }
}

fn config_path() -> PathBuf {
    let config_dir = std::env::var("XDG_CONFIG_HOME").map_or_else(
        |_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
            PathBuf::from(home).join(".config")
        },
        PathBuf::from,
    );
    config_dir.join("vmlaunch").join("state.conf")
}

/// Smallest window we'll restore to. A saved `0` or negative width would
/// otherwise be handed straight to GTK as a size request.
const MIN_DIM: i32 = 320;
/// Largest window we'll restore to — guards against a corrupt config
/// resurrecting a window bigger than any display.
const MAX_DIM: i32 = 16384;

/// Parse the `key=value` config format into a [`WindowState`].
///
/// Unknown keys, malformed numbers, and out-of-range dimensions all fall back
/// to the corresponding default rather than failing the whole load.
fn parse(contents: &str) -> WindowState {
    let mut state = WindowState::default();
    for line in contents.lines() {
        let line = line.trim();
        if let Some(val) = line.strip_prefix("width=") {
            if let Ok(v) = val.parse() {
                state.width = v;
            }
        } else if let Some(val) = line.strip_prefix("height=") {
            if let Ok(v) = val.parse() {
                state.height = v;
            }
        } else if let Some(val) = line.strip_prefix("fullscreen=") {
            state.fullscreen = val == "true";
        }
    }

    let defaults = WindowState::default();
    if !(MIN_DIM..=MAX_DIM).contains(&state.width) {
        log::warn!("config: width {} out of range, using default", state.width);
        state.width = defaults.width;
    }
    if !(MIN_DIM..=MAX_DIM).contains(&state.height) {
        log::warn!("config: height {} out of range, using default", state.height);
        state.height = defaults.height;
    }
    state
}

/// Render a [`WindowState`] back to the `key=value` config format.
fn serialize(state: &WindowState) -> String {
    format!("width={}\nheight={}\nfullscreen={}\n", state.width, state.height, state.fullscreen)
}

/// Load saved window state, or return defaults.
pub fn load() -> WindowState {
    let path = config_path();
    let Ok(contents) = fs::read_to_string(&path) else {
        return WindowState::default();
    };
    parse(&contents)
}

/// Save current window state.
pub fn save(state: &WindowState) {
    let path = config_path();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Err(e) = fs::write(&path, serialize(state)) {
        log::error!("failed to save window state: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_well_formed_config() {
        let state = parse("width=1920\nheight=1080\nfullscreen=true\n");
        assert_eq!(state.width, 1920);
        assert_eq!(state.height, 1080);
        assert!(state.fullscreen);
    }

    #[test]
    fn missing_keys_fall_back_to_defaults() {
        let defaults = WindowState::default();
        let state = parse("width=1600\n");
        assert_eq!(state.width, 1600);
        assert_eq!(state.height, defaults.height);
        assert!(!state.fullscreen);
    }

    #[test]
    fn unparseable_numbers_fall_back_to_defaults() {
        let defaults = WindowState::default();
        let state = parse("width=abc\nheight=\nfullscreen=maybe\n");
        assert_eq!(state.width, defaults.width);
        assert_eq!(state.height, defaults.height);
        assert!(!state.fullscreen, "only the exact string \"true\" enables fullscreen");
    }

    /// A zero or negative dimension would be handed to GTK as a size request.
    /// These must be rejected rather than restored.
    #[test]
    fn non_positive_dimensions_are_rejected() {
        let defaults = WindowState::default();
        for contents in ["width=0\nheight=0\n", "width=-1\nheight=-800\n"] {
            let state = parse(contents);
            assert_eq!(state.width, defaults.width, "{contents:?}");
            assert_eq!(state.height, defaults.height, "{contents:?}");
        }
    }

    /// A corrupt config must not resurrect a window larger than any display.
    #[test]
    fn absurd_dimensions_are_rejected() {
        let defaults = WindowState::default();
        let state = parse("width=99999\nheight=2147483647\n");
        assert_eq!(state.width, defaults.width);
        assert_eq!(state.height, defaults.height);
    }

    #[test]
    fn accepts_dimensions_at_the_range_boundaries() {
        let state = parse(&format!("width={MIN_DIM}\nheight={MAX_DIM}\n"));
        assert_eq!(state.width, MIN_DIM);
        assert_eq!(state.height, MAX_DIM);
    }

    #[test]
    fn ignores_unknown_keys_and_junk_lines() {
        let state = parse("garbage\n\n# comment\nmonitor=DP-1\nwidth=1400\n");
        assert_eq!(state.width, 1400);
    }

    /// Each line is trimmed before the key is split off, so padding on either
    /// side of a `key=value` pair is harmless.
    #[test]
    fn tolerates_surrounding_whitespace() {
        let state = parse("  width=1400\n\theight=900\n  fullscreen=true  \n");
        assert_eq!(state.width, 1400);
        assert_eq!(state.height, 900);
        assert!(state.fullscreen);
    }

    /// `str::lines` leaves the carriage return behind on CRLF input, which
    /// would make `"true\r"` compare unequal to `"true"` were the line not
    /// trimmed first.
    #[test]
    fn tolerates_crlf_line_endings() {
        let state = parse("width=1400\r\nheight=900\r\nfullscreen=true\r\n");
        assert_eq!(state.width, 1400);
        assert_eq!(state.height, 900);
        assert!(state.fullscreen);
    }

    #[test]
    fn empty_input_yields_defaults() {
        let defaults = WindowState::default();
        let state = parse("");
        assert_eq!(state.width, defaults.width);
        assert_eq!(state.height, defaults.height);
        assert!(!state.fullscreen);
    }

    #[test]
    fn serialize_emits_every_key() {
        let out = serialize(&WindowState { width: 1368, height: 806, fullscreen: false });
        assert_eq!(out, "width=1368\nheight=806\nfullscreen=false\n");
    }

    #[test]
    fn parse_and_serialize_round_trip() {
        for original in [
            WindowState { width: 1920, height: 1080, fullscreen: true },
            WindowState { width: 1368, height: 806, fullscreen: false },
            WindowState::default(),
        ] {
            let restored = parse(&serialize(&original));
            assert_eq!(restored.width, original.width);
            assert_eq!(restored.height, original.height);
            assert_eq!(restored.fullscreen, original.fullscreen);
        }
    }

    #[test]
    fn config_path_lands_under_the_xdg_directory() {
        let path = config_path();
        assert!(path.ends_with("vmlaunch/state.conf"), "unexpected path: {}", path.display());
        assert!(path.is_absolute() || std::env::var("HOME").is_err());
    }
}
