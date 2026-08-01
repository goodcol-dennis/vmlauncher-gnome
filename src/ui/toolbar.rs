//! Fullscreen toolbar overlay.
//!
//! A GTK4 Revealer that slides down from the top edge when the mouse
//! reaches the top of the screen. Contains: VM name, Exit Fullscreen,
//! Send Ctrl+Alt+Del, and Close.

use gtk4::prelude::*;
use gtk4::{Align, Box as GtkBox, Button, Label, Orientation, Revealer, RevealerTransitionType};

pub struct Toolbar {
    pub revealer: Revealer,
    pub exit_fullscreen_btn: Button,
    pub ctrl_alt_del_btn: Button,
    pub close_btn: Button,
}

pub fn build() -> Toolbar {
    let hbox = GtkBox::new(Orientation::Horizontal, 12);
    hbox.set_margin_start(12);
    hbox.set_margin_end(12);
    hbox.set_margin_top(6);
    hbox.set_margin_bottom(6);

    let vm_label = Label::new(Some("win11"));
    vm_label.add_css_class("heading");
    vm_label.set_hexpand(true);
    vm_label.set_halign(Align::Start);
    hbox.append(&vm_label);

    let ctrl_alt_del_btn = Button::with_label("Ctrl+Alt+Del");
    hbox.append(&ctrl_alt_del_btn);

    let exit_fullscreen_btn = Button::with_label("Exit Fullscreen");
    hbox.append(&exit_fullscreen_btn);

    let close_btn = Button::with_label("Close");
    close_btn.add_css_class("destructive-action");
    hbox.append(&close_btn);

    // Keep keyboard focus on the display. Otherwise clicking Ctrl+Alt+Del hands
    // focus to the button, and the password you type next goes to the toolbar —
    // with Enter re-triggering Ctrl+Alt+Del.
    for button in [&ctrl_alt_del_btn, &exit_fullscreen_btn, &close_btn] {
        button.set_focus_on_click(false);
    }

    let revealer = Revealer::new();
    revealer.set_transition_type(RevealerTransitionType::SlideDown);
    revealer.set_transition_duration(200);
    revealer.set_reveal_child(false);
    revealer.set_valign(Align::Start);
    revealer.set_halign(Align::Fill);
    revealer.set_child(Some(&hbox));

    Toolbar { revealer, exit_fullscreen_btn, ctrl_alt_del_btn, close_btn }
}
