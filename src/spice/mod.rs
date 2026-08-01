//! SPICE client — session management, display framebuffer, and input forwarding.

pub mod clipboard;
pub mod ffi;
pub mod keymap;

use std::cell::RefCell;
use std::collections::HashSet;
use std::ffi::CString;
use std::ptr;
use std::rc::Rc;

/// Framebuffer data received from the SPICE display channel.
///
/// `data` is borrowed from spice-gtk, which frees it on `display-primary-destroy`.
/// The geometry travels with the pointer so a reader can size its slice from what
/// SPICE actually reported rather than re-deriving it, which would over-read if
/// the two ever disagreed.
pub struct Framebuffer {
    pub data: *const u8,
    pub width: i32,
    pub height: i32,
    pub stride: i32,
}

impl Framebuffer {
    /// Total length of the pixel buffer in bytes.
    pub fn byte_len(&self) -> usize {
        (self.stride as usize).saturating_mul(self.height as usize)
    }
}

/// Shared state for the SPICE client, accessible from `GLib` signal callbacks.
pub struct SpiceState {
    pub session: *mut ffi::SpiceSession,
    pub main_channel: ffi::gpointer,
    pub inputs_channel: *mut ffi::SpiceInputsChannel,
    pub framebuffer: Option<Framebuffer>,
    pub mouse_button_state: u32,
    /// XT scancodes currently held down, so they can be released if the window
    /// loses focus mid-keypress. GTK4 does not synthesize releases on focus-out,
    /// so without this an Alt+Tab away leaves Alt stuck down inside the guest.
    pub held_keys: HashSet<u32>,
    /// Called when the display framebuffer is first created or resized.
    pub on_display_ready: Option<Box<dyn Fn(i32, i32)>>,
    /// Called when a region of the display is invalidated (needs redraw).
    pub on_invalidate: Option<Box<dyn Fn(i32, i32, i32, i32)>>,
    /// Host-clipboard subscription, torn down with the session.
    pub clipboard: Option<clipboard::Handle>,
}

pub type SharedState = Rc<RefCell<SpiceState>>;

/// Create a new SPICE session and connect to the given URI.
/// Returns shared state that can be used for input forwarding and display access.
pub fn connect(
    uri: &str,
    on_ready: impl Fn(i32, i32) + 'static,
    on_invalidate: impl Fn(i32, i32, i32, i32) + 'static,
) -> anyhow::Result<SharedState> {
    let state = Rc::new(RefCell::new(SpiceState {
        session: ptr::null_mut(),
        main_channel: ptr::null_mut(),
        inputs_channel: ptr::null_mut(),
        framebuffer: None,
        mouse_button_state: 0,
        held_keys: HashSet::new(),
        on_display_ready: Some(Box::new(on_ready)),
        on_invalidate: Some(Box::new(on_invalidate)),
        clipboard: None,
    }));

    unsafe {
        let session = ffi::spice_session_new();
        if session.is_null() {
            anyhow::bail!("spice_session_new() returned null");
        }

        // Set the URI property
        let prop = CString::new("uri").unwrap();
        let uri_c = CString::new(uri)?;
        ffi::g_object_set(
            session as ffi::gpointer,
            prop.as_ptr(),
            uri_c.as_ptr(),
            ptr::null::<std::os::raw::c_void>(),
        );

        state.borrow_mut().session = session;

        // Leak an Rc reference for the callback user_data — lives until disconnect.
        let state_ptr = Rc::into_raw(state.clone()) as ffi::gpointer;

        // Connect "channel-new" signal
        ffi::signal_connect(
            session as ffi::gpointer,
            "channel-new",
            std::mem::transmute::<
                unsafe extern "C" fn(*mut ffi::SpiceSession, *mut ffi::SpiceChannel, ffi::gpointer),
                unsafe extern "C" fn(),
            >(on_channel_new),
            state_ptr,
        );

        // Bind an audio sink before connecting, so the playback and record
        // channels have somewhere to go the moment they open. The returned
        // object is owned by the session — do not unref it.
        if ffi::spice_audio_get(session, ptr::null_mut()).is_null() {
            log::warn!("SPICE audio unavailable — the VM will be silent");
        } else {
            log::info!("SPICE audio enabled");
        }

        // Start the connection
        let ok = ffi::spice_session_connect(session);
        if ok == 0 {
            ffi::g_object_unref(session as ffi::gpointer);
            // Reclaim the leaked Rc
            let _ = Rc::from_raw(state_ptr as *const RefCell<SpiceState>);
            anyhow::bail!("spice_session_connect() failed");
        }

        log::info!("SPICE session connecting to {uri}");
    }

    Ok(state)
}

