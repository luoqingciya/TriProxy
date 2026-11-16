// TriProxy — a modern multi-core proxy client.
// Copyright (C) 2026 luoqingci
// SPDX-License-Identifier: GPL-3.0-or-later
//
// This program is free software: you can redistribute it and/or modify it under
// the terms of the GNU General Public License as published by the Free Software
// Foundation, either version 3 of the License, or (at your option) any later
// version. It is distributed in the hope that it will be useful, but WITHOUT ANY
// WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS FOR A
// PARTICULAR PURPOSE. See LICENSE for the full text.
//
// Additional terms under GNU GPL v3 section 7 apply: see ADDITIONAL-TERMS.md.

//! Host distribution facts.
//!
//! `std::env::consts::OS` answers "linux", which is not enough to say whether
//! TUN can start or whether the window will have a tray icon. Three axes matter
//! on top of the kernel:
//!
//! - **Package family** decides which command an install or update instruction
//!   has to name (`dnf` or `apt`). A `.deb` is not installable on Fedora, and
//!   telling a Fedora user to run `dpkg` is worse than saying nothing.
//! - **The desktop session** decides whether a tray icon can appear at all.
//!   GNOME has no tray of its own; icons come from an AppIndicator extension that
//!   Fedora Workstation does not install, so `TrayIconBuilder` fails and the app
//!   silently behaves as "closing the window quits".
//! - **SELinux** is enforcing on Fedora by default. It does not block what
//!   TriProxy does, but a user reporting a permission error deserves to know it
//!   is on rather than guess.
//!
//! Everything here is read-only: two small files, one device node and three
//! environment variables. Nothing decides policy — the `doctor` command and the
//! TUN pre-flight check print it, so a platform gap becomes a sentence instead
//! of a failure with no explanation.
//!
//! `None` is used deliberately for "not applicable" and "cannot tell" alike;
//! callers must not turn either into a warning, because a wrong warning about
//! the tray is more annoying than the missing tray.

use std::path::PathBuf;

// Only the Linux probe looks at the device node; importing `Path` everywhere
// would leave it unused (and failing `-D warnings`) on Windows and macOS.
#[cfg(target_os = "linux")]
use std::path::Path;

/// Linux device node the cores open to create a TUN interface.
///
/// Absent when the `tun` module is not loaded — a core then fails internally
/// with a message that never mentions the device.
pub const TUN_DEVICE: &str = "/dev/net/tun";

/// GNOME extension that provides a StatusNotifier host.
///
/// Without it there is nowhere for a tray icon to go. The UUID is the
/// extension's directory name inside `gnome-shell/extensions`.
pub const APPINDICATOR_EXTENSION: &str = "appindicatorsupport@rgcjonas.gmail.com";

/// Which package manager a distribution speaks.
///
/// Only ever used to phrase an instruction; no behaviour branches on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackageFamily {
    Deb,
    Rpm,
    Other,
}

impl PackageFamily {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Deb => "deb",
            Self::Rpm => "rpm",
            Self::Other => "unknown",
        }
    }
}

/// What `/etc/os-release` says this machine is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Distro {
    pub id: String,
    pub name: String,
    pub version_id: String,
    /// `ID_LIKE` entries. This — not `ID` — is where Ubuntu says `debian` and
    /// Fedora's derivatives say `fedora`, so the family question cannot be
    /// answered from `ID` alone.
    pub id_like: Vec<String>,
}

impl Distro {
    /// Parse the content of `os-release`.
    ///
    /// Unknown keys are ignored rather than rejected: the format is a shell
    /// fragment that distributions keep extending, so a strict parser would
    /// break on the next release without gaining anything. Returns `None` only
    /// when the file carries neither `ID` nor `NAME`.
    pub fn parse(text: &str) -> Option<Self> {
        let mut id = String::new();
        let mut name = String::new();
        let mut version_id = String::new();
        let mut id_like = Vec::new();

        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let value = unquote(value.trim());
            match key.trim() {
                "ID" => id = value,
                "NAME" => name = value,
                "VERSION_ID" => version_id = value,
                "ID_LIKE" => id_like = value.split_whitespace().map(str::to_string).collect(),
                // PRETTY_NAME, HOME_URL, CPE_NAME and friends are for humans
                // and change between releases.
                _ => {}
            }
        }

        if id.is_empty() && name.is_empty() {
            return None;
        }
        if name.is_empty() {
            name = id.clone();
        }

