//! Evdev keycode → XT scancode (AT set 1) conversion.
//!
//! GTK4 hardware keycodes on Linux = evdev keycode + 8.
//! SPICE expects XT scancodes: regular keys are 1 byte,
//! extended keys use 0xE0 prefix in the high byte (e.g. 0xE053 for Delete).

/// Convert a GTK4 hardware keycode to an XT scancode for SPICE.
/// Returns 0 for unknown keys.
pub fn gtk_keycode_to_xt(hardware_keycode: u32) -> u32 {
    if hardware_keycode < 8 {
        return 0;
    }
    let evdev = (hardware_keycode - 8) as u16;
    evdev_to_xt(evdev)
}

/// Convert an evdev keycode to an XT scancode.
fn evdev_to_xt(evdev: u16) -> u32 {
    // Extended keys that differ from the simple evdev == XT mapping.
    match evdev {
        96 => 0xE01C,  // KP Enter
        97 => 0xE01D,  // Right Ctrl
        98 => 0xE035,  // KP /
        99 => 0xE037,  // Print Screen
        100 => 0xE038, // Right Alt
        102 => 0xE047, // Home
        103 => 0xE048, // Up
        104 => 0xE049, // Page Up
        105 => 0xE04B, // Left
        106 => 0xE04D, // Right
        107 => 0xE04F, // End
        108 => 0xE050, // Down
        109 => 0xE051, // Page Down
        110 => 0xE052, // Insert
        111 => 0xE053, // Delete
        119 => 0xE045, // Pause
        125 => 0xE05B, // Left Super
        126 => 0xE05C, // Right Super
        127 => 0xE05D, // Menu

        // Multimedia and power keys. These sit above the 1:1 range but below
        // the old `< 128` cutoff, so they used to be forwarded verbatim — which
        // lands on the Japanese-layout scancodes in AT set 1.
        113 => 0xE020, // Mute
        114 => 0xE02E, // Volume Down
        115 => 0xE030, // Volume Up
        116 => 0xE05E, // Power
        163 => 0xE019, // Next Track
        164 => 0xE022, // Play/Pause
        165 => 0xE010, // Previous Track
        166 => 0xE024, // Stop

        // evdev 1..=88 share their numbering with AT set 1. Past that the two
        // diverge (89..=95 are the Japanese-layout keys), so anything unlisted
        // returns 0 and the caller leaves it to the host.
        _ if evdev <= 88 => u32::from(evdev),
        _ => 0,
    }
}

/// XT scancodes for Ctrl+Alt+Del sequence.
pub const XT_LEFT_CTRL: u32 = 0x1D;
pub const XT_LEFT_ALT: u32 = 0x38;
pub const XT_DELETE: u32 = 0xE053;

#[cfg(test)]
mod tests {
    use super::*;

    /// GTK hardware keycode for a given evdev code, per the +8 offset.
    fn gtk(evdev: u32) -> u32 {
        evdev + 8
    }

    #[test]
    fn plain_keys_pass_through_unchanged() {
        assert_eq!(gtk_keycode_to_xt(gtk(1)), 0x01, "Escape");
        assert_eq!(gtk_keycode_to_xt(gtk(30)), 0x1E, "A");
        assert_eq!(gtk_keycode_to_xt(gtk(57)), 0x39, "Space");
        assert_eq!(gtk_keycode_to_xt(gtk(29)), 0x1D, "Left Ctrl");
        assert_eq!(gtk_keycode_to_xt(gtk(56)), 0x38, "Left Alt");
        assert_eq!(gtk_keycode_to_xt(gtk(42)), 0x2A, "Left Shift");
        assert_eq!(gtk_keycode_to_xt(gtk(28)), 0x1C, "Enter");
    }

    #[test]
    fn extended_keys_get_e0_prefix() {
        assert_eq!(gtk_keycode_to_xt(gtk(96)), 0xE01C, "KP Enter");
        assert_eq!(gtk_keycode_to_xt(gtk(97)), 0xE01D, "Right Ctrl");
        assert_eq!(gtk_keycode_to_xt(gtk(100)), 0xE038, "Right Alt");
        assert_eq!(gtk_keycode_to_xt(gtk(103)), 0xE048, "Up");
        assert_eq!(gtk_keycode_to_xt(gtk(111)), 0xE053, "Delete");
        assert_eq!(gtk_keycode_to_xt(gtk(125)), 0xE05B, "Left Super");
        assert_eq!(gtk_keycode_to_xt(gtk(127)), 0xE05D, "Menu");
    }

