//! SPICE display rendering via `MemoryTexture`.
//!
//! Reads directly from the SPICE framebuffer on each render frame — no local
//! buffer copy needed. Uses B8g8r8x8 format so the alpha/X byte is ignored
//! by GTK, eliminating per-pixel fixup. Dirty-rect tracking ensures we only
//! render when something changed.
//!
//! Deliberately NOT a GL path: `GLTextureBuilder` plus epoxy segfaults here
//! because the epoxy function pointers need a GL context current on the calling
//! thread, and neither `gdk::Display::create_gl_context()` nor a hidden `GLArea`
//! provides a usable one. See the gotchas in CLAUDE.md before revisiting.

use std::cell::Cell;

use gtk4::Picture;
use gtk4::gdk;
use gtk4::glib;

use crate::spice;

/// Display state — tracks dimensions and dirty flag.
/// The actual pixel data lives in the SPICE framebuffer (zero-copy read).
pub struct SpiceDisplay {
    width: i32,
    height: i32,
    dirty: Cell<bool>,
}

impl SpiceDisplay {
    pub fn new(width: i32, height: i32) -> Self {
        log::info!("display: created {width}x{height}");
        Self { width, height, dirty: Cell::new(true) }
    }

    /// Mark the display as dirty (called on SPICE invalidate).
    pub fn mark_dirty(&self) {
        self.dirty.set(true);
    }

    /// If dirty, read the SPICE framebuffer and upload a texture.
    /// Returns true if a new frame was rendered.
    pub fn render_if_dirty(&self, picture: &Picture, spice_state: &spice::SharedState) -> bool {
        if !self.dirty.get() {
            return false;
        }

        let s = spice_state.borrow();
        // `None` here means spice-gtk has freed the surface and not yet handed
        // us a new one. Stay dirty so the next frame retries once it has.
        let Some(ref fb) = s.framebuffer else { return false };
        if fb.data.is_null() || fb.width <= 0 || fb.height <= 0 {
            return false;
        }

        self.dirty.set(false);

        // Size the read from what SPICE reported for THIS buffer, never from
        // our own cached dimensions — those can lag a resize by a frame.
        let fb_slice = unsafe { std::slice::from_raw_parts(fb.data, fb.byte_len()) };

        // from_owned takes a Vec (one memcpy from SPICE buffer).
        // B8g8r8x8 means GTK ignores the alpha byte — no fixup needed.
        let bytes = glib::Bytes::from_owned(fb_slice.to_vec());
        let texture = gdk::MemoryTexture::new(
            fb.width,
            fb.height,
            gdk::MemoryFormat::B8g8r8x8,
            &bytes,
            fb.stride as usize,
        );
        picture.set_paintable(Some(&texture));
        true
    }

    /// Resize for a new VM display resolution.
    pub fn resize(&mut self, width: i32, height: i32) {
        self.width = width;
        self.height = height;
        self.dirty.set(true);
        log::info!("display: resized to {width}x{height}");
    }

    pub fn width(&self) -> i32 {
        self.width
    }
    pub fn height(&self) -> i32 {
        self.height
    }
}