        Some(Self {
            id,
            name,
            version_id,
            id_like,
        })
    }

    /// `ID` first, then `ID_LIKE`: exact knowledge beats an inherited claim.
    pub fn family(&self) -> PackageFamily {
        for id in std::iter::once(&self.id).chain(self.id_like.iter()) {
            let family = match id.as_str() {
                "debian" | "ubuntu" | "linuxmint" | "pop" | "raspbian" => PackageFamily::Deb,
                "kali" | "deepin" | "elementary" | "zorin" => PackageFamily::Deb,
                "fedora" | "rhel" | "centos" | "rocky" | "almalinux" => PackageFamily::Rpm,
                "ol" | "amzn" | "opensuse" | "opensuse-leap" | "sles" => PackageFamily::Rpm,
                _ => PackageFamily::Other,
            };
            if family != PackageFamily::Other {
                return family;
            }
        }
        PackageFamily::Other
    }

    /// `Fedora Linux 45` — for a header line, not for parsing.
    pub fn label(&self) -> String {
        if self.version_id.is_empty() {
            self.name.clone()
        } else {
            format!("{} {}", self.name, self.version_id)
        }
    }
}

/// Strip the optional quotes `os-release` allows around values.
fn unquote(value: &str) -> String {
    let bytes = value.as_bytes();
    if bytes.len() >= 2 {
        let first = bytes[0];
        let last = bytes[bytes.len() - 1];
        if (first == b'"' || first == b'\'') && first == last {
            return value[1..value.len() - 1].to_string();
        }
    }
    value.to_string()
}

/// This machine's distribution, if it publishes one.
pub fn distro() -> Option<Distro> {
    // `/etc/os-release` is a symlink to the second path on most systems; both
    // are read so a stripped image that ships only one still reports something.
    for path in ["/etc/os-release", "/usr/lib/os-release"] {
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        if let Some(distro) = Distro::parse(&text) {
            return Some(distro);
        }
    }
    None
}

/// SELinux enforcement state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Selinux {
    Enforcing,
    Permissive,
}

impl Selinux {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Enforcing => "enforcing",
            Self::Permissive => "permissive",
        }
    }
}

/// SELinux state, or `None` when this machine does not use SELinux.
///
/// `/sys/fs/selinux` is mounted only when SELinux is compiled in *and* not
/// disabled on the kernel command line, so a missing `enforce` file means "no
/// SELinux here" — not "permissive", which would be a false alarm.
pub fn selinux() -> Option<Selinux> {
    let enforce = std::fs::read_to_string("/sys/fs/selinux/enforce").ok()?;
    match enforce.trim() {
        "1" => Some(Selinux::Enforcing),
        "0" => Some(Selinux::Permissive),
        _ => None,
    }
}

/// Whether the TUN device node exists.
///
/// `None` on the platforms that create adapters without a device node
/// (wintun on Windows, utun on macOS), so callers only report what applies.
pub fn tun_device_present() -> Option<bool> {
    #[cfg(target_os = "linux")]
    {
        Some(Path::new(TUN_DEVICE).exists())
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

/// The desktop session TriProxy was started in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Desktop {
    Gnome,
    Kde,
    Other(String),
}

impl Desktop {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Gnome => "GNOME",
            Self::Kde => "KDE Plasma",
            Self::Other(name) => name,
        }
    }
}

/// Parse `XDG_CURRENT_DESKTOP`-style values.
///
/// The value is a colon-separated list whose order carries no meaning:
/// `ubuntu:GNOME`, `GNOME`, `KDE`, `X-Cinnamon`, `sway`. The first entry this
/// build recognises wins; otherwise the first entry is kept for display.
pub fn parse_desktop(value: &str) -> Option<Desktop> {
    let mut first = None;

    for part in value.split(':').map(str::trim).filter(|p| !p.is_empty()) {
        match part.to_ascii_lowercase().as_str() {
            "gnome" | "ubuntu" | "unity" | "pop" => return Some(Desktop::Gnome),
            "gnome-classic" | "gnome-xorg" | "ubuntu-xorg" => return Some(Desktop::Gnome),
            "kde" | "plasma" | "plasmawayland" => return Some(Desktop::Kde),
            _ => {
                first.get_or_insert_with(|| part.to_string());
            }
        };
    }

    first.map(Desktop::Other)
}

/// The session's desktop, from the variables the XDG specification defines.
///
/// `XDG_CURRENT_DESKTOP` is the authoritative one; the other two are set by
/// session managers that predate it (and by `ssh`-less login managers).
pub fn desktop() -> Option<Desktop> {
    let keys = [
        "XDG_CURRENT_DESKTOP",
        "XDG_SESSION_DESKTOP",
        "DESKTOP_SESSION",
    ];
    for key in keys {
        let Ok(value) = std::env::var(key) else {
            continue;
        };
        if let Some(desktop) = parse_desktop(&value) {
            return Some(desktop);
        }
    }
    None
}

