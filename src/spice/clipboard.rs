//! SPICE clipboard bridge — bidirectional text clipboard between host and guest.
//!
//! When the guest copies text, we receive it via the SPICE agent and set it on
//! the GTK4 clipboard. When the host copies text, we detect the change and push
//! it to the guest via the SPICE agent.

use std::cell::RefCell;

use gtk4::gdk;
use gtk4::glib;
use gtk4::prelude::*;

use super::ffi;

/// Set up bidirectional clipboard sharing on the SPICE main channel.
///
/// # Safety
/// `main_channel` must be a valid `SpiceMainChannel*`.
pub fn setup(main_channel: ffi::gpointer, gdk_display: &gdk::Display) {
    let clipboard = gdk_display.clipboard();

    // Store clipboard ref for the C callbacks
    let cb_data = Box::new(ClipboardBridge {
        main_channel,
        clipboard: clipboard.clone(),
        ignore_next_host_change: RefCell::new(false),
    });
    let data_ptr = Box::into_raw(cb_data) as ffi::gpointer;

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

    // Monitor host clipboard for changes → push to guest
    let main_ch = main_channel;
    let bridge_ptr = data_ptr;
    clipboard.connect_changed(move |_clipboard| {
        let bridge = unsafe { &*(bridge_ptr as *const ClipboardBridge) };
        if bridge.ignore_next_host_change.replace(false) {
            return;
        }
        // Tell the guest we have UTF-8 text
        let types = [ffi::VD_AGENT_CLIPBOARD_UTF8_TEXT];
        unsafe {
            ffi::spice_main_channel_clipboard_selection_grab(
                main_ch,
                ffi::VD_AGENT_CLIPBOARD_SELECTION_CLIPBOARD,
                types.as_ptr(),
                types.len() as ffi::guint,
            );
        }
        log::debug!("clipboard: notified guest that host has text");
    });

    log::info!("clipboard: bidirectional sharing enabled");
}

struct ClipboardBridge {
    main_channel: ffi::gpointer,
    clipboard: gdk::Clipboard,
    /// Set to true when we're about to change the host clipboard ourselves
    /// (to avoid a feedback loop).
    ignore_next_host_change: RefCell<bool>,
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
        let type_slice = std::slice::from_raw_parts(types as *const ffi::guint, ntypes as usize);

        // Check if UTF-8 text is among the offered types
        if type_slice.contains(&ffi::VD_AGENT_CLIPBOARD_UTF8_TEXT) {
            ffi::spice_main_channel_clipboard_selection_request(
                bridge.main_channel,
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

        if type_ == ffi::VD_AGENT_CLIPBOARD_UTF8_TEXT && size > 0 {
            let bytes = std::slice::from_raw_parts(data as *const u8, size as usize);
            if let Ok(text) = std::str::from_utf8(bytes) {
                let text = text.trim_end_matches('\0');
                if !text.is_empty() {
                    // Suppress the feedback loop
                    bridge.ignore_next_host_change.replace(true);
                    bridge.clipboard.set_text(text);
                    log::debug!("clipboard: set host clipboard from guest ({} chars)", text.len());
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

        if type_ != ffi::VD_AGENT_CLIPBOARD_UTF8_TEXT {
            return 0; // We only handle text for now
        }

        let main_ch = bridge.main_channel;
        let sel = selection;
        let clipboard = bridge.clipboard.clone();

        // Read clipboard asynchronously
        glib::spawn_future_local(async move {
            match clipboard.read_text_future().await {
                Ok(Some(text)) => {
                    let bytes = text.as_bytes();
                    ffi::spice_main_channel_clipboard_selection_notify(
                        main_ch,
                        sel,
                        ffi::VD_AGENT_CLIPBOARD_UTF8_TEXT,
                        bytes.as_ptr(),
                        bytes.len() as ffi::guint,
                    );
                    log::debug!("clipboard: sent {} bytes to guest", bytes.len());
                }
                Ok(None) => {
                    log::debug!("clipboard: host clipboard empty");
                }
                Err(e) => {
                    log::warn!("clipboard: failed to read host clipboard: {e}");
                }
            }
        });

        1 // Handled
    }
}
