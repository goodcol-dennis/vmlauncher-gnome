use std::path::PathBuf;

fn main() {
    // Without an explicit rerun-if directive, cargo re-probes pkg-config on
    // every source edit.
    println!("cargo:rerun-if-changed=build.rs");

    if pkg_config::Config::new().probe("spice-client-glib-2.0").is_ok() {
        return;
    }

    // Note: cargo only parses build-script STDOUT for `cargo:` directives.
    // These warnings used to go to eprintln!, so they never once reached the user.
    println!(
        "cargo:warning=pkg-config could not find spice-client-glib-2.0 — linking the installed \
         shared library directly. Install libspice-client-glib-2.0-dev for a proper build."
    );

    match find_runtime_library() {
        Some((dir, filename)) => {
            println!("cargo:rustc-link-search=native={}", dir.display());
            println!("cargo:rustc-link-lib=dylib:+verbatim={filename}");
        }
        None => panic!(
            "libspice-client-glib-2.0 was not found on this system.\n\
             Install it with:\n    sudo apt install libspice-client-glib-2.0-dev"
        ),
    }
}

/// Locate the versioned shared library.
///
/// Deliberately discovered rather than hardcoded: the previous fallback pinned
/// both the `x86_64` multiarch triple and the `.so.8` major, so a distro bump to
/// `.so.9` or a non-x86_64 host would produce a bare "cannot find -lspice…"
/// from the linker with nothing pointing at the cause.
fn find_runtime_library() -> Option<(PathBuf, String)> {
    const PREFIX: &str = "libspice-client-glib-2.0.so.";

    for dir in candidate_dirs() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut best: Option<String> = None;
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !name.starts_with(PREFIX) {
                continue;
            }
            // Prefer the shortest match: the bare soname symlink
            // (libfoo.so.8) over the fully versioned file (libfoo.so.8.8.2).
            if best.as_ref().is_none_or(|current| name.len() < current.len()) {
                best = Some(name);
            }
        }
        if let Some(name) = best {
            return Some((dir, name));
        }
    }
    None
}

/// Library directories to search, most specific first.
fn candidate_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();

    // Ask the C compiler for this host's multiarch triple rather than assuming
    // x86_64-linux-gnu.
    if let Ok(output) = std::process::Command::new("cc").arg("-print-multiarch").output()
        && output.status.success()
    {
        let triple = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !triple.is_empty() {
            dirs.push(PathBuf::from(format!("/usr/lib/{triple}")));
        }
    }

    dirs.push(PathBuf::from("/usr/lib"));
    dirs.push(PathBuf::from("/usr/lib64"));
    dirs.push(PathBuf::from("/usr/local/lib"));
    dirs
}
