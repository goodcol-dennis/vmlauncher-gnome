# vmlaunch

## Process Audit Reference
- Development guardrails (core): https://raw.githubusercontent.com/goodcol-dennis/umami/refs/heads/develop/umami.md
- Extension — Desktop (shared): https://raw.githubusercontent.com/goodcol-dennis/umami/refs/heads/develop/umami-desktop.md
- Extension — Desktop Linux: https://raw.githubusercontent.com/goodcol-dennis/umami/refs/heads/develop/desktop/umami-linux.md
  Do NOT fetch these every session. These are reference URLs for periodic process reviews.

## Project Overview

A Rust GTK4 app that launches a Windows 11 KVM/SPICE VM and embeds the SPICE display in a single window. One dock icon to start, use, and shut down the VM. See [PLAN.md](PLAN.md) for full design.

## Environment

- **Host:** Ubuntu 26.04 laptop (jardin), Intel Core Ultra 7 255H, 32GB RAM
- **VM:** win11 (KVM/libvirt), 8 vCPUs, 12GB RAM, SPICE display
- **SPICE URI:** `spice://127.0.0.1:5900` (from `virsh domdisplay win11`)
- **VM IP:** 192.168.122.20 (NAT, for SSH boot detection)
- **GUI:** GTK4 v4.22 (native Wayland), gtk4-rs 0.9 with `v4_14` feature
- **Rust toolchain:** stable, edition 2024, installed via rustup

## System Libraries

Required packages for building:

```bash
sudo apt install libgtk-4-dev libspice-client-glib-2.0-dev libgirepository1.0-dev
```

Note: if `libspice-client-glib-2.0-dev` is not installed, build.rs falls back to linking the soname directly. This works but install the -dev package for proper builds.

## Key Crates

```toml
[dependencies]
gtk4 = { version = "0.9", features = ["v4_14"] }  # GTK4 bindings
# No virt crate — using virsh CLI for VM management (simpler, fewer deps)
# No spice crate — manual FFI to libspice-client-glib-2.0 (no published crate)
```

## Build & Install

```bash
./install.sh          # Build + install binary, icon, desktop file
# Or manually:
cargo build --release
```

## Coding Standards

- `cargo fmt` and `cargo clippy -- -D warnings` must pass (pedantic enabled)
- Use `anyhow` for error handling
- Keep it simple — single binary, no plugin architecture
- Log to stderr with `env_logger`
- No hardcoded paths — use `$HOME`, `XDG_*` where appropriate
- VM name (`win11`) can be a const for now; parameterize later if needed

## What Agents Get Wrong

1. **SPICE channels don't auto-open.** After `channel-new` fires for display/inputs/cursor, you MUST call `spice_channel_connect()` explicitly. Without this, only main/playback/record channels open. Also must call `spice_main_channel_update_display()` on the main channel first.
2. **No `spice_channel_get_channel_type` function.** Use `g_object_get()` with `"channel-type"` and `"channel-id"` properties instead.
3. **GLTextureBuilder + epoxy segfaults.** Epoxy function pointers require a GL context active on the calling thread. Creating a context via `gdk::Display::create_gl_context()` or a hidden GLArea does not produce a usable context for direct GL calls. The MemoryTexture path works.
4. **SPICE xRGB format.** The alpha byte is 0x00. Use `B8g8r8x8` (GTK 4.14+) to tell GTK to ignore it. Using `B8g8r8a8Premultiplied` makes the display washed out.

## Desktop Integration

- **App ID:** `tech.goodcol.vmlaunch`
- **Desktop file:** `tech.goodcol.vmlaunch.desktop` (filename must match app ID for GNOME dock icon)
- **Icon:** `~/.local/share/icons/hicolor/scalable/apps/vmlaunch.svg`
- **Config:** `~/.config/vmlaunch/state.conf` (window size, fullscreen state)

## VM Control Reference

```bash
virsh domstate win11        # Check VM state
virsh start win11           # Start VM
virsh domdisplay win11      # Get SPICE URI (spice://127.0.0.1:5900)
virsh shutdown win11        # ACPI shutdown (graceful)
virsh destroy win11         # Force off (last resort)
```

## SpiceClientGLib FFI

Using SpiceClientGLib (not SpiceClientGtk, which is GTK3-only). Key C types/functions bound in `src/spice/ffi.rs`:

- `spice_session_new`, `spice_session_connect`, `spice_session_disconnect`
- `spice_channel_connect` — must call explicitly for display/inputs/cursor channels
- `spice_main_channel_update_display` — must call before display channel will open
- `spice_inputs_channel_key_press/release`, `position`, `button_press/release`
- `g_object_set/get` — for session URI property and channel type/id
- `g_signal_connect_data` — for `channel-new`, `display-primary-create`, `display-invalidate`

GIR file: `/usr/share/gir-1.0/SpiceClientGLib-2.0.gir` (requires `-dev` package)