/// Disconnect the SPICE session and clean up.
pub fn disconnect(state: &SharedState) {
    let mut s = state.borrow_mut();
    // Detach from the app-global host clipboard first: it outlives the session,
    // and its handler holds a pointer to the channel we are about to finalize.
    if let Some(ref mut handle) = s.clipboard {
        handle.teardown();
    }
    s.clipboard = None;
    s.held_keys.clear();
    if !s.session.is_null() {
        unsafe {
            ffi::spice_session_disconnect(s.session);
            ffi::g_object_unref(s.session as ffi::gpointer);
        }
        log::info!("SPICE session disconnected");
        s.session = ptr::null_mut();
    }
    s.inputs_channel = ptr::null_mut();
    s.framebuffer = None;
}

/// Ask the guest to switch its display to `width` x `height` physical pixels.
///
/// Requires the SPICE agent and a resizable display driver in the guest; where
/// either is missing this is silently ignored, which is the desired fallback.
pub fn set_display_size(state: &SharedState, width: i32, height: i32) {
    let s = state.borrow();
    if s.main_channel.is_null() || width <= 0 || height <= 0 {
        return;
    }
    unsafe {
        ffi::spice_main_channel_update_display(s.main_channel, 0, 0, 0, width, height, 1);
    }
    log::info!("requested guest resolution {width}x{height}");
}

/// Send Ctrl+Alt+Del to the VM.
pub fn send_ctrl_alt_del(state: &SharedState) {
    let s = state.borrow();
    if s.inputs_channel.is_null() {
        log::warn!("no inputs channel — cannot send Ctrl+Alt+Del");
        return;
    }
    unsafe {
        let ch = s.inputs_channel;
        ffi::spice_inputs_channel_key_press(ch, keymap::XT_LEFT_CTRL);
        ffi::spice_inputs_channel_key_press(ch, keymap::XT_LEFT_ALT);
        ffi::spice_inputs_channel_key_press_and_release(ch, keymap::XT_DELETE);
        ffi::spice_inputs_channel_key_release(ch, keymap::XT_LEFT_ALT);
        ffi::spice_inputs_channel_key_release(ch, keymap::XT_LEFT_CTRL);
    }
    log::info!("sent Ctrl+Alt+Del");
}

/// Forward a key press from GTK to SPICE.
///
/// Returns `false` when the key has no XT scancode, so the caller can let it
/// propagate to the host instead of swallowing it.
pub fn key_press(state: &SharedState, hardware_keycode: u32) -> bool {
    let mut s = state.borrow_mut();
    if s.inputs_channel.is_null() {
        return false;
    }
    let scancode = keymap::gtk_keycode_to_xt(hardware_keycode);
    if scancode == 0 {
        return false;
    }
    s.held_keys.insert(scancode);
    unsafe {
        ffi::spice_inputs_channel_key_press(s.inputs_channel, scancode);
    }
    true
}

/// Forward a key release from GTK to SPICE.
pub fn key_release(state: &SharedState, hardware_keycode: u32) -> bool {
    let mut s = state.borrow_mut();
    if s.inputs_channel.is_null() {
        return false;
    }
    let scancode = keymap::gtk_keycode_to_xt(hardware_keycode);
    if scancode == 0 {
        return false;
    }
    s.held_keys.remove(&scancode);
    unsafe {
        ffi::spice_inputs_channel_key_release(s.inputs_channel, scancode);
    }
    true
}

/// Release every key the guest currently believes is held.
///
/// Call this whenever the window loses focus: the host compositor consumes the
/// key-up for whatever chord took focus away (Alt+Tab, Super), so the guest
/// would otherwise never see those keys come back up.
pub fn release_all_keys(state: &SharedState) {
    let mut s = state.borrow_mut();
    if s.inputs_channel.is_null() {
        s.held_keys.clear();
        return;
    }
    let channel = s.inputs_channel;
    let held: Vec<u32> = s.held_keys.drain().collect();
    if held.is_empty() {
        return;
    }
    for scancode in &held {
        unsafe {
            ffi::spice_inputs_channel_key_release(channel, *scancode);
        }
    }
    log::debug!("released {} held key(s) on focus loss", held.len());
}

