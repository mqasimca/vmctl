use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::paths::VmPaths;

mod parser;

use parser::{
    optional_bool, optional_path, optional_port, optional_string, optional_u32, parse_port,
    parse_port_forwards, parse_public_dir, parse_usb_devices, resolve_path, setting_bool,
    valid_host_or_address, valid_network_name, validate_disk_format_value, validate_one_of,
    value_or,
};
pub use parser::{parse_config, parse_tokens};

#[derive(Debug, Clone)]
pub struct VmConfig {
    pub name: String,
    pub config_path: PathBuf,
    pub guest_os: String,
    pub arch: String,
    pub disk_img: PathBuf,
    pub disk_format: String,
    pub disk_size: String,
    pub preallocation: String,
    pub iso: Option<PathBuf>,
    pub fixed_iso: Option<PathBuf>,
    pub unattended_iso: Option<PathBuf>,
    pub cloud_base_img: Option<PathBuf>,
    pub cloud_init_iso: Option<PathBuf>,
    pub floppy: Option<PathBuf>,
    pub img: Option<PathBuf>,
    pub macos_release: Option<String>,
    pub boot: String,
    pub ram: Option<String>,
    pub cpu_cores: Option<u32>,
    pub cpu_model: Option<String>,
    pub display: String,
    pub viewer: String,
    pub access: String,
    pub allow_insecure_remote: bool,
    pub ssh_access: String,
    pub ssh_user: Option<String>,
    pub viewer_extra_args: Vec<String>,
    pub gl: Option<bool>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub max_outputs: Option<u32>,
    pub fullscreen: bool,
    pub clipboard: bool,
    pub braille: bool,
    pub secureboot: bool,
    pub ssh_port: Option<u16>,
    pub spice_port: Option<u16>,
    pub public_dir: Option<PathBuf>,
    pub network: String,
    pub offline: bool,
    pub bridge: Option<String>,
    pub macaddr: Option<String>,
    pub port_forwards: Vec<(u16, u16)>,
    pub usb_devices: Vec<(u16, u16)>,
    pub guest_agent: bool,
    pub monitor: String,
    pub monitor_cmd: Option<String>,
    pub monitor_telnet_port: u16,
    pub monitor_telnet_host: String,
    pub serial: String,
    pub serial_telnet_port: u16,
    pub serial_telnet_host: String,
    pub usb_controller: String,
    pub keyboard: String,
    pub keyboard_layout: String,
    pub mouse: String,
    pub sound_card: String,
    pub sound_duplex: String,
    pub tpm: bool,
    pub status_quo: bool,
    pub ignore_tsc_warning: bool,
    pub cpu_pinning: Option<String>,
    pub extra_args: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Vm {
    pub config: VmConfig,
    pub paths: VmPaths,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VmState {
    Running(i32),
    Stopped,
}

impl Vm {
    pub fn state(&self) -> Result<VmState> {
        let contents = match fs::read_to_string(self.paths.pid_file()) {
            Ok(contents) => contents,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(VmState::Stopped);
            }
            Err(error) => return Err(Error::io(self.paths.pid_file().display(), error)),
        };
        let mut fields = contents.split_whitespace();
        let pid_text = fields.next().ok_or_else(|| {
            Error::message(format!(
                "invalid PID in {}: no PID found",
                self.paths.pid_file().display()
            ))
        })?;
        let pid = pid_text.parse::<i32>().map_err(|error| {
            Error::message(format!(
                "invalid PID in {}: {error}",
                self.paths.pid_file().display()
            ))
        })?;
        let identity = fields.next();
        if crate::qemu::process_matches_checked_with_identity(pid, &self.config.name, identity)? {
            Ok(VmState::Running(pid))
        } else {
            Ok(VmState::Stopped)
        }
    }
}

pub fn discover(root: &Path, state_root: &Path) -> Result<Vec<Vm>> {
    if !root.is_dir() {
        return Ok(Vec::new());
    }

    let entries = fs::read_dir(root).map_err(|error| Error::io(root.display(), error))?;
    let mut vms = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| Error::io(root.display(), error))?;
        let path = entry.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("conf") {
            continue;
        }
        vms.push(load_vm(root, state_root, path)?);
    }

    vms.sort_by(|left, right| left.config.name.cmp(&right.config.name));
    Ok(vms)
}

