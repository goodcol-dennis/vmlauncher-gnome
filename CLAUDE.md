# vmlaunch

## Process Audit Reference
- Development guardrails (core): https://raw.githubusercontent.com/goodcol-dennis/umami/refs/heads/develop/umami.md
- Extension — Desktop (shared): https://raw.githubusercontent.com/goodcol-dennis/umami/refs/heads/develop/umami-desktop.md
- Extension — Desktop Linux: https://raw.githubusercontent.com/goodcol-dennis/umami/refs/heads/develop/desktop/umami-linux.md
  Do NOT fetch these every session. These are reference URLs for periodic process reviews.

## Project Overview

A Rust GTK4 app that launches a Windows 11 KVM/SPICE VM and embeds the SPICE display in a single window. One dock icon to start, use, and shut down the VM.

> [PLAN.md](PLAN.md) is the original design document and is now **historical** — several
> of its decisions were overtaken during implementation. This file and the code are
> authoritative.

## Implemented Features

- VM lifecycle via `virsh` — start, resume a paused domain, wake a suspended one, ACPI shutdown with a Keep Waiting / Force Off prompt
- Embedded SPICE display, keyboard, mouse (absolute positioning), and scroll
- **Audio** — `spice_audio_get` binds a GStreamer sink to the session
- **Bidirectional text clipboard** (`src/spice/clipboard.rs`) with LF↔CRLF translation
- **Guest resolution follows the window**, debounced, scale-factor aware
- Fullscreen Revealer toolbar, persisted window geometry
- Held keys released on focus-out so modifiers don't stick in the guest

Not implemented: guest cursor rendering (cursor channel is connected but its signals
are ignored), windowed-mode headerbar controls, relative/server mouse mode, clipboard
images, USB redirection. See `docs/file-sharing-options.md` for the file-sharing
analysis — the virtio-fs share at `~/shared` → `Z:\` is live and used daily.

## Environment

- **Host:** Ubuntu 26.04 laptop (jardin), Intel Core Ultra 7 255H, 32GB RAM
- **VM:** win11 (KVM/libvirt), 8 vCPUs, 12GB RAM, SPICE display, `<audio type='spice'/>`
- **SPICE URI:** `spice://127.0.0.1:5900` (from `virsh domdisplay win11`)
- **Boot detection:** polling `virsh domdisplay` — there is no SSH code in this project
- **GUI:** GTK4 v4.22 (native Wayland), gtk4-rs 0.9 with `v4_14` feature
- **Rust toolchain:** stable, edition 2024, installed via rustup

## System Libraries

Required packages for building:

```bash
sudo apt install libgtk-4-dev libspice-client-glib-2.0-dev
```

Note: `libspice-client-glib-2.0-dev` is **not currently installed on this machine**, so
build.rs takes its fallback path — it discovers the versioned runtime library and links
it directly. That works, but installing the `-dev` package is the proper fix. No
gobject-introspection package is needed; nothing here does gir codegen.

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

- `cargo fmt` and `cargo clippy --all-targets -- -D warnings` must pass (pedantic enabled).
  `--all-targets` matters: without it, test code is never linted.
- Tests live in `#[cfg(test)]` modules beside the code they cover, so they can reach
  module-private functions. A binary crate's `tests/` directory cannot.
- Use `anyhow` for error handling
- Keep it simple — single binary, no plugin architecture
- Log to stderr with `env_logger`
- No hardcoded paths — use `$HOME`, `XDG_*` where appropriate
- VM name (`win11`) can be a const for now; parameterize later if needed

## What Agents Get Wrong

1. **SPICE channels don't auto-open.** After `channel-new` fires for display/inputs/cursor, you MUST call `spice_channel_connect()` explicitly. Without this, only main/playback/record channels open. You must also call `spice_main_channel_update_display_enabled(channel, 0, TRUE, FALSE)` on the main channel first — **not** `spice_main_channel_update_display()`, which is a different function that pins a specific resolution. We call `update_display()` too, but only later and only to make the guest follow the window size.
2. **No `spice_channel_get_channel_type` function.** Use `g_object_get()` with `"channel-type"` and `"channel-id"` properties instead.
3. **GLTextureBuilder + epoxy segfaults.** Epoxy function pointers require a GL context active on the calling thread. Creating a context via `gdk::Display::create_gl_context()` or a hidden GLArea does not produce a usable context for direct GL calls. The MemoryTexture path works.
4. **SPICE xRGB format.** The alpha byte is 0x00. Use `B8g8r8x8` (GTK 4.14+) to tell GTK to ignore it. Using `B8g8r8a8Premultiplied` makes the display washed out.
5. **Mouse button masks are not `1 << button`.** The protocol values are LEFT `0x1`, MIDDLE `0x2`, RIGHT `0x4`, SIDE `0x20`, EXTRA `0x40`. Deriving them from the button number produces a self-consistent but wrong table, and because `spice_inputs_channel_position` forwards the mask verbatim, a left-drag then arrives in the guest as a middle-drag. Verified by disassembling `spice_inputs_channel_button_press`.
6. **`display-primary-destroy` must be handled.** spice-gtk frees the primary surface on every guest resolution change. A cached framebuffer pointer that isn't cleared there will be read after free.

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
- `spice_main_channel_update_display_enabled` — call this before the display channel will open
- `spice_main_channel_update_display` — pushes a specific resolution to the guest; used for window-follows-resize
- `spice_audio_get` — binds playback/record to a GStreamer sink; returns transfer-none, do NOT unref
- `spice_inputs_channel_key_press/release`, `position`, `button_press/release`
- `spice_main_channel_clipboard_selection_grab/notify/request/release`
- `g_object_set/get` — for session URI property and channel type/id
- `g_signal_connect_data` — for `channel-new`, `display-primary-create`, `display-primary-destroy`, `display-invalidate`, and the `main-clipboard-selection-*` family

**No GIR file on this host.** `/usr/share/gir-1.0/SpiceClientGLib-2.0.gir` ships with the
`-dev` package, which is not installed. To check a signature against reality, disassemble
the runtime library instead:

```bash
objdump -d --disassemble=spice_inputs_channel_button_press \
  /usr/lib/x86_64-linux-gnu/libspice-client-glib-2.0.so.8
nm -D --defined-only /usr/lib/x86_64-linux-gnu/libspice-client-glib-2.0.so.8 | grep spice_audio
```