/// Forward mouse position to SPICE (absolute coordinates in VM display space).
pub fn mouse_position(state: &SharedState, x: i32, y: i32) {
    let s = state.borrow();
    if s.inputs_channel.is_null() {
        return;
    }
    unsafe {
        ffi::spice_inputs_channel_position(
            s.inputs_channel,
            x,
            y,
            0, // display channel 0
            s.mouse_button_state,
        );
    }
}

/// Forward mouse button press to SPICE.
pub fn mouse_button_press(state: &SharedState, button: i32) {
    let mut s = state.borrow_mut();
    if s.inputs_channel.is_null() {
        return;
    }
    let mask = button_to_mask(button);
    s.mouse_button_state |= mask;
    unsafe {
        ffi::spice_inputs_channel_button_press(s.inputs_channel, button, s.mouse_button_state);
    }
}

/// Forward mouse button release to SPICE.
pub fn mouse_button_release(state: &SharedState, button: i32) {
    let mut s = state.borrow_mut();
    if s.inputs_channel.is_null() {
        return;
    }
    let mask = button_to_mask(button);
    s.mouse_button_state &= !mask;
    unsafe {
        ffi::spice_inputs_channel_button_release(s.inputs_channel, button, s.mouse_button_state);
    }
}

/// Forward mouse scroll to SPICE (as button press+release).
pub fn mouse_scroll(state: &SharedState, direction_up: bool) {
    let s = state.borrow();
    if s.inputs_channel.is_null() {
        return;
    }
    let button =
        if direction_up { ffi::SPICE_MOUSE_BUTTON_UP } else { ffi::SPICE_MOUSE_BUTTON_DOWN };
    unsafe {
        ffi::spice_inputs_channel_button_press(s.inputs_channel, button, s.mouse_button_state);
        ffi::spice_inputs_channel_button_release(s.inputs_channel, button, s.mouse_button_state);
    }
}

fn button_to_mask(button: i32) -> u32 {
    match button {
        ffi::SPICE_MOUSE_BUTTON_LEFT => ffi::SPICE_MOUSE_BUTTON_MASK_LEFT,
        ffi::SPICE_MOUSE_BUTTON_MIDDLE => ffi::SPICE_MOUSE_BUTTON_MASK_MIDDLE,
        ffi::SPICE_MOUSE_BUTTON_RIGHT => ffi::SPICE_MOUSE_BUTTON_MASK_RIGHT,
        ffi::SPICE_MOUSE_BUTTON_SIDE => ffi::SPICE_MOUSE_BUTTON_MASK_SIDE,
        ffi::SPICE_MOUSE_BUTTON_EXTRA => ffi::SPICE_MOUSE_BUTTON_MASK_EXTRA,
        _ => 0,
    }
}

// --- GLib signal callbacks (called from C) ---

