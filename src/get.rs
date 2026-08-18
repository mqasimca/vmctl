use std::env;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use crate::cli::{GetArgs, OutputFormat};
use crate::error::{Error, Result};
use crate::paths::Dirs;

#[derive(Debug, Clone, Copy)]
pub struct OsInfo {
    pub id: &'static str,
    pub name: &'static str,
    pub homepage: &'static str,
    pub guest_os: &'static str,
    pub architectures: &'static str,
    pub releases: &'static str,
    pub editions: &'static str,
}

const FREEBSD_ISO_IMAGES: &str = "https://download.freebsd.org/releases/amd64/amd64/ISO-IMAGES/";

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

static OS_CATALOG: &[OsInfo] = &[
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Operation {
    List,
    ListCsv,
    ListJson,
    Version,
    Show,
    Homepage,
    Url,
    Check { all_architectures: bool },
    Download,
    CreateConfig,
    CreateVm,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ImageKind {
    Iso,
    Img,
    Disk,
    Archive,
}

#[derive(Debug, Clone)]
struct ResolvedImage {
    os: String,
    release: String,
    edition: Option<String>,
    architecture: String,
    url: String,
    file_name: String,
    kind: ImageKind,
    checksum: Option<String>,
}

pub fn run(args: &GetArgs, dirs: &Dirs, output: OutputFormat) -> Result<()> {
    let mut args = args.clone();
    let insecure_flag = args.insecure;
    args.insecure |= env::var("VMCTL_INSECURE").is_ok_and(|value| value == "1");
    let operation = select_operation(&args)?;
    validate_operation_arguments(&args, operation, insecure_flag)?;
    if args.insecure
        && output != OutputFormat::Json
        && matches!(
            operation,
            Operation::Check { .. }
                | Operation::Download
                | Operation::CreateConfig
                | Operation::CreateVm
        )
    {
        eprintln!(
            "vmctl: warning: --insecure disables TLS certificate verification for this get operation"
        );
    }
    match operation {
        Operation::List => list_human(&args, output),
        Operation::ListCsv => list_csv(output),
        Operation::ListJson => list_json(),
        Operation::Version => print_version(output),
        Operation::Show => show(&args, output),
        Operation::Homepage => open_homepage(&args, output),
        Operation::Url => print_images(&args, output),
        Operation::Check { all_architectures } => check_images(&args, all_architectures, output),
        Operation::Download => download_image(&args, dirs, false, output),
        Operation::CreateConfig => create_custom_config(&args, dirs, output),
        Operation::CreateVm => download_image(&args, dirs, true, output),
    }
}

fn validate_operation_arguments(
    args: &GetArgs,
    operation: Operation,
    insecure_flag: bool,
) -> Result<()> {
    if args.arch.is_some()
        && !matches!(
            operation,
            Operation::Url | Operation::Check { .. } | Operation::Download | Operation::CreateVm
        )
    {
        return Err(Error::invalid_argument(
            "--arch",
            "only URL, check, download, and VM creation operations accept it",
        ));
    }
    if insecure_flag
        && !matches!(
            operation,
            Operation::Check { .. }
                | Operation::Download
                | Operation::CreateConfig
                | Operation::CreateVm
        )
    {
        return Err(Error::invalid_argument(
            "--insecure",
            "only network checks and image/config creation operations accept it",
        ));
    }
    if args.disable_unattended
        && !matches!(operation, Operation::CreateConfig | Operation::CreateVm)
    {
        return Err(Error::invalid_argument(
            "--disable-unattended",
            "only VM/config creation operations accept it",
        ));
    }
    if matches!(
        operation,
        Operation::ListCsv | Operation::ListJson | Operation::Version
    ) && (args.os.is_some()
        || args.release_or_input.is_some()
        || args.edition_or_language.is_some())
    {
        return Err(Error::message(format!(
            "get {} does not take positional arguments",
            match operation {
                Operation::ListCsv => "--list-csv",
                Operation::ListJson => "--list-json",
                Operation::Version => "--version",
                _ => unreachable!(),
            }
        )));
    }
    Ok(())
}

fn select_operation(args: &GetArgs) -> Result<Operation> {
    let flags = [
        (args.list, Operation::List),
        (args.list_csv, Operation::ListCsv),
        (args.list_json, Operation::ListJson),
        (args.version, Operation::Version),
        (args.show, Operation::Show),
        (args.open_homepage, Operation::Homepage),
        (args.url, Operation::Url),
        (
            args.check || args.check_all_arch,
            Operation::Check {
                all_architectures: args.check_all_arch,
            },
        ),
        (args.download, Operation::Download),
        (args.create_config, Operation::CreateConfig),
    ];
    let mut selected = flags.iter().filter(|(set, _)| *set).map(|(_, op)| *op);
    let Some(operation) = selected.next() else {
        if args.release_or_input.is_none() && args.edition_or_language.is_none() {
            match args.os.as_deref() {
                Some("list") => return Ok(Operation::List),
                Some("list_csv") => return Ok(Operation::ListCsv),
                Some("list_json") => return Ok(Operation::ListJson),
                _ => {}
            }
        }
        return if args.os.is_some() {
            if args.release_or_input.is_none() && args.edition_or_language.is_none() {
                Ok(Operation::Show)
            } else {
                Ok(Operation::CreateVm)
            }
        } else {
            Ok(Operation::List)
        };
    };
    if selected.next().is_some() {
        return Err(Error::message("get accepts one operation flag at a time"));
    }
    Ok(operation)
}

fn list_human(args: &GetArgs, output: OutputFormat) -> Result<()> {
    if args.os.is_some() || args.release_or_input.is_some() || args.edition_or_language.is_some() {
        return Err(Error::message("--list does not take positional arguments"));
    }
    if output == OutputFormat::Json {
        return list_json();
    }
    for info in OS_CATALOG {
        println!("{}", info.id);
    }
    Ok(())
}

fn list_csv(output: OutputFormat) -> Result<()> {
    if output == OutputFormat::Json {
        return list_json();
    }
    println!("Display Name,OS,Release,Option,Homepage,Architecture");
    for info in OS_CATALOG {
        println!(
            "{},{},{},{},{},{}",
            csv_field(info.name),
            info.id,
            csv_field(info.releases),
            csv_field(info.editions),
            info.homepage,
            info.architectures.replace(' ', "|")
        );
    }
    Ok(())
}

fn list_json() -> Result<()> {
    let values: Vec<Value> = OS_CATALOG.iter().map(info_json).collect();
    println!(
        "{}",
        serde_json::to_string_pretty(&values).unwrap_or_default()
    );
    Ok(())
}

fn print_version(output: OutputFormat) -> Result<()> {
    if output == OutputFormat::Json {
        println!("{}", json!({"version": env!("CARGO_PKG_VERSION")}));
    } else {
        println!("{}", env!("CARGO_PKG_VERSION"));
    }
    Ok(())
}

fn show(args: &GetArgs, output: OutputFormat) -> Result<()> {
    if args.release_or_input.is_some() || args.edition_or_language.is_some() {
        return Err(Error::message("--show accepts only an optional OS"));
    }
    let Some(os) = args.os.as_deref() else {
        return if output == OutputFormat::Json {
            list_json()
        } else {
            for info in OS_CATALOG {
                print_info(info, None);
            }
            Ok(())
        };
    };
    let info = find_os(os)?;
    let releases = (info.id == "freebsd").then(freebsd_releases).transpose()?;
    if output == OutputFormat::Json {
        let mut value = info_json(&info);
        if let Some(releases) = &releases {
            value["releases"] = json!(releases);
        }
        println!(
            "{}",
            serde_json::to_string_pretty(&value).unwrap_or_default()
        );
    } else {
        print_info(&info, releases.as_deref());
        if info.id == "freebsd" {
            println!("  use:           vmctl get freebsd <RELEASE> <disc1|dvd1>");
        }
    }
    Ok(())
}

fn print_info(info: &OsInfo, releases: Option<&[String]>) {
    println!("{} ({})", info.name, info.id);
    println!("  homepage:      {}", info.homepage);
    println!("  guest OS:      {}", info.guest_os);
    println!("  architectures: {}", info.architectures.replace(' ', ", "));
    println!(
        "  releases:      {}",
        releases.map_or_else(|| info.releases.to_string(), |releases| releases.join(", "))
    );
    if !info.editions.is_empty() {
        println!("  editions:      {}", info.editions);
    }
}

fn freebsd_releases() -> Result<Vec<String>> {
    let listing = fetch_text("https://download.freebsd.org/releases/amd64/amd64/ISO-IMAGES/")
        .map_err(|error| {
            Error::message(format!(
                "could not list current FreeBSD releases: {error}; retry later or specify a release, for example: vmctl get freebsd 15.1"
            ))
        })?;
    let mut releases = Vec::new();
    for release in freebsd_releases_from_listing(&listing) {
        let listing = fetch_text(&format!("{FREEBSD_ISO_IMAGES}{release}/")).map_err(|error| {
            Error::message(format!(
                "could not inspect FreeBSD {release} media: {error}; retry later or specify a release, for example: vmctl get freebsd 15.1 disc1"
            ))
        })?;
        if freebsd_release_is_available(&release, &listing) {
            releases.push(release);
        }
    }
    if releases.is_empty() {
        return Err(Error::message(
            "FreeBSD release listing contained no current RELEASE images; retry later or specify a release, for example: vmctl get freebsd 15.1",
        ));
    }
    Ok(releases)
}

fn freebsd_releases_from_listing(listing: &str) -> Vec<String> {
    let listing = listing.to_ascii_lowercase();
    let mut releases = Vec::new();
    let mut offset = 0;
    while let Some(index) = listing[offset..].find("href") {
        let after = offset + index + "href".len();
        offset = after;
        let value = listing[after..].trim_start();
        let Some(value) = value.strip_prefix('=').map(str::trim_start) else {
            continue;
        };
        let value = match value.chars().next() {
            Some(quote @ ('\'' | '"')) => value[1..].split(quote).next().unwrap_or_default(),
            _ => value
                .split(|character: char| character.is_whitespace() || character == '>')
                .next()
                .unwrap_or_default(),
        };
        let Some(release) = value.strip_suffix('/') else {
            continue;
        };
        if release.split('.').all(|part| {
            !part.is_empty() && part.chars().all(|character| character.is_ascii_digit())
        }) && !releases.iter().any(|value| value == release)
        {
            releases.push(release.to_string());
        }
    }
    releases
}

fn freebsd_release_is_available(release: &str, listing: &str) -> bool {
    ["disc1", "dvd1"]
        .iter()
        .all(|edition| listing.contains(&format!("FreeBSD-{release}-RELEASE-amd64-{edition}.iso")))
}

fn open_homepage(args: &GetArgs, output: OutputFormat) -> Result<()> {
    let Some(os) = args.os.as_deref() else {
        return Err(Error::message("--open-homepage requires an OS"));
    };
    if args.release_or_input.is_some() || args.edition_or_language.is_some() {
        return Err(Error::message("--open-homepage accepts only an OS"));
    }
    let info = find_os(os)?;
    let command = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(target_os = "windows") {
        "start"
    } else {
        "xdg-open"
    };
    Command::new(command)
        .arg(info.homepage)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| Error::command_unavailable(command, error))?;
    if output == OutputFormat::Json {
        println!(
            "{}",
            json!({"os": info.id, "homepage": info.homepage, "opened": true})
        );
    } else {
        println!("Opened {}", info.homepage);
    }
    Ok(())
}

fn print_images(args: &GetArgs, output: OutputFormat) -> Result<()> {
    let os = required_arg(args.os.as_deref(), "OS")?;
    let release = required_arg(args.release_or_input.as_deref(), "RELEASE")?;
    for architecture in requested_architectures(args, os)? {
        let image = resolve_remote_image(
            os,
            release,
            args.edition_or_language.as_deref(),
            &architecture,
        )?;
        print_image(&image, output, None);
    }
    Ok(())
}

fn check_images(args: &GetArgs, all_architectures: bool, output: OutputFormat) -> Result<()> {
    let os = required_arg(args.os.as_deref(), "OS")?;
    let release = required_arg(args.release_or_input.as_deref(), "RELEASE")?;
    let architectures = if all_architectures {
        vec!["amd64".to_string(), "arm64".to_string()]
    } else {
        requested_architectures(args, os)?
    };
    let mut json_results = Vec::new();
    let mut first_failure: Option<(String, String)> = None;
    for architecture in architectures {
        let image = match resolve_remote_image(
            os,
            release,
            args.edition_or_language.as_deref(),
            &architecture,
        ) {
            Ok(image) => image,
            Err(error) if all_architectures => {
                if first_failure.is_none() {
                    first_failure = Some((architecture.clone(), error.to_string()));
                }
                if output == OutputFormat::Json {
                    json_results.push(check_result_json(
                        os,
                        release,
                        args.edition_or_language.as_deref(),
                        &architecture,
                        false,
                        Some(error.to_string()),
                    ));
                } else {
                    print_check_result(
                        os,
                        release,
                        args.edition_or_language.as_deref(),
                        &architecture,
                        false,
                        &error,
                    );
                }
                continue;
            }
            Err(error) => return Err(error),
        };
        let available = if find_os(os)?.id == "macos" {
            let recovery = fetch_macos_recovery(release)?;
            let headers = vec![
                "Host: oscdn.apple.com".to_string(),
                "Connection: close".to_string(),
                "User-Agent: InternetRecovery/1.0".to_string(),
                format!("Cookie: AssetToken={}", recovery.asset_token),
            ];
            url_available_with_headers(&image.url, &headers, args.insecure)?
        } else {
            url_available(&image.url, args.insecure)?
        };
        if !available && first_failure.is_none() {
            first_failure = Some((architecture.clone(), "image URL is unavailable".to_string()));
        }
        if output == OutputFormat::Json {
            json_results.push(check_result_json(
                os,
                release,
                image.edition.as_deref(),
                &architecture,
                available,
                (!available).then(|| "image URL is unavailable".to_string()),
            ));
        } else {
            print_check_result(
                os,
                release,
                image.edition.as_deref(),
                &architecture,
                available,
                &Error::message("image URL is unavailable"),
            );
        }
    }
    if let Some((architecture, cause)) = first_failure {
        return Err(Error::image_unavailable(os, release, &architecture, cause));
    }
    if output == OutputFormat::Json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json_results).unwrap_or_default()
        );
    }
    Ok(())
}

fn download_image(
    args: &GetArgs,
    dirs: &Dirs,
    create_config: bool,
    output: OutputFormat,
) -> Result<()> {
    let os = find_os(required_arg(args.os.as_deref(), "OS")?).map(|info| info.id)?;
    let release = required_arg(args.release_or_input.as_deref(), "RELEASE")?;
    let architecture = requested_architectures(args, os)?
        .into_iter()
        .next()
        .ok_or_else(|| Error::message("an architecture is required"))?;
    if os == "macos" {
        return download_macos(args, dirs, release, &architecture, create_config, output);
    }
    if matches!(os, "windows" | "windows-server") {
        return download_windows(
            args,
            dirs,
            os,
            release,
            &architecture,
            create_config,
            output,
        );
    }
    let image = resolve_remote_image(
        os,
        release,
        args.edition_or_language.as_deref(),
        &architecture,
    )?;
    let name = suggested_name(os, release, image.edition.as_deref(), &architecture);
    validate_vm_name(&name)?;
    let root = if create_config {
        dirs.vm_dir.clone()
    } else {
        env::current_dir().map_err(|error| Error::io("current directory", error))?
    };
    let target_dir = if create_config {
        root.join(&name)
    } else {
        root.clone()
    };
    if create_config {
        let config_path = root.join(format!("{name}.conf"));
        if config_path.exists() {
            return Err(Error::message(format!(
                "configuration already exists: {}",
                config_path.display()
            )));
        }
    }
    fs::create_dir_all(&target_dir).map_err(|error| Error::io(target_dir.display(), error))?;
    let target = target_dir.join(&image.file_name);
    download_file(&image.url, &target, args.insecure)?;
    if let Err(error) = verify_checksum(&target, image.checksum.as_deref()) {
        let _ = fs::remove_file(&target);
        return Err(error);
    }
    let target = if create_config {
        prepare_resolved_image(os, &target)?
    } else {
        target
    };
    let config_path = if create_config {
        Some(write_vm_config(
            &root,
            &name,
            os,
            release,
            image.edition.as_deref(),
            &architecture,
            &target,
        )?)
    } else {
        None
    };
    let result = json!({
        "os": os,
        "release": release,
        "edition": image.edition,
        "architecture": architecture,
        "url": image.url,
        "kind": image_kind_name(image.kind),
        "checksum": image.checksum,
        "image": target,
        "config": config_path,
    });
    if output == OutputFormat::Json {
        println!(
            "{}",
            serde_json::to_string_pretty(&result).unwrap_or_default()
        );
    } else if let Some(config_path) = config_path {
        println!("Downloaded {}", target.display());
        println!("Created {}", config_path.display());
    } else {
        println!("Downloaded {}", target.display());
    }
    Ok(())
}

