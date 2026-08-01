//! Raw FFI bindings to SpiceClientGLib-2.0.

#![allow(non_camel_case_types, dead_code)]

use std::os::raw::{c_char, c_int, c_void};

pub type gboolean = c_int;
pub type gpointer = *mut c_void;
pub type guint = std::os::raw::c_uint;
pub type gulong = std::os::raw::c_ulong;
pub type gint = c_int;

// Opaque GObject types
#[repr(C)]
pub struct SpiceSession {
    _opaque: [u8; 0],
}
#[repr(C)]
pub struct SpiceChannel {
    _opaque: [u8; 0],
}
#[repr(C)]
pub struct SpiceDisplayChannel {
    _opaque: [u8; 0],
}
#[repr(C)]
pub struct SpiceInputsChannel {
    _opaque: [u8; 0],
}

// Channel type constants
pub const SPICE_CHANNEL_MAIN: c_int = 1;
pub const SPICE_CHANNEL_DISPLAY: c_int = 2;
pub const SPICE_CHANNEL_INPUTS: c_int = 3;
pub const SPICE_CHANNEL_CURSOR: c_int = 4;
pub const SPICE_CHANNEL_PLAYBACK: c_int = 5;

// Clipboard selection constants
pub const VD_AGENT_CLIPBOARD_SELECTION_CLIPBOARD: guint = 0;

// Clipboard data type constants (VD_AGENT_CLIPBOARD_*)
pub const VD_AGENT_CLIPBOARD_NONE: guint = 0;
pub const VD_AGENT_CLIPBOARD_UTF8_TEXT: guint = 1;
pub const VD_AGENT_CLIPBOARD_IMAGE_PNG: guint = 2;
pub const VD_AGENT_CLIPBOARD_IMAGE_BMP: guint = 3;

// Mouse button constants
pub const SPICE_MOUSE_BUTTON_LEFT: c_int = 1;
pub const SPICE_MOUSE_BUTTON_MIDDLE: c_int = 2;
pub const SPICE_MOUSE_BUTTON_RIGHT: c_int = 3;
pub const SPICE_MOUSE_BUTTON_UP: c_int = 4;
pub const SPICE_MOUSE_BUTTON_DOWN: c_int = 5;
pub const SPICE_MOUSE_BUTTON_SIDE: c_int = 6;
pub const SPICE_MOUSE_BUTTON_EXTRA: c_int = 7;

// Mouse button state mask.
//
// These are NOT `1 << button`. Verified by disassembling
// `spice_inputs_channel_button_press` in libspice-client-glib-2.0.so.8.8.2:
// its jump table ORs 0x1 for LEFT, 0x2 for MIDDLE, 0x4 for RIGHT, 0x20 for
// SIDE and 0x40 for EXTRA, and the scroll buttons contribute nothing.
// Getting these wrong makes a left-drag arrive in the guest as a middle-drag,
// because `spice_inputs_channel_position` forwards this mask verbatim.
pub const SPICE_MOUSE_BUTTON_MASK_LEFT: guint = 1 << 0;
pub const SPICE_MOUSE_BUTTON_MASK_MIDDLE: guint = 1 << 1;
pub const SPICE_MOUSE_BUTTON_MASK_RIGHT: guint = 1 << 2;
pub const SPICE_MOUSE_BUTTON_MASK_SIDE: guint = 1 << 5;
pub const SPICE_MOUSE_BUTTON_MASK_EXTRA: guint = 1 << 6;

unsafe extern "C" {
    // --- SpiceSession ---
    pub fn spice_session_new() -> *mut SpiceSession;
    pub fn spice_session_connect(session: *mut SpiceSession) -> gboolean;
    pub fn spice_session_disconnect(session: *mut SpiceSession);

    // --- GObject property access ---
    pub fn g_object_get(object: gpointer, first_property: *const c_char, ...);

    // --- SpiceChannel ---
    pub fn spice_channel_connect(channel: *mut SpiceChannel) -> gboolean;

    // --- SpiceAudio ---
    // Binds a GStreamer sink to the session's playback/record channels. Without
    // it those channels still open but their samples go nowhere.
    // Returns (transfer none) — owned by the session, must NOT be unreffed.
    pub fn spice_audio_get(session: *mut SpiceSession, context: gpointer) -> gpointer;

    // --- SpiceMainChannel ---
    pub fn spice_main_channel_update_display(
        channel: gpointer, // SpiceMainChannel*
        id: c_int,
        x: c_int,
        y: c_int,
        width: c_int,
        height: c_int,
        enabled: gboolean,
    );
    pub fn spice_main_channel_update_display_enabled(
        channel: gpointer,
        id: c_int,
        enabled: gboolean,
        update: gboolean,
    );

    // --- SpiceMainChannel clipboard ---
    pub fn spice_main_channel_clipboard_selection_grab(
        channel: gpointer,
        selection: guint,
        types: *const guint,
        ntypes: guint,
    );
    pub fn spice_main_channel_clipboard_selection_notify(
        channel: gpointer,
        selection: guint,
        type_: guint,
        data: *const u8,
        size: guint,
    );
    pub fn spice_main_channel_clipboard_selection_request(
        channel: gpointer,
        selection: guint,
        type_: guint,
    );
    pub fn spice_main_channel_clipboard_selection_release(channel: gpointer, selection: guint);

    // --- SpiceInputsChannel ---
    pub fn spice_inputs_channel_key_press(channel: *mut SpiceInputsChannel, scancode: guint);
    pub fn spice_inputs_channel_key_release(channel: *mut SpiceInputsChannel, scancode: guint);
    pub fn spice_inputs_channel_key_press_and_release(
        channel: *mut SpiceInputsChannel,
        scancode: guint,
    );
    pub fn spice_inputs_channel_position(
        channel: *mut SpiceInputsChannel,
        x: gint,
        y: gint,
        display: gint,
        button_state: guint,
    );
    pub fn spice_inputs_channel_button_press(
        channel: *mut SpiceInputsChannel,
        button: gint,
        button_state: guint,
    );
    pub fn spice_inputs_channel_button_release(
        channel: *mut SpiceInputsChannel,
        button: gint,
        button_state: guint,
    );

    // --- GObject ---
    pub fn g_object_set(object: gpointer, first_property: *const c_char, ...);
    pub fn g_object_unref(object: gpointer);

    // g_signal_connect_data — underlying impl of g_signal_connect macro
    pub fn g_signal_connect_data(
        instance: gpointer,
        detailed_signal: *const c_char,
        c_handler: Option<unsafe extern "C" fn()>,
        data: gpointer,
        destroy_data: Option<unsafe extern "C" fn(gpointer, gpointer)>,
        connect_flags: c_int,
    ) -> gulong;
}

/// Helper to connect a `GLib` signal with user data.
///
/// # Safety
/// `callback` must match the signal's C callback signature.
pub unsafe fn signal_connect(
    instance: gpointer,
    signal: &str,
    callback: unsafe extern "C" fn(),
    data: gpointer,
) -> gulong {
    let signal_cstr = std::ffi::CString::new(signal).unwrap();
    unsafe { g_signal_connect_data(instance, signal_cstr.as_ptr(), Some(callback), data, None, 0) }
}