pub fn find(root: &Path, state_root: &Path, name_or_path: &str) -> Result<Vm> {
    let candidate = Path::new(name_or_path);
    if candidate.is_file() {
        let config_path =
            fs::canonicalize(candidate).map_err(|error| Error::io(candidate.display(), error))?;
        let config_root = config_path.parent().unwrap_or(root).to_path_buf();
        return load_vm(&config_root, state_root, config_path);
    }

    let wanted = name_or_path.strip_suffix(".conf").unwrap_or(name_or_path);
    discover(root, state_root)?
        .into_iter()
        .find(|vm| vm.config.name == wanted)
        .ok_or_else(|| Error::vm_not_found(name_or_path, root))
}

pub fn load_vm(root: &Path, state_root: &Path, config_path: PathBuf) -> Result<Vm> {
    let name = config_path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|stem| !stem.is_empty())
        .ok_or_else(|| {
            Error::message(format!(
                "invalid VM configuration path {}",
                config_path.display()
            ))
        })?
        .to_string();
    if name == "."
        || name == ".."
        || name.chars().any(|character| {
            character == ',' || character == '=' || character == '\\' || character.is_control()
        })
    {
        return Err(Error::message(format!(
            "VM name '{name}' contains characters unsafe for QEMU process naming"
        )));
    }
    let contents = fs::read_to_string(&config_path)
        .map_err(|error| Error::io(config_path.display(), error))?;
    let values = parse_config(&contents);
    let config = VmConfig::from_values(name, config_path, root, &values)?;
    let paths = VmPaths::new(state_root, &config.name);
    Ok(Vm { config, paths })
}

