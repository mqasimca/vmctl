#[derive(Debug, Clone, Copy)]
pub(super) struct OsInfo {
    pub id: &'static str,
    pub name: &'static str,
    pub homepage: &'static str,
    pub guest_os: &'static str,
    pub architectures: &'static str,
    pub releases: &'static str,
    pub editions: &'static str,
}

pub(super) const FREEBSD_ISO_IMAGES: &str =
    "https://download.freebsd.org/releases/amd64/amd64/ISO-IMAGES/";

macro_rules! os {
    ($id:literal, $name:literal, $homepage:literal, $guest:literal, $arch:literal, $releases:literal, $editions:literal) => {
        OsInfo {
            id: $id,
            name: $name,
            homepage: $homepage,
            guest_os: $guest,
            architectures: $arch,
            releases: $releases,
            editions: $editions,
        }
    };
}

pub(super) static OS_CATALOG: &[OsInfo] = &[
    os!(
        "alma",
        "AlmaLinux",
        "https://almalinux.org/",
        "linux",
        "amd64 arm64",
        "9 8",
        "boot minimal dvd"
    ),
    os!(
        "alpine",
        "Alpine Linux",
        "https://alpinelinux.org/",
        "linux",
        "amd64 arm64",
        "dynamic",
        ""
    ),
    os!(
        "android",
        "Android x86",
        "https://www.android-x86.org/",
        "linux",
        "amd64",
        "9.0 8.1 7.1",
        "x86_64 x86"
    ),
    os!(
        "antix",
        "antiX",
        "https://antixlinux.com/",
        "linux",
        "amd64",
        "23.1 23 22 21",
        "net-sysv core-sysv base-sysv full-sysv net-runit core-runit base-runit full-runit"
    ),
    os!(
        "archcraft",
        "Archcraft",
        "https://archcraft.io/",
        "linux",
        "amd64",
        "latest",
        ""
    ),
    os!(
        "archlinux",
        "Arch Linux",
        "https://archlinux.org/",
        "linux",
        "amd64",
        "latest",
        ""
    ),
    os!(
        "artixlinux",
        "Artix Linux",
        "https://artixlinux.org/",
        "linux",
        "amd64",
        "dynamic",
        "dynamic"
    ),
    os!(
        "azurelinux",
        "Azure Linux",
        "https://github.com/microsoft/azurelinux",
        "linux",
        "amd64 arm64",
        "3.0",
        ""
    ),
    os!(
        "batocera",
        "Batocera",
        "https://batocera.org/",
        "batocera",
        "amd64",
        "dynamic",
        ""
    ),
    os!(
        "bazzite",
        "Bazzite",
        "https://github.com/ublue-os/bazzite/",
        "linux",
        "amd64",
        "latest",
        "gnome plasma deck-gnome deck-plasma"
    ),
    os!(
        "biglinux",
        "BigLinux",
        "https://www.biglinux.com.br/",
        "linux",
        "amd64",
        "dynamic",
        "dynamic"
    ),
    os!(
        "blendos",
        "BlendOS",
        "https://blendos.co/",
        "linux",
        "amd64",
        "latest",
        ""
    ),
    os!(
        "bodhi",
        "Bodhi Linux",
        "https://www.bodhilinux.com/",
        "linux",
        "amd64",
        "7.0.0",
        "standard hwe s76"
    ),
    os!(
        "bunsenlabs",
        "BunsenLabs",
        "https://www.bunsenlabs.org/",
        "linux",
        "amd64",
        "boron",
        ""
    ),
    os!(
        "cachyos",
        "CachyOS",
        "https://cachyos.org/",
        "linux",
        "amd64",
        "latest",
        "desktop handheld"
    ),
    os!(
        "centos-stream",
        "CentOS Stream",
        "https://www.centos.org/centos-stream/",
        "linux",
        "amd64",
        "dynamic",
        "boot dvd1"
    ),
    os!(
        "chimeralinux",
        "Chimera Linux",
        "https://chimera-linux.org/",
        "linux",
        "amd64",
        "latest",
        "base gnome"
    ),
    os!(
        "crunchbang++",
        "Crunchbangplusplus",
        "https://www.crunchbangplusplus.org/",
        "linux",
        "amd64",
        "dynamic",
        ""
    ),
    os!(
        "debian",
        "Debian",
        "https://www.debian.org/",
        "linux",
        "amd64 arm64",
        "dynamic",
        "standard cinnamon gnome kde lxde lxqt mate xfce netinst"
    ),
    os!(
        "deepin",
        "Deepin",
        "https://www.deepin.org/",
        "linux",
        "amd64",
        "dynamic",
        ""
    ),
    os!(
        "devuan",
        "Devuan",
        "https://www.devuan.org/",
        "linux",
        "amd64",
        "daedalus chimaera",
        ""
    ),
    os!(
        "dragonflybsd",
        "DragonFly BSD",
        "https://www.dragonflybsd.org/",
        "dragonflybsd",
        "amd64",
        "dynamic",
        ""
    ),
    os!(
        "easyos",
        "EasyOS",
        "https://easyos.org/",
        "linux",
        "amd64",
        "dynamic",
        ""
    ),
    os!(
        "edubuntu",
        "Edubuntu",
        "https://www.edubuntu.org/",
        "linux",
        "amd64",
        "dynamic",
        ""
    ),
    os!(
        "elementary",
        "elementary OS",
        "https://elementary.io/",
        "linux",
        "amd64",
        "8.1 8.0 7.1 7.0",
        ""
    ),
    os!(
        "endeavouros",
        "EndeavourOS",
        "https://endeavouros.com/",
        "linux",
        "amd64",
        "dynamic",
        ""
    ),
    os!(
        "endless",
        "Endless OS",
        "https://www.endlessos.org/os",
        "linux",
        "amd64",
        "6.0.4",
        "base en fr pt_BR es"
    ),
    os!(
        "fedora",
        "Fedora",
        "https://www.fedoraproject.org/",
        "linux",
        "amd64 arm64",
        "dynamic",
        "Server Kinoite Onyx Silverblue Sericea Workstation KDE"
    ),
    os!(
        "freebsd",
        "FreeBSD",
        "https://www.freebsd.org/",
        "freebsd",
        "amd64",
        "dynamic",
        "disc1 dvd1"
    ),
    os!(
        "freedos",
        "FreeDOS",
        "https://freedos.org/",
        "freedos",
        "amd64",
        "1.4 1.3 1.2",
        ""
    ),
    os!(
        "garuda",
        "Garuda Linux",
        "https://garudalinux.org/",
        "linux",
        "amd64",
        "latest",
        "cinnamon dr460nized dr460nized-gaming gnome hyprland i3 kde-lite mokka sway xfce"
    ),
    os!(
        "gentoo",
        "Gentoo",
        "https://www.gentoo.org/",
        "linux",
        "amd64",
        "latest",
        "minimal livegui"
    ),
    os!(
        "ghostbsd",
        "GhostBSD",
        "https://www.ghostbsd.org/",
        "freebsd",
        "amd64",
        "dynamic",
        "mate xfce"
    ),
    os!(
        "gnomeos",
        "GNOME OS",
        "https://os.gnome.org/",
        "linux",
        "amd64",
        "nightly dynamic",
        ""
    ),
    os!(
        "guix",
        "Guix",
        "https://guix.gnu.org/",
        "linux",
        "amd64",
        "1.5.0 1.4.0",
        ""
    ),
    os!(
        "haiku",
        "Haiku",
        "https://www.haiku-os.org/",
        "haiku",
        "amd64",
        "r1beta5 r1beta4 r1beta3",
        "x86_64 x86_gcc2h"
    ),
    os!(
        "kali",
        "Kali Linux",
        "https://www.kali.org/",
        "linux",
        "amd64",
        "current kali-weekly",
        ""
    ),
    os!(
        "kdeneon",
        "KDE neon",
        "https://neon.kde.org/",
        "linux",
        "amd64",
        "user testing unstable",
        "bigscreen desktop dev ko mobile"
    ),
    os!(
        "kdelinux",
        "KDE Linux",
        "https://kde.org/linux/",
        "linux",
        "amd64",
        "latest",
        ""
    ),
    os!(
        "kolibrios",
        "KolibriOS",
        "https://kolibrios.org/en/",
        "kolibrios",
        "amd64",
        "latest",
        "en_US ru_RU es_ES"
    ),
    os!(
        "kubuntu",
        "Kubuntu",
        "https://kubuntu.org/",
        "linux",
        "amd64",
        "dynamic",
        ""
    ),
    os!(
        "linuxlite",
        "Linux Lite",
        "https://www.linuxliteos.com/",
        "linux",
        "amd64",
        "6.6 6.4 6.2 6.0",
        ""
    ),
    os!(
        "linuxmint",
        "Linux Mint",
        "https://linuxmint.com/",
        "linux",
        "amd64",
        "22.1 22 21.3 21.2 21.1 21 20.3 20.2",
        "cinnamon mate xfce"
    ),
    os!(
        "lmde",
        "Linux Mint Debian Edition",
        "https://www.linuxmint.com/download_lmde.php",
        "linux",
        "amd64",
        "6",
        "cinnamon"
    ),
    os!(
        "lubuntu",
        "Lubuntu",
        "https://lubuntu.me/",
        "linux",
        "amd64",
        "dynamic",
        ""
    ),
    os!(
        "maboxlinux",
        "Mabox Linux",
        "https://maboxlinux.org/",
        "linux",
        "amd64",
        "latest",
        ""
    ),
    os!(
        "macos",
        "macOS",
        "https://www.apple.com/macos/",
        "macos",
        "amd64",
        "mojave catalina big-sur monterey ventura sonoma sequoia tahoe",
        ""
    ),
    os!(
        "mageia",
        "Mageia",
        "https://www.mageia.org/",
        "linux",
        "amd64",
        "9",
        "Plasma GNOME Xfce"
    ),
    os!(
        "manjaro",
        "Manjaro",
        "https://manjaro.org/",
        "linux",
        "amd64",
        "xfce gnome plasma cinnamon i3 sway",
        "full minimal"
    ),
    os!(
        "mxlinux",
        "MX Linux",
        "https://mxlinux.org/",
        "linux",
        "amd64",
        "dynamic",
        "Xfce KDE Fluxbox"
    ),
    os!(
        "netboot",
        "netboot.xyz",
        "https://netboot.xyz/",
        "linux",
        "amd64",
        "latest",
        ""
    ),
    os!(
        "netbsd",
        "NetBSD",
        "https://www.netbsd.org/",
        "netbsd",
        "amd64",
        "dynamic",
        ""
    ),
    os!(
        "nitrux",
        "Nitrux",
        "https://nxos.org/",
        "linux",
        "amd64",
        "latest",
        ""
    ),
    os!(
        "nixos",
        "NixOS",
        "https://nixos.org/",
        "linux",
        "amd64",
        "unstable dynamic",
        "minimal graphical"
    ),
    os!(
        "nwg-shell",
        "nwg-shell",
        "https://nwg-piotr.github.io/nwg-shell/",
        "linux",
        "amd64",
        "dynamic",
        ""
    ),
    os!(
        "openbsd",
        "OpenBSD",
        "https://www.openbsd.org/",
        "openbsd",
        "amd64",
        "dynamic",
        ""
    ),
    os!(
        "openindiana",
        "OpenIndiana",
        "https://www.openindiana.org/",
        "solaris",
        "amd64",
        "dynamic",
        "gui text minimal"
    ),
    os!(
        "opensuse",
        "openSUSE",
        "https://www.opensuse.org/",
        "linux",
        "amd64",
        "dynamic",
        ""
    ),
    os!(
        "oraclelinux",
        "Oracle Linux",
        "https://www.oracle.com/linux/",
        "linux",
        "amd64",
        "9.3 9.2 9.1 9.0 8.9 8.8 8.7 8.6 8.5 8.4 7.9 7.8 7.7",
        ""
    ),
    os!(
        "parrotsec",
        "Parrot Security",
        "https://www.parrotsec.org/",
        "linux",
        "amd64",
        "dynamic",
        "home security"
    ),
    os!(
        "pclinuxos",
        "PCLinuxOS",
        "https://www.pclinuxos.com/",
        "linux",
        "amd64",
        "latest",
        "kde kde-darkstar mate xfce"
    ),
    os!(
        "peppermint",
        "PeppermintOS",
        "https://peppermintos.com/",
        "linux",
        "amd64",
        "latest",
        "devuan-xfce devuan-gnome debian-xfce debian-gnome"
    ),
    os!(
        "popos",
        "Pop!_OS",
        "https://pop.system76.com/",
        "linux",
        "amd64",
        "22.04 20.04 24.04",
        "intel nvidia"
    ),
    os!(
        "porteus",
        "Porteus",
        "http://www.porteus.org/",
        "linux",
        "amd64",
        "5.01",
        "cinnamon gnome kde lxde lxqt mate openbox xfce"
    ),
    os!(
        "primtux",
        "PrimTux",
        "https://primtux.fr/",
        "linux",
        "amd64",
        "7",
        "2022-10"
    ),
    os!(
        "proxmox-ve",
        "Proxmox VE",
        "https://proxmox.com/en/proxmox-virtual-environment/",
        "linux",
        "amd64",
        "dynamic",
        ""
    ),
    os!(
        "pureos",
        "PureOS",
        "https://www.pureos.net/",
        "linux",
        "amd64",
        "dynamic",
        "gnome plasma"
    ),
    os!(
        "reactos",
        "ReactOS",
        "https://reactos.org/",
        "reactos",
        "amd64",
        "latest",
        ""
    ),
    os!(
        "rebornos",
        "RebornOS",
        "https://rebornos.org/",
        "linux",
        "amd64",
        "latest",
        ""
    ),
    os!(
        "rockylinux",
        "Rocky Linux",
        "https://rockylinux.org/",
        "linux",
        "amd64",
        "dynamic",
        "minimal dvd boot"
    ),
    os!(
        "siduction",
        "siduction",
        "https://siduction.org/",
        "linux",
        "amd64",
        "latest",
        "dynamic"
    ),
    os!(
        "slackware",
        "Slackware",
        "http://www.slackware.com/",
        "linux",
        "amd64",
        "dynamic",
        ""
    ),
    os!(
        "slax",
        "Slax",
        "https://www.slax.org/",
        "linux",
        "amd64",
        "latest",
        "debian slackware"
    ),
    os!(
        "slint",
        "Slint",
        "https://slint.fr/",
        "linux",
        "amd64",
        "15.0-10",
        ""
    ),
    os!(
        "slitaz",
        "SliTaz",
        "https://www.slitaz.org/en/",
        "linux",
        "amd64",
        "dynamic",
        ""
    ),
    os!(
        "solus",
        "Solus",
        "https://getsol.us/",
        "linux",
        "amd64",
        "dynamic",
        "dynamic"
    ),
    os!(
        "sparkylinux",
        "SparkyLinux",
        "https://sparkylinux.org/",
        "linux",
        "amd64",
        "dynamic",
        "dynamic"
    ),
    os!(
        "spirallinux",
        "SpiralLinux",
        "https://spirallinux.github.io/",
        "linux",
        "amd64",
        "latest",
        "Plasma XFCE Mate LXQt Gnome Budgie Cinnamon Builder"
    ),
    os!(
        "tails",
        "Tails",
        "https://tails.net/",
        "linux",
        "amd64",
        "stable",
        ""
    ),
    os!(
        "tinycore",
        "Tiny Core Linux",
        "http://www.tinycorelinux.net/",
        "linux",
        "amd64",
        "15 14",
        "Core TinyCore CorePlus CorePure64 TinyCorePure64"
    ),
    os!(
        "trisquel",
        "Trisquel",
        "https://trisquel.info/",
        "linux",
        "amd64",
        "11.0 10.0.1",
        "mate lxde kde sugar"
    ),
    os!(
        "tuxedo-os",
        "Tuxedo OS",
        "https://www.tuxedocomputers.com/en/",
        "linux",
        "amd64",
        "current",
        ""
    ),
    os!(
        "ubuntu",
        "Ubuntu",
        "https://ubuntu.com/",
        "linux",
        "amd64 arm64",
        "dynamic",
        ""
    ),
    os!(
        "ubuntu-budgie",
        "Ubuntu Budgie",
        "https://ubuntubudgie.org/",
        "linux",
        "amd64",
        "dynamic",
        ""
    ),
    os!(
        "ubuntucinnamon",
        "Ubuntu Cinnamon",
        "https://ubuntucinnamon.org/",
        "linux",
        "amd64",
        "dynamic",
        ""
    ),
    os!(
        "ubuntukylin",
        "Ubuntu Kylin",
        "https://ubuntukylin.com/",
        "linux",
        "amd64",
        "dynamic",
        ""
    ),
    os!(
        "ubuntu-mate",
        "Ubuntu MATE",
        "https://ubuntu-mate.org/",
        "linux",
        "amd64",
        "dynamic",
        ""
    ),
    os!(
        "ubuntu-server",
        "Ubuntu Server",
        "https://ubuntu.com/server",
        "linux",
        "amd64 arm64",
        "dynamic",
        ""
    ),
    os!(
        "ubuntustudio",
        "Ubuntu Studio",
        "https://ubuntustudio.org/",
        "linux",
        "amd64",
        "dynamic",
        ""
    ),
    os!(
        "ubuntu-unity",
        "Ubuntu Unity",
        "https://ubuntuunity.org/",
        "linux",
        "amd64",
        "dynamic",
        ""
    ),
    os!(
        "vanillaos",
        "Vanilla OS",
        "https://vanillaos.org/",
        "linux",
        "amd64",
        "dynamic",
        ""
    ),
    os!(
        "void",
        "Void Linux",
        "https://voidlinux.org/",
        "linux",
        "amd64",
        "dynamic",
        "glibc musl xfce-glibc xfce-musl"
    ),
    os!(
        "windows",
        "Windows",
        "https://www.microsoft.com/en-us/windows/",
        "windows",
        "amd64",
        "11 10",
        "English International"
    ),
    os!(
        "windows-server",
        "Windows Server",
        "https://www.microsoft.com/en-us/windows-server/",
        "windows-server",
        "amd64",
        "2022 2019 2016",
        "English International"
    ),
    os!(
        "xubuntu",
        "Xubuntu",
        "https://xubuntu.org/",
        "linux",
        "amd64",
        "dynamic",
        ""
    ),
    os!(
        "zorin",
        "Zorin OS",
        "https://zorin.com/os/",
        "linux",
        "amd64",
        "18 17 16",
        "core64 lite64 education64"
    ),
];
