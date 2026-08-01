//! SUPERSEDED — safe to delete this whole file.
//!
//! These tests re-implement the logic they claim to cover instead of calling
//! it, so they cannot fail when the real code regresses. They have been
//! replaced by `#[cfg(test)]` modules next to the functions themselves:
//! `src/spice/keymap.rs`, `src/config.rs`, `src/vm.rs`, `src/spice/mod.rs`,
//! and `src/ui/mod.rs`. A binary crate's `tests/` directory cannot reach
//! module-private functions, which is why the coverage lives inline.

mod keymap {
    // Import the keymap module via the binary crate
    // Since these are const/pure functions, test them directly.

    /// Evdev→XT scancode conversion (mirrors `spice::keymap` logic).
    fn gtk_keycode_to_xt(hardware_keycode: u32) -> u32 {
        if hardware_keycode < 8 {
            return 0;
        }
        let evdev = (hardware_keycode - 8) as u16;
        match evdev {
            96 => 0xE01C,
            97 => 0xE01D,
            98 => 0xE035,
            99 => 0xE037,
            100 => 0xE038,
            102 => 0xE047,
            103 => 0xE048,
            104 => 0xE049,
            105 => 0xE04B,
            106 => 0xE04D,
            107 => 0xE04F,
            108 => 0xE050,
            109 => 0xE051,
            110 => 0xE052,
            111 => 0xE053,
            119 => 0xE045,
            125 => 0xE05B,
            126 => 0xE05C,
            127 => 0xE05D,
            _ if evdev < 128 => u32::from(evdev),
            _ => 0,
        }
    }

    #[test]
    fn basic_keys() {
        // Escape: GTK keycode 9 → evdev 1 → XT 0x01
        assert_eq!(gtk_keycode_to_xt(9), 0x01);
        // 'A' key: GTK keycode 38 → evdev 30 → XT 0x1E
        assert_eq!(gtk_keycode_to_xt(38), 0x1E);
        // Space: GTK keycode 65 → evdev 57 → XT 0x39
        assert_eq!(gtk_keycode_to_xt(65), 0x39);
        // Left Ctrl: GTK keycode 37 → evdev 29 → XT 0x1D
        assert_eq!(gtk_keycode_to_xt(37), 0x1D);
        // Left Alt: GTK keycode 64 → evdev 56 → XT 0x38
        assert_eq!(gtk_keycode_to_xt(64), 0x38);
    }

    #[test]
    fn extended_keys() {
        // Delete: GTK keycode 119 → evdev 111 → XT 0xE053
        assert_eq!(gtk_keycode_to_xt(119), 0xE053);
        // Home: GTK keycode 110 → evdev 102 → XT 0xE047
        assert_eq!(gtk_keycode_to_xt(110), 0xE047);
        // Up arrow: GTK keycode 111 → evdev 103 → XT 0xE048
        assert_eq!(gtk_keycode_to_xt(111), 0xE048);
        // Right Ctrl: GTK keycode 105 → evdev 97 → XT 0xE01D
        assert_eq!(gtk_keycode_to_xt(105), 0xE01D);
        // Left Super: GTK keycode 133 → evdev 125 → XT 0xE05B
        assert_eq!(gtk_keycode_to_xt(133), 0xE05B);
    }

    #[test]
    fn edge_cases() {
        // Below 8: invalid
        assert_eq!(gtk_keycode_to_xt(0), 0);
        assert_eq!(gtk_keycode_to_xt(7), 0);
        // Exactly 8: evdev 0 → XT 0
        assert_eq!(gtk_keycode_to_xt(8), 0);
        // High keycode: evdev >= 128 → unknown
        assert_eq!(gtk_keycode_to_xt(200), 0);
    }
}

mod config {
    use std::fs;
    use std::path::PathBuf;

    fn test_config_path() -> PathBuf {
        let dir = std::env::temp_dir().join("vmlaunch-test");
        let _ = fs::create_dir_all(&dir);
        dir.join("state.conf")
    }

    #[test]
    fn round_trip_config() {
        let path = test_config_path();
        let contents = "width=1920\nheight=1080\nfullscreen=true\n";
        fs::write(&path, contents).unwrap();

        let text = fs::read_to_string(&path).unwrap();
        let mut width = 1280;
        let mut height = 800;
        let mut fullscreen = false;

        for line in text.lines() {
            let line = line.trim();
            if let Some(val) = line.strip_prefix("width=") {
                width = val.parse().unwrap_or(1280);
            } else if let Some(val) = line.strip_prefix("height=") {
                height = val.parse().unwrap_or(800);
            } else if let Some(val) = line.strip_prefix("fullscreen=") {
                fullscreen = val == "true";
            }
        }

        assert_eq!(width, 1920);
        assert_eq!(height, 1080);
        assert!(fullscreen);

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn defaults_on_missing_file() {
        let path = test_config_path().with_file_name("nonexistent.conf");
        assert!(!path.exists());
        // Would return defaults — just verify the path logic doesn't panic
    }

    #[test]
    fn handles_malformed_config() {
        let path = test_config_path().with_file_name("bad.conf");
        fs::write(&path, "garbage\nwidth=abc\nfullscreen=maybe\n").unwrap();

        let text = fs::read_to_string(&path).unwrap();
        let mut width = 1280;
        let mut fullscreen = false;

        for line in text.lines() {
            if let Some(val) = line.strip_prefix("width=") {
                width = val.parse().unwrap_or(1280);
            } else if let Some(val) = line.strip_prefix("fullscreen=") {
                fullscreen = val == "true";
            }
        }

        assert_eq!(width, 1280); // falls back to default
        assert!(!fullscreen); // "maybe" != "true"

        let _ = fs::remove_file(&path);
    }
}