fn create_custom_config(args: &GetArgs, dirs: &Dirs, output: OutputFormat) -> Result<()> {
    let name = validate_vm_name(required_arg(args.os.as_deref(), "VM name")?)?;
    let input = required_arg(args.release_or_input.as_deref(), "image path or URL")?;
    if args.edition_or_language.is_some() {
        return Err(Error::message(
            "--create-config accepts VM_NAME and IMAGE_PATH_OR_URL",
        ));
    }
    let root = &dirs.vm_dir;
    let config_path = root.join(format!("{name}.conf"));
    if config_path.exists() {
        return Err(Error::message(format!(
            "configuration already exists: {}",
            config_path.display()
        )));
    }
    let vm_dir = root.join(name);
    fs::create_dir_all(root).map_err(|error| Error::io(root.display(), error))?;
    if fs::symlink_metadata(&vm_dir).is_ok() {
        return Err(Error::message(format!(
            "VM data directory already exists: {}",
            vm_dir.display()
        )));
    }
    fs::create_dir(&vm_dir).map_err(|error| Error::io(vm_dir.display(), error))?;
    let source_name = input_file_name(input)?;
    let destination = vm_dir.join(&source_name);
    if input.starts_with("http://") || input.starts_with("https://") {
        download_file(input, &destination, args.insecure)?;
    } else {
        let source = PathBuf::from(input);
        if !source.is_file() {
            return Err(Error::message(format!(
                "image path does not exist: {}",
                source.display()
            )));
        }
        if fs::canonicalize(&source).ok() != fs::canonicalize(&destination).ok() {
            fs::copy(&source, &destination)
                .map_err(|error| Error::io(destination.display(), error))?;
        }
    }
    let image = prepare_image(&destination)?;
    let os = infer_guest_os(&image);
    let (fixed_iso, unattended_iso) =
        if matches!(os, "windows" | "windows-server") && !args.disable_unattended {
            let fixed_iso = download_virtio_iso(&vm_dir, args.insecure)?;
            let unattended_iso = create_unattended_iso(&vm_dir, args.insecure)?;
            (Some(fixed_iso), Some(unattended_iso))
        } else {
            (None, None)
        };
    let config_path = write_vm_config(root, name, os, "custom", None, host_architecture(), &image)?;
    if let Some(fixed_iso) = fixed_iso.as_deref() {
        append_iso(root, &config_path, "fixed_iso", fixed_iso)?;
    }
    if let Some(unattended_iso) = unattended_iso.as_deref() {
        append_iso(root, &config_path, "unattended_iso", unattended_iso)?;
    }
    let result = json!({
        "name": name,
        "guest_os": os,
        "image": image,
        "fixed_iso": fixed_iso,
        "unattended_iso": unattended_iso,
        "config": config_path,
    });
    if output == OutputFormat::Json {
        println!(
            "{}",
            serde_json::to_string_pretty(&result).unwrap_or_default()
        );
    } else {
        println!("Created {}", config_path.display());
    }
    Ok(())
}

fn print_image(image: &ResolvedImage, output: OutputFormat, available: Option<bool>) {
    if output == OutputFormat::Json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "os": image.os,
                "release": image.release,
                "edition": image.edition,
                "architecture": image.architecture,
                "url": image.url,
                "file_name": image.file_name,
                "kind": image_kind_name(image.kind),
                "checksum": image.checksum,
                "available": available,
            }))
            .unwrap_or_default()
        );
    } else if let Some(available) = available {
        println!(
            "{}: {} {}",
            if available { "PASS" } else { "FAIL" },
            image.os,
            image.url
        );
    } else {
        println!("{}", image.url);
    }
}

fn print_check_result(
    os: &str,
    release: &str,
    edition: Option<&str>,
    architecture: &str,
    available: bool,
    error: &Error,
) {
    let suffix = edition.map(|value| format!("-{value}")).unwrap_or_default();
    let detail = if available {
        String::new()
    } else {
        format!(" - image URL unavailable ({error})")
    };
    println!(
        "{}: {}-{}{} ({architecture}){}",
        if available { "PASS" } else { "FAIL" },
        os,
        release,
        suffix,
        detail,
    );
}

fn check_result_json(
    os: &str,
    release: &str,
    edition: Option<&str>,
    architecture: &str,
    available: bool,
    error: Option<String>,
) -> Value {
    json!({
        "os": os,
        "release": release,
        "edition": edition,
        "architecture": architecture,
        "available": available,
        "error": error,
    })
}

fn info_json(info: &OsInfo) -> Value {
    json!({
        "name": info.name,
        "os": info.id,
        "homepage": info.homepage,
        "guest_os": info.guest_os,
        "architectures": info.architectures.split_whitespace().collect::<Vec<_>>(),
        "releases": info.releases.split_whitespace().collect::<Vec<_>>(),
        "editions": info.editions.split_whitespace().collect::<Vec<_>>(),
    })
}

fn resolve_image(
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

fn required_edition(info: OsInfo, edition: Option<&str>) -> Result<Option<String>> {
    if info.editions.is_empty() {
        return Ok(None);
    }
    if matches!(info.id, "windows" | "windows-server") {
        if let Some(edition) = edition
            && edition != info.editions
        {
            return Err(Error::message(format!(
                "{edition} is not a supported {} language (supported: {})",
                info.name, info.editions
            )));
        }
        return Ok(Some(info.editions.to_string()));
    }
    if let Some(edition) = edition {
        if info.editions != "dynamic"
            && !info
                .editions
                .split_whitespace()
                .any(|value| value == edition)
        {
            return Err(Error::message(format!(
                "{edition} is not a supported {} edition (supported: {})",
                info.name, info.editions
            )));
        }
        return Ok(Some(edition.to_string()));
    }
    if info.editions == "dynamic" {
        return Err(Error::message(format!(
            "{} editions are discovered from the upstream provider; specify one explicitly",
            info.name
        )));
    }
    if info.editions.split_whitespace().count() == 1 {
        return Ok(info.editions.split_whitespace().next().map(str::to_string));
    }
    Err(Error::message(format!(
        "{} requires an edition; supported: {}",
        info.name, info.editions
    )))
}

fn resolve_remote_image(
    os: &str,
    release: &str,
    edition: Option<&str>,
    architecture: &str,
) -> Result<ResolvedImage> {
    let os = find_os(os)?.id;
    if is_dynamic_provider(os) {
        return resolve_dynamic_image(os, release, edition, architecture);
    }
    let mut image = resolve_image(os, release, edition, architecture)?;
    if is_ubuntu_family(os) {
        if let Some((url, file_name, checksum)) = ubuntu_asset(os, release, architecture) {
            image.url = url;
            image.file_name = file_name;
            image.checksum = checksum;
        }
    } else if os == "debian"
        && let Some((url, file_name, checksum)) = debian_asset(release, edition, architecture)
    {
        image.url = url;
        image.file_name = file_name;
        image.checksum = checksum;
    }
    Ok(image)
}

fn is_dynamic_provider(os: &str) -> bool {
    matches!(
        os,
        "alpine"
            | "android"
            | "antix"
            | "archcraft"
            | "artixlinux"
            | "azurelinux"
            | "batocera"
            | "bazzite"
            | "biglinux"
            | "blendos"
            | "bodhi"
            | "bunsenlabs"
            | "cachyos"
            | "chimeralinux"
            | "crunchbang++"
            | "easyos"
            | "elementary"
            | "endeavouros"
            | "endless"
            | "fedora"
            | "garuda"
            | "gentoo"
            | "ghostbsd"
            | "gnomeos"
            | "kdeneon"
            | "kdelinux"
            | "kali"
            | "kolibrios"
            | "mageia"
            | "manjaro"
            | "mxlinux"
            | "nitrux"
            | "nwg-shell"
            | "pclinuxos"
            | "peppermint"
            | "primtux"
            | "pureos"
            | "popos"
            | "rebornos"
            | "siduction"
            | "slax"
            | "solus"
            | "sparkylinux"
            | "spirallinux"
            | "tails"
            | "tuxedo-os"
            | "vanillaos"
            | "windows"
            | "windows-server"
            | "macos"
            | "zorin"
    )
}

fn resolve_dynamic_image(
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
            "{} is not available for {}",
            info.name, architecture
        )));
    }
    let edition = required_edition(info, edition)?;
    if os == "fedora" && architecture == "arm64" && edition.as_deref() == Some("Onyx") {
        return Err(Error::message("Fedora Onyx is not available for ARM64"));
    }
    let (url, kind, checksum) = match os {
        "alpine" => alpine_asset(release, architecture)?,
        "antix" => antix_asset(release, edition.as_deref())?,
        "archcraft" => archcraft_asset(release)?,
        "artixlinux" => artixlinux_asset(release, edition.as_deref())?,
        "azurelinux" => azurelinux_asset(release, architecture)?,
        "batocera" => batocera_asset(release)?,
        "bazzite" => bazzite_asset(edition.as_deref())?,
        "biglinux" => biglinux_asset(release, edition.as_deref())?,
        "blendos" => blendos_asset()?,
        "bodhi" => bodhi_asset(release, edition.as_deref())?,
        "bunsenlabs" => bunsenlabs_asset()?,
        "cachyos" => cachyos_asset(edition.as_deref())?,
        "chimeralinux" => chimeralinux_asset(release, edition.as_deref())?,
        "crunchbang++" => crunchbang_asset(release)?,
        "easyos" => easyos_asset(release)?,
        "endeavouros" => endeavouros_asset(release)?,
        "endless" => endless_asset(release, edition.as_deref())?,
        "fedora" => fedora_asset(release, edition.as_deref(), architecture)?,
        "garuda" => garuda_asset(edition.as_deref())?,
        "gentoo" => gentoo_asset(release, edition.as_deref())?,
        "ghostbsd" => ghostbsd_asset(release, edition.as_deref())?,
        "gnomeos" => gnomeos_asset(release)?,
        "kdeneon" => kdeneon_asset(release, edition.as_deref())?,
        "kdelinux" => kdelinux_asset(release)?,
        "kali" => kali_asset(release, architecture)?,
        "kolibrios" => kolibrios_asset(edition.as_deref())?,
        "mageia" => mageia_asset(release, edition.as_deref())?,
        "manjaro" => manjaro_asset(release, edition.as_deref())?,
        "mxlinux" => mxlinux_asset(release, edition.as_deref())?,
        "nitrux" => nitrux_asset()?,
        "nwg-shell" => nwg_shell_asset(release)?,
        "pclinuxos" => pclinuxos_asset(edition.as_deref())?,
        "peppermint" => peppermint_asset(edition.as_deref())?,
        "primtux" => primtux_asset(release, edition.as_deref())?,
        "pureos" => pureos_asset(release, edition.as_deref())?,
        "popos" => popos_asset(release, edition.as_deref())?,
        "rebornos" => rebornos_asset()?,
        "slax" => slax_asset(edition.as_deref())?,
        "solus" => solus_asset(release, edition.as_deref())?,
        "sparkylinux" => sparkylinux_asset(release, edition.as_deref())?,
        "spirallinux" => spirallinux_asset(edition.as_deref())?,
        "tails" => tails_asset(release)?,
        "tuxedo-os" => tuxedo_asset()?,
        "vanillaos" => vanillaos_asset(release)?,
        "zorin" => zorin_asset(release, edition.as_deref())?,
        "android" => android_asset(release, edition.as_deref())?,
        "elementary" => elementary_asset(release)?,
        "macos" => macos_asset(release, architecture)?,
        "windows" | "windows-server" => windows_asset(os, release, edition.as_deref())?,
        "siduction" => siduction_asset(edition.as_deref())?,
        _ => return Err(dynamic_url_error(os)),
    };
    let file_name = file_name_from_url(&url).unwrap_or_else(|| format!("{os}-{release}.iso"));
    Ok(ResolvedImage {
        os: os.to_string(),
        release: release.to_string(),
        edition,
        architecture: architecture.to_string(),
        url,
        file_name,
        kind,
        checksum,
    })
}

fn fetch_redirect(url: &str) -> Result<String> {
    let output = Command::new("curl")
        .args([
            "--disable",
            "--fail",
            "--location",
            "--silent",
            "--show-error",
            "--head",
            "--output",
            "/dev/null",
            "--write-out",
            "%{url_effective}",
            "--",
        ])
        .arg(url)
        .output()
        .map_err(|error| Error::command_unavailable("curl", error))?;
    if !output.status.success() {
        return Err(Error::command_failed_status("curl", output.status));
    }
    let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!value.is_empty())
        .then_some(value)
        .ok_or_else(|| Error::message(format!("provider returned no redirect for {url}")))
}

fn sourceforge_asset(project: &str, path: &str) -> Result<String> {
    fetch_redirect(&format!(
        "https://sourceforge.net/projects/{project}/files/{path}/download"
    ))
}

fn first_token(text: &str, predicate: impl Fn(&str) -> bool) -> Option<String> {
    text.split(|character: char| {
        character.is_whitespace() || matches!(character, '"' | '\'' | '<' | '>')
    })
    .map(str::trim)
    .find(|value| !value.is_empty() && predicate(value))
    .map(str::to_string)
}

fn checksum_from_text(text: &str, file: &str, algorithm: &str) -> Option<String> {
    let length = match algorithm {
        "sha256" => 64,
        "sha512" => 128,
        _ => return None,
    };
    text.lines()
        .filter(|line| line.contains(file))
        .flat_map(|line| line.split(|character: char| !character.is_ascii_hexdigit()))
        .find(|value| value.len() == length)
        .map(|value| format!("{algorithm}:{value}"))
}

fn checksum_at(url: &str, file: &str, algorithm: &str) -> Option<String> {
    fetch_text(url)
        .ok()
        .and_then(|text| checksum_from_text(&text, file, algorithm))
}

fn alpine_asset(release: &str, architecture: &str) -> Result<(String, ImageKind, Option<String>)> {
    let qemu_arch = qemu_architecture(architecture);
    let base = format!("https://dl-cdn.alpinelinux.org/alpine/{release}/releases/{qemu_arch}");
    let yaml = fetch_text(&format!("{base}/latest-releases.yaml"))?;
    let mut virtual_section = false;
    let mut version = None;
    let mut checksum = None;
    for line in yaml.lines() {
        if line.contains("\"Virtual\"") {
            virtual_section = true;
        } else if virtual_section && line.contains("\"Xen\"") {
            break;
        } else if virtual_section && line.trim_start().starts_with("version:") {
            version = line
                .split_once(':')
                .map(|(_, value)| value.trim().to_string());
        } else if virtual_section && line.trim_start().starts_with("sha256:") {
            checksum = line
                .split_once(':')
                .map(|(_, value)| format!("sha256:{}", value.trim().trim_matches('"')));
        }
    }
    let version = version.ok_or_else(|| dynamic_url_error("alpine"))?;
    let file = format!("alpine-virt-{version}-{qemu_arch}.iso");
    Ok((format!("{base}/{file}"), ImageKind::Iso, checksum))
}