impl VmConfig {
    fn from_values(
        name: String,
        config_path: PathBuf,
        root: &Path,
        values: &BTreeMap<String, String>,
    ) -> Result<Self> {
        let guest_os = value_or(values, "guest_os", "linux").to_ascii_lowercase();
        let serial = values
            .get("serial")
            .filter(|value| !value.is_empty())
            .cloned()
            .unwrap_or_else(|| {
                if matches!(guest_os.as_str(), "windows" | "windows-server" | "macos") {
                    "none".to_string()
                } else {
                    "socket".to_string()
                }
            });
        let braille = setting_bool(values, "braille", false, &config_path)?;
        let display = if braille {
            "sdl".to_string()
        } else {
            value_or(values, "display", "gtk").to_ascii_lowercase()
        };
        let usb_controller = if braille {
            "xhci".to_string()
        } else if guest_os == "solaris"
            || matches!(
                values.get("macos_release").map(String::as_str),
                Some("big-sur" | "monterey" | "ventura" | "sonoma" | "sequoia" | "tahoe")
            )
        {
            value_or(values, "usb_controller", "xhci").to_ascii_lowercase()
        } else {
            value_or(values, "usb_controller", "ehci").to_ascii_lowercase()
        };
        let sound_card = values
            .get("sound_card")
            .filter(|value| !value.is_empty())
            .cloned()
            .unwrap_or_else(|| {
                if guest_os == "freedos" {
                    "sb16".to_string()
                } else if guest_os == "solaris" {
                    "ac97".to_string()
                } else if matches!(
                    values.get("macos_release").map(String::as_str),
                    Some("monterey" | "ventura" | "sonoma" | "sequoia" | "tahoe")
                ) {
                    "virtio-sound-pci".to_string()
                } else {
                    "intel-hda".to_string()
                }
            })
            .to_ascii_lowercase();
        let disk_format = value_or(values, "disk_format", "qcow2").to_ascii_lowercase();
        validate_disk_format_value(&config_path, &disk_format)?;

        let config = Self {
            disk_img: values
                .get("disk_img")
                .filter(|value| !value.is_empty() && value.as_str() != "none")
                .map(|value| resolve_path(root, value))
                .transpose()?
                .unwrap_or_else(|| root.join(&name).join(format!("disk.{disk_format}"))),
            name,
            config_path: config_path.clone(),
            guest_os: guest_os.clone(),
            arch: value_or(values, "arch", "x86_64").to_ascii_lowercase(),
            disk_format,
            disk_size: value_or(values, "disk_size", "16G"),
            preallocation: value_or(values, "preallocation", "off").to_ascii_lowercase(),
            iso: optional_path(root, values.get("iso"))?,
            fixed_iso: optional_path(root, values.get("fixed_iso"))?,
            unattended_iso: optional_path(root, values.get("unattended_iso"))?,
            cloud_base_img: optional_path(root, values.get("cloud_base_img"))?,
            cloud_init_iso: optional_path(root, values.get("cloud_init_iso"))?,
            floppy: optional_path(root, values.get("floppy"))?,
            img: optional_path(root, values.get("img"))?,
            macos_release: optional_string(values, "macos_release"),
            boot: value_or(values, "boot", "efi").to_ascii_lowercase(),
            ram: optional_string(values, "ram"),
            cpu_cores: optional_u32(values, "cpu_cores", &config_path)?,
            cpu_model: optional_string(values, "cpu_model"),
            display,
            viewer: value_or(values, "viewer", "remote-viewer").to_ascii_lowercase(),
            access: value_or(values, "access", "local").to_ascii_lowercase(),
            allow_insecure_remote: setting_bool(
                values,
                "allow_insecure_remote",
                false,
                &config_path,
            )?,
            ssh_access: value_or(values, "ssh_access", "local").to_ascii_lowercase(),
            ssh_user: optional_string(values, "ssh_user"),
            viewer_extra_args: parse_tokens(values.get("viewer_extra_args")),
            gl: optional_bool(values, "gl", &config_path)?,
            width: optional_u32(values, "width", &config_path)?,
            height: optional_u32(values, "height", &config_path)?,
            max_outputs: optional_u32(values, "max_outputs", &config_path)?,
            fullscreen: setting_bool(values, "fullscreen", false, &config_path)?,
            clipboard: setting_bool(values, "clipboard", false, &config_path)?,
            braille,
            secureboot: setting_bool(values, "secureboot", false, &config_path)?,
            ssh_port: optional_port(values, "ssh_port", &config_path)?,
            spice_port: optional_port(values, "spice_port", &config_path)?,
            public_dir: parse_public_dir(root, values)?,
            network: value_or(values, "network", ""),
            offline: false,
            bridge: optional_string(values, "bridge"),
            macaddr: optional_string(values, "macaddr"),
            port_forwards: parse_port_forwards(values.get("port_forwards"), &config_path)?,
            usb_devices: parse_usb_devices(values.get("usb_devices"), &config_path)?,
            guest_agent: setting_bool(values, "guest_agent", true, &config_path)?,
            monitor: value_or(values, "monitor", "socket").to_ascii_lowercase(),
            monitor_cmd: optional_string(values, "monitor_cmd"),
            monitor_telnet_port: parse_port(
                &value_or(values, "monitor_telnet_port", "4440"),
                &config_path,
                "monitor_telnet_port",
            )?,
            monitor_telnet_host: value_or(values, "monitor_telnet_host", "localhost"),
            serial,
            serial_telnet_port: parse_port(
                &value_or(values, "serial_telnet_port", "6660"),
                &config_path,
                "serial_telnet_port",
            )?,
            serial_telnet_host: value_or(values, "serial_telnet_host", "localhost"),
            usb_controller: if sound_card == "usb-audio" {
                "xhci".to_string()
            } else {
                usb_controller
            },
            keyboard: value_or(
                values,
                "keyboard",
                if guest_os == "reactos" { "ps2" } else { "usb" },
            )
            .to_ascii_lowercase(),
            keyboard_layout: value_or(values, "keyboard_layout", "en-us"),
            mouse: value_or(
                values,
                "mouse",
                if matches!(guest_os.as_str(), "freebsd" | "ghostbsd") {
                    "usb"
                } else {
                    "tablet"
                },
            )
            .to_ascii_lowercase(),
            sound_card,
            sound_duplex: value_or(values, "sound_duplex", "hda-micro").to_ascii_lowercase(),
            tpm: setting_bool(values, "tpm", false, &config_path)?,
            status_quo: false,
            ignore_tsc_warning: setting_bool(values, "ignore_tsc_warning", false, &config_path)?,
            cpu_pinning: None,
            extra_args: parse_tokens(values.get("extra_args")),
        };
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<()> {
        validate_one_of(
            &self.config_path,
            "arch",
            &self.arch,
            &["x86_64", "aarch64"],
        )?;
        if let Some(user) = &self.ssh_user
            && (user.is_empty()
                || user.len() > 32
                || !user.chars().all(|character| {
                    character.is_ascii_alphanumeric() || matches!(character, '-' | '_')
                }))
        {
            return Err(Error::config(
                &self.config_path,
                "ssh_user must contain only letters, digits, hyphens, and underscores",
            ));
        }
        validate_one_of(&self.config_path, "boot", &self.boot, &["efi", "legacy"])?;
        if self.guest_os == "macos" && self.boot != "efi" {
            return Err(Error::config(
                &self.config_path,
                "macOS guests require EFI boot",
            ));
        }
        if self.guest_os == "macos" && self.arch != "x86_64" {
            return Err(Error::config(
                &self.config_path,
                "macOS guests currently require x86_64 QEMU",
            ));
        }
        validate_one_of(
            &self.config_path,
            "display",
            &self.display,
            &["gtk", "sdl", "cocoa", "none", "spice", "spice-app"],
        )?;
        if self.clipboard && self.display != "gtk" {
            return Err(Error::config(
                &self.config_path,
                "clipboard requires display=gtk",
            ));
        }
        validate_one_of(
            &self.config_path,
            "viewer",
            &self.viewer,
            &["spicy", "remote-viewer", "none"],
        )?;
        validate_one_of(
            &self.config_path,
            "monitor",
            &self.monitor,
            &["socket", "telnet", "none"],
        )?;
        validate_one_of(
            &self.config_path,
            "serial",
            &self.serial,
            &["socket", "telnet", "none"],
        )?;
        validate_one_of(
            &self.config_path,
            "usb_controller",
            &self.usb_controller,
            &["ehci", "xhci", "none"],
        )?;
        validate_one_of(
            &self.config_path,
            "keyboard",
            &self.keyboard,
            &["usb", "ps2", "virtio"],
        )?;
        validate_one_of(
            &self.config_path,
            "mouse",
            &self.mouse,
            &["tablet", "ps2", "usb", "virtio"],
        )?;
        validate_one_of(
            &self.config_path,
            "preallocation",
            &self.preallocation,
            &["off", "metadata", "falloc", "full"],
        )?;
        for (key, value) in [("access", &self.access), ("ssh_access", &self.ssh_access)] {
            if !matches!(value.as_str(), "local" | "remote") && !valid_host_or_address(value) {
                return Err(Error::config(
                    &self.config_path,
                    format!("{key} must be local, remote, or a host/IP address"),
                ));
            }
        }
        for (key, value) in [
            ("network", Some(self.network.as_str())),
            ("bridge", self.bridge.as_deref()),
        ] {
            if let Some(value) = value.filter(|value| !value.is_empty())
                && !valid_network_name(value)
            {
                return Err(Error::config(
                    &self.config_path,
                    format!("{key} contains QEMU option separators or whitespace"),
                ));
            }
        }
        for (key, value) in [
            ("monitor_telnet_host", &self.monitor_telnet_host),
            ("serial_telnet_host", &self.serial_telnet_host),
        ] {
            if !valid_host_or_address(value) {
                return Err(Error::config(
                    &self.config_path,
                    format!("{key} must be a host or IP address without QEMU option separators"),
                ));
            }
        }
        validate_one_of(
            &self.config_path,
            "sound_card",
            &self.sound_card,
            &[
                "ich9-intel-hda",
                "intel-hda",
                "ac97",
                "es1370",
                "sb16",
                "usb-audio",
                "virtio-sound-pci",
                "none",
            ],
        )?;
        validate_one_of(
            &self.config_path,
            "sound_duplex",
            &self.sound_duplex,
            &["hda-micro", "hda-duplex", "hda-output"],
        )?;
        if self.monitor_telnet_port == 0 || self.serial_telnet_port == 0 {
            return Err(Error::config(
                &self.config_path,
                "telnet ports must be greater than zero",
            ));
        }
        if self.ssh_port == Some(0) || self.spice_port == Some(0) {
            return Err(Error::config(
                &self.config_path,
                "ssh and SPICE ports must be greater than zero",
            ));
        }
        if let Some(macaddr) = &self.macaddr
            && (macaddr.split(':').count() != 6
                || macaddr
                    .split(':')
                    .any(|part| part.len() != 2 || u8::from_str_radix(part, 16).is_err()))
        {
            return Err(Error::config(
                &self.config_path,
                "macaddr must be six hexadecimal octets",
            ));
        }
        if self.monitor_cmd.is_some() && self.monitor == "none" {
            return Err(Error::config(
                &self.config_path,
                "monitor_cmd requires an enabled monitor",
            ));
        }
        if self.usb_controller == "none"
            && (self.keyboard == "usb" || matches!(self.mouse.as_str(), "usb" | "tablet"))
        {
            return Err(Error::config(
                &self.config_path,
                "USB keyboard or mouse requires a USB controller",
            ));
        }
        if self.width.is_some() != self.height.is_some() {
            return Err(Error::config(
                &self.config_path,
                "width and height must be provided together",
            ));
        }
        if self.cpu_cores == Some(0)
            || self.width == Some(0)
            || self.height == Some(0)
            || self.max_outputs == Some(0)
        {
            return Err(Error::config(
                &self.config_path,
                "cpu_cores, width, height, and max_outputs must be greater than zero",
            ));
        }
        if self.disk_format.is_empty() || self.disk_size.is_empty() {
            return Err(Error::config(
                &self.config_path,
                "disk_format and disk_size must not be empty",
            ));
        }
        validate_disk_format_value(&self.config_path, &self.disk_format)?;
        if let Some(argument) = self
            .extra_args
            .iter()
            .find(|argument| unsafe_extra_argument(argument))
        {
            return Err(Error::config(
                &self.config_path,
                format!(
                    "extra_args contains '{argument}', which can override vmctl safety controls; use a supported configuration field"
                ),
            ));
        }
        Ok(())
    }
}

fn unsafe_extra_argument(argument: &str) -> bool {
    let option = argument
        .split_once('=')
        .map_or(argument, |(option, _)| option);
    matches!(
        option,
        "-name"
            | "-pidfile"
            | "-daemonize"
            | "-display"
            | "-spice"
            | "-qmp"
            | "-mon"
            | "-monitor"
            | "-serial"
            | "-drive"
            | "-blockdev"
            | "-device"
            | "-object"
            | "-fsdev"
            | "-netdev"
            | "-nic"
            | "-chardev"
            | "-bios"
            | "-global"
            | "-S"
    )
}

#[cfg(test)]
mod tests;
