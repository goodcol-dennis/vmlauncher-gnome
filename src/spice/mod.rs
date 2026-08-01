//! SPICE client — session management, display framebuffer, and input forwarding.

pub mod clipboard;
pub mod ffi;
pub mod keymap;

use std::cell::RefCell;
use std::ffi::CString;
use std::ptr;
use std::rc::Rc;

/// Framebuffer data received from the SPICE display channel.
pub struct Framebuffer {
    pub data: *const u8,
}

/// Shared state for the SPICE client, accessible from `GLib` signal callbacks.
pub struct SpiceState {
    pub session: *mut ffi::SpiceSession,
    pub main_channel: ffi::gpointer,
    pub inputs_channel: *mut ffi::SpiceInputsChannel,
    pub framebuffer: Option<Framebuffer>,
    pub mouse_button_state: u32,
    /// Called when the display framebuffer is first created or resized.
    pub on_display_ready: Option<Box<dyn Fn(i32, i32)>>,
    /// Called when a region of the display is invalidated (needs redraw).
    pub on_invalidate: Option<Box<dyn Fn(i32, i32, i32, i32)>>,
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
        on_display_ready: Some(Box::new(on_ready)),
        on_invalidate: Some(Box::new(on_invalidate)),
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
pub fn key_press(state: &SharedState, hardware_keycode: u32) {
    let s = state.borrow();
    if s.inputs_channel.is_null() {
        return;
    }
    let scancode = keymap::gtk_keycode_to_xt(hardware_keycode);
    if scancode != 0 {
        unsafe {
            ffi::spice_inputs_channel_key_press(s.inputs_channel, scancode);
        }
    }
}

/// Forward a key release from GTK to SPICE.
pub fn key_release(state: &SharedState, hardware_keycode: u32) {
    let s = state.borrow();
    if s.inputs_channel.is_null() {
        return;
    }
    let scancode = keymap::gtk_keycode_to_xt(hardware_keycode);
    if scancode != 0 {
        unsafe {
            ffi::spice_inputs_channel_key_release(s.inputs_channel, scancode);
        }
    }
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
            s.framebuffer = Some(Framebuffer { data: data as *const u8 });
        }

        let s = state.borrow();
        if let Some(ref cb) = s.on_display_ready {
            cb(width, height);
        }
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