fn antix_asset(
    release: &str,
    edition: Option<&str>,
) -> Result<(String, ImageKind, Option<String>)> {
    let edition = edition.ok_or_else(|| Error::message("antiX requires an edition"))?;
    let mut name = format!("antiX-{release}");
    let mut base = format!("Final/antiX-{release}");
    if edition.contains("runit") {
        name.push_str("-runit");
        if release == "21" {
            base.push_str("/runit-bullseye");
        } else {
            base.push_str(&format!("/runit-antiX-{release}"));
        }
    }
    let suffix = if edition.starts_with("base-") {
        "_x64-base.iso"
    } else if edition.starts_with("core-") {
        "_x64-core.iso"
    } else if edition.starts_with("full-") {
        "_x64-full.iso"
    } else {
        "-net_x64-net.iso"
    };
    name.push_str(suffix);
    let url = sourceforge_asset("antix-linux", &format!("{base}/{name}"))?;
    Ok((url, ImageKind::Iso, None))
}

fn archcraft_asset(release: &str) -> Result<(String, ImageKind, Option<String>)> {
    let url = sourceforge_asset("archcraft", release)?;
    Ok((url, ImageKind::Iso, None))
}

fn artixlinux_asset(
    release: &str,
    edition: Option<&str>,
) -> Result<(String, ImageKind, Option<String>)> {
    let edition = edition.ok_or_else(|| Error::message("Artix Linux requires an edition"))?;
    let base = "https://iso.artixlinux.org/iso";
    let page = fetch_text(&format!("{base}/"))?;
    let prefix = format!("artix-{edition}-");
    let file = first_token(&page, |value| {
        value.starts_with(&prefix)
            && value.ends_with("-x86_64.iso")
            && (release == "latest" || value.contains(release))
    })
    .ok_or_else(|| dynamic_url_error("artixlinux"))?;
    let checksum = checksum_at(&format!("{base}/sha256sums"), &file, "sha256");
    Ok((format!("{base}/{file}"), ImageKind::Iso, checksum))
}

fn azurelinux_asset(
    release: &str,
    architecture: &str,
) -> Result<(String, ImageKind, Option<String>)> {
    let arch = qemu_architecture(architecture);
    let url = fetch_redirect(&format!("https://aka.ms/azurelinux-{release}-{arch}.iso"))?;
    Ok((url, ImageKind::Iso, None))
}

fn batocera_asset(release: &str) -> Result<(String, ImageKind, Option<String>)> {
    let base = format!("https://mirrors.o2switch.fr/batocera/x86_64/stable/{release}");
    let page = fetch_text(&format!("{base}/"))?;
    let file = first_token(&page, |value| {
        value.starts_with("batocera") && value.ends_with("img.gz")
    })
    .ok_or_else(|| dynamic_url_error("batocera"))?;
    Ok((format!("{base}/{file}"), ImageKind::Archive, None))
}

fn bazzite_asset(edition: Option<&str>) -> Result<(String, ImageKind, Option<String>)> {
    let edition = edition.ok_or_else(|| Error::message("Bazzite requires an edition"))?;
    let file = match edition {
        "gnome" => "bazzite-gnome-stable-amd64.iso",
        "plasma" => "bazzite-stable-amd64.iso",
        "deck-gnome" => "bazzite-deck-gnome-stable-amd64.iso",
        "deck-plasma" => "bazzite-deck-stable-amd64.iso",
        _ => return Err(dynamic_url_error("bazzite")),
    };
    let base = "https://download.bazzite.gg";
    let checksum = checksum_at(&format!("{base}/{file}-CHECKSUM"), file, "sha256");
    Ok((format!("{base}/{file}"), ImageKind::Iso, checksum))
}

fn biglinux_asset(
    release: &str,
    edition: Option<&str>,
) -> Result<(String, ImageKind, Option<String>)> {
    let edition = edition.ok_or_else(|| Error::message("BigLinux requires an edition"))?;
    let file = format!("biglinux_{release}_{edition}.iso");
    Ok((
        format!("https://iso.biglinux.com.br/{file}"),
        ImageKind::Iso,
        None,
    ))
}

fn blendos_asset() -> Result<(String, ImageKind, Option<String>)> {
    let base = "https://git.blendos.co/api/v4/projects/32/jobs/artifacts/main/raw";
    let file = "blendOS.iso";
    let checksum = fetch_text(&format!("{base}/checksum?job=build-job"))
        .ok()
        .and_then(|value| {
            value
                .split_whitespace()
                .next()
                .map(|hash| format!("sha256:{hash}"))
        });
    Ok((
        format!("{base}/{file}?job=build-job"),
        ImageKind::Iso,
        checksum,
    ))
}

fn bodhi_asset(
    release: &str,
    edition: Option<&str>,
) -> Result<(String, ImageKind, Option<String>)> {
    let edition = edition.unwrap_or("standard");
    let file = if edition == "standard" {
        format!("bodhi-{release}-64.iso")
    } else {
        format!("bodhi-{release}-64-{edition}.iso")
    };
    let base = format!("https://sourceforge.net/projects/bodhilinux/files/{release}");
    let url = sourceforge_asset("bodhilinux", &format!("{release}/{file}"))?;
    let checksum = checksum_at(&format!("{base}/{file}.sha256"), &file, "sha256");
    Ok((url, ImageKind::Iso, checksum))
}

fn bunsenlabs_asset() -> Result<(String, ImageKind, Option<String>)> {
    let base = "https://ddl.bunsenlabs.org/ddl";
    let sums = fetch_text(&format!("{base}/release.sha256.txt"))?;
    let file = first_token(&sums, |value| value.ends_with(".iso"))
        .ok_or_else(|| dynamic_url_error("bunsenlabs"))?;
    let checksum = checksum_from_text(&sums, &file, "sha256");
    Ok((format!("{base}/{file}"), ImageKind::Iso, checksum))
}

fn cachyos_asset(edition: Option<&str>) -> Result<(String, ImageKind, Option<String>)> {
    let edition = edition.ok_or_else(|| Error::message("CachyOS requires an edition"))?;
    let page = fetch_text("https://cachyos.org/download/")?;
    let file = first_token(&page, |value| {
        value.ends_with(".iso") && value.contains(edition) && !value.ends_with(".sha256")
    })
    .ok_or_else(|| dynamic_url_error("cachyos"))?;
    let url = if file.starts_with("http") {
        file.clone()
    } else {
        format!("https://iso.cachyos.org/{file}")
    };
    let checksum = checksum_at(&format!("{url}.sha256"), &file, "sha256");
    Ok((url, ImageKind::Iso, checksum))
}

fn chimeralinux_asset(
    release: &str,
    edition: Option<&str>,
) -> Result<(String, ImageKind, Option<String>)> {
    let edition = edition.ok_or_else(|| Error::message("Chimera Linux requires an edition"))?;
    let base = format!("https://repo.chimera-linux.org/live/{release}");
    let sums = fetch_text(&format!("{base}/sha256sums.txt"))?;
    let file = first_token(&sums, |value| {
        value.ends_with(".iso") && value.contains("x86_64-LIVE") && value.contains(edition)
    })
    .ok_or_else(|| dynamic_url_error("chimeralinux"))?;
    let checksum = checksum_from_text(&sums, &file, "sha256");
    Ok((format!("{base}/{file}"), ImageKind::Iso, checksum))
}

fn crunchbang_asset(release: &str) -> Result<(String, ImageKind, Option<String>)> {
    let data = fetch_text("https://api.github.com/repos/CBPP/cbpp/releases")?;
    let values: Value = serde_json::from_str(&data)
        .map_err(|error| Error::message(format!("invalid CrunchBang++ release data: {error}")))?;
    let url = values
        .as_array()
        .into_iter()
        .flatten()
        .filter(|entry| {
            release == "latest" || entry.get("tag_name").and_then(Value::as_str) == Some(release)
        })
        .flat_map(|entry| entry.get("assets").and_then(Value::as_array))
        .flatten()
        .find_map(|asset| {
            let name = asset.get("name").and_then(Value::as_str)?;
            (name.contains("amd64") && name.ends_with(".iso"))
                .then(|| asset.get("browser_download_url").and_then(Value::as_str))
                .flatten()
        })
        .ok_or_else(|| dynamic_url_error("crunchbang++"))?;
    Ok((url.to_string(), ImageKind::Iso, None))
}

fn android_asset(
    release: &str,
    edition: Option<&str>,
) -> Result<(String, ImageKind, Option<String>)> {
    let edition = edition.ok_or_else(|| Error::message("Android x86 requires x86 or x86_64"))?;
    let page = fetch_text("https://www.fosshub.com/Android-x86-old.html")?;
    let settings = page
        .split_once("var settings =")
        .and_then(|(_, value)| value.split_once(';').map(|(json, _)| json.trim()))
        .ok_or_else(|| dynamic_url_error("android"))?;
    let values: Value = serde_json::from_str(settings)
        .map_err(|error| Error::message(format!("invalid Android release data: {error}")))?;
    let prefix = format!("android-{edition}-{release}");
    let image = values
        .pointer("/pool/f")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find(|entry| {
            entry
                .get("n")
                .and_then(Value::as_str)
                .is_some_and(|name| name.starts_with(&prefix) && name.ends_with(".iso"))
        })
        .ok_or_else(|| dynamic_url_error("android"))?;
    let file = image
        .get("n")
        .and_then(Value::as_str)
        .ok_or_else(|| dynamic_url_error("android"))?;
    let checksum = image
        .pointer("/hash/sha256")
        .and_then(Value::as_str)
        .map(|hash| format!("sha256:{hash}"));
    let mirror = "https://mirrors.gigenet.com/OSDN/android-x86";
    let directories = fetch_text(mirror)?;
    for directory in directories
        .split(|character: char| !character.is_ascii_digit())
        .filter(|value| value.len() == 5)
    {
        let base = format!("{mirror}/{directory}");
        if fetch_text(&format!("{base}/")).is_ok_and(|listing| listing.contains(file)) {
            return Ok((format!("{base}/{file}"), ImageKind::Iso, checksum));
        }
    }
    Err(dynamic_url_error("android"))
}

fn elementary_asset(release: &str) -> Result<(String, ImageKind, Option<String>)> {
    let stamp = match release {
        "7.0" => ".20230129rc",
        "7.1" => ".20230926rc",
        "8.0" => ".20241122rc",
        "8.1" => "-amd64.20251211",
        _ => return Err(dynamic_url_error("elementary")),
    };
    let file = format!("elementaryos-{release}-stable{stamp}.iso");
    let token = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_secs().to_string())
        .unwrap_or_else(|_| "download".to_string());
    let checksum = (release == "8.1").then(|| {
        "sha256:eee6cad081664717681bec767fbfe1aa1fd920938fedad6c83b41fd341e8f306".to_string()
    });
    Ok((
        format!("https://ams3.dl.elementary.io/download/{token}/{file}"),
        ImageKind::Iso,
        checksum,
    ))
}

fn siduction_asset(edition: Option<&str>) -> Result<(String, ImageKind, Option<String>)> {
    let edition = edition.ok_or_else(|| Error::message("siduction requires an edition"))?;
    let root = fetch_text("https://mirror.math.princeton.edu/pub/siduction/iso/")?;
    let release = first_token(&root, |value| {
        value.starts_with(|character: char| character.is_ascii_digit()) && value.ends_with('/')
    })
    .ok_or_else(|| dynamic_url_error("siduction"))?
    .trim_end_matches('/')
    .to_string();
    let base = format!("https://mirrors.dotsrc.org/siduction/iso/{release}/{edition}");
    let page = fetch_text(&base)?;
    let file = first_token(&page, |value| value.ends_with(".iso"))
        .ok_or_else(|| dynamic_url_error("siduction"))?;
    let checksum = checksum_at(&format!("{base}/{file}.md5"), &file, "sha256");
    Ok((format!("{base}/{file}"), ImageKind::Iso, checksum))
}

#[derive(Debug, Clone)]
struct MacosRecovery {
    url: String,
    asset_token: String,
    chunklist_url: String,
    chunklist_token: String,
}

fn macos_asset(release: &str, architecture: &str) -> Result<(String, ImageKind, Option<String>)> {
    if architecture != "amd64" {
        return Err(Error::message("macOS recovery is only available for amd64"));
    }
    let recovery = fetch_macos_recovery(release)?;
    Ok((recovery.url, ImageKind::Img, None))
}

fn fetch_macos_recovery(release: &str) -> Result<MacosRecovery> {
    let (board_id, mlb, os_type) = match release {
        "mojave" => ("Mac-7BA5B2DFE22DDD8C", "00000000000KXPG00", "default"),
        "catalina" => ("Mac-00BE6ED71E35EB86", "00000000000000000", "default"),
        "big-sur" => ("Mac-2BD1B31983FE1663", "00000000000000000", "default"),
        "monterey" => ("Mac-B809C3757DA9BB8D", "00000000000000000", "latest"),
        "ventura" => ("Mac-4B682C642B45593E", "00000000000000000", "latest"),
        "sonoma" => ("Mac-827FAC58A8FDFA22", "00000000000000000", "default"),
        "sequoia" => ("Mac-7BA5B2D9E42DDD94", "00000000000000000", "default"),
        "tahoe" => ("Mac-CFF7D910A743CAAF", "00000000000000000", "latest"),
        _ => {
            return Err(Error::message(format!(
                "unsupported macOS release '{release}'"
            )));
        }
    };
    let session = apple_session()?;
    let body = format!(
        "cid={}\nsn={mlb}\nbid={board_id}\nk={}\nfg={}\nos={os_type}",
        random_hex(16),
        random_hex(64),
        random_hex(64)
    );
    let info = curl_request(
        "https://osrecovery.apple.com/InstallationPayload/RecoveryImage",
        &[
            "Host: osrecovery.apple.com",
            "Connection: close",
            "User-Agent: InternetRecovery/1.0",
            "Content-Type: text/plain",
        ],
        Some(&format!("session=\"{session}\"")),
        Some(&body),
    )?;
    let url = first_token(&info, |value| {
        value.contains("oscdn") && value.contains(".dmg")
    })
    .ok_or_else(|| Error::message("Apple did not return a macOS recovery image"))?;
    let chunklist_url = first_token(&info, |value| {
        value.contains("oscdn") && value.contains("chunklist")
    })
    .ok_or_else(|| Error::message("Apple did not return a recovery chunk list"))?;
    let asset_token = apple_asset_token(&info, "dmg")?;
    let chunklist_token = apple_asset_token(&info, "chunklist")?;
    Ok(MacosRecovery {
        url,
        asset_token,
        chunklist_url,
        chunklist_token,
    })
}

fn apple_session() -> Result<String> {
    let output = Command::new("curl")
        .args([
            "--disable",
            "--silent",
            "--show-error",
            "--dump-header",
            "-",
            "--output",
            "/dev/null",
            "-H",
            "Host: osrecovery.apple.com",
            "-H",
            "Connection: close",
            "-A",
            "InternetRecovery/1.0",
            "--",
            "https://osrecovery.apple.com/",
        ])
        .output()
        .map_err(|error| Error::command_unavailable("curl", error))?;
    if !output.status.success() {
        return Err(Error::command_failed_status("curl", output.status));
    }
    let headers = String::from_utf8_lossy(&output.stdout);
    headers
        .split([';', '\n', '\r'])
        .find_map(|part| part.split_once("session=").map(|(_, value)| value))
        .map(|value| value.trim_matches('"').trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| Error::message("Apple did not return a recovery session"))
}

fn apple_asset_token(info: &str, kind: &str) -> Result<String> {
    let token = info.lines().find_map(|line| {
        if !line.contains(kind) || !line.contains("expires=") {
            return None;
        }
        line.split_once("expires=").and_then(|(_, value)| {
            value
                .split_whitespace()
                .next()
                .map(|value| value.trim_matches(['"', '\'', ';']).to_string())
        })
    });
    token
        .filter(|value| !value.is_empty())
        .ok_or_else(|| Error::message(format!("Apple did not return a {kind} asset token")))
}

fn random_hex(length: usize) -> String {
    let seed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_nanos() as u64)
        .unwrap_or_default()
        ^ u64::from(std::process::id());
    let mut state = seed | 1;
    let mut result = String::with_capacity(length);
    while result.len() < length {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        result.push_str(&format!("{state:016x}"));
    }
    result.truncate(length);
    result
}