unsafe extern "C" fn on_channel_new(
    _session: *mut ffi::SpiceSession,
    channel: *mut ffi::SpiceChannel,
    user_data: ffi::gpointer,
) {
    unsafe {
        let state = &*(user_data as *const RefCell<SpiceState>);

        // Read channel-type and channel-id via GObject properties
        let mut channel_type: std::os::raw::c_int = 0;
        let mut channel_id: std::os::raw::c_int = 0;
        let prop_type = std::ffi::CString::new("channel-type").unwrap();
        let prop_id = std::ffi::CString::new("channel-id").unwrap();
        ffi::g_object_get(
            channel as ffi::gpointer,
            prop_type.as_ptr(),
            &raw mut channel_type,
            prop_id.as_ptr(),
            &raw mut channel_id,
            std::ptr::null::<std::os::raw::c_void>(),
        );
        log::info!("SPICE channel-new: type={channel_type} id={channel_id}");

        match channel_type {
            ffi::SPICE_CHANNEL_MAIN => {
                state.borrow_mut().main_channel = channel as ffi::gpointer;
                log::info!("SPICE main channel stored");

                // Enable the display channel without specifying dimensions.
                // The VM keeps its own resolution; we adapt to whatever it sends.
                ffi::spice_main_channel_update_display_enabled(channel as ffi::gpointer, 0, 1, 0);
            }
            ffi::SPICE_CHANNEL_DISPLAY => {
                ffi::signal_connect(
                    channel as ffi::gpointer,
                    "display-primary-create",
                    std::mem::transmute::<
                        unsafe extern "C" fn(
                            *mut ffi::SpiceDisplayChannel,
                            ffi::gint,
                            ffi::gint,
                            ffi::gint,
                            ffi::gint,
                            ffi::gint,
                            ffi::gpointer,
                            ffi::gpointer,
                        ),
                        unsafe extern "C" fn(),
                    >(on_display_primary_create),
                    user_data,
                );
                ffi::signal_connect(
                    channel as ffi::gpointer,
                    "display-invalidate",
                    std::mem::transmute::<
                        unsafe extern "C" fn(
                            *mut ffi::SpiceDisplayChannel,
                            ffi::gint,
                            ffi::gint,
                            ffi::gint,
                            ffi::gint,
                            ffi::gpointer,
                        ),
                        unsafe extern "C" fn(),
                    >(on_display_invalidate),
                    user_data,
                );
                // Without this, the cached framebuffer pointer outlives the
                // buffer spice-gtk frees on a guest resolution change, and the
                // next render reads freed memory.
                ffi::signal_connect(
                    channel as ffi::gpointer,
                    "display-primary-destroy",
                    std::mem::transmute::<
                        unsafe extern "C" fn(*mut ffi::SpiceDisplayChannel, ffi::gpointer),
                        unsafe extern "C" fn(),
                    >(on_display_primary_destroy),
                    user_data,
                );

                // Explicitly connect the display channel
                log::info!("Explicitly connecting display channel");
                ffi::spice_channel_connect(channel);
            }
            ffi::SPICE_CHANNEL_INPUTS => {
                state.borrow_mut().inputs_channel = channel.cast::<ffi::SpiceInputsChannel>();
                log::info!("SPICE inputs channel — explicitly connecting");
                ffi::spice_channel_connect(channel);
            }
            ffi::SPICE_CHANNEL_CURSOR => {
                log::info!("Explicitly connecting cursor channel");
                ffi::spice_channel_connect(channel);
            }
            _ => {}
        }
    }
}

unsafe extern "C" fn on_display_primary_create(
    _channel: *mut ffi::SpiceDisplayChannel,
    format: ffi::gint,
    width: ffi::gint,
    height: ffi::gint,
    stride: ffi::gint,
    _shmid: ffi::gint,
    data: ffi::gpointer,
    user_data: ffi::gpointer,
) {
    unsafe {
        let state = &*(user_data as *const RefCell<SpiceState>);
        log::info!(
            "SPICE display primary created: {width}x{height} stride={stride} format={format}"
        );

        {
            let mut s = state.borrow_mut();
            s.framebuffer = Some(Framebuffer { data: data as *const u8, width, height, stride });
        }

        let s = state.borrow();
        if let Some(ref cb) = s.on_display_ready {
            cb(width, height);
        }
    }
}

/// spice-gtk is about to free the primary surface. Drop our borrowed pointer
/// before it dangles — the renderer already treats `None` as "nothing to draw".
unsafe extern "C" fn on_display_primary_destroy(
    _channel: *mut ffi::SpiceDisplayChannel,
    user_data: ffi::gpointer,
) {
    unsafe {
        let state = &*(user_data as *const RefCell<SpiceState>);
        state.borrow_mut().framebuffer = None;
        log::info!("SPICE display primary destroyed — framebuffer released");
    }
}

