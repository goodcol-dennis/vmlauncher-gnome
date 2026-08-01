# File Sharing & Clipboard — Options Analysis

## Current Infrastructure (verified live)

| Component | Status | Details |
|-----------|--------|---------|
| **virtio-fs share** | **Working** | Host `/home/dennis/shared` → guest `Z:\` via virtiofs. Files visible on both sides. |
| **SPICE agent (vdagent)** | **Working** | Agent connected, clipboard + display channels active |
| **OpenSSH server** | **Working** | `robot@192.168.122.20`, key auth via `~/.ssh/id_ed25519` |
| **SMB (port 445)** | **Blocked** | Firewall rules exist but ALL disabled (both inbound and outbound). `mount -t cifs` fails with "could not connect". |
| **SMB fstab entry** | **Stale** | `/etc/fstab` has `//192.168.122.20/Dennis` → `/home/dennis/windows` with creds at `~/.smbcredentials-win11`, but no `Dennis` share exists (only admin shares: C$, Z$, ADMIN$, IPC$) |
| **QEMU guest agent** | **Partially working** | Channel exists but `guest-network-get-interfaces` is disabled |

### Windows VM Drives

| Drive | Type | Filesystem | Size | Free |
|-------|------|-----------|------|------|
| C: | Local disk (system) | NTFS | 200 GB | 148 GB |
| Z: | virtio-fs (`~/shared`) | NTFS (virtiofs) | 465 GB | 321 GB |
| D:, E: | CD-ROM | — | — | — |

### Accounts

| Account | Purpose | Access |
|---------|---------|--------|
| `robot` | SSH admin (hidden from login) | Key-based SSH, admin privs |
| `dennis` | Primary user | Desktop login, SMB credential on file |

---

## Option 1: SPICE Clipboard (bidirectional text/image)

**How it works:** SPICE main channel clipboard protocol. The vdagent in Windows intercepts clipboard changes and forwards them. We bridge GTK4 `gdk::Clipboard` ↔ SPICE clipboard signals.

**Available APIs:**
```
spice_main_channel_clipboard_selection_grab(channel, selection, types, ntypes)
spice_main_channel_clipboard_selection_notify(channel, selection, type, data, size)
spice_main_channel_clipboard_selection_request(channel, selection, type)
spice_main_channel_clipboard_selection_release(channel, selection)
```

**Signals on SpiceMainChannel:**
- `main-clipboard-selection-grab` — guest has new clipboard content
- `main-clipboard-selection-request` — guest wants our clipboard data
- `main-clipboard-selection` — guest sending clipboard data to us
- `main-clipboard-selection-release` — guest released clipboard

**Supported types:** text/plain (UTF-8), text/html, image/png, image/bmp

| Pros | Cons |
|------|------|
| Already wired — just bridge GTK↔SPICE | Large images may be slow |
| Text AND images | File clipboard needs special handling |
| No network needed (in-memory) | Format negotiation complexity |
| How remote-viewer does it | |

**Effort:** Medium (~200 lines FFI + clipboard bridge)

---

## Option 2: SPICE File Transfer (host→guest drag-and-drop)

**How it works:** Drag files onto vmlaunch → `spice_main_channel_file_copy_async()` sends through vdagent to guest.

**Available APIs:**
```
spice_main_channel_file_copy_async(channel, sources, flags, cancellable, progress_cb, cb, user_data)
spice_file_transfer_task_get_progress(task)
spice_file_transfer_task_get_filename(task)
spice_file_transfer_task_cancel(task)
```

| Pros | Cons |
|------|------|
| Built into SPICE | **One-way only** (host→guest) |
| Progress tracking included | Files go to fixed guest location |
| GTK4 DropTarget is easy | No folder transfer |
| No network needed | |

**Effort:** Medium (~150 lines FFI + DropTarget)

---

## Option 3: virtio-fs Shared Folder (bidirectional — ALREADY WORKING)

**Current state:** Fully operational. Host `/home/dennis/shared` is `Z:\` in Windows. Files already being exchanged (RecordPerfect project files visible on both sides).

**What vmlaunch could add:**
- Quick-access button → opens `~/shared` in host file manager
- Drag-and-drop onto window → copies file to `~/shared` (instant visibility on `Z:\`)
- File browser panel showing `~/shared` contents
- inotify watcher for notifications when guest adds files

| Pros | Cons |
|------|------|
| **Already working** — zero setup | Must use a specific folder |
| Bidirectional | User has to know about Z:\ |
| Near-native speed (virtio-fs) | Can't browse arbitrary Windows paths |
| Files AND folders | |

**Effort:** Low (button) to Medium (file browser panel)

---

## Option 4: SMB/CIFS Shares

**Current state:** NOT WORKING. All firewall rules disabled. No user shares exist (only admin shares C$, Z$). The fstab entry references a nonexistent `Dennis` share.

**To make it work would require:**
1. Enable SMB firewall rules on the guest
2. Create a user share (or use admin shares with elevated creds)
3. Update fstab entry with correct share name
4. Or: vmlaunch manages mount/unmount lifecycle

| Pros | Cons |
|------|------|
| Standard protocol | **Currently broken** — firewall + no shares |
| Access full Windows filesystem via C$ | Credentials management |
| Works with any file manager | Requires enabling firewall rules |
| | Slower than virtio-fs |

**Effort:** Medium (mount management) + guest configuration needed

---

## Option 5: SSH/SFTP (bidirectional — WORKING)

**Current state:** SSH working via `robot@192.168.122.20` with key auth. Can execute commands and transfer files.

**What vmlaunch could add:**
- Drag-and-drop → SCP to VM (any destination path)
- SFTP file browser sidebar showing Windows filesystem
- Drag files OUT — browse VM → drag to host
- Remote PowerShell commands from app

| Pros | Cons |
|------|------|
| **Working now** — full filesystem access | Needs SFTP library in deps |
| Bidirectional | Heavier UI work (file browser) |
| Encrypted | Requires network (NAT) |
| Can also run commands | SSH must be up (boot delay) |

**Effort:** High (SFTP library + file browser UI)

---

## Option 6: QEMU Guest Agent

**Current state:** Channel exists but partially disabled (network queries blocked).

Not recommended — low-level file I/O only (open/read/write/close), no directory listing, base64 overhead. Everything the agent can do, SSH does better.

---

## Recommended Approach

### Phase 1 — Leverage what's already working (low effort, high value)
1. **SPICE clipboard sharing** — biggest daily UX win
2. **Quick-access `~/shared` button** — toolbar button opens shared folder in file manager
3. **Drag-and-drop → `~/shared`** — drop file on window → copies to shared folder → instantly on Z:\

### Phase 2 — Enhanced integration
4. **SPICE host→guest file drop** — for sending files to specific Windows locations (not just Z:\)
5. **SFTP sidebar** — browse Windows filesystem via SSH, drag files between systems

### Phase 3 — Full management
6. **SMB share management** — enable firewall rules, create shares, auto-mount (if user wants SMB)
7. **Remote command panel** — run PowerShell from vmlaunch

### What NOT to build
- QEMU guest agent file ops — SSH is strictly better
- Custom file transfer protocol — SPICE + SFTP + virtio-fs covers everything
- WebDAV via SPICE — virtio-fs is already faster and simpler