fn curl_request(
    url: &str,
    headers: &[&str],
    cookie: Option<&str>,
    body: Option<&str>,
) -> Result<String> {
    let mut command = Command::new("curl");
    command.args([
        "--disable",
        "--fail",
        "--location",
        "--silent",
        "--show-error",
    ]);
    for header in headers {
        command.args(["-H", header]);
    }
    if let Some(cookie) = cookie {
        command.args(["--cookie", cookie]);
    }
    if let Some(body) = body {
        command.args(["--request", "POST", "--data-raw", body]);
    }
    let output = command
        .arg("--")
        .arg(url)
        .output()
        .map_err(|error| Error::command_unavailable("curl", error))?;
    if !output.status.success() {
        return Err(Error::command_failed_status("curl", output.status));
    }
    String::from_utf8(output.stdout)
        .map_err(|error| Error::message(format!("invalid UTF-8 from {url}: {error}")))
}

fn download_file_with_headers(
    url: &str,
    destination: &Path,
    headers: &[String],
    insecure: bool,
) -> Result<()> {
    if fs::symlink_metadata(destination)
        .ok()
        .is_some_and(|metadata| metadata.file_type().is_symlink())
    {
        return Err(Error::message(format!(
            "refusing to download through symlink {}",
            destination.display()
        )));
    }
    let parent = destination
        .parent()
        .ok_or_else(|| Error::message("download destination has no parent directory"))?;
    fs::create_dir_all(parent).map_err(|error| Error::io(parent.display(), error))?;
    let mut command = Command::new("curl");
    command.args([
        "--disable",
        "--fail",
        "--location",
        "--continue-at",
        "-",
        "--output",
    ]);
    command.args(curl_security_args(insecure));
    command.arg(destination);
    for header in headers {
        command.args(["--header", header]);
    }
    let status = command
        .arg("--")
        .arg(url)
        .status()
        .map_err(|error| Error::command_unavailable("curl", error))?;
    if status.success() {
        Ok(())
    } else {
        Err(Error::command_failed_status("curl", status))
    }
}

fn download_macos(
    args: &GetArgs,
    dirs: &Dirs,
    release: &str,
    architecture: &str,
    create_config: bool,
    output: OutputFormat,
) -> Result<()> {
    if args.edition_or_language.is_some() {
        return Err(Error::message("macOS does not take an edition"));
    }
    if architecture != "amd64" {
        return Err(Error::message("macOS recovery is only available for amd64"));
    }
    let name = suggested_name("macos", release, None, architecture);
    validate_vm_name(&name)?;
    let root = if create_config {
        dirs.vm_dir.clone()
    } else {
        env::current_dir().map_err(|error| Error::io("current directory", error))?
    };
    let target_dir = if create_config {
        root.join(&name)
    } else {
        root.clone()
    };
    if create_config && root.join(format!("{name}.conf")).exists() {
        return Err(Error::message(format!(
            "configuration already exists: {}",
            root.join(format!("{name}.conf")).display()
        )));
    }
    fs::create_dir_all(&target_dir).map_err(|error| Error::io(target_dir.display(), error))?;
    let recovery = fetch_macos_recovery(release)?;
    let recovery_dmg = target_dir.join("RecoveryImage.dmg");
    let recovery_img = target_dir.join("RecoveryImage.img");
    let dmg_headers = vec![
        "Host: oscdn.apple.com".to_string(),
        "Connection: close".to_string(),
        "User-Agent: InternetRecovery/1.0".to_string(),
        format!("Cookie: AssetToken={}", recovery.asset_token),
    ];
    let chunk_headers = vec![
        "Host: oscdn.apple.com".to_string(),
        "Connection: close".to_string(),
        "User-Agent: InternetRecovery/1.0".to_string(),
        format!("Cookie: AssetToken={}", recovery.chunklist_token),
    ];
    download_file_with_headers(&recovery.url, &recovery_dmg, &dmg_headers, args.insecure)?;
    download_file_with_headers(
        &recovery.chunklist_url,
        &target_dir.join("RecoveryImage.chunklist"),
        &chunk_headers,
        args.insecure,
    )?;
    if command_exists("chunkcheck") {
        let status = Command::new("chunkcheck")
            .arg(&target_dir)
            .status()
            .map_err(|error| Error::command_unavailable("chunkcheck", error))?;
        if !status.success() {
            eprintln!("vmctl: warning: Apple recovery chunk verification failed");
        }
    }
    if !recovery_img.exists() {
        let status = Command::new("qemu-img")
            .args([
                "convert",
                recovery_dmg.to_string_lossy().as_ref(),
                "-O",
                "raw",
                recovery_img.to_string_lossy().as_ref(),
            ])
            .status()
            .map_err(|error| Error::command_unavailable("qemu-img", error))?;
        if !status.success() {
            return Err(Error::command_failed_status("qemu-img convert", status));
        }
    }
    let _ = fs::remove_file(&recovery_dmg);
    let _ = fs::remove_file(target_dir.join("RecoveryImage.chunklist"));
    if create_config {
        let commit = "da4b23b5e92c5b939568700034367e8b7649fe90";
        for (file, url) in [
            (
                "OpenCore.qcow2",
                format!("https://github.com/kholia/OSX-KVM/raw/{commit}/OpenCore/OpenCore.qcow2"),
            ),
            (
                "OVMF_CODE.fd",
                format!("https://github.com/kholia/OSX-KVM/raw/{commit}/OVMF_CODE.fd"),
            ),
            (
                "OVMF_VARS-1920x1080.fd",
                format!("https://github.com/kholia/OSX-KVM/raw/{commit}/OVMF_VARS-1920x1080.fd"),
            ),
        ] {
            download_file(&url, &target_dir.join(file), args.insecure)?;
        }
    }
    let config_path = if create_config {
        Some(write_vm_config(
            &root,
            &name,
            "macos",
            release,
            None,
            architecture,
            &recovery_img,
        )?)
    } else {
        None
    };
    let result = json!({
        "os": "macos",
        "release": release,
        "architecture": architecture,
        "image": recovery_img,
        "config": config_path,
    });
    if output == OutputFormat::Json {
        println!(
            "{}",
            serde_json::to_string_pretty(&result).unwrap_or_default()
        );
    } else if let Some(config_path) = config_path {
        println!("Created {}", config_path.display());
    } else {
        println!("Downloaded {}", recovery_img.display());
    }
    Ok(())
}

fn windows_asset(
    os: &str,
    release: &str,
    language: Option<&str>,
) -> Result<(String, ImageKind, Option<String>)> {
    let language = language.unwrap_or("English International");
    let url = if os == "windows-server" {
        windows_server_url(release)?
    } else {
        windows_workstation_url(release, language)?
    };
    Ok((url, ImageKind::Iso, None))
}

fn windows_server_url(release: &str) -> Result<String> {
    let page = fetch_text(&format!(
        "https://www.microsoft.com/en-us/evalcenter/download-windows-server-{release}"
    ))?;
    let link = first_token(&page, |value| {
        value.starts_with("https://go.microsoft.com/fwlink/p/?")
            && value.contains("culture=en-us")
            && value.contains("country=US")
    })
    .ok_or_else(|| dynamic_url_error("windows-server"))?;
    fetch_redirect(&link)
}

fn windows_workstation_url(release: &str, language: &str) -> Result<String> {
    let page_url = if release == "10" {
        "https://www.microsoft.com/en-us/software-download/windows10ISO".to_string()
    } else {
        format!("https://www.microsoft.com/en-us/software-download/windows{release}")
    };
    let user_agent = "Mozilla/5.0 (X11; Linux x86_64; rv:100.0) Gecko/20100101 Firefox/100.0";
    let user_agent_header = format!("User-Agent: {user_agent}");
    let page = curl_request(&page_url, &["Accept:", &user_agent_header], None, None)?;
    let product_id = page
        .split("<option value=\"")
        .skip(1)
        .find_map(|part| {
            let (value, rest) = part.split_once('"')?;
            (rest.starts_with(">Windows")
                && value.chars().all(|character| character.is_ascii_digit()))
            .then_some(value)
        })
        .ok_or_else(|| dynamic_url_error("windows"))?;
    let session = format!(
        "{}-{}-{}-{}-{}",
        random_hex(8),
        random_hex(4),
        random_hex(4),
        random_hex(4),
        random_hex(12)
    );
    curl_request(
        &format!("https://vlscppe.microsoft.com/tags?org_id=y6jn8c31&session_id={session}"),
        &["Accept:", &user_agent_header],
        None,
        None,
    )?;
    windows_ov_df_handshake(&session, &user_agent_header)?;
    let sku_data = curl_request(
        &format!(
            "https://www.microsoft.com/software-download-connector/api/getskuinformationbyproductedition?profile=606624d44113&ProductEditionId={product_id}&SKU=undefined&friendlyFileName=undefined&Locale=en-US&sessionID={session}"
        ),
        &["Accept:", &user_agent_header],
        None,
        None,
    )?;
    let sku_values: Value = serde_json::from_str(&sku_data)
        .map_err(|error| Error::message(format!("invalid Microsoft SKU data: {error}")))?;
    let sku = sku_values
        .get("Skus")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find(|entry| {
            entry
                .get("LocalizedLanguage")
                .and_then(Value::as_str)
                .is_some_and(|value| value == language)
                || entry
                    .get("Language")
                    .and_then(Value::as_str)
                    .is_some_and(|value| value == language)
        })
        .and_then(|entry| entry.get("Id"))
        .and_then(Value::as_str)
        .ok_or_else(|| Error::message(format!("Microsoft does not offer Windows in {language}")))?;
    let links_data = curl_request(
        &format!(
            "https://www.microsoft.com/software-download-connector/api/GetProductDownloadLinksBySku?profile=606624d44113&productEditionId=undefined&SKU={sku}&friendlyFileName=undefined&Locale=en-US&sessionID={session}"
        ),
        &[
            "Accept:",
            &user_agent_header,
            &format!("Referer: {page_url}"),
        ],
        None,
        None,
    )?;
    if links_data.contains("Sentinel marked this request as rejected") {
        return Err(Error::message(
            "Microsoft rejected the automated Windows download request; download the ISO in a browser and use --create-config VM_NAME IMAGE_PATH_OR_URL",
        ));
    }
    let links: Value = serde_json::from_str(&links_data)
        .map_err(|error| Error::message(format!("invalid Microsoft download data: {error}")))?;
    links
        .get("ProductDownloadOptions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.get("Uri").and_then(Value::as_str))
        .find(|uri| uri.to_ascii_lowercase().contains("x64"))
        .map(str::to_string)
        .ok_or_else(|| dynamic_url_error("windows"))
}

fn windows_ov_df_handshake(session: &str, user_agent_header: &str) -> Result<()> {
    let instance_id = "560dc9f3-1aa5-4a2f-b63c-9e18f8d0e175";
    let headers = ["Accept:", user_agent_header];
    let response = curl_request(
        &format!(
            "https://ov-df.microsoft.com/mdt.js?instanceId={instance_id}&PageId=si&session_id={session}"
        ),
        &headers,
        None,
        None,
    )?;
    let width = windows_ov_df_value(&response, "w", |character| character.is_ascii_hexdigit())
        .ok_or_else(|| Error::message("Microsoft Windows download response did not include w"))?;
    let rticks = windows_ov_df_value(&response, "rticks", |character| character.is_ascii_digit())
        .ok_or_else(|| {
        Error::message("Microsoft Windows download response did not include rticks")
    })?;
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| Error::message(format!("system clock is before the Unix epoch: {error}")))?
        .as_millis();
    curl_request(
        &format!(
            "https://ov-df.microsoft.com/?session_id={session}&CustomerId={instance_id}&PageId=si&w={width}&mdt={timestamp}&rticks={rticks}"
        ),
        &headers,
        None,
        None,
    )?;
    Ok(())
}

fn windows_ov_df_value(response: &str, key: &str, valid: fn(char) -> bool) -> Option<String> {
    let marker = format!("{key}=");
    response.match_indices(&marker).find_map(|(start, _)| {
        let value = response[start + marker.len()..].trim_start_matches('+');
        let value: String = value
            .chars()
            .take_while(|character| valid(*character))
            .collect();
        (!value.is_empty()).then_some(value)
    })
}

fn download_windows(
    args: &GetArgs,
    dirs: &Dirs,
    os: &str,
    release: &str,
    architecture: &str,
    create_config: bool,
    output: OutputFormat,
) -> Result<()> {
    if architecture != "amd64" {
        return Err(Error::message(
            "Windows downloads are only available for amd64",
        ));
    }
    let edition = required_edition(find_os(os)?, args.edition_or_language.as_deref())?;
    let image = windows_asset(os, release, edition.as_deref())?;
    let name = suggested_name(os, release, edition.as_deref(), architecture);
    validate_vm_name(&name)?;
    let root = if create_config {
        dirs.vm_dir.clone()
    } else {
        env::current_dir().map_err(|error| Error::io("current directory", error))?
    };
    let target_dir = if create_config {
        root.join(&name)
    } else {
        root.clone()
    };
    let config_file = root.join(format!("{name}.conf"));
    if create_config && config_file.exists() {
        return Err(Error::message(format!(
            "configuration already exists: {}",
            config_file.display()
        )));
    }
    fs::create_dir_all(&target_dir).map_err(|error| Error::io(target_dir.display(), error))?;
    let file_name = file_name_from_url(&image.0).unwrap_or_else(|| format!("{os}-{release}.iso"));
    let iso = target_dir.join(file_name);
    download_file(&image.0, &iso, args.insecure)?;
    let (fixed_iso, unattended_iso) = if create_config {
        let fixed_iso = download_virtio_iso(&target_dir, args.insecure)?;
        let unattended_iso = if args.disable_unattended {
            None
        } else {
            Some(create_unattended_iso(&target_dir, args.insecure)?)
        };
        (Some(fixed_iso), unattended_iso)
    } else {
        (None, None)
    };
    let config = if create_config {
        let config = write_vm_config(
            &root,
            &name,
            os,
            release,
            edition.as_deref(),
            architecture,
            &iso,
        )?;
        if let Some(fixed_iso) = fixed_iso.as_deref() {
            append_iso(&root, &config, "fixed_iso", fixed_iso)?;
        }
        if let Some(unattended_iso) = unattended_iso.as_deref() {
            append_iso(&root, &config, "unattended_iso", unattended_iso)?;
        }
        Some(config)
    } else {
        None
    };
    let result = json!({
        "os": os,
        "release": release,
        "edition": edition,
        "architecture": architecture,
        "url": image.0,
        "image": iso,
        "fixed_iso": fixed_iso,
        "unattended_iso": unattended_iso,
        "config": config,
        "unattended": unattended_iso.is_some(),
    });
    if output == OutputFormat::Json {
        println!(
            "{}",
            serde_json::to_string_pretty(&result).unwrap_or_default()
        );
    } else if let Some(config) = config {
        println!("Downloaded {}", iso.display());
        println!("Created {}", config.display());
    } else {
        println!("Downloaded {}", iso.display());
    }
    Ok(())
}

fn download_virtio_iso(target_dir: &Path, insecure: bool) -> Result<PathBuf> {
    let path = target_dir.join("virtio-win.iso");
    download_file(
        "https://fedorapeople.org/groups/virt/virtio-win/direct-downloads/stable-virtio/virtio-win.iso",
        &path,
        insecure,
    )?;
    Ok(path)
}

