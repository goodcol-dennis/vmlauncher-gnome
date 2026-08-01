//! VM lifecycle management via libvirt CLI.
//!
//! Uses `virsh` commands for simplicity — avoids linking libvirt directly
//! and keeps the dependency surface small.

use std::process::Command;

const VM_NAME: &str = "win11";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VmState {
    Running,
    Shutoff,
    Paused,
    Other,
}

/// Parse `virsh domstate` output into a [`VmState`].
///
/// Any state virsh reports that we don't model explicitly — `in shutdown`,
/// `pmsuspended`, `crashed`, `idle` — becomes [`VmState::Other`], as does the
/// empty output virsh produces when the domain doesn't exist.
fn parse_state(raw: &str) -> VmState {
    match raw.trim() {
        "running" => VmState::Running,
        "shut off" => VmState::Shutoff,
        "paused" => VmState::Paused,
        _ => VmState::Other,
    }
}

/// Query current VM state.
pub fn state() -> anyhow::Result<VmState> {
    let output = Command::new("virsh").args(["domstate", VM_NAME]).output()?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    log::debug!("virsh domstate {VM_NAME}: {}", stdout.trim());
    Ok(parse_state(&stdout))
}

/// Start the VM. No-op if already running.
pub fn start() -> anyhow::Result<()> {
    if state()? == VmState::Running {
        log::info!("VM {VM_NAME} already running");
        return Ok(());
    }
    log::info!("starting VM: {VM_NAME}");
    let output = Command::new("virsh").args(["start", VM_NAME]).output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("virsh start failed: {stderr}");
    }
    Ok(())
}

/// Send ACPI shutdown to the VM.
pub fn shutdown() -> anyhow::Result<()> {
    log::info!("sending ACPI shutdown to VM: {VM_NAME}");
    let output = Command::new("virsh").args(["shutdown", VM_NAME]).output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("virsh shutdown failed: {stderr}");
    }
    Ok(())
}

/// Force-off the VM (destroy).
pub fn force_off() -> anyhow::Result<()> {
    log::warn!("force-off VM: {VM_NAME}");
    let output = Command::new("virsh").args(["destroy", VM_NAME]).output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("virsh destroy failed: {stderr}");
    }
    Ok(())
}

/// Parse `virsh domdisplay` output into a display URI.
///
/// A domain with more than one graphics device makes virsh print one URI per
/// line, so we take the first non-empty line rather than trimming the whole
/// blob — joining them would produce a single unusable URI.
fn parse_display_uri(raw: &str) -> Option<String> {
    raw.lines().map(str::trim).find(|line| !line.is_empty()).map(ToString::to_string)
}

/// Get the SPICE display URI (e.g. <spice://127.0.0.1:5900>).
/// Returns None if VM is not running or display not available yet.
pub fn spice_uri() -> anyhow::Result<Option<String>> {
    let output = Command::new("virsh").args(["domdisplay", VM_NAME]).output()?;
    if !output.status.success() {
        return Ok(None);
    }
    Ok(parse_display_uri(&String::from_utf8_lossy(&output.stdout)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_the_states_we_act_on() {
        assert_eq!(parse_state("running"), VmState::Running);
        assert_eq!(parse_state("shut off"), VmState::Shutoff);
        assert_eq!(parse_state("paused"), VmState::Paused);
    }

    /// `virsh` terminates its output with a newline and a blank line; parsing
    /// must survive that or every state check falls through to `Other`.
    #[test]
    fn tolerates_real_virsh_output_framing() {
        assert_eq!(parse_state("running\n\n"), VmState::Running);
        assert_eq!(parse_state("shut off\n"), VmState::Shutoff);
        assert_eq!(parse_state("  paused  \n"), VmState::Paused);
    }

    /// States libvirt can report that we deliberately don't model. These must
    /// land on `Other` so the shutdown poll keeps waiting rather than
    /// concluding the VM is off.
    #[test]
    fn unmodelled_states_become_other() {
        for raw in ["in shutdown", "pmsuspended", "crashed", "idle", "no state"] {
            assert_eq!(parse_state(raw), VmState::Other, "{raw}");
        }
    }

    /// A nonexistent domain makes virsh fail with empty stdout. That must not
    /// be mistaken for "shut off", which would let `start()` skip straight to
    /// a `virsh start` that then fails confusingly.
    #[test]
    fn empty_output_is_not_shutoff() {
        assert_eq!(parse_state(""), VmState::Other);
        assert_eq!(parse_state("\n"), VmState::Other);
        assert_eq!(parse_state("   "), VmState::Other);
    }

    #[test]
    fn extracts_a_single_display_uri() {
        assert_eq!(
            parse_display_uri("spice://127.0.0.1:5900\n").as_deref(),
            Some("spice://127.0.0.1:5900")
        );
    }

    /// A domain with two graphics devices prints one URI per line. Trimming the
    /// whole blob would splice them into one unusable string, so we take the
    /// first.
    #[test]
    fn takes_the_first_uri_when_several_are_listed() {
        let raw = "spice://127.0.0.1:5900\nvnc://127.0.0.1:5901\n";
        assert_eq!(parse_display_uri(raw).as_deref(), Some("spice://127.0.0.1:5900"));
    }

    #[test]
    fn skips_leading_blank_lines() {
        assert_eq!(
            parse_display_uri("\n\n  spice://127.0.0.1:5900\n").as_deref(),
            Some("spice://127.0.0.1:5900")
        );
    }

    /// While the VM is booting virsh prints nothing — the caller polls on
    /// `None`, so blank output must never yield `Some("")`.
    #[test]
    fn blank_output_yields_no_uri() {
        assert_eq!(parse_display_uri(""), None);
        assert_eq!(parse_display_uri("\n\n"), None);
        assert_eq!(parse_display_uri("   \n  \n"), None);
    }
}
