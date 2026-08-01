//! SPICE clipboard bridge — bidirectional text clipboard between host and guest.
//!
//! When the guest copies text, we receive it via the SPICE agent and set it on
//! the GTK4 clipboard. When the host copies text, we detect the change and push
//! it to the guest via the SPICE agent.
//!
//! Line endings are translated in both directions: the guest is a Windows VM
//! that expects CRLF, the host expects LF. Without this a multi-line copy into
//! Notepad arrives as a single line, and a copy out of the guest arrives with a
//! trailing `^M` on every line.

use std::cell::Cell;
use std::ptr;

use gtk4::gdk;
use gtk4::glib;
use gtk4::prelude::*;

use super::ffi;

/// Keeps the host-side clipboard subscription alive and lets it be torn down.
///
/// The GTK clipboard is app-global and outlives any single SPICE session, so the
/// `changed` handler must be disconnected when the session goes away — otherwise
/// the next host copy dereferences a finalized channel.
pub struct Handle {
    clipboard: gdk::Clipboard,
    handler_id: Option<glib::SignalHandlerId>,
    /// Leaked bridge shared with the C callbacks. Nulling its channel pointer
    /// makes every callback — including an in-flight async read — bail out.
    bridge: *mut ClipboardBridge,
}

impl Handle {
    /// Detach from the host clipboard and invalidate the channel pointer.
    pub fn teardown(&mut self) {
        if let Some(id) = self.handler_id.take() {
            self.clipboard.disconnect(id);
        }
        if !self.bridge.is_null() {
            unsafe {
                (*self.bridge).main_channel.set(ptr::null_mut());
            }
        }
        log::debug!("clipboard: bridge torn down");
    }
}

/// Translate guest (CRLF) line endings to host (LF).
fn dos_to_unix(text: &str) -> String {
    text.replace("\r\n", "\n")
}

/// Translate host (LF) line endings to guest (CRLF).
///
/// Normalises first so text that already contains CRLF isn't doubled up.
fn unix_to_dos(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\n', "\r\n")
}

/// Set up bidirectional clipboard sharing on the SPICE main channel.
///
/// # Safety
/// `main_channel` must be a valid `SpiceMainChannel*`.
pub fn setup(main_channel: ffi::gpointer, gdk_display: &gdk::Display) -> Handle {
    let clipboard = gdk_display.clipboard();

    // The C signal handlers below hold this pointer for the life of the
    // channel, so it is deliberately leaked — one bridge per session.
    let cb_data = Box::new(ClipboardBridge {
        main_channel: Cell::new(main_channel),
        clipboard: clipboard.clone(),
        ignore_next_host_change: Cell::new(false),
    });
    let bridge = Box::into_raw(cb_data);
    let data_ptr = bridge as ffi::gpointer;

    unsafe {
        // Guest grabbed clipboard (has new content)
        ffi::signal_connect(
            main_channel,
            "main-clipboard-selection-grab",
            std::mem::transmute::<
                unsafe extern "C" fn(
                    ffi::gpointer,
                    ffi::guint,
                    ffi::gpointer,
                    ffi::guint,
                    ffi::gpointer,
                ),
                unsafe extern "C" fn(),
            >(on_guest_clipboard_grab),
            data_ptr,
        );

        // Guest sending clipboard data to us
        ffi::signal_connect(
            main_channel,
            "main-clipboard-selection",
            std::mem::transmute::<
                unsafe extern "C" fn(
                    ffi::gpointer,
                    ffi::guint,
                    ffi::guint,
                    ffi::gpointer,
                    ffi::guint,
                    ffi::gpointer,
                ),
                unsafe extern "C" fn(),
            >(on_guest_clipboard_data),
            data_ptr,
        );

        // Guest wants our clipboard data
        ffi::signal_connect(
            main_channel,
            "main-clipboard-selection-request",
            std::mem::transmute::<
                unsafe extern "C" fn(
                    ffi::gpointer,
                    ffi::guint,
                    ffi::guint,
                    ffi::gpointer,
                ) -> ffi::gboolean,
                unsafe extern "C" fn(),
            >(on_guest_clipboard_request),
            data_ptr,
        );
    }

    // Monitor host clipboard for changes → advertise to guest
    let handler_id = clipboard.connect_changed(move |clipboard| {
        let bridge = unsafe { &*(data_ptr as *const ClipboardBridge) };
        let channel = bridge.main_channel.get();
        if channel.is_null() {
            return;
        }
        if bridge.ignore_next_host_change.replace(false) {
            return;
        }
        // Only advertise text we can actually deliver. Grabbing for a PNG we
        // never serve leaves the guest's paste blocked until the next copy.
        if !clipboard.formats().contains_type(glib::types::Type::STRING) {
            log::debug!("clipboard: host content is not text, not advertising");
            return;
        }
        let types = [ffi::VD_AGENT_CLIPBOARD_UTF8_TEXT];
        unsafe {
            ffi::spice_main_channel_clipboard_selection_grab(
                channel,
                ffi::VD_AGENT_CLIPBOARD_SELECTION_CLIPBOARD,
                types.as_ptr(),
                types.len() as ffi::guint,
            );
        }
        log::debug!("clipboard: notified guest that host has text");
    });

    log::info!("clipboard: bidirectional sharing enabled");
    Handle { clipboard, handler_id: Some(handler_id), bridge }
}

struct ClipboardBridge {
    /// Nulled by [`Handle::teardown`]; every callback checks it first.
    main_channel: Cell<ffi::gpointer>,
    clipboard: gdk::Clipboard,
    /// Set to true when we're about to change the host clipboard ourselves
    /// (to avoid a feedback loop).
    ignore_next_host_change: Cell<bool>,
}