const WINDOWS_UNATTENDED_XML: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<unattend xmlns="urn:schemas-microsoft-com:unattend"
  xmlns:wcm="http://schemas.microsoft.com/WMIConfig/2002/State"
  xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
  <settings pass="windowsPE">
    <component name="Microsoft-Windows-Setup" processorArchitecture="amd64" publicKeyToken="31bf3856ad364e35" language="neutral" versionScope="nonSxS">
      <DiskConfiguration>
        <Disk wcm:action="add">
          <DiskID>0</DiskID>
          <WillWipeDisk>true</WillWipeDisk>
          <CreatePartitions>
            <CreatePartition wcm:action="add"><Order>1</Order><Type>EFI</Type><Size>260</Size></CreatePartition>
            <CreatePartition wcm:action="add"><Order>2</Order><Type>MSR</Type><Size>128</Size></CreatePartition>
            <CreatePartition wcm:action="add"><Order>3</Order><Type>Primary</Type><Extend>true</Extend></CreatePartition>
          </CreatePartitions>
          <ModifyPartitions>
            <ModifyPartition wcm:action="add"><Order>1</Order><PartitionID>1</PartitionID><Format>FAT32</Format><Label>System</Label></ModifyPartition>
            <ModifyPartition wcm:action="add"><Order>2</Order><PartitionID>2</PartitionID></ModifyPartition>
            <ModifyPartition wcm:action="add"><Order>3</Order><PartitionID>3</PartitionID><Format>NTFS</Format><Label>Windows</Label><Letter>C</Letter></ModifyPartition>
          </ModifyPartitions>
        </Disk>
      </DiskConfiguration>
      <ImageInstall><OSImage><InstallTo><DiskID>0</DiskID><PartitionID>3</PartitionID></InstallTo></OSImage></ImageInstall>
      <RunSynchronous>
        <RunSynchronousCommand wcm:action="add"><Order>1</Order><Path>reg add HKLM\System\Setup\LabConfig /v BypassCPUCheck /t REG_DWORD /d 1 /f</Path></RunSynchronousCommand>
        <RunSynchronousCommand wcm:action="add"><Order>2</Order><Path>reg add HKLM\System\Setup\LabConfig /v BypassRAMCheck /t REG_DWORD /d 1 /f</Path></RunSynchronousCommand>
        <RunSynchronousCommand wcm:action="add"><Order>3</Order><Path>reg add HKLM\System\Setup\LabConfig /v BypassSecureBootCheck /t REG_DWORD /d 1 /f</Path></RunSynchronousCommand>
        <RunSynchronousCommand wcm:action="add"><Order>4</Order><Path>reg add HKLM\System\Setup\LabConfig /v BypassTPMCheck /t REG_DWORD /d 1 /f</Path></RunSynchronousCommand>
      </RunSynchronous>
      <UserData>
        <AcceptEula>true</AcceptEula>
        <FullName>vmctl</FullName>
        <Organization>vmctl</Organization>
        <ProductKey><Key>W269N-WFGWX-YVC9B-4J6C9-T83GX</Key><WillShowUI>Never</WillShowUI></ProductKey>
      </UserData>
    </component>
    <component name="Microsoft-Windows-PnpCustomizationsWinPE" processorArchitecture="amd64" publicKeyToken="31bf3856ad364e35" language="neutral" versionScope="nonSxS">
      <DriverPaths>
        <PathAndCredentials wcm:action="add" wcm:keyValue="1"><Path>E:\qemufwcfg\w10\amd64</Path></PathAndCredentials>
        <PathAndCredentials wcm:action="add" wcm:keyValue="2"><Path>E:\vioscsi\w10\amd64</Path></PathAndCredentials>
        <PathAndCredentials wcm:action="add" wcm:keyValue="3"><Path>E:\viostor\w10\amd64</Path></PathAndCredentials>
        <PathAndCredentials wcm:action="add" wcm:keyValue="4"><Path>E:\NetKVM\w10\amd64</Path></PathAndCredentials>
      </DriverPaths>
    </component>
  </settings>
  <settings pass="oobeSystem">
    <component name="Microsoft-Windows-Shell-Setup" processorArchitecture="amd64" publicKeyToken="31bf3856ad364e35" language="neutral" versionScope="nonSxS">
      <AutoLogon><Password><Value>vmctl</Value><PlainText>true</PlainText></Password><Enabled>true</Enabled><Username>vmctl</Username></AutoLogon>
      <OOBE><HideEULAPage>true</HideEULAPage><HideOnlineAccountScreens>true</HideOnlineAccountScreens><HideWirelessSetupInOOBE>true</HideWirelessSetupInOOBE><NetworkLocation>Home</NetworkLocation><ProtectYourPC>3</ProtectYourPC><SkipMachineOOBE>true</SkipMachineOOBE><SkipUserOOBE>true</SkipUserOOBE></OOBE>
      <UserAccounts><LocalAccounts><LocalAccount wcm:action="add"><Password><Value>vmctl</Value><PlainText>true</PlainText></Password><Description>vmctl</Description><DisplayName>vmctl</DisplayName><Group>Administrators</Group><Name>vmctl</Name></LocalAccount></LocalAccounts></UserAccounts>
      <FirstLogonCommands>
        <SynchronousCommand wcm:action="add"><Order>1</Order><CommandLine>msiexec /i E:\guest-agent\qemu-ga-x86_64.msi /quiet /qn</CommandLine><Description>Install QEMU Guest Agent</Description></SynchronousCommand>
        <SynchronousCommand wcm:action="add"><Order>2</Order><CommandLine>msiexec /i F:\spice-webdavd-x64-latest.msi /quiet /qn</CommandLine><Description>Install SPICE WebDAV</Description></SynchronousCommand>
        <SynchronousCommand wcm:action="add"><Order>3</Order><CommandLine>msiexec /i F:\spice-vdagent-x64-0.10.0.msi /quiet /qn</CommandLine><Description>Install SPICE agent</Description></SynchronousCommand>
      </FirstLogonCommands>
    </component>
  </settings>
</unattend>
"#;

fn create_unattended_iso(target_dir: &Path, insecure: bool) -> Result<PathBuf> {
    let builder = ["mkisofs", "genisoimage", "xorriso"]
        .into_iter()
        .find(|command| command_exists(command))
        .ok_or_else(|| {
            Error::message(
                "creating unattended Windows media requires mkisofs, genisoimage, or xorriso",
            )
        })?;
    let source_dir = target_dir.join("unattended");
    fs::create_dir_all(&source_dir).map_err(|error| Error::io(source_dir.display(), error))?;
    let xml = source_dir.join("autounattend.xml");
    if fs::symlink_metadata(&xml)
        .ok()
        .is_some_and(|metadata| metadata.file_type().is_symlink())
    {
        return Err(Error::message(format!(
            "refusing to write through symlink {}",
            xml.display()
        )));
    }
    fs::write(&xml, WINDOWS_UNATTENDED_XML).map_err(|error| Error::io(xml.display(), error))?;
    for (url, file) in [
        (
            "https://www.spice-space.org/download/windows/spice-webdavd/spice-webdavd-x64-latest.msi",
            "spice-webdavd-x64-latest.msi",
        ),
        (
            "https://www.spice-space.org/download/windows/vdagent/vdagent-win-0.10.0/spice-vdagent-x64-0.10.0.msi",
            "spice-vdagent-x64-0.10.0.msi",
        ),
    ] {
        download_file(url, &source_dir.join(file), insecure)?;
    }
    let destination = target_dir.join("unattended.iso");
    if fs::symlink_metadata(&destination)
        .ok()
        .is_some_and(|metadata| metadata.file_type().is_symlink())
    {
        return Err(Error::message(format!(
            "refusing to write through symlink {}",
            destination.display()
        )));
    }
    let status = if builder == "xorriso" {
        Command::new(builder)
            .args(["-as", "mkisofs", "-quiet", "-J", "-o"])
            .arg(&destination)
            .arg(&source_dir)
            .status()
    } else {
        Command::new(builder)
            .args(["-q", "-J", "-o"])
            .arg(&destination)
            .arg(&source_dir)
            .status()
    }
    .map_err(|error| Error::command_unavailable(builder, error))?;
    let _ = fs::remove_dir_all(&source_dir);
    if !status.success() {
        return Err(Error::command_failed_status(builder, status));
    }
    Ok(destination)
}

fn append_iso(root: &Path, config: &Path, key: &str, image: &Path) -> Result<()> {
    let relative = image
        .strip_prefix(root)
        .unwrap_or(image)
        .to_string_lossy()
        .replace('\\', "/");
    let mut file = OpenOptions::new()
        .append(true)
        .open(config)
        .map_err(|error| Error::io(config.display(), error))?;
    writeln!(file, "{key}=\"{}\"", config_value(&relative))
        .map_err(|error| Error::io(config.display(), error))
}

fn prepare_resolved_image(os: &str, path: &Path) -> Result<PathBuf> {
    let image = prepare_image(path)?;
    match os {
        "batocera" => {
            let status = Command::new("qemu-img")
                .args(["resize", "-f", "raw"])
                .arg(&image)
                .arg("128G")
                .status()
                .map_err(|error| Error::command_unavailable("qemu-img", error))?;
            if !status.success() {
                return Err(Error::command_failed_status("qemu-img resize", status));
            }
            Ok(image)
        }
        "easyos" => {
            let parent = image
                .parent()
                .ok_or_else(|| Error::message("EasyOS image has no parent directory"))?;
            let disk = parent.join("disk.qcow2");
            if fs::symlink_metadata(&disk)
                .ok()
                .is_some_and(|metadata| metadata.file_type().is_symlink())
            {
                return Err(Error::message(format!(
                    "refusing to write through symlink {}",
                    disk.display()
                )));
            }
            let status = Command::new("qemu-img")
                .args(["convert", "-f", "raw", "-O", "qcow2"])
                .arg(&image)
                .arg(&disk)
                .status()
                .map_err(|error| Error::command_unavailable("qemu-img", error))?;
            if !status.success() {
                return Err(Error::command_failed_status("qemu-img convert", status));
            }
            Ok(disk)
        }
        _ => Ok(image),
    }
}

fn easyos_asset(release: &str) -> Result<(String, ImageKind, Option<String>)> {
    let base = "https://distro.ibiblio.org/easyos/amd64/releases/kirkstone";
    for year in (2020..=2035).rev() {
        let directory = format!("{base}/{year}/{release}");
        let Ok(page) = fetch_text(&format!("{directory}/")) else {
            continue;
        };
        let file = first_token(&page, |value| {
            value.starts_with("easy-") && value.ends_with("-amd64.img")
        })
        .unwrap_or_else(|| format!("easy-{release}-amd64.img"));
        let checksum = checksum_at(&format!("{directory}/md5.sum.txt"), &file, "sha256");
        return Ok((format!("{directory}/{file}"), ImageKind::Img, checksum));
    }
    Err(dynamic_url_error("easyos"))
}

fn endeavouros_asset(release: &str) -> Result<(String, ImageKind, Option<String>)> {
    let base = "https://mirror.alpix.eu/endeavouros/iso";
    let page = fetch_text(&format!("{base}/"))?;
    let file = first_token(&page, |value| {
        value.ends_with(".iso")
            && !value.contains("x86_64")
            && (release == "latest"
                || value
                    .to_ascii_lowercase()
                    .contains(&release.to_ascii_lowercase()))
    })
    .ok_or_else(|| dynamic_url_error("endeavouros"))?;
    let checksum = checksum_at(&format!("{base}/{file}.sha512sum"), &file, "sha512");
    Ok((format!("{base}/{file}"), ImageKind::Iso, checksum))
}

fn endless_asset(
    release: &str,
    edition: Option<&str>,
) -> Result<(String, ImageKind, Option<String>)> {
    let edition = edition.ok_or_else(|| Error::message("Endless OS requires a language"))?;
    let timestamp = match (release, edition) {
        ("6.0.4", "base") => "241023-183516",
        ("6.0.4", "en") => "241023-200926",
        ("6.0.4", "es") => "241023-184649",
        ("6.0.4", "fr") => "241023-191212",
        ("6.0.4", "pt_BR") => "241023-191427",
        _ => return Err(dynamic_url_error("endless")),
    };
    let short = release.chars().take(3).collect::<String>();
    let file = format!("eos-eos{short}-amd64-amd64.{timestamp}.{edition}.iso");
    let base =
        format!("https://images-dl.endlessm.com/release/{release}/eos-amd64-amd64/{edition}");
    Ok((format!("{base}/{file}"), ImageKind::Iso, None))
}

fn garuda_asset(edition: Option<&str>) -> Result<(String, ImageKind, Option<String>)> {
    let edition = edition.ok_or_else(|| Error::message("Garuda Linux requires an edition"))?;
    let base = "https://iso.builds.garudalinux.org/iso/latest/garuda";
    let file = format!("{edition}/latest.iso");
    let checksum = checksum_at(&format!("{base}/{file}.sha256"), &file, "sha256");
    Ok((format!("{base}/{file}"), ImageKind::Iso, checksum))
}

fn gentoo_asset(
    release: &str,
    edition: Option<&str>,
) -> Result<(String, ImageKind, Option<String>)> {
    let edition = edition.ok_or_else(|| Error::message("Gentoo requires an edition"))?;
    let base = "https://mirrors.kernel.org/gentoo/releases/amd64/autobuilds";
    let listing = fetch_text(&format!("{base}/{release}-iso.txt"))?;
    let marker = if edition == "livegui" {
        "livegui"
    } else {
        "install"
    };
    let file = first_token(&listing, |value| {
        value.ends_with(".iso") && value.contains(marker)
    })
    .ok_or_else(|| dynamic_url_error("gentoo"))?;
    let sums = fetch_text(&format!("{base}/{file}.DIGESTS"))?;
    let checksum = checksum_from_text(&sums, &file, "sha512");
    Ok((format!("{base}/{file}"), ImageKind::Iso, checksum))
}

fn ghostbsd_asset(
    release: &str,
    edition: Option<&str>,
) -> Result<(String, ImageKind, Option<String>)> {
    let edition = edition.ok_or_else(|| Error::message("GhostBSD requires an edition"))?;
    let file = match edition {
        "mate" => format!("GhostBSD-{release}.iso"),
        "xfce" => format!("GhostBSD-{release}-XFCE.iso"),
        _ => return Err(dynamic_url_error("ghostbsd")),
    };
    let base = format!("https://download.ghostbsd.org/releases/amd64/{release}");
    let checksum = checksum_at(&format!("{base}/{file}.sha256"), &file, "sha256");
    Ok((format!("{base}/{file}"), ImageKind::Iso, checksum))
}

fn gnomeos_asset(release: &str) -> Result<(String, ImageKind, Option<String>)> {
    let (base, file) = if release == "nightly" {
        ("https://os.gnome.org/download/latest".to_string(), None)
    } else {
        (
            format!("https://download.gnome.org/gnomeos/{release}"),
            Some(format!("gnome_os_installer_{release}.iso")),
        )
    };
    let url = if let Some(file) = file {
        fetch_redirect(&format!("{base}/{file}"))?
    } else {
        fetch_redirect(&base)?
    };
    Ok((url, ImageKind::Iso, None))
}

fn kdeneon_asset(
    release: &str,
    edition: Option<&str>,
) -> Result<(String, ImageKind, Option<String>)> {
    let edition = edition.ok_or_else(|| Error::message("KDE neon requires an edition"))?;
    let base = format!("https://files.kde.org/neon/images/{edition}/{release}/current");
    let sums = fetch_text(&format!(
        "{base}/neon-{release}-{edition}-current.sha256sum"
    ))?;
    let file = first_token(&sums, |value| value.ends_with(".iso"))
        .ok_or_else(|| dynamic_url_error("kdeneon"))?;
    let checksum = checksum_from_text(&sums, &file, "sha256");
    Ok((format!("{base}/{file}"), ImageKind::Iso, checksum))
}

fn kdelinux_asset(release: &str) -> Result<(String, ImageKind, Option<String>)> {
    if release != "latest" {
        return Err(Error::message("KDE Linux currently supports only latest"));
    }
    let base = "https://files.kde.org/kde-linux";
    let listing = fetch_text(&format!("{base}/?C=M;O=D"))?;
    let file =
        first_token(&listing, is_kde_linux_iso).ok_or_else(|| dynamic_url_error("kdelinux"))?;
    let sums = fetch_text(&format!("{base}/SHA256SUMS"))?;
    let checksum = checksum_from_text(&sums, &file, "sha256");
    Ok((format!("{base}/{file}"), ImageKind::Iso, checksum))
}

fn is_kde_linux_iso(value: &str) -> bool {
    value
        .strip_prefix("kde-linux_")
        .and_then(|value| value.strip_suffix(".iso"))
        .is_some_and(|value| value.len() == 12 && value.bytes().all(|byte| byte.is_ascii_digit()))
}

fn kolibrios_asset(edition: Option<&str>) -> Result<(String, ImageKind, Option<String>)> {
    let edition = edition.ok_or_else(|| Error::message("KolibriOS requires a language"))?;
    let base = format!("http://builds.kolibrios.org/{edition}");
    let file = "latest-iso.7z";
    let checksum = checksum_at(&format!("{base}/sha256sums.txt"), file, "sha256");
    Ok((format!("{base}/{file}"), ImageKind::Archive, checksum))
}