/// Whether the current session can show a StatusNotifier tray icon.
///
/// `None` means "cannot tell from the environment" — the caller must stay
/// quiet. The tray either appears or it does not, and the application already
/// degrades to plain-window behaviour when it does not, so a guess adds noise
/// without adding information.
pub fn tray_host_available() -> Option<bool> {
    match desktop()? {
        Desktop::Gnome => Some(gnome_appindicator_installed()),
        // Plasma hosts StatusNotifier items itself.
        Desktop::Kde => Some(true),
        Desktop::Other(_) => None,
    }
}

/// Is the AppIndicator extension present for the shell and for the user?
fn gnome_appindicator_installed() -> bool {
    let mut roots = vec![PathBuf::from("/usr/share/gnome-shell/extensions")];
    if let Some(data) = dirs::data_dir() {
        roots.push(data.join("gnome-shell/extensions"));
    }

    roots
        .iter()
        .any(|root| root.join(APPINDICATOR_EXTENSION).is_dir())
}

/// Command that installs the AppIndicator extension on this distribution.
///
/// Returned for the user to run, never run by TriProxy: installing packages is
/// the user's decision, and the application works without it.
pub fn appindicator_install_hint() -> String {
    match distro().map(|distro| distro.family()) {
        Some(PackageFamily::Rpm) => {
            "sudo dnf install gnome-shell-extension-appindicator".to_string()
        }
        Some(PackageFamily::Deb) => {
            "sudo apt install gnome-shell-extension-appindicator".to_string()
        }
        _ => "安装 gnome-shell-extension-appindicator 扩展".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FEDORA: &str = r#"
NAME="Fedora Linux"
VERSION="45 (Workstation Edition)"
ID=fedora
VERSION_ID=45
PRETTY_NAME="Fedora Linux 45 (Workstation Edition)"
ID_LIKE="rhel centos"
VARIANT="Workstation Edition"
"#;

    #[test]
    fn parses_fedora_os_release() {
        let distro = Distro::parse(FEDORA).expect("fedora is parseable");
        assert_eq!(distro.id, "fedora");
        assert_eq!(distro.name, "Fedora Linux");
        assert_eq!(distro.version_id, "45");
        assert_eq!(distro.label(), "Fedora Linux 45");
        assert_eq!(distro.family(), PackageFamily::Rpm);
    }

    /// The family has to come from `ID_LIKE` on derivatives: Rocky and Alma
    /// advertise `rhel`, never `fedora`.
    #[test]
    fn a_derivative_inherits_its_family_from_id_like() {
        let rocky = "NAME=Rocky\nID=rocky\nVERSION_ID=9.4\nID_LIKE=\"rhel centos fedora\"\n";
        assert_eq!(Distro::parse(rocky).unwrap().family(), PackageFamily::Rpm);

        let mint = "NAME=Mint\nID=linuxmint\nID_LIKE=\"ubuntu debian\"\n";
        assert_eq!(Distro::parse(mint).unwrap().family(), PackageFamily::Deb);
    }

    #[test]
    fn unquoted_values_and_unknown_keys_are_handled() {
        let text = "ID=fedora\nSOMETHING_NEW=1\n# comment\n\n";
        let distro = Distro::parse(text).unwrap();
        assert_eq!(distro.id, "fedora");
        // No NAME: the id stands in, rather than an empty header line.
        assert_eq!(distro.name, "fedora");
        assert_eq!(distro.label(), "fedora");
    }

    #[test]
    fn a_file_without_identity_is_rejected() {
        assert!(Distro::parse("").is_none());
        assert!(Distro::parse("PRETTY_NAME=\"Nowhere\"\n").is_none());
    }

    #[test]
    fn desktop_lists_are_order_independent() {
        assert_eq!(parse_desktop("ubuntu:GNOME"), Some(Desktop::Gnome));
        assert_eq!(parse_desktop("GNOME"), Some(Desktop::Gnome));
        assert_eq!(parse_desktop("KDE"), Some(Desktop::Kde));
        assert_eq!(
            parse_desktop("X-Cinnamon"),
            Some(Desktop::Other("X-Cinnamon".into()))
        );
        assert_eq!(parse_desktop(""), None);
        assert_eq!(parse_desktop("::"), None);
    }

    /// Probing must never fail: `doctor` calls it on every platform, including
    /// the ones where the concept does not exist.
    #[test]
    fn probes_do_not_panic() {
        let _ = distro();
        let _ = selinux();
        let _ = tun_device_present();
        let _ = tray_host_available();
        let _ = appindicator_install_hint();
    }

    /// The hint names a package manager, because "install the extension" alone
    /// leaves the user to guess which one their machine has.
    #[test]
    fn the_appindicator_hint_names_a_command() {
        let hint = appindicator_install_hint();
        assert!(hint.contains("appindicator"), "{hint}");
    }
}