    #[test]
    fn rejects_codes_below_the_evdev_offset() {
        for keycode in 0..8 {
            assert_eq!(gtk_keycode_to_xt(keycode), 0, "keycode {keycode} is not a valid evdev key");
        }
    }

    #[test]
    fn rejects_codes_above_the_xt_range() {
        assert_eq!(gtk_keycode_to_xt(gtk(128)), 0);
        assert_eq!(gtk_keycode_to_xt(gtk(240)), 0);
        assert_eq!(gtk_keycode_to_xt(u32::MAX), 0);
    }

    /// Media and power keys are above the 1:1 range and need explicit extended
    /// scancodes. Forwarding them verbatim — as the old `< 128` cutoff did —
    /// lands on the Japanese-layout keys instead.
    #[test]
    fn media_keys_use_extended_scancodes() {
        assert_eq!(gtk_keycode_to_xt(gtk(113)), 0xE020, "Mute");
        assert_eq!(gtk_keycode_to_xt(gtk(114)), 0xE02E, "Volume Down");
        assert_eq!(gtk_keycode_to_xt(gtk(115)), 0xE030, "Volume Up");
        assert_eq!(gtk_keycode_to_xt(gtk(116)), 0xE05E, "Power");
        assert_eq!(gtk_keycode_to_xt(gtk(163)), 0xE019, "Next Track");
        assert_eq!(gtk_keycode_to_xt(gtk(164)), 0xE022, "Play/Pause");
        assert_eq!(gtk_keycode_to_xt(gtk(165)), 0xE010, "Previous Track");
        assert_eq!(gtk_keycode_to_xt(gtk(166)), 0xE024, "Stop");
    }

    /// The 1:1 identity holds up to F12 (evdev 88) and no further.
    #[test]
    fn identity_range_stops_at_f12() {
        assert_eq!(gtk_keycode_to_xt(gtk(87)), 0x57, "F11");
        assert_eq!(gtk_keycode_to_xt(gtk(88)), 0x58, "F12");
    }

    /// evdev 89..=95 are Japanese-layout keys whose XT codes are not their
    /// evdev numbers. Returning 0 leaves them to the host rather than sending
    /// the guest a wrong keystroke.
    #[test]
    fn japanese_layout_range_is_not_forwarded() {
        for evdev in 89..=95 {
            assert_eq!(gtk_keycode_to_xt(gtk(evdev)), 0, "evdev {evdev}");
        }
    }

    /// evdev 0 is "reserved" and maps to XT 0, which is also our sentinel for
    /// "unknown key". Callers must therefore treat 0 as "do not send".
    #[test]
    fn evdev_zero_is_indistinguishable_from_unknown() {
        assert_eq!(gtk_keycode_to_xt(gtk(0)), 0);
    }

    /// Every extended scancode must be above one byte, otherwise it would
    /// collide with a plain key and send the wrong keystroke to the guest.
    #[test]
    fn extended_scancodes_never_collide_with_plain_ones() {
        let extended = [
            96, 97, 98, 99, 100, 102, 103, 104, 105, 106, 107, 108, 109, 110, 111, 113, 114, 115,
            116, 119, 125, 126, 127, 163, 164, 165, 166,
        ];
        for evdev in extended {
            let xt = gtk_keycode_to_xt(gtk(evdev));
            assert!(xt > 0xFF, "evdev {evdev} produced {xt:#06X}, which collides with a plain key");
            assert_eq!(xt >> 8, 0xE0, "evdev {evdev} should carry the 0xE0 prefix");
        }
    }

    /// The hardcoded Ctrl+Alt+Del constants must agree with what the mapping
    /// produces for those physical keys — otherwise the toolbar button sends
    /// a different chord than the keyboard does.
    #[test]
    fn ctrl_alt_del_constants_match_the_mapping() {
        assert_eq!(XT_LEFT_CTRL, gtk_keycode_to_xt(gtk(29)));
        assert_eq!(XT_LEFT_ALT, gtk_keycode_to_xt(gtk(56)));
        assert_eq!(XT_DELETE, gtk_keycode_to_xt(gtk(111)));
    }
}