fn mageia_asset(
    release: &str,
    edition: Option<&str>,
) -> Result<(String, ImageKind, Option<String>)> {
    let edition = edition.ok_or_else(|| Error::message("Mageia requires an edition"))?;
    let query = format!("Mageia-{release}-Live-{edition}-x86_64.iso");
    let page = fetch_text(&format!(
        "https://www.mageia.org/en/downloads/get/?q={query}"
    ))?;
    let url = first_token(&page, |value| {
        value.starts_with("http") && value.ends_with(".iso")
    })
    .ok_or_else(|| dynamic_url_error("mageia"))?;
    let checksum = checksum_at(&format!("{url}.sha512"), &url, "sha512");
    Ok((url, ImageKind::Iso, checksum))
}

fn manjaro_asset(
    release: &str,
    edition: Option<&str>,
) -> Result<(String, ImageKind, Option<String>)> {
    let data = if release == "sway" {
        fetch_text("https://mirror.manjaro-sway.download/manjaro-sway/release.json")?
    } else {
        fetch_text("https://gitlab.manjaro.org/web/iso-info/-/raw/master/file-info.json")?
    };
    let values: Value = serde_json::from_str(&data)
        .map_err(|error| Error::message(format!("invalid Manjaro release data: {error}")))?;
    let url = if release == "sway" {
        values.as_array().into_iter().flatten().find_map(|entry| {
            let name = entry.get("name").and_then(Value::as_str)?;
            (name.starts_with("manjaro-sway-") && name.ends_with(".iso"))
                .then(|| entry.get("url").and_then(Value::as_str))
                .flatten()
        })
    } else {
        let group = if matches!(release, "cinnamon" | "i3") {
            "community"
        } else {
            "official"
        };
        let suffix = if edition == Some("minimal") {
            ".minimal"
        } else {
            ""
        };
        let key = format!("{release}{suffix}");
        values
            .get(group)
            .and_then(|group| group.get(&key))
            .and_then(|entry| entry.get("image"))
            .and_then(Value::as_str)
    }
    .ok_or_else(|| dynamic_url_error("manjaro"))?;
    let checksum = checksum_at(&format!("{url}.sha512"), url, "sha512");
    Ok((url.to_string(), ImageKind::Iso, checksum))
}

fn mxlinux_asset(
    release: &str,
    edition: Option<&str>,
) -> Result<(String, ImageKind, Option<String>)> {
    let edition = edition.ok_or_else(|| Error::message("MX Linux requires an edition"))?;
    let suffix = match edition {
        "Xfce" => "Xfce",
        "KDE" => "KDE",
        "Fluxbox" => "fluxbox",
        _ => return Err(dynamic_url_error("mxlinux")),
    };
    let file = format!("MX-{release}_{suffix}_x64.iso");
    let url = sourceforge_asset("mx-linux", &format!("Final/{edition}/{file}"))?;
    let checksum = checksum_at(
        &format!("https://sourceforge.net/projects/mx-linux/files/Final/{edition}/{file}.sha256"),
        &file,
        "sha256",
    );
    Ok((url, ImageKind::Iso, checksum))
}

fn nitrux_asset() -> Result<(String, ImageKind, Option<String>)> {
    let page = fetch_text("https://sourceforge.net/projects/nitruxos/rss?path=/Release/ISO")?;
    let file = first_token(&page, |value| value.ends_with(".iso"))
        .ok_or_else(|| dynamic_url_error("nitrux"))?;
    let url = sourceforge_asset("nitruxos", &format!("Release/ISO/{file}"))?;
    Ok((url, ImageKind::Iso, None))
}

fn nwg_shell_asset(release: &str) -> Result<(String, ImageKind, Option<String>)> {
    let page = fetch_text("https://sourceforge.net/projects/nwg-iso/rss?path=/")?;
    let file = first_token(&page, |value| {
        value.ends_with(".iso") && value.contains("nwg-live") && value.contains(release)
    })
    .ok_or_else(|| dynamic_url_error("nwg-shell"))?;
    let url = sourceforge_asset("nwg-iso", &file)?;
    Ok((url, ImageKind::Iso, None))
}

fn pclinuxos_asset(edition: Option<&str>) -> Result<(String, ImageKind, Option<String>)> {
    let edition = edition.ok_or_else(|| Error::message("PCLinuxOS requires an edition"))?;
    let base = "https://ftp.fau.de/pclinuxos/pclinuxos/iso";
    let page = fetch_text(&format!("{base}/"))?;
    let prefix = format!("pclinuxos64-{edition}-");
    let file = first_token(&page, |value| {
        value.starts_with(&prefix) && value.ends_with(".iso")
    })
    .ok_or_else(|| dynamic_url_error("pclinuxos"))?;
    let checksum = checksum_at(&format!("{base}/{file}.md5sum"), &file, "sha256");
    Ok((format!("{base}/{file}"), ImageKind::Iso, checksum))
}

fn peppermint_asset(edition: Option<&str>) -> Result<(String, ImageKind, Option<String>)> {
    let edition = edition.ok_or_else(|| Error::message("PeppermintOS requires an edition"))?;
    let (directory, file) = match edition {
        "devuan-xfce" => ("XFCE", "PeppermintOS-devuan_64_xfce.iso"),
        "debian-xfce" => ("XFCE", "PeppermintOS-Debian-64.iso"),
        "devuan-gnome" => ("Gnome_FlashBack", "PeppermintOS-devuan_64_gfb.iso"),
        "debian-gnome" => ("Gnome_FlashBack", "PeppermintOS-Debian_64_gfb.iso"),
        _ => return Err(dynamic_url_error("peppermint")),
    };
    let base = format!("https://sourceforge.net/projects/peppermintos/files/isos/{directory}");
    let url = sourceforge_asset("peppermintos", &format!("isos/{directory}/{file}"))?;
    let checksum = checksum_at(&format!("{base}/{file}-sha512.checksum"), file, "sha512");
    Ok((url, ImageKind::Iso, checksum))
}

fn primtux_asset(
    release: &str,
    edition: Option<&str>,
) -> Result<(String, ImageKind, Option<String>)> {
    let edition = edition.ok_or_else(|| Error::message("PrimTux requires an edition"))?;
    let file = format!("PrimTux{release}-amd64-{edition}.iso");
    let url = sourceforge_asset("primtux", &format!("Distribution/{file}"))?;
    Ok((url, ImageKind::Iso, None))
}

fn pureos_asset(
    release: &str,
    edition: Option<&str>,
) -> Result<(String, ImageKind, Option<String>)> {
    let edition = edition.ok_or_else(|| Error::message("PureOS requires an edition"))?;
    let page = fetch_text("https://www.pureos.net/download/")?;
    let url = first_token(&page, |value| {
        value.starts_with("https://downloads.puri.sm/") && value.ends_with(".iso")
    })
    .or_else(|| {
        let lower = edition.to_ascii_lowercase();
        first_token(&page, |value| {
            value.starts_with("https://downloads.puri.sm/")
                && value.contains(&lower)
                && value.contains(release)
                && value.ends_with(".iso")
        })
    })
    .ok_or_else(|| dynamic_url_error("pureos"))?;
    let file = file_name_from_url(&url).unwrap_or_else(|| format!("pureos-{release}.iso"));
    let checksum = checksum_at(
        &format!(
            "{}/{}.checksums_sha256.txt",
            url.trim_end_matches(&file),
            file.trim_end_matches(".iso")
        ),
        &file,
        "sha256",
    );
    Ok((url, ImageKind::Iso, checksum))
}

fn rebornos_asset() -> Result<(String, ImageKind, Option<String>)> {
    let data = fetch_text("https://meta.cdn.soulharsh007.dev/RebornOS-ISO?format=json")?;
    let values: Value = serde_json::from_str(&data)
        .map_err(|error| Error::message(format!("invalid RebornOS release data: {error}")))?;
    let url = values
        .get("url")
        .and_then(Value::as_str)
        .ok_or_else(|| dynamic_url_error("rebornos"))?;
    let checksum = values
        .get("sha256")
        .or_else(|| values.get("md5"))
        .and_then(Value::as_str)
        .filter(|value| value.len() == 64)
        .map(|value| format!("sha256:{value}"));
    Ok((url.to_string(), ImageKind::Iso, checksum))
}

fn slax_asset(edition: Option<&str>) -> Result<(String, ImageKind, Option<String>)> {
    let edition = edition.ok_or_else(|| Error::message("Slax requires an edition"))?;
    let base = match edition {
        "debian" => "https://ftp.fi.muni.cz/pub/linux/slax/Slax-12.x",
        "slackware" => "https://ftp.fi.muni.cz/pub/linux/slax/Slax-15.x",
        _ => return Err(dynamic_url_error("slax")),
    };
    let sums = fetch_text(&format!("{base}/md5.txt"))?;
    let file = first_token(&sums, |value| {
        value.contains("64bit-") && value.ends_with(".iso")
    })
    .ok_or_else(|| dynamic_url_error("slax"))?;
    Ok((format!("{base}/{file}"), ImageKind::Iso, None))
}

fn solus_asset(
    release: &str,
    edition: Option<&str>,
) -> Result<(String, ImageKind, Option<String>)> {
    let edition = edition.ok_or_else(|| Error::message("Solus requires an edition"))?;
    let base = format!("https://downloads.getsol.us/isos/{release}");
    let data = fetch_text(&format!("{base}/"))?;
    let values: Value = serde_json::from_str(&data)
        .map_err(|error| Error::message(format!("invalid Solus release data: {error}")))?;
    let edition_upper = edition.to_ascii_uppercase();
    let file = values
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.get("name").and_then(Value::as_str))
        .find(|name| {
            name.ends_with(".iso")
                && name.contains(&edition_upper)
                && (name.contains("Release") || name.contains("Beta"))
        })
        .ok_or_else(|| dynamic_url_error("solus"))?;
    let checksum = checksum_at(&format!("{base}/{file}.sha256sum"), file, "sha256");
    Ok((format!("{base}/{file}"), ImageKind::Iso, checksum))
}

fn sparkylinux_asset(
    release: &str,
    edition: Option<&str>,
) -> Result<(String, ImageKind, Option<String>)> {
    let edition = edition.ok_or_else(|| Error::message("SparkyLinux requires an edition"))?;
    let file = format!("sparkylinux-{release}-x86_64-{edition}.iso");
    let directory = match edition {
        "minimalcli" => "cli",
        "minimalgui" => "base",
        _ => edition,
    };
    let base = format!("https://sourceforge.net/projects/sparkylinux/files/{directory}");
    let url = sourceforge_asset("sparkylinux", &format!("{directory}/{file}"))?;
    let checksum = checksum_at(&format!("{base}/{file}.allsums.txt"), &file, "sha256");
    Ok((url, ImageKind::Iso, checksum))
}

fn spirallinux_asset(edition: Option<&str>) -> Result<(String, ImageKind, Option<String>)> {
    let edition = edition.ok_or_else(|| Error::message("SpiralLinux requires an edition"))?;
    let file = format!("SpiralLinux_{edition}_12.231005_x86-64.iso");
    let url = sourceforge_asset("spirallinux", &format!("12.231005/{file}"))?;
    Ok((url, ImageKind::Iso, None))
}

fn tuxedo_asset() -> Result<(String, ImageKind, Option<String>)> {
    let page = fetch_text("https://os.tuxedocomputers.com/")?;
    let url = first_token(&page, |value| {
        value.starts_with("http") && value.ends_with("current.iso")
    })
    .or_else(|| {
        first_token(&page, |value| {
            value.starts_with('/') && value.ends_with("current.iso")
        })
        .map(|value| format!("https://os.tuxedocomputers.com{value}"))
    })
    .ok_or_else(|| dynamic_url_error("tuxedo-os"))?;
    let file = file_name_from_url(&url).unwrap_or_else(|| "current.iso".to_string());
    let checksum = checksum_at(
        &format!("https://os.tuxedocomputers.com/checksums/{file}.sha256"),
        &file,
        "sha256",
    );
    Ok((url, ImageKind::Iso, checksum))
}

fn vanillaos_asset(release: &str) -> Result<(String, ImageKind, Option<String>)> {
    let data = fetch_text("https://api.github.com/repos/Vanilla-OS/live-iso/releases")?;
    let values: Value = serde_json::from_str(&data)
        .map_err(|error| Error::message(format!("invalid Vanilla OS release data: {error}")))?;
    let release_data = values
        .as_array()
        .into_iter()
        .flatten()
        .find(|entry| {
            release == "latest" || entry.get("tag_name").and_then(Value::as_str) == Some(release)
        })
        .ok_or_else(|| dynamic_url_error("vanillaos"))?;
    let url = release_data
        .get("assets")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find_map(|asset| {
            let name = asset.get("name").and_then(Value::as_str)?;
            name.ends_with(".iso")
                .then(|| asset.get("browser_download_url").and_then(Value::as_str))
                .flatten()
        })
        .ok_or_else(|| dynamic_url_error("vanillaos"))?;
    let checksum = fetch_text(&format!("{url}.sha256.txt"))
        .ok()
        .and_then(|value| {
            value
                .split_whitespace()
                .next()
                .map(|hash| format!("sha256:{hash}"))
        });
    Ok((url.to_string(), ImageKind::Iso, checksum))
}

fn zorin_asset(
    release: &str,
    edition: Option<&str>,
) -> Result<(String, ImageKind, Option<String>)> {
    let edition = edition.ok_or_else(|| Error::message("Zorin OS requires an edition"))?;
    let edition_name = match edition {
        "core64" => "Core-64-bit",
        "lite64" => "Lite-64-bit",
        "education64" => "Education-64-bit",
        _ => return Err(dynamic_url_error("zorin")),
    };
    let base = format!("https://plug-mirror.rcac.purdue.edu/zorin-iso/{release}");
    let page = fetch_text(&format!("{base}/"))?;
    let prefix = format!("Zorin-OS-{release}-{edition_name}");
    let file = first_token(&page, |value| {
        value.starts_with(&prefix) && value.ends_with(".iso") && !value.contains("Beta")
    })
    .ok_or_else(|| dynamic_url_error("zorin"))?;
    let checksum = checksum_at(&format!("{base}/SHA256SUMS.txt"), &file, "sha256");
    Ok((format!("{base}/{file}"), ImageKind::Iso, checksum))
}

fn fedora_asset(
    release: &str,
    edition: Option<&str>,
    architecture: &str,
) -> Result<(String, ImageKind, Option<String>)> {
    let edition = edition.ok_or_else(|| Error::message("Fedora requires an edition"))?;
    let data = fetch_text("https://getfedora.org/releases.json")?;
    let values: Value = serde_json::from_str(&data)
        .map_err(|error| Error::message(format!("invalid Fedora release data: {error}")))?;
    let wanted_release = release.replace('_', " ");
    let qemu_arch = if architecture == "arm64" {
        "aarch64"
    } else {
        "x86_64"
    };
    let entry = values
        .as_array()
        .into_iter()
        .flatten()
        .find(|entry| {
            entry.get("version").and_then(Value::as_str) == Some(wanted_release.as_str())
                && entry.get("arch").and_then(Value::as_str) == Some(qemu_arch)
                && entry.get("subvariant").and_then(Value::as_str) == Some(edition)
                && entry
                    .get("link")
                    .and_then(Value::as_str)
                    .is_some_and(|value| value.ends_with(".iso"))
        })
        .ok_or_else(|| dynamic_url_error("fedora"))?;
    let link = entry
        .get("link")
        .and_then(Value::as_str)
        .ok_or_else(|| dynamic_url_error("fedora"))?;
    let checksum = entry
        .get("sha256")
        .and_then(Value::as_str)
        .map(str::to_string);
    Ok((link.to_string(), ImageKind::Iso, checksum))
}

