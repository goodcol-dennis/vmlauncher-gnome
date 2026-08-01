# vmlaunch

> **HISTORICAL — superseded by [CLAUDE.md](CLAUDE.md) and the code.**
> This is the original design document, kept for the reasoning behind the big
> decisions. Several details were overtaken during implementation: VM control
> uses `virsh` rather than libvirt-rs, `src/spice.rs` became a directory, the
> desktop file is `tech.goodcol.vmlaunch.desktop` (the filename must match the
> app ID or the dock icon breaks), the app does not launch fullscreen by
> default, and the three "Open Decisions" at the bottom are all settled.
> Do not treat anything below as a description of current behaviour.

Single-click GTK4 app that starts a KVM/SPICE Windows 11 VM, embeds the SPICE display, and ties VM lifecycle to the app window. Replaces the current `win11-vm` shell script + `remote-viewer` combo with one dock icon.

## Goals

- One click in the dock starts the VM and opens the display
- Closing the app shuts down the VM (ACPI)
- No dependency on `remote-viewer` — embed SPICE display directly
- Looks and feels like a native app, not a wrapper

## Tech Stack

- **Language:** Rust
- **GUI:** GTK4 via gtk4-rs
- **SPICE display:** SpiceClientGLib-2.0 for session/channel management + custom rendering into a GTK4 widget. SpiceClientGtk-3.0 is GTK3-only and cannot be directly embedded in GTK4.
- **VM control:** libvirt-rs (start, shutdown, domstate, domdisplay)
- **Desktop integration:** `.desktop` file + SVG icon

## Architecture

```
vmlaunch
├── src/
│   ├── main.rs           # Entry point, inits logging and launches app
│   ├── app.rs            # GTK4 Application setup and lifecycle
│   ├── vm.rs             # VM lifecycle via libvirt (start, shutdown, state, spice_uri)
│   ├── spice.rs          # SPICE client via SpiceClientGLib FFI (session, display, input)
│   └── ui/
│       ├── mod.rs        # Main window, loading screen, SPICE display widget
│       └── toolbar.rs    # Fullscreen top-edge toolbar overlay (Revealer)
├── assets/
│   └── vmlaunch.svg      # Multicolor Windows icon for dock
├── vmlaunch.desktop       # Desktop entry
├── Cargo.toml
├── PLAN.md
└── CLAUDE.md
```

### App Flow

1. **Launch** — Check `virsh domstate win11`
2. **If not running** — Start VM, show loading screen with status text
3. **Wait for SPICE** — Poll `virsh domdisplay` until URI available
4. **Connect** — Embed SPICE client widget, switch to fullscreen display
5. **Running** — Standard keyboard/mouse passthrough, fullscreen controls (see below)
6. **Close window** — Send ACPI shutdown (`virsh shutdown win11`), wait up to 60s for VM to stop, then exit. If shutdown times out, prompt the user to force-off or keep waiting.

### Fullscreen & Toolbar

The app launches into fullscreen by default. A toolbar overlay appears when the mouse hits the top edge of the screen (like VMware/VirtualBox):

- **Trigger:** Mouse moves to the top ~2px of the screen in fullscreen mode
- **Toolbar contents:** VM name label, "Exit Fullscreen" button, "Send Ctrl+Alt+Del" button, Close button
- **Auto-hide:** Toolbar slides away after the mouse leaves it (short delay, ~500ms)
- **F11:** Also toggles fullscreen as a keyboard shortcut
- **Windowed mode:** Standard GTK4 headerbar with the same controls, no hover behavior needed

The toolbar is a GTK4 `Revealer` overlaid on top of the SPICE display widget. In windowed mode, the controls live in the normal headerbar.

### SPICE Integration

SpiceClientGtk is GTK3-only and cannot be embedded in a GTK4 application. The approach is to use **SpiceClientGLib** for session and channel management, and render the display framebuffer into a GTK4 widget (e.g., `gtk4::Picture` or a custom `Paintable`) via the `display` channel's framebuffer data.

Key GLib-level pieces:
- `SpiceSession` — connection management, signals for channel lifecycle
- `SpiceDisplayChannel` — provides framebuffer updates (`display-invalidate` signal)
- `SpiceInputsChannel` — keyboard and mouse input forwarding
- `SpiceMainChannel` — clipboard, agent communication

This avoids any GTK3 dependency and works natively on Wayland.

## Current VM Details

- **VM name:** win11
- **Hypervisor:** QEMU/KVM via libvirt (local)
- **SPICE URI:** `spice://127.0.0.1:5900` (from `virsh domdisplay`)
- **Display:** SPICE + virtio-vga
- **VM IP:** 192.168.122.20 (for boot detection via SSH)
- **SSH key:** `~/.ssh/id_ed25519`
- **Wayland issue:** GTK3 display clients need `GDK_BACKEND=x11`

## Dependencies (system)

Already installed on the laptop:
- `libspice-client-gtk-3.0-5` — GTK3 SPICE widget
- `libspice-client-glib-2.0-8` — GLib SPICE client
- `gir1.2-spiceclientgtk-3.0` — GObject introspection
- `libvirt` — VM management

Rust crates needed:
- `gtk4` (gtk4-rs)
- `libvirt` (via virt-rs or similar)
- `spice-client-glib` — custom `-sys` bindings via `gir` or manual FFI (no published crate)

## Open Decisions

- [ ] Spike SpiceClientGLib FFI from Rust — generate `-sys` bindings via `gir` from `/usr/share/gir-1.0/SpiceClientGLib-2.0.gir`, or write manual FFI for the core types. This is the biggest technical risk.
- [ ] Whether to support multiple VM profiles or hardcode win11 (hardcode for v1, parameterize later)
- [ ] Tray icon / minimize-to-tray behavior (skip for v1)
