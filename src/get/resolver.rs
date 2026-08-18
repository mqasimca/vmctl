use super::*;

pub(super) fn resolve_image(
    os: &str,
    release: &str,
    edition: Option<&str>,
    architecture: &str,
) -> Result<ResolvedImage> {
    let info = find_os(os)?;
    let os = info.id;
    let architecture = normalize_architecture(architecture)?;
    if !info
        .architectures
        .split_whitespace()
        .any(|value| value == architecture)
    {
        return Err(Error::message(format!(
            "{} is not available for {} (supported: {})",
            info.name, architecture, info.architectures
        )));
    }
    let edition = required_edition(info, edition)?;
    if os == "debian" && architecture == "arm64" && edition.as_deref() != Some("netinst") {
        return Err(Error::message(
            "Debian ARM64 get images currently support only the netinst edition",
        ));
    }
    if is_ubuntu_desktop(os) && architecture == "arm64" && !ubuntu_arm64_release(release) {
        return Err(Error::message(format!(
            "{os} {release} is not available for ARM64 desktop images"
        )));
    }
    if os == "fedora" && architecture == "arm64" && edition.as_deref() == Some("Onyx") {
        return Err(Error::message("Fedora Onyx is not available for ARM64"));
    }
    let (url, kind) = match os {
        "alma" => {
            let arch = qemu_architecture(architecture);
            let edition = edition.as_deref().unwrap_or("dvd");
            (
                format!(
                    "https://repo.almalinux.org/almalinux/{release}/isos/{arch}/AlmaLinux-{release}-latest-{arch}-{edition}.iso"
                ),
                ImageKind::Iso,
            )
        }
        "archlinux" => (
            "https://geo.mirror.pkgbuild.com/iso/latest/archlinux-x86_64.iso".to_string(),
            ImageKind::Iso,
        ),
        "centos-stream" => (
            format!(
                "https://linuxsoft.cern.ch/centos-stream/{release}-stream/BaseOS/x86_64/iso/CentOS-Stream-{release}-latest-x86_64-{}.iso",
                edition.as_deref().unwrap_or("dvd1")
            ),
            ImageKind::Iso,
        ),
        "debian" => {
            let arch = architecture;
            let edition = edition.as_deref().unwrap_or("standard");
            let live = if edition == "netinst" { "" } else { "-live" };
            (
                format!(
                    "https://cdimage.debian.org/debian-cd/{release}{live}/{arch}/iso-hybrid/debian{live}-{release}-{arch}-{edition}.iso"
                ),
                ImageKind::Iso,
            )
        }
        "deepin" => (
            format!(
                "https://cdimage.deepin.com/releases/{release}/amd64/deepin-desktop-community-{release}-amd64.iso"
            ),
            ImageKind::Iso,
        ),
        "devuan" => {
            let version = match release {
                "daedalus" => "5.0.0",
                "chimaera" => "4.0.3",
                other => other,
            };
            (
                format!(
                    "https://files.devuan.org/devuan_{release}/desktop-live/devuan_{release}_{version}_amd64_desktop-live.iso"
                ),
                ImageKind::Iso,
            )
        }
        "dragonflybsd" => (
            format!(
                "https://mirror-master.dragonflybsd.org/iso-images/dfly-x86_64-{release}_REL.iso.bz2"
            ),
            ImageKind::Archive,
        ),
        "freebsd" => (
            format!(
                "{FREEBSD_ISO_IMAGES}{release}/FreeBSD-{release}-RELEASE-amd64-{}.iso",
                edition.as_deref().unwrap_or("disc1")
            ),
            ImageKind::Iso,
        ),
        "freedos" => (
            format!(
                "https://www.ibiblio.org/pub/micro/pc-stuff/freedos/files/distributions/{release}/{}",
                if release == "1.2" {
                    "official/FD12CD.iso"
                } else {
                    "FD14-LiveCD.zip"
                }
            ),
            if release == "1.2" {
                ImageKind::Iso
            } else {
                ImageKind::Archive
            },
        ),
        "guix" => (
            format!(
                "https://ftpmirror.gnu.org/gnu/guix/guix-system-install-{release}.x86_64-linux.iso"
            ),
            ImageKind::Iso,
        ),
        "haiku" => (
            format!(
                "https://mirror.rit.edu/haiku/{release}/haiku-{release}-{}-anyboot.iso",
                edition.as_deref().unwrap_or("x86_64")
            ),
            ImageKind::Iso,
        ),
        "linuxlite" => (
            format!(
                "https://sourceforge.net/projects/linux-lite/files/{release}/linux-lite-{release}-64bit.iso"
            ),
            ImageKind::Iso,
        ),
        "linuxmint" => (
            format!(
                "https://mirrors.kernel.org/linuxmint/stable/{release}/linuxmint-{release}-{}-64bit.iso",
                edition.as_deref().unwrap_or("cinnamon")
            ),
            ImageKind::Iso,
        ),
        "lmde" => (
            format!(
                "https://mirrors.kernel.org/linuxmint/debian/lmde-{release}-{}-64bit.iso",
                edition.as_deref().unwrap_or("cinnamon")
            ),
            ImageKind::Iso,
        ),
        "netboot" => (
            "https://boot.netboot.xyz/ipxe/netboot.xyz.iso".to_string(),
            ImageKind::Iso,
        ),
        "netbsd" => (
            format!(
                "https://cdn.netbsd.org/pub/NetBSD/NetBSD-{release}/images/NetBSD-{release}-amd64.iso"
            ),
            ImageKind::Iso,
        ),
        "nixos" => (
            format!(
                "https://channels.nixos.org/nixos-{release}/latest-nixos-{}-x86_64-linux.iso",
                edition.as_deref().unwrap_or("graphical")
            ),
            ImageKind::Iso,
        ),
        "openbsd" => (
            format!(
                "https://mirror.leaseweb.com/pub/OpenBSD/{release}/amd64/install{}.iso",
                release.replace('.', "")
            ),
            ImageKind::Iso,
        ),
        "openindiana" => (
            format!(
                "https://dlc.openindiana.org/isos/hipster/{release}/OI-hipster-{}-{release}.iso",
                edition.as_deref().unwrap_or("gui")
            ),
            ImageKind::Iso,
        ),
        "opensuse" => {
            let (base, file) = match release {
                "tumbleweed" => (
                    "https://download.opensuse.org/tumbleweed/iso",
                    "openSUSE-Tumbleweed-DVD-x86_64-Current.iso",
                ),
                "microos" => (
                    "https://download.opensuse.org/tumbleweed/iso",
                    "openSUSE-MicroOS-DVD-x86_64-Current.iso",
                ),
                _ => (
                    "https://download.opensuse.org/distribution/leap/{release}/iso",
                    "openSUSE-Leap-{release}-DVD-x86_64-Current.iso",
                ),
            };
            (
                format!("{base}/{file}").replace("{release}", release),
                ImageKind::Iso,
            )
        }
        "oraclelinux" => {
            let major = release.split('.').next().unwrap_or(release);
            let minor = release.split('.').nth(1).unwrap_or("0");
            let file = if major == "7" {
                format!("OracleLinux-R{major}-U{minor}-Server-x86_64-dvd.iso")
            } else {
                format!("OracleLinux-R{major}-U{minor}-x86_64-dvd.iso")
            };
            (
                format!("https://yum.oracle.com/ISOS/OracleLinux/OL{major}/u{minor}/x86_64/{file}"),
                ImageKind::Iso,
            )
        }
        "parrotsec" => (
            format!(
                "https://download.parrot.sh/parrot/iso/{release}/Parrot-{}-{release}_amd64.iso",
                edition.as_deref().unwrap_or("home")
            ),
            ImageKind::Iso,
        ),
        "porteus" => (
            format!(
                "https://mirrors.dotsrc.org/porteus/x86_64/Porteus-v{release}/Porteus-{}-v{release}-x86_64.iso",
                edition.as_deref().unwrap_or("Xfce")
            ),
            ImageKind::Iso,
        ),
        "proxmox-ve" => (
            format!("https://enterprise.proxmox.com/iso/proxmox-ve_{release}.iso"),
            ImageKind::Iso,
        ),
        "rockylinux" => (
            format!(
                "https://dl.rockylinux.org/vault/rocky/{release}/isos/x86_64/Rocky-{release}-x86_64-{}.iso",
                edition.as_deref().unwrap_or("dvd")
            ),
            ImageKind::Iso,
        ),
        "slackware" => (
            format!(
                "https://slackware.nl/slackware/slackware-iso/slackware64-{release}-iso/slackware64-{release}-install-dvd.iso"
            ),
            ImageKind::Iso,
        ),
        "slint" => (
            format!(
                "https://slackware.uk/slint/x86_64/slint-{}/iso/slint64-{release}.iso",
                release.split('-').next().unwrap_or(release)
            ),
            ImageKind::Iso,
        ),
        "slitaz" => (
            format!("http://mirror.slitaz.org/iso/rolling/slitaz-rolling-{release}.iso"),
            ImageKind::Iso,
        ),
        "tinycore" => {
            let arch = if edition.as_deref().unwrap_or("").contains("Pure") {
                "x86_64"
            } else {
                "x86"
            };
            (
                format!(
                    "http://www.tinycorelinux.net/{release}.x/{arch}/release/{}-{release}.0.iso",
                    edition.as_deref().unwrap_or("Core")
                ),
                ImageKind::Iso,
            )
        }
        "trisquel" => {
            let file = match edition.as_deref().unwrap_or("mate") {
                "lxde" => format!("trisquel-mini_{release}_amd64.iso"),
                "kde" => format!("triskel_{release}_amd64.iso"),
                "sugar" => format!("trisquel-sugar_{release}_amd64.iso"),
                _ => format!("trisquel_{release}_amd64.iso"),
            };
            (
                format!("https://mirrors.ocf.berkeley.edu/trisquel-images/{file}"),
                ImageKind::Iso,
            )
        }
        "ubuntu" | "edubuntu" | "kubuntu" | "lubuntu" | "ubuntu-budgie" | "ubuntucinnamon"
        | "ubuntukylin" | "ubuntu-mate" | "ubuntustudio" | "ubuntu-unity" | "xubuntu" => {
            if release.contains("daily") || release == "dvd" {
                return Err(dynamic_url_error(os));
            }
            let arch = architecture;
            let base = if os == "ubuntu" {
                format!("https://releases.ubuntu.com/{release}")
            } else {
                format!("https://cdimage.ubuntu.com/{os}/releases/{release}/release")
            };
            let file = format!("{os}-{release}-desktop-{arch}.iso");
            (format!("{base}/{file}"), ImageKind::Iso)
        }
        "ubuntu-server" => {
            if release.contains("daily") {
                return Err(dynamic_url_error(os));
            }
            let arch = architecture;
            let base = if architecture == "arm64" {
                format!("https://cdimage.ubuntu.com/releases/{release}/release")
            } else {
                format!("https://releases.ubuntu.com/{release}")
            };
            let name = if release.starts_with("14") || release.starts_with("16") {
                "server"
            } else {
                "live-server"
            };
            (
                format!("{base}/ubuntu-{release}-{name}-{arch}.iso"),
                ImageKind::Iso,
            )
        }
        "void" => {
            let file = match edition.as_deref().unwrap_or("glibc") {
                "musl" => format!("void-live-x86_64-musl-{release}-base.iso"),
                "xfce-glibc" => format!("void-live-x86_64-{release}-xfce.iso"),
                "xfce-musl" => format!("void-live-x86_64-musl-{release}-xfce.iso"),
                _ => format!("void-live-x86_64-{release}-base.iso"),
            };
            (
                format!("https://repo-default.voidlinux.org/live/{release}/{file}"),
                ImageKind::Iso,
            )
        }
        "reactos" => (
            "https://sourceforge.net/projects/reactos/files/latest/download".to_string(),
            ImageKind::Archive,
        ),
        _ => return Err(dynamic_url_error(os)),
    };
    let file_name = file_name_from_url(&url).unwrap_or_else(|| {
        let suffix = match kind {
            ImageKind::Archive => "zip",
            ImageKind::Img => "img",
            ImageKind::Disk => "qcow2",
            ImageKind::Iso => "iso",
        };
        format!("{os}-{release}.{suffix}")
    });
    Ok(ResolvedImage {
        os: os.to_string(),
        release: release.to_string(),
        edition,
        architecture: architecture.to_string(),
        url,
        file_name,
        kind,
        checksum: None,
    })
}
