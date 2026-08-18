use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::paths::{VmPaths, default_public_dir};

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
    pub viewer_extra_args: Vec<String>,
    pub gl: Option<bool>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub max_outputs: Option<u32>,
    pub fullscreen: bool,
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
            viewer_extra_args: parse_tokens(values.get("viewer_extra_args")),
            gl: optional_bool(values, "gl", &config_path)?,
            width: optional_u32(values, "width", &config_path)?,
            height: optional_u32(values, "height", &config_path)?,
            max_outputs: optional_u32(values, "max_outputs", &config_path)?,
            fullscreen: setting_bool(values, "fullscreen", false, &config_path)?,
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

fn validate_disk_format_value(path: &Path, format: &str) -> Result<()> {
    if format.is_empty()
        || format.starts_with('-')
        || format.chars().any(|character| {
            character.is_ascii_control()
                || character.is_ascii_whitespace()
                || !(character.is_ascii_alphanumeric() || ".-_".contains(character))
        })
    {
        return Err(Error::config(
            path,
            format!("disk_format '{format}' contains unsafe QEMU option characters"),
        ));
    }
    Ok(())
}

pub fn parse_config(contents: &str) -> BTreeMap<String, String> {
    contents
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with("#!") {
                return None;
            }
            let line = strip_comment(line);
            let line = line.trim();
            let line = line.strip_prefix("export ").unwrap_or(line);
            let (key, value) = line.split_once('=')?;
            let key = key.trim();
            if key.is_empty()
                || !key
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric() || character == '_')
            {
                return None;
            }
            Some((key.to_string(), unquote(value.trim())))
        })
        .collect()
}

pub fn parse_tokens(value: Option<&String>) -> Vec<String> {
    let Some(value) = value else {
        return Vec::new();
    };
    let value = value.trim();
    let value = value
        .strip_prefix('(')
        .and_then(|value| value.strip_suffix(')'))
        .unwrap_or(value);

    let mut tokens = Vec::new();
    let mut token = String::new();
    let mut quote = None;
    let mut escaped = false;

    for character in value.chars() {
        if escaped {
            token.push(character);
            escaped = false;
            continue;
        }
        if character == '\\' {
            escaped = true;
            continue;
        }
        match (quote, character) {
            (Some(current), character) if character == current => quote = None,
            (Some(_), character) => token.push(character),
            (None, '\'' | '"') => quote = Some(character),
            (None, character) if character.is_whitespace() => {
                if !token.is_empty() {
                    tokens.push(std::mem::take(&mut token));
                }
            }
            (None, character) => token.push(character),
        }
    }
    if escaped {
        token.push('\\');
    }
    if !token.is_empty() {
        tokens.push(token);
    }
    tokens
}

fn parse_port_forwards(value: Option<&String>, path: &Path) -> Result<Vec<(u16, u16)>> {
    parse_tokens(value)
        .into_iter()
        .map(|forward| {
            let (host, guest) = forward
                .split_once(':')
                .ok_or_else(|| Error::config(path, format!("invalid port forward '{forward}'")))?;
            let host = parse_port(host, path, "host port")?;
            let guest = parse_port(guest, path, "guest port")?;
            Ok((host, guest))
        })
        .collect()
}

fn parse_usb_devices(value: Option<&String>, path: &Path) -> Result<Vec<(u16, u16)>> {
    parse_tokens(value)
        .into_iter()
        .map(|device| {
            let (vendor, product) = device
                .split_once(':')
                .ok_or_else(|| Error::config(path, format!("invalid USB device '{device}'")))?;
            let vendor = u16::from_str_radix(vendor, 16)
                .map_err(|_| Error::config(path, format!("invalid USB vendor in '{device}'")))?;
            let product = u16::from_str_radix(product, 16)
                .map_err(|_| Error::config(path, format!("invalid USB product in '{device}'")))?;
            Ok((vendor, product))
        })
        .collect()
}