fn kali_asset(release: &str, architecture: &str) -> Result<(String, ImageKind, Option<String>)> {
    let arch = if architecture == "arm64" {
        "arm64"
    } else {
        "amd64"
    };
    let base = format!("https://cdimage.kali.org/{release}");
    let page = fetch_text(&format!("{base}/?C=M;O=D"))?;
    let name = page
        .split("kali-linux-")
        .skip(1)
        .filter_map(|value| value.split(['\"', '<', '>']).next())
        .map(|value| format!("kali-linux-{value}"))
        .find(|value| value.ends_with(".iso") && value.contains(arch))
        .ok_or_else(|| dynamic_url_error("kali"))?;
    let checksum = fetch_text(&format!("{base}/SHA256SUMS"))
        .ok()
        .and_then(|sums| {
            sums.lines().find_map(|line| {
                let mut fields = line.split_whitespace();
                let hash = fields.next()?;
                let file = fields.next()?.trim_start_matches('*');
                (file == name).then(|| hash.to_string())
            })
        });
    Ok((format!("{base}/{name}"), ImageKind::Iso, checksum))
}

fn popos_asset(
    release: &str,
    edition: Option<&str>,
) -> Result<(String, ImageKind, Option<String>)> {
    let edition = edition.ok_or_else(|| Error::message("Pop!_OS requires intel or nvidia"))?;
    let data = fetch_text(&format!(
        "https://api.pop-os.org/builds/{release}/{edition}"
    ))?;
    let values: Value = serde_json::from_str(&data)
        .map_err(|error| Error::message(format!("invalid Pop!_OS release data: {error}")))?;
    let url = values
        .get("url")
        .and_then(Value::as_str)
        .ok_or_else(|| dynamic_url_error("popos"))?;
    let checksum = values
        .get("sha_sum")
        .and_then(Value::as_str)
        .map(str::to_string);
    Ok((url.to_string(), ImageKind::Iso, checksum))
}

fn tails_asset(release: &str) -> Result<(String, ImageKind, Option<String>)> {
    let data = fetch_text(&format!(
        "https://tails.boum.org/install/v2/Tails/amd64/{release}/latest.json"
    ))?;
    let values: Value = serde_json::from_str(&data)
        .map_err(|error| Error::message(format!("invalid Tails release data: {error}")))?;
    let file = values
        .get("installations")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|installation| installation.get("installation-paths"))
        .flat_map(|paths| paths.as_array().into_iter().flatten())
        .find(|path| path.get("type").and_then(Value::as_str) == Some("iso"))
        .and_then(|path| path.get("target-files"))
        .and_then(Value::as_array)
        .and_then(|files| files.first())
        .and_then(|file| file.as_object())
        .ok_or_else(|| dynamic_url_error("tails"))?;
    let url = file
        .get("url")
        .and_then(Value::as_str)
        .ok_or_else(|| dynamic_url_error("tails"))?;
    let checksum = file
        .get("sha256")
        .and_then(Value::as_str)
        .map(str::to_string);
    Ok((url.to_string(), ImageKind::Iso, checksum))
}

fn is_ubuntu_family(os: &str) -> bool {
    matches!(
        os,
        "ubuntu"
            | "edubuntu"
            | "kubuntu"
            | "lubuntu"
            | "ubuntu-budgie"
            | "ubuntucinnamon"
            | "ubuntukylin"
            | "ubuntu-mate"
            | "ubuntu-server"
            | "ubuntustudio"
            | "ubuntu-unity"
            | "xubuntu"
    )
}

fn is_ubuntu_desktop(os: &str) -> bool {
    is_ubuntu_family(os) && os != "ubuntu-server"
}

fn ubuntu_arm64_release(release: &str) -> bool {
    let Some((major, minor)) = release.split_once('.') else {
        return false;
    };
    let Ok(major) = major.parse::<u32>() else {
        return false;
    };
    let Ok(minor) = minor.parse::<u32>() else {
        return false;
    };
    major > 25 || (major == 25 && minor >= 10)
}

fn ubuntu_asset(
    os: &str,
    release: &str,
    architecture: &str,
) -> Option<(String, String, Option<String>)> {
    let base = if release.contains("daily") || release == "dvd" {
        format!("https://cdimage.ubuntu.com/{os}/{release}/current")
    } else if matches!(os, "ubuntu" | "ubuntu-server") && architecture == "amd64" {
        format!("https://releases.ubuntu.com/{release}")
    } else if os == "ubuntu" {
        format!("https://cdimage.ubuntu.com/releases/{release}/release")
    } else {
        format!("https://cdimage.ubuntu.com/{os}/releases/{release}/release")
    };
    let sums = fetch_text(&format!("{base}/SHA256SUMS")).ok()?;
    let (file, checksum) = sums
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let hash = fields.next()?;
            let file = fields.next()?.trim_start_matches('*');
            let variant = if os == "ubuntu-server" {
                file.contains("server")
            } else {
                file.contains("desktop") || file.contains("dvd")
            };
            let is_image = file.ends_with(".iso")
                && file.contains(architecture)
                && !file.contains("+mac")
                && variant;
            is_image.then(|| (file.to_string(), hash.to_string()))
        })
        .next_back()?;
    Some((format!("{base}/{file}"), file, Some(checksum)))
}

fn debian_asset(
    release: &str,
    edition: Option<&str>,
    architecture: &str,
) -> Option<(String, String, Option<String>)> {
    let edition = edition.unwrap_or("standard");
    let (base, sums_name) = if edition == "netinst" {
        (
            format!("https://cdimage.debian.org/debian-cd/{release}/{architecture}/iso-cd"),
            "SHA512SUMS",
        )
    } else {
        (
            format!(
                "https://cdimage.debian.org/debian-cd/{release}-live/{architecture}/iso-hybrid"
            ),
            "SHA512SUMS",
        )
    };
    let sums = fetch_text(&format!("{base}/{sums_name}")).ok()?;
    let (file, checksum) = sums
        .lines()
        .filter_map(|line| {
            let checksum = line.split_whitespace().next()?;
            let file = line.split_whitespace().nth(1)?.trim_start_matches('*');
            (file.ends_with(".iso") && file.contains(edition) && file.contains(architecture))
                .then(|| (file.to_string(), checksum.to_string()))
        })
        .next_back()?;
    Some((
        format!("{base}/{file}"),
        file,
        Some(format!("sha512:{checksum}")),
    ))
}

fn fetch_text(url: &str) -> Result<String> {
    let output = Command::new("curl")
        .args([
            "--disable",
            "--fail",
            "--location",
            "--silent",
            "--show-error",
            "--max-filesize",
            "8M",
            "--connect-timeout",
            "30",
            "--max-time",
            "60",
            "--user-agent",
            "Mozilla/5.0 (X11; Linux x86_64; rv:100.0) Gecko/20100101 Firefox/100.0",
            "--header",
            "Accept:",
            "--",
        ])
        .arg(url)
        .output()
        .map_err(|error| Error::command_unavailable("curl", error))?;
    if !output.status.success() {
        return Err(Error::command_failed_status("curl", output.status));
    }
    String::from_utf8(output.stdout)
        .map_err(|error| Error::message(format!("invalid UTF-8 from {url}: {error}")))
}

fn requested_architectures(args: &GetArgs, os: &str) -> Result<Vec<String>> {
    if let Some(arch) = args.arch.as_deref() {
        return Ok(vec![normalize_architecture(arch)?.to_string()]);
    }
    let info = find_os(os)?;
    let host = normalize_architecture(host_architecture())?;
    if info
        .architectures
        .split_whitespace()
        .any(|arch| arch == host)
    {
        Ok(vec![host.to_string()])
    } else {
        Err(Error::message(format!(
            "{} is not available on this host architecture",
            info.name
        )))
    }
}

fn download_file(url: &str, destination: &Path, insecure: bool) -> Result<()> {
    if fs::symlink_metadata(destination)
        .ok()
        .is_some_and(|metadata| metadata.file_type().is_symlink())
    {
        return Err(Error::message(format!(
            "refusing to download through symlink {}",
            destination.display()
        )));
    }
    let parent = destination
        .parent()
        .ok_or_else(|| Error::message("download destination has no parent directory"))?;
    fs::create_dir_all(parent).map_err(|error| Error::io(parent.display(), error))?;
    let mut command = Command::new("curl");
    command.args([
        "--disable",
        "--fail",
        "--location",
        "--continue-at",
        "-",
        "--output",
    ]);
    command.args(curl_security_args(insecure));
    let status = command
        .arg(destination)
        .arg("--")
        .arg(url)
        .status()
        .map_err(|error| Error::command_unavailable("curl", error))?;
    if status.success() {
        Ok(())
    } else {
        Err(Error::command_failed_status("curl", status))
    }
}

fn verify_checksum(path: &Path, expected: Option<&str>) -> Result<()> {
    let Some(expected) = expected else {
        return Ok(());
    };
    let (algorithm, expected) = expected.split_once(':').unwrap_or(("sha256", expected));
    let expected = expected.to_ascii_lowercase();
    let (command, arguments): (&str, &[&str]) = match algorithm.to_ascii_lowercase().as_str() {
        "sha256" => {
            if command_exists("sha256sum") {
                ("sha256sum", &[])
            } else if command_exists("shasum") {
                ("shasum", &["-a", "256"])
            } else {
                return Err(Error::message(
                    "cannot verify the downloaded image: sha256sum or shasum is required",
                ));
            }
        }
        "sha512" => {
            if command_exists("sha512sum") {
                ("sha512sum", &[])
            } else if command_exists("shasum") {
                ("shasum", &["-a", "512"])
            } else {
                return Err(Error::message(
                    "cannot verify the downloaded image: sha512sum or shasum is required",
                ));
            }
        }
        other => {
            return Err(Error::message(format!(
                "unsupported checksum algorithm '{other}'"
            )));
        }
    };
    let output = Command::new(command)
        .args(arguments)
        .arg(path)
        .output()
        .map_err(|error| Error::command_unavailable(command, error))?;
    if !output.status.success() {
        return Err(Error::command_failed_status(command, output.status));
    }
    let actual = String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    if actual == expected {
        Ok(())
    } else {
        Err(Error::message(format!(
            "checksum mismatch for {} (expected {expected}, got {actual})",
            path.display()
        )))
    }
}

fn command_exists(command: &str) -> bool {
    Command::new(command)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn curl_security_args(insecure: bool) -> &'static [&'static str] {
    if insecure { &["--insecure"] } else { &[] }
}

fn url_available(url: &str, insecure: bool) -> Result<bool> {
    url_available_with_headers(url, &[], insecure)
}

fn url_available_with_headers(url: &str, headers: &[String], insecure: bool) -> Result<bool> {
    let mut command = Command::new("curl");
    command.args([
        "--disable",
        "--silent",
        "--show-error",
        "--head",
        "--fail",
        "--location",
        "--connect-timeout",
        "30",
        "--max-time",
        "30",
    ]);
    command.args(curl_security_args(insecure));
    for header in headers {
        command.args(["--header", header]);
    }
    let status = command
        .args(["--", url])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|error| Error::command_unavailable("curl", error))?;
    Ok(status.success())
}

fn prepare_image(path: &Path) -> Result<PathBuf> {
    if fs::symlink_metadata(path)
        .ok()
        .is_some_and(|metadata| metadata.file_type().is_symlink())
    {
        return Err(Error::message(format!(
            "refusing to process symlink {}",
            path.display()
        )));
    }
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if !matches!(extension.as_str(), "zip" | "7z" | "gz" | "bz2") {
        return Ok(path.to_path_buf());
    }
    let parent = path
        .parent()
        .ok_or_else(|| Error::message("archive has no parent directory"))?;
    if extension == "gz" || extension == "bz2" {
        let output = path.with_extension("");
        if fs::symlink_metadata(&output)
            .ok()
            .is_some_and(|metadata| metadata.file_type().is_symlink())
        {
            return Err(Error::message(format!(
                "refusing to decompress through symlink {}",
                output.display()
            )));
        }
        let command = if extension == "gz" { "gzip" } else { "bzip2" };
        let status = Command::new(command)
            .args(["-d", "-f"])
            .arg(path)
            .status()
            .map_err(|error| Error::command_unavailable(command, error))?;
        if !status.success() {
            return Err(Error::command_failed_status(command, status));
        }
        return Ok(output);
    }
    let extract_dir = extraction_directory(parent)?;
    let result = extract_archive(path, extract_dir.as_path(), &extension);
    let _ = fs::remove_dir_all(&extract_dir);
    result
}

fn extraction_directory(parent: &Path) -> Result<PathBuf> {
    for attempt in 0..100u32 {
        let candidate = parent.join(format!(".vmctl-extract-{}-{attempt}", std::process::id()));
        match fs::create_dir(&candidate) {
            Ok(()) => return Ok(candidate),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(Error::io(candidate.display(), error)),
        }
    }
    Err(Error::message(format!(
        "could not create a private extraction directory in {}",
        parent.display()
    )))
}

fn extract_archive(path: &Path, extract_dir: &Path, extension: &str) -> Result<PathBuf> {
    let command = if extension == "zip" { "unzip" } else { "7z" };
    let status = if extension == "zip" {
        Command::new(command)
            .args(["-q", "-o", "-j"])
            .arg(path)
            .arg("-d")
            .arg(extract_dir)
            .status()
    } else {
        Command::new(command)
            .args(["e", "-y"])
            .arg(format!("-o{}", extract_dir.display()))
            .arg(path)
            .status()
    }
    .map_err(|error| Error::command_unavailable(command, error))?;
    if !status.success() {
        return Err(Error::command_failed_status(command, status));
    }

    let mut candidates = fs::read_dir(extract_dir)
        .map_err(|error| Error::io(extract_dir.display(), error))?
        .filter_map(std::result::Result::ok)
        .map(|entry| entry.path())
        .filter(|candidate| {
            candidate
                .extension()
                .and_then(|value| value.to_str())
                .is_some_and(|value| matches!(value.to_ascii_lowercase().as_str(), "iso" | "img"))
                && fs::symlink_metadata(candidate)
                    .is_ok_and(|metadata| metadata.file_type().is_file())
        })
        .collect::<Vec<_>>();
    candidates.sort();
    let candidate = candidates.into_iter().next().ok_or_else(|| {
        Error::message(format!(
            "no ISO or IMG found after extracting {}",
            path.display()
        ))
    })?;
    let parent = path
        .parent()
        .ok_or_else(|| Error::message("archive has no parent directory"))?;
    let file_name = candidate
        .file_name()
        .ok_or_else(|| Error::message("archive entry has no file name"))?;
    let destination = parent.join(file_name);
    if let Ok(metadata) = fs::symlink_metadata(&destination) {
        if metadata.file_type().is_symlink() {
            return Err(Error::message(format!(
                "refusing to replace symlink {}",
                destination.display()
            )));
        }
        if metadata.is_file() {
            return Ok(destination);
        }
        return Err(Error::message(format!(
            "archive output is not a regular file: {}",
            destination.display()
        )));
    }
    fs::rename(&candidate, &destination)
        .map_err(|error| Error::io(destination.display(), error))?;
    Ok(destination)
}

fn write_vm_config(
    root: &Path,
    name: &str,
    os: &str,
    release: &str,
    edition: Option<&str>,
    architecture: &str,
    image: &Path,
) -> Result<PathBuf> {
    let name = validate_vm_name(name)?;
    let config_path = root.join(format!("{name}.conf"));
    let image = image
        .strip_prefix(root)
        .unwrap_or(image)
        .to_string_lossy()
        .replace('\\', "/");
    let image_type = image_kind(&image);
    let guest_os = guest_os(os, release);
    let disk_size = disk_size(os, edition);
    let mut lines = vec![
        format!("guest_os=\"{}\"", config_value(guest_os)),
        format!("arch=\"{}\"", config_value(qemu_architecture(architecture))),
        format!("disk_img=\"{name}/disk.qcow2\""),
    ];
    match image_type {
        ImageKind::Disk => lines[2] = format!("disk_img=\"{}\"", config_value(&image)),
        ImageKind::Img => lines.push(format!("img=\"{}\"", config_value(&image))),
        ImageKind::Iso | ImageKind::Archive => {
            lines.push(format!("iso=\"{}\"", config_value(&image)));
        }
    }
    if let Some(disk_size) = disk_size {
        lines.push(format!("disk_size=\"{disk_size}\""));
    }
    for (key, value) in config_tweaks(os, release) {
        lines.push(format!("{key}=\"{}\"", config_value(value)));
    }
    if guest_os == "macos" {
        lines.push(format!("macos_release=\"{}\"", config_value(release)));
    }
    if matches!(
        os,
        "dragonflybsd"
            | "haiku"
            | "openbsd"
            | "netbsd"
            | "openindiana"
            | "slackware"
            | "slax"
            | "tinycore"
            | "freedos"
            | "kolibrios"
            | "reactos"
    ) {
        lines.push("boot=\"legacy\"".to_string());
    }
    if matches!(os, "windows" | "windows-server") && matches!(release, "11" | "2022") {
        lines.push("tpm=\"on\"".to_string());
        lines.push("secureboot=\"off\"".to_string());
    }
    let contents = format!("{}\n", lines.join("\n"));
    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent).map_err(|error| Error::io(parent.display(), error))?;
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&config_path)
        .map_err(|error| Error::io(config_path.display(), error))?;
    file.write_all(contents.as_bytes())
        .map_err(|error| Error::io(config_path.display(), error))?;
    Ok(config_path)
}

