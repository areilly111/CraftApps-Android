//! Linux: Preferences › Performance › Linux Display Server › X11 opens the window through XWayland
//! on a Wayland session, where dropped files arrive (winit 0.30 has no Wayland drag and drop,
//! #1151). Only when XWayland can run the window: an X display is set and the X11 libraries winit
//! loads at runtime are installed. Otherwise the window opens on Wayland, so a missing library
//! never keeps the app from starting.
//!
//! The library check comes from PhotoCraft's `linux_libs.rs`: no `dlopen` (that needs `unsafe`),
//! just the dynamic linker's cache (`ldconfig -p`) and the usual library directories.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// What winit 0.30 `dlopen`s for an X11 window: x11-dl opens Xlib, Xlib-xcb, Xcursor and XInput2,
/// x11rb opens libxcb, and xkbcommon-dl opens xkbcommon and xkbcommon-x11.
const X11_LIBS: &[&str] =
    &["libX11.so.6", "libX11-xcb.so.1", "libxcb.so.1", "libXcursor.so.1", "libXi.so.6", "libxkbcommon.so.0", "libxkbcommon-x11.so.0"];

/// Whether the saved `linuxDisplayServer` and the environment ask for XWayland: `x11` chosen, a
/// Wayland session (winit picks Wayland whenever `WAYLAND_DISPLAY` or `WAYLAND_SOCKET` is set)
/// and an X display to open.
pub fn wanted(pref: Option<&str>, var: impl Fn(&str) -> Option<OsString>) -> bool {
    let set = |name: &str| var(name).is_some_and(|v| !v.is_empty());
    pref == Some("x11") && (set("WAYLAND_DISPLAY") || set("WAYLAND_SOCKET")) && set("DISPLAY")
}

/// Are all the libraries an X11 window needs found on this system? Libraries the check can't find
/// count as missing.
pub fn x11_libraries_found() -> bool {
    let cache = read_ldconfig();
    let dirs = search_dirs(std::env::var("LD_LIBRARY_PATH").ok().as_deref());
    X11_LIBS
        .iter()
        .all(|lib| cache.as_deref().is_some_and(|out| ldconfig_has(out, lib, ldconfig_arch_tag())) || dirs.iter().any(|d| d.join(lib).exists()))
}

/// The `ldconfig -p` architecture tag for this build, when it's one we can tell apart from a
/// 32-bit library of the same name (`libc6,x86-64` vs plain `libc6` for i386).
fn ldconfig_arch_tag() -> Option<&'static str> {
    if cfg!(target_arch = "x86_64") {
        Some("x86-64")
    } else if cfg!(target_arch = "aarch64") {
        Some("AArch64")
    } else {
        None
    }
}

/// Does `ldconfig -p` output list `soname` for our architecture? Lines look like
/// `\tlibxkbcommon-x11.so.0 (libc6,x86-64) => /lib/x86_64-linux-gnu/libxkbcommon-x11.so.0`.
fn ldconfig_has(output: &str, soname: &str, arch_tag: Option<&str>) -> bool {
    output.lines().any(|line| {
        let Some((name, rest)) = line.trim_start().split_once(' ') else { return false };
        if name != soname {
            return false;
        }
        match arch_tag {
            None => true,
            Some(tag) => {
                let flags = rest.split_once('(').and_then(|(_, r)| r.split_once(')')).map_or("", |(f, _)| f);
                flags.split(',').any(|f| f.trim() == tag)
            }
        }
    })
}

/// `ldconfig -p`, from the usual locations (`/sbin` isn't on a normal user's PATH on Debian). A
/// real listing starts with "N libs found in cache".
fn read_ldconfig() -> Option<String> {
    ["/sbin/ldconfig", "/usr/sbin/ldconfig", "ldconfig"].into_iter().find_map(|cmd| {
        let out = std::process::Command::new(cmd).arg("-p").output().ok().filter(|out| out.status.success())?;
        let text = String::from_utf8_lossy(&out.stdout).into_owned();
        text.lines().next().is_some_and(|l| l.contains("libs found in cache")).then_some(text)
    })
}

/// Library directories to search: `LD_LIBRARY_PATH`, then the usual system and Flatpak paths.
fn search_dirs(ld_library_path: Option<&str>) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = ld_library_path.unwrap_or("").split(':').filter(|d| !d.is_empty()).map(PathBuf::from).collect();
    let triplet = if cfg!(target_arch = "aarch64") {
        "aarch64-linux-gnu"
    } else if cfg!(target_arch = "riscv64") {
        "riscv64-linux-gnu"
    } else {
        "x86_64-linux-gnu"
    };
    dirs.extend(["/app/lib", "/usr/local/lib", "/usr/lib64", "/lib64", "/usr/lib", "/lib"].map(PathBuf::from));
    dirs.push(Path::new("/usr/lib").join(triplet));
    dirs.push(Path::new("/lib").join(triplet));
    dirs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<OsString> {
        move |k| pairs.iter().find(|(name, _)| *name == k).map(|(_, v)| OsString::from(v))
    }

    #[test]
    fn xwayland_only_when_chosen_on_wayland_with_an_x_display() {
        let wayland = env(&[("WAYLAND_DISPLAY", "wayland-0"), ("DISPLAY", ":0")]);
        assert!(wanted(Some("x11"), &wayland));
        assert!(wanted(Some("x11"), env(&[("WAYLAND_SOCKET", "3"), ("DISPLAY", ":0")])));
        assert!(!wanted(Some("automatic"), &wayland));
        assert!(!wanted(None, &wayland));
        assert!(!wanted(Some("wayland"), &wayland));
        // An X11 session already opens on X11; without an X display there is no XWayland to use.
        assert!(!wanted(Some("x11"), env(&[("DISPLAY", ":0")])));
        assert!(!wanted(Some("x11"), env(&[("WAYLAND_DISPLAY", "wayland-0"), ("DISPLAY", "")])));
        assert!(!wanted(Some("x11"), env(&[("WAYLAND_DISPLAY", "wayland-0")])));
    }

    #[test]
    fn ldconfig_parsing_matches_name_and_architecture() {
        let cache = "12 libs found in cache `/etc/ld.so.cache'\n\
            \tlibX11.so.6 (libc6,x86-64) => /lib/x86_64-linux-gnu/libX11.so.6\n\
            \tlibxkbcommon-x11.so.0 (libc6) => /lib/i386-linux-gnu/libxkbcommon-x11.so.0\n";
        assert!(ldconfig_has(cache, "libX11.so.6", Some("x86-64")));
        assert!(!ldconfig_has(cache, "libX11.so.6", Some("AArch64")));
        // Only the 32-bit build is installed: doesn't count for a 64-bit app.
        assert!(!ldconfig_has(cache, "libxkbcommon-x11.so.0", Some("x86-64")));
        assert!(ldconfig_has(cache, "libxkbcommon-x11.so.0", None));
        assert!(!ldconfig_has(cache, "libX11.so", None));
    }
}