fn strip_comment(value: &str) -> &str {
    let mut quote = None;
    let mut escaped = false;
    for (index, character) in value.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if character == '\\' {
            escaped = true;
            continue;
        }
        match (quote, character) {
            (Some(current), character) if current == character => quote = None,
            (None, '\'' | '"') => quote = Some(character),
            (None, '#') => return &value[..index],
            _ => {}
        }
    }
    value
}

fn unquote(value: &str) -> String {
    let value = if value.len() >= 2 {
        let quoted = (value.starts_with('"') && value.ends_with('"'))
            || (value.starts_with('\'') && value.ends_with('\''));
        if quoted {
            &value[1..value.len() - 1]
        } else {
            value
        }
    } else {
        value
    };

    let mut result = String::with_capacity(value.len());
    let mut escaped = false;
    for character in value.chars() {
        if escaped {
            result.push(character);
            escaped = false;
        } else if character == '\\' {
            escaped = true;
        } else {
            result.push(character);
        }
    }
    if escaped {
        result.push('\\');
    }
    result
}

fn value_or(values: &BTreeMap<String, String>, key: &str, default: &str) -> String {
    values
        .get(key)
        .cloned()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| default.to_string())
}

fn optional_string(values: &BTreeMap<String, String>, key: &str) -> Option<String> {
    values
        .get(key)
        .cloned()
        .filter(|value| !value.is_empty() && value != "none")
}

fn optional_path(root: &Path, value: Option<&String>) -> Result<Option<PathBuf>> {
    value
        .filter(|value| !value.is_empty() && value.as_str() != "none")
        .map(|value| resolve_path(root, value))
        .transpose()
}

fn parse_public_dir(root: &Path, values: &BTreeMap<String, String>) -> Result<Option<PathBuf>> {
    match values.get("public_dir") {
        Some(value) if value == "none" => Ok(None),
        Some(value) if !value.is_empty() => Ok(Some(resolve_path(root, value)?)),
        _ => default_public_dir(),
    }
}

fn resolve_path(root: &Path, value: &str) -> Result<PathBuf> {
    if value == "~" {
        return crate::paths::home_dir();
    }
    if let Some(value) = value.strip_prefix("~/") {
        return Ok(crate::paths::home_dir()?.join(value));
    }
    let path = Path::new(value);
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(root.join(path))
    }
}

fn parse_bool(value: &str) -> Option<bool> {
    match value.to_ascii_lowercase().as_str() {
        "1" | "on" | "true" | "yes" => Some(true),
        "0" | "off" | "false" | "no" => Some(false),
        _ => None,
    }
}

fn setting_bool(
    values: &BTreeMap<String, String>,
    key: &str,
    default: bool,
    path: &Path,
) -> Result<bool> {
    values.get(key).map_or(Ok(default), |value| {
        parse_bool(value).ok_or_else(|| Error::config(path, format!("{key} must be true or false")))
    })
}

fn optional_bool(
    values: &BTreeMap<String, String>,
    key: &str,
    path: &Path,
) -> Result<Option<bool>> {
    values.get(key).map_or(Ok(None), |value| {
        parse_bool(value)
            .map(Some)
            .ok_or_else(|| Error::config(path, format!("{key} must be true or false")))
    })
}

fn optional_u32(values: &BTreeMap<String, String>, key: &str, path: &Path) -> Result<Option<u32>> {
    values
        .get(key)
        .filter(|value| !value.is_empty())
        .map_or(Ok(None), |value| {
            value
                .parse()
                .map(Some)
                .map_err(|_| Error::config(path, format!("{key} must be a positive number")))
        })
}

fn optional_port(values: &BTreeMap<String, String>, key: &str, path: &Path) -> Result<Option<u16>> {
    values
        .get(key)
        .filter(|value| !value.is_empty())
        .map(|value| parse_port(value, path, key))
        .transpose()
}

fn parse_port(value: &str, path: &Path, label: &str) -> Result<u16> {
    let port = value
        .parse::<u16>()
        .map_err(|_| Error::config(path, format!("{label} must be a valid port number")))?;
    if port == 0 {
        return Err(Error::config(
            path,
            format!("{label} must be greater than zero"),
        ));
    }
    Ok(port)
}