fn image_kind(path: &str) -> ImageKind {
    match Path::new(path)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "qcow2" | "raw" => ImageKind::Disk,
        "img" | "dmg" => ImageKind::Img,
        "zip" | "7z" | "gz" | "bz2" => ImageKind::Archive,
        _ => ImageKind::Iso,
    }
}

fn image_kind_name(kind: ImageKind) -> &'static str {
    match kind {
        ImageKind::Iso => "iso",
        ImageKind::Img => "img",
        ImageKind::Disk => "disk",
        ImageKind::Archive => "archive",
    }
}

fn infer_guest_os(path: &Path) -> &'static str {
    let value = path.to_string_lossy().to_ascii_lowercase();
    if value.contains("freebsd") || value.contains("ghostbsd") {
        "freebsd"
    } else if value.contains("reactos") {
        "reactos"
    } else if value.contains("kolibrios") {
        "kolibrios"
    } else if value.contains("windows-server")
        || value.contains("eval_oemret")
        || value.contains("eval_x")
    {
        "windows-server"
    } else if value.contains("windows") || value.contains("win10") || value.contains("win11") {
        "windows"
    } else {
        "linux"
    }
}

fn config_tweaks(os: &str, release: &str) -> Vec<(&'static str, &'static str)> {
    match os {
        "archlinux" => vec![("secureboot", "on"), ("tpm", "on"), ("disk_size", "32G")],
        "debian" => {
            let mut tweaks = vec![("secureboot", "on")];
            if release.starts_with("12") || release.starts_with("13") {
                tweaks.push(("tpm", "on"));
            }
            tweaks
        }
        "deepin" => vec![("ram", "4G")],
        "freedos" => vec![("ram", "256M")],
        "kolibrios" => vec![("ram", "128M")],
        "proxmox-ve" => vec![("ram", "4G")],
        "reactos" => vec![("ram", "2048M")],
        "slitaz" => vec![("ram", "512M")],
        "ubuntu-server" => {
            let mut tweaks = vec![("ram", "4G")];
            if release == "22.04" {
                tweaks.push(("tpm", "on"));
            }
            tweaks
        }
        "elementary" if release == "8.1" => vec![("display", "spice")],
        "macos" if release == "monterey" => vec![("cpu_cores", "2")],
        _ => Vec::new(),
    }
}

fn disk_size(os: &str, edition: Option<&str>) -> Option<&'static str> {
    match os {
        "alma" | "centos-stream" | "endless" | "garuda" | "gentoo" | "kali" | "nixos"
        | "oraclelinux" | "popos" | "rockylinux" => Some("32G"),
        "batocera" => Some("8G"),
        "bazzite" => Some("64G"),
        "deepin" => Some("64G"),
        "freedos" => Some("4G"),
        "kolibrios" => Some("2G"),
        "macos" => Some("128G"),
        "openindiana" => Some("32G"),
        "proxmox-ve" => Some("20G"),
        "reactos" => Some("12G"),
        "slint" => Some("50G"),
        "slitaz" => Some("4G"),
        "ubuntu-server" => Some("10G"),
        "vanillaos" => Some("64G"),
        "windows" | "windows-server" => Some("64G"),
        "zorin" if edition == Some("education64") => Some("32G"),
        _ => None,
    }
}

fn guest_os(os: &str, release: &str) -> &'static str {
    if is_ubuntu_family(os)
        && os != "ubuntu-server"
        && !release.contains("daily")
        && release
            .replace('.', "")
            .parse::<u32>()
            .is_ok_and(|version| version < 1604)
    {
        "linux_old"
    } else {
        find_os(os).map_or("linux", |info| info.guest_os)
    }
}

fn suggested_name(os: &str, release: &str, edition: Option<&str>, architecture: &str) -> String {
    let mut name = format!("{os}-{release}");
    if let Some(edition) = edition {
        name.push('-');
        name.push_str(&edition.replace([' ', '(', ')'], "-"));
    }
    if architecture != host_architecture() {
        name.push('-');
        name.push_str(architecture);
    }
    name.trim_end_matches('-').to_string()
}

fn input_file_name(input: &str) -> Result<String> {
    if input.starts_with("http://") || input.starts_with("https://") {
        let path = input.split(['?', '#']).next().unwrap_or(input);
        let name = path.rsplit('/').next().unwrap_or_default();
        if !name.is_empty() && name != "." && name != ".." {
            return Ok(name.to_string());
        }
        return Err(Error::message("image URL does not contain a file name"));
    }
    let path = Path::new(input);
    path.file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty() && *value != "." && *value != "..")
        .map(str::to_string)
        .ok_or_else(|| Error::message("image path does not contain a file name"))
}

fn validate_vm_name(name: &str) -> Result<&str> {
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.chars().any(|character| {
            character == '/'
                || character == '\\'
                || character.is_control()
                || character == '='
                || character == ','
        })
    {
        return Err(Error::message(
            "VM name contains unsafe path or process-name characters",
        ));
    }
    Ok(name)
}

fn find_os(os: &str) -> Result<OsInfo> {
    let os = os.to_ascii_lowercase();
    OS_CATALOG
        .iter()
        .find(|info| info.id == os)
        .copied()
        .ok_or_else(|| {
            Error::message(format!(
                "unsupported OS '{os}' (use --list to see supported systems)"
            ))
        })
}

fn normalize_architecture(value: &str) -> Result<&'static str> {
    match value.to_ascii_lowercase().as_str() {
        "amd64" | "x86_64" => Ok("amd64"),
        "arm64" | "aarch64" => Ok("arm64"),
        _ => Err(Error::message(
            "architecture must be amd64, x86_64, arm64, or aarch64",
        )),
    }
}

fn qemu_architecture(value: &str) -> &str {
    if value == "arm64" {
        "aarch64"
    } else {
        "x86_64"
    }
}

fn host_architecture() -> &'static str {
    if cfg!(any(target_arch = "aarch64", target_arch = "arm")) {
        "arm64"
    } else {
        "amd64"
    }
}

fn file_name_from_url(url: &str) -> Option<String> {
    let path = url.split(['?', '#']).next()?;
    let name = path.rsplit('/').next()?;
    (name.contains('.') && !name.is_empty()).then(|| name.to_string())
}

fn dynamic_url_error(os: &str) -> Error {
    Error::message(format!(
        "{os} uses a dynamic provider URL; use --create-config VM_NAME IMAGE_PATH_OR_URL, or choose an OS with a stable URL template"
    ))
}

fn required_arg<'a>(value: Option<&'a str>, name: &str) -> Result<&'a str> {
    value.ok_or_else(|| Error::message(format!("{name} is required")))
}

fn config_value(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn csv_field(value: &str) -> String {
    if value.contains([',', '"', '\n']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn normalizes_image_architectures() {
        assert_eq!(normalize_architecture("x86_64").unwrap(), "amd64");
        assert_eq!(normalize_architecture("aarch64").unwrap(), "arm64");
        assert!(normalize_architecture("ppc64le").is_err());
    }

    #[test]
    fn config_values_are_shell_safe() {
        assert_eq!(config_value(r#"a\b"c"#), r#"a\\b\"c"#);
    }

    #[test]
    fn catalog_contains_supported_systems() {
        assert_eq!(find_os("ubuntu").unwrap().name, "Ubuntu");
        assert_eq!(
            find_os("windows-server").unwrap().guest_os,
            "windows-server"
        );
        assert_eq!(find_os("kdelinux").unwrap().releases, "latest");
        assert!(required_edition(find_os("kdeneon").unwrap(), None).is_err());
        assert!(find_os("not-an-os").is_err());
    }

    #[test]
    fn get_without_release_shows_os_options() {
        let args = GetArgs {
            os: Some("freebsd".to_string()),
            ..GetArgs::default()
        };
        assert_eq!(select_operation(&args).unwrap(), Operation::Show);

        let args = GetArgs {
            os: Some("freebsd".to_string()),
            release_or_input: Some("15.1".to_string()),
            ..GetArgs::default()
        };
        assert_eq!(select_operation(&args).unwrap(), Operation::CreateVm);
    }

    #[test]
    fn parses_freebsd_release_directories() {
        let listing = r#"
            <a href="../">Parent directory</a>
            <a HREF = '14.4/'>14.4/</a>
            <a href=15.1/>15.1/</a>
            <a href="15.1/">15.1/</a>
            <a href="README.TXT">README.TXT</a>
        "#;
        assert_eq!(
            freebsd_releases_from_listing(listing),
            vec!["14.4".to_string(), "15.1".to_string()]
        );
        assert!(freebsd_release_is_available(
            "15.1",
            "FreeBSD-15.1-RELEASE-amd64-disc1.iso FreeBSD-15.1-RELEASE-amd64-dvd1.iso"
        ));
        assert!(!freebsd_release_is_available(
            "14.5",
            "FreeBSD-14.5-BETA2-amd64-disc1.iso FreeBSD-14.5-BETA2-amd64-dvd1.iso"
        ));
    }

    #[test]
    fn recognizes_kde_linux_release_images() {
        assert!(is_kde_linux_iso("kde-linux_202608171234.iso"));
        assert!(!is_kde_linux_iso("kde-linux_latest.iso"));
        assert!(!is_kde_linux_iso("kde-linux_20260817123.iso"));
    }

    #[test]
    fn validates_windows_download_handshake_tokens() {
        let response = r#"window.location='?w=ABC123&rticks=+456';"#;
        assert_eq!(
            windows_ov_df_value(response, "w", |character| character.is_ascii_hexdigit()),
            Some("ABC123".to_string())
        );
        assert_eq!(
            windows_ov_df_value(response, "rticks", |character| character.is_ascii_digit()),
            Some("456".to_string())
        );
        assert!(
            windows_ov_df_value("rticks=not-a-number", "rticks", |character| {
                character.is_ascii_digit()
            })
            .is_none()
        );
    }

    #[test]
    fn insecure_curl_mode_is_explicit() {
        assert_eq!(curl_security_args(false), &[] as &[&str]);
        assert_eq!(curl_security_args(true), &["--insecure"]);
    }

    #[test]
    fn rejects_get_flags_that_do_not_apply_to_the_selected_operation() {
        let mut args = GetArgs {
            arch: Some("amd64".to_string()),
            ..GetArgs::default()
        };
        let error = validate_operation_arguments(&args, Operation::Show, false).unwrap_err();
        assert_eq!(error.code(), "invalid_argument");

        args.arch = None;
        assert!(validate_operation_arguments(&args, Operation::Show, true).is_err());
        args.insecure = true;
        assert!(
            validate_operation_arguments(
                &args,
                Operation::Check {
                    all_architectures: false
                },
                true
            )
            .is_ok()
        );
    }

    #[test]
    fn generated_configs_get_current_arch_and_debian_defaults() {
        assert_eq!(
            config_tweaks("archlinux", "latest"),
            vec![("secureboot", "on"), ("tpm", "on"), ("disk_size", "32G")]
        );
        assert_eq!(
            config_tweaks("debian", "12"),
            vec![("secureboot", "on"), ("tpm", "on")]
        );
        assert_eq!(config_tweaks("debian", "11"), vec![("secureboot", "on")]);
    }

    #[test]
    fn windows_and_legacy_guest_defaults_are_explicit() {
        assert_eq!(
            required_edition(find_os("windows").unwrap(), None).unwrap(),
            Some("English International".to_string())
        );
        assert_eq!(guest_os("ubuntu", "14.04"), "linux_old");
        assert_eq!(guest_os("ubuntu", "24.04"), "linux");
        assert!(WINDOWS_UNATTENDED_XML.contains("<unattend"));
    }

    #[test]
    fn stable_url_templates_are_pure() {
        let image = resolve_image("ubuntu", "24.04", None, "amd64").unwrap();
        assert_eq!(image.file_name, "ubuntu-24.04-desktop-amd64.iso");
        assert!(image.url.starts_with("https://"));
        let freebsd = resolve_image("freebsd", "15.1", Some("disc1"), "amd64").unwrap();
        assert!(freebsd.url.starts_with(FREEBSD_ISO_IMAGES));
        assert_eq!(
            resolve_image("Ubuntu", "24.04", None, "amd64").unwrap().os,
            "ubuntu"
        );
        assert!(!ubuntu_arm64_release("24.04"));
        assert!(ubuntu_arm64_release("25.10"));
        assert!(resolve_image("debian", "12", Some("standard"), "arm64").is_err());
    }

    #[test]
    fn parses_provider_checksums_without_shell_parsing() {
        let hash = "a".repeat(128);
        let sums = format!("{hash}  image.iso\n");
        assert_eq!(
            checksum_from_text(&sums, "image.iso", "sha512"),
            Some(format!("sha512:{hash}"))
        );
        assert_eq!(
            first_token("href=\"https://example.test/image.iso\"", |value| {
                value.ends_with(".iso")
            }),
            Some("https://example.test/image.iso".to_string())
        );
    }

    #[test]
    fn unsafe_custom_names_are_rejected() {
        assert!(validate_vm_name("../vm").is_err());
        assert!(validate_vm_name("good-vm").is_ok());
    }

    #[test]
    fn generated_config_is_relative_and_not_overwritten() {
        let root = tempdir().unwrap();
        let image_dir = root.path().join("ubuntu-24.04");
        fs::create_dir_all(&image_dir).unwrap();
        let image = image_dir.join("ubuntu.iso");
        fs::write(&image, b"test").unwrap();

        let config = write_vm_config(
            root.path(),
            "ubuntu-24.04",
            "ubuntu",
            "24.04",
            None,
            "amd64",
            &image,
        )
        .unwrap();
        let contents = fs::read_to_string(&config).unwrap();
        assert!(contents.contains("iso=\"ubuntu-24.04/ubuntu.iso\""));
        assert!(contents.contains("disk_img=\"ubuntu-24.04/disk.qcow2\""));
        assert!(
            write_vm_config(
                root.path(),
                "ubuntu-24.04",
                "ubuntu",
                "24.04",
                None,
                "amd64",
                &image,
            )
            .is_err()
        );
    }

    #[test]
    fn custom_config_copies_a_local_image_without_sourcing_it() {
        let root = tempdir().unwrap();
        let source = root.path().join("installer.iso");
        fs::write(&source, b"not a shell script").unwrap();
        let vm_dir = root.path().join("vms");
        let dirs = Dirs {
            vm_dir: vm_dir.clone(),
            state_root: root.path().join("state"),
        };
        let args = GetArgs {
            os: Some("demo".to_string()),
            release_or_input: Some(source.display().to_string()),
            ..GetArgs::default()
        };

        create_custom_config(&args, &dirs, OutputFormat::Json).unwrap();
        let config = vm_dir.join("demo.conf");
        assert!(config.is_file());
        assert!(vm_dir.join("demo/installer.iso").is_file());
        assert!(
            fs::read_to_string(config)
                .unwrap()
                .contains("guest_os=\"linux\"")
        );
    }

    #[test]
    fn checksum_verification_rejects_tampering() {
        let root = tempdir().unwrap();
        let image = root.path().join("image.iso");
        fs::write(&image, b"test").unwrap();
        assert!(
            verify_checksum(
                &image,
                Some("9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08")
            )
            .is_ok()
        );
        assert!(verify_checksum(&image, Some("deadbeef")).is_err());
    }
}
