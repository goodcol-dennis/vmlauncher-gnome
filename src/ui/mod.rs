//! UI — main window with loading screen, SPICE display, and fullscreen toolbar.

pub mod display;
pub mod toolbar;

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk4::gdk;
use gtk4::glib;
use gtk4::prelude::*;
use gtk4::{
    AlertDialog, Align, Application, ApplicationWindow, Box as GtkBox, ContentFit,
    EventControllerKey, EventControllerLegacy, EventControllerMotion, Label, Orientation, Overlay,
    Picture, Spinner, Stack,
};

use crate::config;
use crate::spice;
use crate::vm;

/// Build the main application window and wire up the full lifecycle.
#[allow(clippy::too_many_lines)]
pub fn build_window(app: &Application) {
    let saved_state = config::load();
    let window = ApplicationWindow::builder()
        .application(app)
        .title("vmlaunch")
        .default_width(saved_state.width)
        .default_height(saved_state.height)
        .icon_name("vmlaunch")
        .build();

    if saved_state.fullscreen {
        window.fullscreen();
    }

    // --- CSS for toolbar background ---
    let css = gtk4::CssProvider::new();
    css.load_from_string(".toolbar-bg { background-color: alpha(@headerbar_bg_color, 0.92); }");
    gtk4::style_context_add_provider_for_display(
        &gdk::Display::default().unwrap(),
        &css,
        gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );

    // --- Loading screen ---
    let loading_box = GtkBox::new(Orientation::Vertical, 16);
    loading_box.set_valign(Align::Center);
    loading_box.set_halign(Align::Center);
    let spinner = Spinner::new();
    spinner.set_spinning(true);
    spinner.set_width_request(48);
    spinner.set_height_request(48);
    let status_label = Label::new(Some("Starting VM..."));
    status_label.add_css_class("title-2");
    loading_box.append(&spinner);
    loading_box.append(&status_label);

    // --- SPICE display (Picture — GPU-accelerated via GLTexture) ---
    let picture = Picture::new();
    picture.set_hexpand(true);
    picture.set_vexpand(true);
    picture.set_focusable(true);
    picture.set_can_focus(true);
    picture.set_content_fit(ContentFit::Contain);
    // Hide the host cursor over the VM display — the VM has its own cursor

    // --- Stack to switch between loading and display ---
    let stack = Stack::new();
    stack.add_named(&loading_box, Some("loading"));
    stack.add_named(&picture, Some("display"));
    stack.set_visible_child_name("loading");

    // --- Toolbar overlay ---
    let tb = toolbar::build();
    tb.revealer.add_css_class("toolbar-bg");

    let overlay = Overlay::new();
    overlay.set_child(Some(&stack));
    overlay.add_overlay(&tb.revealer);

    window.set_child(Some(&overlay));
    // TODO: Hide host cursor once coordinate alignment is fixed
    // picture.set_cursor_from_name(Some("none"));

    // --- Shared state ---
    let spice_state: Rc<RefCell<Option<spice::SharedState>>> = Rc::new(RefCell::new(None));
    let gpu_display: Rc<RefCell<Option<display::GpuDisplay>>> = Rc::new(RefCell::new(None));
    let is_fullscreen = Rc::new(Cell::new(saved_state.fullscreen));
    let shutdown_in_progress = Rc::new(Cell::new(false));

    // --- Keyboard input forwarding ---
    {
        let ss = spice_state.clone();
        let key_ctrl = EventControllerKey::new();

        let ss2 = ss.clone();
        key_ctrl.connect_key_pressed(move |_, keyval, keycode, _mods| {
            if keyval == gdk::Key::F11 {
                return glib::Propagation::Proceed;
            }
            let state_opt = ss2.borrow();
            if let Some(ref state) = *state_opt {
                spice::key_press(state, keycode);
            }
            glib::Propagation::Stop
        });

        let ss3 = ss.clone();
        key_ctrl.connect_key_released(move |_, keyval, keycode, _mods| {
            if keyval == gdk::Key::F11 {
                return;
            }
            let state_opt = ss3.borrow();
            if let Some(ref state) = *state_opt {
                spice::key_release(state, keycode);
            }
        });

        picture.add_controller(key_ctrl);
    }

    // --- Mouse input forwarding (all events via EventControllerLegacy) ---
    // Using a single legacy controller ensures we see motion events during
    // button-held drags, which separate EventControllerMotion misses.
    {
        let spice_state_m = spice_state.clone();
        let gpu_m = gpu_display.clone();
        let pic_m = picture.clone();
        let legacy = EventControllerLegacy::new();
        legacy.connect_event(move |_, event| {
            let state_opt = spice_state_m.borrow();
            let Some(ref state) = *state_opt else {
                return glib::Propagation::Proceed;
            };
            let disp = gpu_m.borrow();
            let Some(ref d) = *disp else {
                return glib::Propagation::Proceed;
            };
            let fb_w = d.width();
            let fb_h = d.height();
            drop(disp);

            // event.position() returns SURFACE coordinates (includes headerbar).
            // Convert to widget-local coordinates by computing the Picture's
            // offset within the surface.
            let to_local = |event: &gdk::Event, pic: &Picture| -> Option<(f64, f64)> {
                let (sx, sy) = event.position()?;
                let native = pic.native()?;
                let native_widget: gtk4::Widget = native.upcast();
                // compute_point: maps point FROM self's space TO target's space
                // So pic(0,0) → native gives us the picture's origin in surface coords
                let origin = pic.compute_point(
                    &native_widget,
                    &gtk4::graphene::Point::new(0.0, 0.0),
                )?;
                let lx = sx - f64::from(origin.x());
                let ly = sy - f64::from(origin.y());
                log::trace!(
                    "coords: surface=({sx:.0},{sy:.0}) origin=({:.0},{:.0}) local=({lx:.0},{ly:.0}) widget={}x{}",
                    origin.x(), origin.y(), pic.width(), pic.height()
                );
                Some((lx, ly))
            };

            match event.event_type() {
                gdk::EventType::MotionNotify => {
                    if let Some((lx, ly)) = to_local(event, &pic_m) {
                        let (vx, vy) = widget_to_vm(
                            f64::from(pic_m.width()),
                            f64::from(pic_m.height()),
                            fb_w,
                            fb_h,
                            lx,
                            ly,
                        );
                        spice::mouse_position(state, vx, vy);
                    }
                    glib::Propagation::Stop
                }
                gdk::EventType::ButtonPress => {
                    pic_m.grab_focus();
                    if let Some(be) = event.downcast_ref::<gdk::ButtonEvent>() {
                        if let Some((lx, ly)) = to_local(event, &pic_m) {
                            let (vx, vy) = widget_to_vm(
                                f64::from(pic_m.width()),
                                f64::from(pic_m.height()),
                                fb_w,
                                fb_h,
                                lx,
                                ly,
                            );
                            spice::mouse_position(state, vx, vy);
                        }
                        spice::mouse_button_press(state, gtk_button_to_spice(be.button()));
                    }
                    glib::Propagation::Stop
                }
                gdk::EventType::ButtonRelease => {
                    if let Some(be) = event.downcast_ref::<gdk::ButtonEvent>() {
                        spice::mouse_button_release(state, gtk_button_to_spice(be.button()));
                    }
                    glib::Propagation::Stop
                }
                gdk::EventType::Scroll => {
                    if let Some(se) = event.downcast_ref::<gdk::ScrollEvent>() {
                        let (_, dy) = se.deltas();
                        if dy.abs() > 0.01 {
                            spice::mouse_scroll(state, dy < 0.0);
                        }
                    }
                    glib::Propagation::Stop
                }
                _ => glib::Propagation::Proceed,
            }
        });
        picture.add_controller(legacy);
    }

    // --- Fullscreen toolbar hover detection ---
    {
        let revealer = tb.revealer.clone();
        let is_fs = is_fullscreen.clone();
        let hide_timer: Rc<RefCell<Option<(glib::SourceId, Rc<Cell<bool>>)>>> =
            Rc::new(RefCell::new(None));

        let motion_ctrl = EventControllerMotion::new();
        let rev = revealer.clone();
        let ht = hide_timer.clone();
        let is_fs2 = is_fs.clone();
        motion_ctrl.connect_motion(move |_, _x, y| {
            if !is_fs2.get() {
                return;
            }
            if y <= 2.0 {
                rev.set_reveal_child(true);
                if let Some((id, fired)) = ht.borrow_mut().take()
                    && !fired.get()
                {
                    id.remove();
                }
            }
        });
        window.add_controller(motion_ctrl);

        let toolbar_motion = EventControllerMotion::new();
        let rev2 = revealer.clone();
        let ht2 = hide_timer.clone();
        toolbar_motion.connect_leave(move |_| {
            let rev = rev2.clone();
            let fired = Rc::new(Cell::new(false));
            let fired2 = fired.clone();
            let id =
                glib::timeout_add_local_once(std::time::Duration::from_millis(500), move || {
                    fired2.set(true);
                    rev.set_reveal_child(false);
                });
            ht2.borrow_mut().replace((id, fired));
        });
        let ht3 = hide_timer.clone();
        toolbar_motion.connect_enter(move |_, _x, _y| {
            if let Some((id, fired)) = ht3.borrow_mut().take()
                && !fired.get()
            {
                id.remove();
            }
        });
        tb.revealer.add_controller(toolbar_motion);
    }

    // --- F11 fullscreen toggle ---
    {
        let win = window.clone();
        let is_fs = is_fullscreen.clone();
        let revealer = tb.revealer.clone();
        let key_ctrl = EventControllerKey::new();
        key_ctrl.connect_key_pressed(move |_, keyval, _, _| {
            if keyval == gdk::Key::F11 {
                if is_fs.get() {
                    win.unfullscreen();
                    is_fs.set(false);
                    revealer.set_reveal_child(false);
                } else {
                    win.fullscreen();
                    is_fs.set(true);
                }
                save_window_state(&win, is_fs.get());
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        window.add_controller(key_ctrl);
    }

    // --- Toolbar button handlers ---
    {
        let win = window.clone();
        let is_fs = is_fullscreen.clone();
        let revealer = tb.revealer.clone();
        tb.exit_fullscreen_btn.connect_clicked(move |_| {
            win.unfullscreen();
            is_fs.set(false);
            revealer.set_reveal_child(false);
            save_window_state(&win, false);
        });
    }
    {
        let ss = spice_state.clone();
        tb.ctrl_alt_del_btn.connect_clicked(move |_| {
            let state_opt = ss.borrow();
            if let Some(ref state) = *state_opt {
                spice::send_ctrl_alt_del(state);
            }
        });
    }
    {
        let win = window.clone();
        tb.close_btn.connect_clicked(move |_| {
            win.close();
        });
    }

    // --- Window close → confirmation dialog ---
    {
        let spice_state = spice_state.clone();
        let shutdown_in_progress = shutdown_in_progress.clone();
        let stack = stack.clone();
        let status_label = status_label.clone();
        window.connect_close_request(move |win| {
            if shutdown_in_progress.get() {
                return glib::Propagation::Stop;
            }

            // Show confirmation dialog
            let dialog = AlertDialog::builder()
                .message("Close VM?")
                .detail("Choose how to close the window.")
                .build();
            dialog.set_buttons(&["Shut Down", "Detach", "Minimize", "Cancel"]);
            dialog.set_cancel_button(3);
            dialog.set_default_button(0);

            let win_for_dialog = win.clone();
            let win = win.clone();
            let spice_state = spice_state.clone();
            let shutdown_in_progress = shutdown_in_progress.clone();
            let stack = stack.clone();
            let status_label = status_label.clone();

            dialog.choose(Some(&win_for_dialog), None::<&gtk4::gio::Cancellable>, move |result| {
                match result {
                    Ok(0) => {
                        // Shut Down
                        shutdown_in_progress.set(true);
                        {
                            let state_opt = spice_state.borrow();
                            if let Some(ref state) = *state_opt {
                                spice::disconnect(state);
                            }
                        }
                        status_label.set_text("Shutting down VM...");
                        stack.set_visible_child_name("loading");

                        if let Err(e) = vm::shutdown() {
                            log::error!("shutdown failed: {e}");
                        }

                        let win = win.clone();
                        let tick = Rc::new(Cell::new(0u32));
                        let status = status_label.clone();
                        glib::timeout_add_local(std::time::Duration::from_secs(1), move || {
                            let count = tick.get() + 1;
                            tick.set(count);
                            status.set_text(&format!("Shutting down VM... ({count}s)"));
                            match vm::state() {
                                Ok(vm::VmState::Shutoff) => {
                                    log::info!("VM shut off cleanly");
                                    win.destroy();
                                    glib::ControlFlow::Break
                                }
                                Ok(_) if count >= 60 => {
                                    log::warn!("shutdown timeout — forcing off");
                                    let _ = vm::force_off();
                                    win.destroy();
                                    glib::ControlFlow::Break
                                }
                                Err(e) => {
                                    log::error!("state check error: {e}");
                                    win.destroy();
                                    glib::ControlFlow::Break
                                }
                                _ => glib::ControlFlow::Continue,
                            }
                        });
                    }
                    Ok(1) => {
                        // Detach — close window, leave VM running
                        log::info!("detaching — VM keeps running");
                        {
                            let state_opt = spice_state.borrow();
                            if let Some(ref state) = *state_opt {
                                spice::disconnect(state);
                            }
                        }
                        save_window_state(&win, false);
                        win.destroy();
                    }
                    Ok(2) => {
                        // Minimize to dock
                        win.minimize();
                    }
                    _ => {
                        // Cancel — do nothing
                    }
                }
            });

            glib::Propagation::Stop
        });
    }

    // --- Start the VM and connect SPICE ---
    let window_for_present = window.clone();
    {
        let status_label = status_label.clone();
        let stack = stack.clone();
        let picture = picture.clone();
        let gpu_display = gpu_display.clone();

        glib::idle_add_local_once(move || {
            status_label.set_text("Starting VM...");
            if let Err(e) = vm::start() {
                status_label.set_text(&format!("Failed to start VM: {e}"));
                log::error!("VM start failed: {e}");
                return;
            }

            status_label.set_text("Waiting for SPICE display...");

            let status_label = status_label.clone();
            let stack = stack.clone();
            let picture = picture.clone();
            let spice_state = spice_state.clone();
            let gpu_display = gpu_display.clone();

            glib::timeout_add_local(std::time::Duration::from_secs(1), move || {
                match vm::spice_uri() {
                    Ok(Some(uri)) => {
                        log::info!("SPICE URI: {uri}");
                        status_label.set_text("Connecting to display...");

                        let pic_ready = picture.clone();
                        let pic_render = picture.clone();
                        let stack2 = stack.clone();
                        let gpu_ready = gpu_display.clone();
                        let gpu_inval = gpu_display.clone();
                        let gpu_render = gpu_display.clone();
                        let spice_for_render = spice_state.clone();

                        // 60fps render timer — reads SPICE framebuffer directly
                        glib::timeout_add_local(std::time::Duration::from_millis(16), move || {
                            let disp = gpu_render.borrow();
                            let spice_opt = spice_for_render.borrow();
                            if let (Some(d), Some(ss)) = (&*disp, &*spice_opt) {
                                d.render_if_dirty(&pic_render, ss);
                            }
                            glib::ControlFlow::Continue
                        });

                        match spice::connect(
                            &uri,
                            // on_display_ready
                            move |w, h| {
                                log::info!("display ready {w}x{h}");
                                let stride = w * 4;

                                let mut disp = gpu_ready.borrow_mut();
                                if let Some(ref mut existing) = *disp {
                                    existing.resize(w, h, stride);
                                } else {
                                    *disp = Some(display::GpuDisplay::new(w, h, stride));
                                }

                                stack2.set_visible_child_name("display");
                                pic_ready.grab_focus();
                            },
                            // on_invalidate — just mark dirty, render timer handles the rest
                            move |_x, _y, _w, _h| {
                                let disp = gpu_inval.borrow();
                                if let Some(ref d) = *disp {
                                    d.mark_dirty();
                                }
                            },
                        ) {
                            Ok(state) => {
                                // Set up clipboard sharing once agent is ready
                                let ss_clip = state.clone();
                                let display = gdk::Display::default().unwrap();
                                glib::timeout_add_local(
                                    std::time::Duration::from_secs(2),
                                    move || {
                                        let s = ss_clip.borrow();
                                        if !s.main_channel.is_null() {
                                            drop(s);
                                            spice::clipboard::setup(
                                                ss_clip.borrow().main_channel,
                                                &display,
                                            );
                                            return glib::ControlFlow::Break;
                                        }
                                        glib::ControlFlow::Continue
                                    },
                                );
                                spice_state.borrow_mut().replace(state);
                            }
                            Err(e) => {
                                log::error!("SPICE connect failed: {e}");
                                status_label.set_text(&format!("SPICE error: {e}"));
                            }
                        }
                        return glib::ControlFlow::Break;
                    }
                    Ok(None) => {
                        log::debug!("SPICE URI not yet available, polling...");
                    }
                    Err(e) => {
                        log::error!("spice_uri error: {e}");
                        status_label.set_text(&format!("Error: {e}"));
                        return glib::ControlFlow::Break;
                    }
                }
                glib::ControlFlow::Continue
            });
        });
    }

    window_for_present.present();
}

/// Map widget-local coordinates to VM display coordinates.
///
/// Mirrors [`ContentFit::Contain`]: the framebuffer is scaled by the smaller of
/// the two axis ratios and centred, so the result is letterboxed or pillarboxed
/// depending on which aspect ratio is wider. Coordinates landing on the letterbox
/// bars clamp to the nearest edge pixel.
fn widget_to_vm(
    widget_w: f64,
    widget_h: f64,
    fb_w: i32,
    fb_h: i32,
    wx: f64,
    wy: f64,
) -> (i32, i32) {
    if widget_w <= 0.0 || widget_h <= 0.0 || fb_w <= 0 || fb_h <= 0 {
        return (0, 0);
    }
    let scale_x = widget_w / f64::from(fb_w);
    let scale_y = widget_h / f64::from(fb_h);
    let scale = scale_x.min(scale_y);
    let offset_x = (widget_w - f64::from(fb_w) * scale) / 2.0;
    let offset_y = (widget_h - f64::from(fb_h) * scale) / 2.0;
    let vx = ((wx - offset_x) / scale).clamp(0.0, f64::from(fb_w - 1)) as i32;
    let vy = ((wy - offset_y) / scale).clamp(0.0, f64::from(fb_h - 1)) as i32;
    (vx, vy)
}

/// Save current window state to config file.
fn save_window_state(window: &ApplicationWindow, fullscreen: bool) {
    config::save(&config::WindowState {
        width: window.width(),
        height: window.height(),
        fullscreen,
    });
}

/// Map GTK button number to SPICE button constant.
///
/// GTK numbers the side buttons 8 (back) and 9 (forward); SPICE calls the same
/// two SIDE and EXTRA. Mapping them explicitly stops them falling through to a
/// left click.
fn gtk_button_to_spice(gtk_button: u32) -> i32 {
    match gtk_button {
        2 => spice::ffi::SPICE_MOUSE_BUTTON_MIDDLE,
        3 => spice::ffi::SPICE_MOUSE_BUTTON_RIGHT,
        8 => spice::ffi::SPICE_MOUSE_BUTTON_SIDE,
        9 => spice::ffi::SPICE_MOUSE_BUTTON_EXTRA,
        _ => spice::ffi::SPICE_MOUSE_BUTTON_LEFT,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_when_widget_matches_framebuffer() {
        let map = |x, y| widget_to_vm(1920.0, 1080.0, 1920, 1080, x, y);
        assert_eq!(map(0.0, 0.0), (0, 0));
        assert_eq!(map(960.0, 540.0), (960, 540));
        assert_eq!(map(1919.0, 1079.0), (1919, 1079));
    }

    #[test]
    fn scales_up_uniformly_when_aspect_ratios_agree() {
        let map = |x, y| widget_to_vm(1600.0, 1200.0, 800, 600, x, y);
        assert_eq!(map(0.0, 0.0), (0, 0));
        assert_eq!(map(800.0, 600.0), (400, 300));
        assert_eq!(map(1598.0, 1198.0), (799, 599));
    }

    #[test]
    fn scales_down_uniformly_when_aspect_ratios_agree() {
        let map = |x, y| widget_to_vm(960.0, 540.0, 1920, 1080, x, y);
        assert_eq!(map(0.0, 0.0), (0, 0));
        assert_eq!(map(480.0, 270.0), (960, 540));
    }

    /// Widget wider than the framebuffer's aspect ratio: bars appear left and
    /// right, and the image starts at x = 400.
    #[test]
    fn pillarboxes_a_wide_widget() {
        let map = |x, y| widget_to_vm(1600.0, 600.0, 800, 600, x, y);
        assert_eq!(map(400.0, 0.0), (0, 0), "top-left of the image");
        assert_eq!(map(800.0, 300.0), (400, 300), "centre");
        assert_eq!(map(1199.0, 599.0), (799, 599), "bottom-right of the image");
    }

    /// Widget taller than the framebuffer's aspect ratio: bars appear top and
    /// bottom, and the image starts at y = 300.
    #[test]
    fn letterboxes_a_tall_widget() {
        let map = |x, y| widget_to_vm(800.0, 1200.0, 800, 600, x, y);
        assert_eq!(map(0.0, 300.0), (0, 0), "top-left of the image");
        assert_eq!(map(400.0, 600.0), (400, 300), "centre");
        assert_eq!(map(799.0, 899.0), (799, 599), "bottom-right of the image");
    }

    /// The centre of the widget must always be the centre of the guest display,
    /// whichever way the letterboxing falls. This is the invariant that would
    /// break first if the fit maths drifted out of sync with `ContentFit::Contain`.
    #[test]
    fn widget_centre_always_maps_to_framebuffer_centre() {
        let cases = [
            (1920.0, 1080.0, 1920, 1080),
            (1600.0, 600.0, 800, 600),
            (800.0, 1200.0, 800, 600),
            (1366.0, 768.0, 1920, 1080),
            (2560.0, 1600.0, 1920, 1080),
        ];
        for (ww, wh, fw, fh) in cases {
            let got = widget_to_vm(ww, wh, fw, fh, ww / 2.0, wh / 2.0);
            assert_eq!(got, (fw / 2, fh / 2), "widget {ww}x{wh} over framebuffer {fw}x{fh}");
        }
    }

    /// Pointer positions over the letterbox bars clamp to the nearest edge
    /// pixel rather than escaping the framebuffer.
    #[test]
    fn positions_outside_the_image_clamp_to_the_edge() {
        let map = |x, y| widget_to_vm(1600.0, 600.0, 800, 600, x, y);
        assert_eq!(map(0.0, 300.0), (0, 300), "left bar");
        assert_eq!(map(1599.0, 300.0), (799, 300), "right bar");
        assert_eq!(map(-500.0, -500.0), (0, 0));
        assert_eq!(map(99999.0, 99999.0), (799, 599));
    }

    /// Whatever the inputs, the result must be a valid framebuffer coordinate —
    /// an out-of-range value would be forwarded straight to the guest.
    #[test]
    fn output_is_always_within_framebuffer_bounds() {
        let (fb_w, fb_h) = (1920, 1080);
        for wx in [-1000.0, -1.0, 0.0, 1.0, 640.0, 1365.0, 1366.0, 5000.0] {
            for wy in [-1000.0, -1.0, 0.0, 1.0, 384.0, 767.0, 768.0, 5000.0] {
                let (vx, vy) = widget_to_vm(1366.0, 768.0, fb_w, fb_h, wx, wy);
                assert!((0..fb_w).contains(&vx), "x={vx} out of bounds for ({wx}, {wy})");
                assert!((0..fb_h).contains(&vy), "y={vy} out of bounds for ({wx}, {wy})");
            }
        }
    }

    /// Before the first `display-primary-create`, and during a window resize to
    /// zero, these degenerate sizes are real. They must not divide by zero.
    #[test]
    fn degenerate_sizes_yield_the_origin() {
        assert_eq!(widget_to_vm(0.0, 1080.0, 1920, 1080, 100.0, 100.0), (0, 0));
        assert_eq!(widget_to_vm(1920.0, 0.0, 1920, 1080, 100.0, 100.0), (0, 0));
        assert_eq!(widget_to_vm(1920.0, 1080.0, 0, 1080, 100.0, 100.0), (0, 0));
        assert_eq!(widget_to_vm(1920.0, 1080.0, 1920, 0, 100.0, 100.0), (0, 0));
        assert_eq!(widget_to_vm(-1.0, -1.0, -1, -1, 100.0, 100.0), (0, 0));
    }

    #[test]
    fn maps_the_three_standard_mouse_buttons() {
        assert_eq!(gtk_button_to_spice(1), spice::ffi::SPICE_MOUSE_BUTTON_LEFT);
        assert_eq!(gtk_button_to_spice(2), spice::ffi::SPICE_MOUSE_BUTTON_MIDDLE);
        assert_eq!(gtk_button_to_spice(3), spice::ffi::SPICE_MOUSE_BUTTON_RIGHT);
    }

    /// The side buttons must reach the guest as SIDE/EXTRA rather than
    /// falling through to a left click.
    #[test]
    fn maps_the_side_buttons() {
        assert_eq!(gtk_button_to_spice(8), spice::ffi::SPICE_MOUSE_BUTTON_SIDE);
        assert_eq!(gtk_button_to_spice(9), spice::ffi::SPICE_MOUSE_BUTTON_EXTRA);
    }

    /// Anything else is treated as a left click, which is the safe default for
    /// an unrecognised button.
    #[test]
    fn unknown_buttons_fall_back_to_left_click() {
        assert_eq!(gtk_button_to_spice(1), spice::ffi::SPICE_MOUSE_BUTTON_LEFT);
        assert_eq!(gtk_button_to_spice(0), spice::ffi::SPICE_MOUSE_BUTTON_LEFT);
        assert_eq!(gtk_button_to_spice(42), spice::ffi::SPICE_MOUSE_BUTTON_LEFT);
    }
}