/// Guest grabbed the clipboard — it has new content. Request it as UTF-8.
unsafe extern "C" fn on_guest_clipboard_grab(
    _channel: ffi::gpointer,
    selection: ffi::guint,
    types: ffi::gpointer,
    ntypes: ffi::guint,
    user_data: ffi::gpointer,
) {
    unsafe {
        let bridge = &*(user_data as *const ClipboardBridge);
        let channel = bridge.main_channel.get();
        if channel.is_null() {
            return;
        }
        let type_slice = std::slice::from_raw_parts(types as *const ffi::guint, ntypes as usize);

        // Check if UTF-8 text is among the offered types
        if type_slice.contains(&ffi::VD_AGENT_CLIPBOARD_UTF8_TEXT) {
            ffi::spice_main_channel_clipboard_selection_request(
                channel,
                selection,
                ffi::VD_AGENT_CLIPBOARD_UTF8_TEXT,
            );
            log::debug!("clipboard: requested UTF-8 text from guest");
        }
    }
}

/// Guest sent clipboard data. Put it on the host clipboard.
unsafe extern "C" fn on_guest_clipboard_data(
    _channel: ffi::gpointer,
    _selection: ffi::guint,
    type_: ffi::guint,
    data: ffi::gpointer,
    size: ffi::guint,
    user_data: ffi::gpointer,
) {
    unsafe {
        let bridge = &*(user_data as *const ClipboardBridge);
        if bridge.main_channel.get().is_null() {
            return;
        }

        if type_ == ffi::VD_AGENT_CLIPBOARD_UTF8_TEXT && size > 0 {
            let bytes = std::slice::from_raw_parts(data as *const u8, size as usize);
            if let Ok(text) = std::str::from_utf8(bytes) {
                let text = dos_to_unix(text.trim_end_matches('\0'));
                if !text.is_empty() {
                    // Suppress the feedback loop
                    bridge.ignore_next_host_change.replace(true);
                    bridge.clipboard.set_text(&text);
                    log::debug!("clipboard: set host clipboard from guest ({} bytes)", text.len());
                }
            }
        }
    }
}

/// Guest wants our clipboard data. Read from GTK clipboard and send it.
unsafe extern "C" fn on_guest_clipboard_request(
    _channel: ffi::gpointer,
    selection: ffi::guint,
    type_: ffi::guint,
    user_data: ffi::gpointer,
) -> ffi::gboolean {
    unsafe {
        let bridge = &*(user_data as *const ClipboardBridge);
        let bridge_ptr = user_data as usize;
        if bridge.main_channel.get().is_null() {
            return 0;
        }

        if type_ != ffi::VD_AGENT_CLIPBOARD_UTF8_TEXT {
            return 0; // We only handle text for now
        }

        let sel = selection;
        let clipboard = bridge.clipboard.clone();

        // Read clipboard asynchronously
        glib::spawn_future_local(async move {
            let text = clipboard.read_text_future().await;
            // Re-check after the await: the session may have gone away while
            // the read was in flight.
            let bridge = &*(bridge_ptr as *const ClipboardBridge);
            let channel = bridge.main_channel.get();
            if channel.is_null() {
                return;
            }
            if let Ok(Some(text)) = text {
                let converted = unix_to_dos(&text);
                let bytes = converted.as_bytes();
                ffi::spice_main_channel_clipboard_selection_notify(
                    channel,
                    sel,
                    ffi::VD_AGENT_CLIPBOARD_UTF8_TEXT,
                    bytes.as_ptr(),
                    bytes.len() as ffi::guint,
                );
                log::debug!("clipboard: sent {} bytes to guest", bytes.len());
            } else {
                // vdagent blocks until it gets an answer, so an empty or failed
                // read must still be answered — with NONE.
                if let Err(ref e) = text {
                    log::warn!("clipboard: failed to read host clipboard: {e}");
                }
                ffi::spice_main_channel_clipboard_selection_notify(
                    channel,
                    sel,
                    ffi::VD_AGENT_CLIPBOARD_NONE,
                    ptr::null(),
                    0,
                );
            }
        });

        1 // Handled
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_guest_line_endings_to_host() {
        assert_eq!(dos_to_unix("a\r\nb\r\nc"), "a\nb\nc");
        assert_eq!(dos_to_unix("no newlines"), "no newlines");
        assert_eq!(dos_to_unix(""), "");
    }

    #[test]
    fn converts_host_line_endings_to_guest() {
        assert_eq!(unix_to_dos("a\nb\nc"), "a\r\nb\r\nc");
        assert_eq!(unix_to_dos("no newlines"), "no newlines");
        assert_eq!(unix_to_dos(""), "");
    }

    /// Text that already uses CRLF must not gain a second carriage return.
    #[test]
    fn does_not_double_convert_existing_crlf() {
        assert_eq!(unix_to_dos("a\r\nb"), "a\r\nb");
        assert_eq!(unix_to_dos("a\r\nb\nc"), "a\r\nb\r\nc");
    }

    #[test]
    fn line_ending_conversion_round_trips() {
        for original in ["a\nb\nc", "single", "", "\n", "trailing\n"] {
            assert_eq!(dos_to_unix(&unix_to_dos(original)), original);
        }
    }

    /// A lone carriage return is not a line ending on either side and is left
    /// alone rather than being turned into a newline.
    #[test]
    fn leaves_bare_carriage_returns_alone() {
        assert_eq!(unix_to_dos("a\rb"), "a\rb");
        assert_eq!(dos_to_unix("a\rb"), "a\rb");
    }
}