fn validate_one_of(path: &Path, key: &str, value: &str, allowed: &[&str]) -> Result<()> {
    if allowed.contains(&value) {
        Ok(())
    } else {
        Err(Error::config(
            path,
            format!("{key} must be one of {}", allowed.join(", ")),
        ))
    }
}

fn valid_host_or_address(value: &str) -> bool {
    if value.parse::<std::net::IpAddr>().is_ok() {
        return true;
    }
    !value.is_empty()
        && value.len() <= 253
        && !value.chars().any(|character| {
            character.is_whitespace()
                || character.is_control()
                || matches!(character, ',' | '=' | '/' | '\\' | '[' | ']')
        })
        && value.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric() || character == '-')
        })
}

fn valid_network_name(value: &str) -> bool {
    !value.is_empty()
        && !value.chars().any(|character| {
            character.is_whitespace()
                || character.is_control()
                || matches!(character, ',' | '=' | '/' | '\\')
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn parses_config_without_executing_shell() {
        let values = parse_config(
            r##"#!/usr/bin/env vmctl --vm
guest_os="linux"
disk_img="ubuntu/disk.qcow2"
ssh_port="22220"
port_forwards=("8080:80" "8443:443")
"##,
        );

        assert_eq!(values.get("guest_os"), Some(&"linux".to_string()));
        assert_eq!(
            values.get("disk_img"),
            Some(&"ubuntu/disk.qcow2".to_string())
        );
        assert_eq!(
            parse_tokens(values.get("port_forwards")),
            ["8080:80", "8443:443"]
        );
    }

    #[test]
    fn accepts_default_empty_values() {
        let root = tempdir().unwrap();
        let path = root.path().join("ubuntu.conf");
        fs::write(
            &path,
            r#"boot="efi"
cpu_cores=""
disk_img=""
disk_size=""
display=""
guest_os="linux"
iso=""
ram=""
ssh_port=""
spice_port=""
monitor="socket"
serial=""
port_forwards=()
usb_devices=()
secureboot="off"
tpm="off"
"#,
        )
        .unwrap();

        let vm = load_vm(root.path(), root.path(), path).unwrap();
        assert_eq!(vm.config.disk_size, "16G");
        assert_eq!(vm.config.cpu_cores, None);
        assert_eq!(vm.config.ssh_port, None);
        assert_eq!(vm.config.serial, "socket");
        assert_eq!(vm.config.viewer, "remote-viewer");
    }

    #[test]
    fn tokenizes_quoted_values_and_escapes() {
        assert_eq!(
            parse_tokens(Some(&"-device 'virtio-rng-pci'".to_string())),
            vec!["-device", "virtio-rng-pci"]
        );
        assert_eq!(
            parse_tokens(Some(&r#"("path with spaces" plain\ value)"#.to_string())),
            vec!["path with spaces", "plain value"]
        );
    }

    #[test]
    fn strips_comments_only_outside_quotes() {
        let values = parse_config("name=one # comment\npath=\"a#b\"\n");
        assert_eq!(values.get("name"), Some(&"one".to_string()));
        assert_eq!(values.get("path"), Some(&"a#b".to_string()));
    }

    #[test]
    fn loads_paths_relative_to_the_config_directory() {
        let root = tempdir().unwrap();
        let config_path = root.path().join("ubuntu.conf");
        fs::write(
            &config_path,
            "boot=legacy\ndisk_img=ubuntu/disk.qcow2\nssh_port=22220\n",
        )
        .unwrap();

        let vm = load_vm(
            root.path(),
            root.path().join("state").as_path(),
            config_path,
        )
        .unwrap();
        assert_eq!(vm.config.disk_img, root.path().join("ubuntu/disk.qcow2"));
        assert_eq!(
            vm.paths.qmp_socket(),
            root.path().join("state/vms/ubuntu/qmp.sock")
        );
    }

    #[test]
    fn rejects_invalid_ports_and_modes() {
        let root = tempdir().unwrap();
        let path = root.path().join("broken.conf");
        fs::write(&path, "ssh_port=0\ndisplay=bad\n").unwrap();
        let error = load_vm(root.path(), root.path(), path).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("ssh_port must be greater than zero")
        );
    }

    #[test]
    fn rejects_network_option_injection() {
        let root = tempdir().unwrap();
        let path = root.path().join("broken-network.conf");
        fs::write(&path, "network=br0,helper=/tmp/helper\n").unwrap();
        let error = load_vm(root.path(), root.path(), path).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("network contains QEMU option separators")
        );

        let path = root.path().join("case-sensitive-bridge.conf");
        fs::write(&path, "network=Br0\n").unwrap();
        assert_eq!(
            load_vm(root.path(), root.path(), path)
                .unwrap()
                .config
                .network,
            "Br0"
        );
    }

    #[test]
    fn rejects_disk_and_extra_argument_injection() {
        let root = tempdir().unwrap();
        let disk_path = root.path().join("unsafe-disk.conf");
        fs::write(&disk_path, "disk_format=qcow2,backing_file=/tmp/base\n").unwrap();
        assert!(load_vm(root.path(), root.path(), disk_path).is_err());

        let args_path = root.path().join("unsafe-args.conf");
        fs::write(&args_path, "extra_args=(\"-qmp\" \"tcp:0.0.0.0:4444\")\n").unwrap();
        let error = load_vm(root.path(), root.path(), args_path).unwrap_err();
        assert!(error.to_string().contains("extra_args contains '-qmp'"));
    }

    #[test]
    fn accepts_named_spice_bind_addresses() {
        let root = tempdir().unwrap();
        let path = root.path().join("remote.conf");
        fs::write(&path, "access=vm.example.test\ndisplay=spice\n").unwrap();
        let vm = load_vm(root.path(), root.path(), path).unwrap();
        assert_eq!(vm.config.access, "vm.example.test");
    }

    #[test]
    fn rejects_telnet_option_injection() {
        let root = tempdir().unwrap();
        let path = root.path().join("unsafe-telnet.conf");
        fs::write(&path, "monitor_telnet_host=127.0.0.1,server=off\n").unwrap();
        let error = load_vm(root.path(), root.path(), path).unwrap_err();
        assert!(error.to_string().contains("monitor_telnet_host"));
    }

    #[test]
    fn parses_ssh_bind_and_viewer_arguments() {
        let root = tempdir().unwrap();
        let path = root.path().join("remote.conf");
        fs::write(
            &path,
            "ssh_access=192.0.2.10\nviewer_extra_args=(\"--foo\" \"bar baz\")\n",
        )
        .unwrap();
        let vm = load_vm(root.path(), root.path(), path).unwrap();
        assert_eq!(vm.config.ssh_access, "192.0.2.10");
        assert_eq!(vm.config.viewer_extra_args, ["--foo", "bar baz"]);
    }

    #[test]
    fn parses_usb_devices_as_hex_pairs() {
        let values = parse_config("usb_devices=(\"046d:082d\")\n");
        let root = tempdir().unwrap();
        let path = root.path().join("usb.conf");
        fs::write(&path, "usb_devices=(\"046d:082d\")\n").unwrap();
        let vm = load_vm(root.path(), root.path(), path).unwrap();
        assert_eq!(vm.config.usb_devices, vec![(0x046d, 0x082d)]);
        assert_eq!(parse_tokens(values.get("usb_devices")), vec!["046d:082d"]);
    }

    #[test]
    fn parses_windows_install_media() {
        let root = tempdir().unwrap();
        let path = root.path().join("windows.conf");
        fs::write(
            &path,
            "boot=legacy\niso=windows.iso\nfixed_iso=virtio-win.iso\nunattended_iso=unattended.iso\n",
        )
        .unwrap();
        let vm = load_vm(root.path(), root.path(), path).unwrap();
        assert_eq!(
            vm.config.unattended_iso,
            Some(root.path().join("unattended.iso"))
        );
    }
}