unsafe extern "C" fn on_display_invalidate(
    _channel: *mut ffi::SpiceDisplayChannel,
    x: ffi::gint,
    y: ffi::gint,
    width: ffi::gint,
    height: ffi::gint,
    user_data: ffi::gpointer,
) {
    unsafe {
        let state = &*(user_data as *const RefCell<SpiceState>);
        let s = state.borrow();
        if let Some(ref cb) = s.on_invalidate {
            cb(x, y, width, height);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pinned to the literal values the SPICE protocol defines, NOT to the
    /// other constants — an earlier version of this table used `1 << button`,
    /// which is self-consistent but wrong, and a test written in terms of the
    /// constants passed happily against it.
    ///
    /// Ground truth: the jump table in `spice_inputs_channel_button_press`
    /// (libspice-client-glib-2.0.so.8.8.2) ORs these exact values.
    #[test]
    fn masks_match_the_spice_protocol_values() {
        assert_eq!(button_to_mask(ffi::SPICE_MOUSE_BUTTON_LEFT), 0x1);
        assert_eq!(button_to_mask(ffi::SPICE_MOUSE_BUTTON_MIDDLE), 0x2);
        assert_eq!(button_to_mask(ffi::SPICE_MOUSE_BUTTON_RIGHT), 0x4);
        assert_eq!(button_to_mask(ffi::SPICE_MOUSE_BUTTON_SIDE), 0x20);
        assert_eq!(button_to_mask(ffi::SPICE_MOUSE_BUTTON_EXTRA), 0x40);
    }

    /// The button numbers are protocol constants too, and an off-by-one here
    /// would silently send the wrong button.
    #[test]
    fn button_numbers_match_the_spice_protocol_values() {
        assert_eq!(ffi::SPICE_MOUSE_BUTTON_LEFT, 1);
        assert_eq!(ffi::SPICE_MOUSE_BUTTON_MIDDLE, 2);
        assert_eq!(ffi::SPICE_MOUSE_BUTTON_RIGHT, 3);
        assert_eq!(ffi::SPICE_MOUSE_BUTTON_UP, 4);
        assert_eq!(ffi::SPICE_MOUSE_BUTTON_DOWN, 5);
        assert_eq!(ffi::SPICE_MOUSE_BUTTON_SIDE, 6);
        assert_eq!(ffi::SPICE_MOUSE_BUTTON_EXTRA, 7);
    }

    /// Scroll wheel "buttons" are momentary events, not held state, so they
    /// must not contribute a bit to the persistent button mask.
    #[test]
    fn scroll_buttons_contribute_no_mask_bits() {
        assert_eq!(button_to_mask(ffi::SPICE_MOUSE_BUTTON_UP), 0);
        assert_eq!(button_to_mask(ffi::SPICE_MOUSE_BUTTON_DOWN), 0);
    }

    #[test]
    fn unknown_buttons_contribute_no_mask_bits() {
        assert_eq!(button_to_mask(0), 0);
        assert_eq!(button_to_mask(8), 0);
        assert_eq!(button_to_mask(-1), 0);
    }

    /// Every button mask must be a distinct single bit — if any two overlapped,
    /// releasing one button would clear the other's state and the guest would
    /// see a stuck button.
    #[test]
    fn masks_are_distinct_single_bits() {
        let masks = [
            ffi::SPICE_MOUSE_BUTTON_MASK_LEFT,
            ffi::SPICE_MOUSE_BUTTON_MASK_MIDDLE,
            ffi::SPICE_MOUSE_BUTTON_MASK_RIGHT,
            ffi::SPICE_MOUSE_BUTTON_MASK_SIDE,
            ffi::SPICE_MOUSE_BUTTON_MASK_EXTRA,
        ];
        for mask in masks {
            assert_eq!(mask.count_ones(), 1, "{mask:#b} is not a single bit");
        }
        for (i, a) in masks.iter().enumerate() {
            for b in &masks[i + 1..] {
                assert_eq!(a & b, 0, "{a:#b} overlaps {b:#b}");
            }
        }
    }

    /// Press then release of the same button must return the mask to its
    /// starting value, and interleaved buttons must not disturb each other.
    #[test]
    fn press_release_sequences_restore_the_mask() {
        let mut mask: u32 = 0;
        for button in [ffi::SPICE_MOUSE_BUTTON_LEFT, ffi::SPICE_MOUSE_BUTTON_RIGHT] {
            mask |= button_to_mask(button);
        }
        assert_eq!(mask.count_ones(), 2);

        mask &= !button_to_mask(ffi::SPICE_MOUSE_BUTTON_LEFT);
        assert_eq!(mask, ffi::SPICE_MOUSE_BUTTON_MASK_RIGHT);

        mask &= !button_to_mask(ffi::SPICE_MOUSE_BUTTON_RIGHT);
        assert_eq!(mask, 0);
    }
}
