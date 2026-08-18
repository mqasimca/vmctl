use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};

use clap::{Args, Parser, Subcommand, ValueEnum};
use clap_complete::{
    Shell,
    engine::{ArgValueCompleter, CompletionCandidate},
};

#[derive(Debug, Clone, Copy, ValueEnum, PartialEq, Eq)]
pub enum OutputFormat {
    Human,
    Json,
}

#[derive(Debug, Clone, Copy, ValueEnum, PartialEq, Eq)]
pub enum StartWait {
    /// Wait until the guest SSH service accepts connections.
    Ssh,
}

#[derive(Debug, Parser)]
#[command(
    name = "vmctl",
    version,
    about = "Manage QEMU/KVM virtual machines",
    long_about = "Manage QEMU/KVM virtual machines directly from the host.\n\nVM configuration files are read as data and never executed.",
    after_long_help = "Examples:\n  vmctl get ubuntu 24.04      Download an Ubuntu image and create its VM configuration\n  vmctl start ubuntu-24.04    Start the VM\n  vmctl stop ubuntu-24.04     Request a clean shutdown\n  vmctl doctor ubuntu-24.04   Check host and VM readiness"
)]
pub struct Cli {
    /// Directory containing VM .conf files.
    #[arg(
        short = 'd',
        long = "dir",
        visible_alias = "vm-dir",
        value_name = "PATH",
        global = true
    )]
    pub vm_dir: Option<PathBuf>,

    /// Directory for runtime files, sockets, logs, and PIDs.
    #[arg(long, value_name = "PATH", global = true)]
    pub state_dir: Option<PathBuf>,

    /// Output format for commands that return structured data.
    #[arg(long, value_enum, default_value_t = OutputFormat::Human, global = true)]
    pub output: OutputFormat,

    /// Increase diagnostic output. Repeat for more detail.
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    pub verbose: u8,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// List VM configurations and their current state.
    List,

    /// Generate a shell completion script.
    Completion {
        #[arg(value_enum, value_name = "SHELL")]
        shell: Shell,
    },

    /// Show one VM in detail, or list all VMs when no name is given.
    Status {
        #[arg(value_name = "VM", add = ArgValueCompleter::new(complete_vm_names))]
        vm: Option<String>,
    },

    /// Print the QEMU command that would be executed.
    Plan {
        #[arg(value_name = "VM", add = ArgValueCompleter::new(complete_vm_names))]
        vm: String,
        /// Redact common secret values in plan output.
        #[arg(long)]
        redact: bool,
        #[command(flatten)]
        options: LaunchOptions,
    },

    /// Start a VM with QEMU.
    Start {
        #[arg(value_name = "VM", add = ArgValueCompleter::new(complete_vm_names))]
        vm: String,
        /// Wait for a guest service after QEMU starts.
        #[arg(long, value_enum, value_name = "READY")]
        wait: Option<StartWait>,
        /// Maximum seconds to wait for --wait (default: 120).
        #[arg(
            long,
            default_value_t = 120,
            value_parser = clap::value_parser!(u64).range(1..=86_400)
        )]
        wait_timeout: u64,
        #[command(flatten)]
        options: LaunchOptions,
    },

    /// Open an SSH session through a running VM's forwarded port.
    Ssh {
        #[arg(value_name = "VM", add = ArgValueCompleter::new(complete_vm_names))]
        vm: String,
        /// Guest login name (defaults to the current host user).
        #[arg(short = 'l', long, value_name = "USER")]
        user: Option<String>,
    },

    /// Open a graphical SPICE console for a running VM.
    #[command(visible_alias = "connect")]
    View {
        #[arg(value_name = "VM", add = ArgValueCompleter::new(complete_vm_names))]
        vm: String,
        /// SPICE viewer command (defaults to the VM setting or remote-viewer).
        #[arg(long, value_name = "COMMAND")]
        viewer: Option<String>,
    },

    /// Request a graceful guest shutdown through QMP.
    Stop {
        #[arg(value_name = "VM", add = ArgValueCompleter::new(complete_vm_names))]
        vm: String,
        /// Wait up to this many seconds for the process to exit.
        #[arg(
            long,
            default_value_t = 10,
            value_parser = clap::value_parser!(u64).range(1..=86_400)
        )]
        timeout: u64,
        /// Fall back to killing the process if graceful shutdown times out.
        #[arg(long)]
        force: bool,
    },

    /// Immediately terminate a running VM process.
    Kill {
        #[arg(value_name = "VM", add = ArgValueCompleter::new(complete_vm_names))]
        vm: String,
    },

    /// Show the tail of a VM's QEMU log.
    Logs {
        #[arg(value_name = "VM", add = ArgValueCompleter::new(complete_vm_names))]
        vm: String,
        /// Number of log lines to return.
        #[arg(
            long,
            default_value_t = 100,
            value_parser = clap::value_parser!(u64).range(1..=10_000)
        )]
        lines: u64,
    },

    /// Stop and start a VM.
    Restart {
        #[arg(value_name = "VM", add = ArgValueCompleter::new(complete_vm_names))]
        vm: String,
        /// Wait up to this many seconds for graceful shutdown.
        #[arg(
            long,
            default_value_t = 10,
            value_parser = clap::value_parser!(u64).range(1..=86_400)
        )]
        timeout: u64,
        /// Kill QEMU if graceful shutdown times out or is unavailable.
        #[arg(long)]
        force: bool,
        #[command(flatten)]
        options: LaunchOptions,
    },

    /// Manage an internal QEMU disk snapshot.
    Snapshot {
        #[arg(value_name = "VM", add = ArgValueCompleter::new(complete_vm_names))]
        vm: String,
        #[command(subcommand)]
        action: SnapshotAction,
    },

    /// Inspect and manage a VM disk image.
    Disk {
        #[arg(value_name = "VM", add = ArgValueCompleter::new(complete_vm_names))]
        vm: String,
        #[command(subcommand)]
        action: DiskAction,
    },

    /// Delete a VM disk and its persistent UEFI variables.
    #[command(visible_alias = "delete")]
    DeleteDisk {
        #[arg(value_name = "VM", add = ArgValueCompleter::new(complete_vm_names))]
        vm: String,
        /// Confirm deletion without an interactive prompt.
        #[arg(long)]
        yes: bool,
    },

    /// Delete a VM configuration, disk, and runtime data.
    DeleteVm {
        #[arg(value_name = "VM", add = ArgValueCompleter::new(complete_vm_names))]
        vm: String,
        /// Confirm deletion without an interactive prompt.
        #[arg(long)]
        yes: bool,
    },

    /// Send an HMP command to the legacy QEMU monitor.
    Monitor {
        #[arg(value_name = "VM", add = ArgValueCompleter::new(complete_vm_names))]
        vm: String,
        #[arg(value_name = "COMMAND", trailing_var_arg = true)]
        command: Vec<String>,
    },

    /// Run a command through the QEMU Guest Agent.
    Guest {
        #[arg(value_name = "VM", add = ArgValueCompleter::new(complete_vm_names))]
        vm: String,
        #[command(subcommand)]
        action: GuestAction,
    },

    /// Create a desktop launcher for a VM.
    Shortcut {
        #[arg(value_name = "VM", add = ArgValueCompleter::new(complete_vm_names))]
        vm: String,
        /// Write the launcher to this path instead of the desktop applications directory.
        #[arg(long, value_name = "PATH")]
        path: Option<PathBuf>,
    },

    /// Report host virtualization and QEMU capabilities.
    Report,

    /// Check host and optional VM readiness without changing state.
    Doctor {
        #[arg(value_name = "VM", add = ArgValueCompleter::new(complete_vm_names))]
        vm: Option<String>,
    },

    /// Apply host-level virtualization settings.
    Host {
        #[command(subcommand)]
        action: HostAction,
    },

    /// Download OS images and create VM configurations.
    Get(GetArgs),
}

fn complete_vm_names(current: &OsStr) -> Vec<CompletionCandidate> {
    vm_name_candidates(&completion_vm_dir(), current)
}

fn completion_vm_dir() -> PathBuf {
    let args = std::env::args_os().skip(1).collect::<Vec<_>>();
    completion_vm_dir_from_args(&args)
        .unwrap_or_else(|| crate::paths::default_vm_dir().unwrap_or_default())
}

fn completion_vm_dir_from_args(args: &[OsString]) -> Option<PathBuf> {
    let args = args
        .iter()
        .position(|arg| arg == "--")
        .map_or(args, |index| &args[index + 1..]);
    let mut dir = None;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        if arg == "-d" || arg == "--dir" || arg == "--vm-dir" {
            dir = args.next().cloned().map(PathBuf::from);
        } else if let Some(value) = arg.to_str().and_then(|arg| {
            arg.strip_prefix("--dir=")
                .or_else(|| arg.strip_prefix("--vm-dir="))
        }) {
            dir = Some(PathBuf::from(value));
        } else if let Some(value) = arg
            .to_str()
            .and_then(|arg| arg.strip_prefix("-d").filter(|value| !value.is_empty()))
        {
            dir = Some(PathBuf::from(value));
        }
    }
    dir
}

fn vm_name_candidates(dir: &Path, current: &OsStr) -> Vec<CompletionCandidate> {
    let Some(current) = current.to_str() else {
        return Vec::new();
    };
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut names = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            if path.is_file()
                && path
                    .extension()
                    .is_some_and(|extension| extension == "conf")
            {
                path.file_stem().and_then(OsStr::to_str).map(str::to_string)
            } else {
                None
            }
        })
        .filter(|name| name.starts_with(current))
        .collect::<Vec<_>>();
    names.sort();
    names.into_iter().map(CompletionCandidate::new).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vm_name_completion_lists_matching_config_stems() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("alpha.conf"), []).unwrap();
        fs::write(root.path().join("beta.conf"), []).unwrap();
        fs::write(root.path().join("ignored.txt"), []).unwrap();

        let candidates = vm_name_candidates(root.path(), OsStr::new("a"));
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].get_value(), OsStr::new("alpha"));
    }

    #[test]
    fn vm_name_completion_honors_dir_arguments() {
        let args = [
            OsString::from("--"),
            OsString::from("vmctl"),
            OsString::from("start"),
            OsString::from("--dir"),
            OsString::from("/tmp/vmctl-vms"),
            OsString::new(),
        ];
        assert_eq!(
            completion_vm_dir_from_args(&args),
            Some(PathBuf::from("/tmp/vmctl-vms"))
        );
    }
}

#[derive(Debug, Clone, Subcommand)]
pub enum HostAction {
    /// Persist KVM's ignore-msrs setting and rebuild initramfs when available.
    IgnoreMsrsAlways,
}

#[derive(Debug, Clone, Args, Default)]
pub struct GetArgs {
    /// Normalize image URLs and downloads for this architecture.
    #[arg(long, value_name = "ARCH")]
    pub arch: Option<String>,

    /// Download an image without creating a VM configuration.
    #[arg(long)]
    pub download: bool,

    /// Create a VM configuration from a local image or URL.
    #[arg(long)]
    pub create_config: bool,

    /// Open the operating system homepage.
    #[arg(long)]
    pub open_homepage: bool,

    /// Show operating system information.
    #[arg(long)]
    pub show: bool,

    /// Print image URL(s) without downloading.
    #[arg(long)]
    pub url: bool,

    /// Check image URL(s) with an HTTP HEAD request.
    #[arg(long)]
    pub check: bool,

    /// Check the requested image for amd64 and arm64.
    #[arg(long)]
    pub check_all_arch: bool,

    /// List supported operating systems.
    #[arg(long)]
    pub list: bool,

    /// List supported systems as CSV.
    #[arg(long)]
    pub list_csv: bool,

    /// List supported systems as JSON.
    #[arg(long)]
    pub list_json: bool,

    /// Show vmctl's version.
    #[arg(long)]
    pub version: bool,

    /// Skip generated Windows unattended-installation media.
    #[arg(long)]
    pub disable_unattended: bool,

    /// Disable TLS certificate verification for network checks and downloads (unsafe; also VMCTL_INSECURE=1).
    #[arg(long)]
    pub insecure: bool,

    /// OS to inspect or download, or VM name / `custom` for --create-config.
    #[arg(value_name = "OS")]
    pub os: Option<String>,

    /// Release, or the local image/URL for --create-config.
    #[arg(value_name = "RELEASE_OR_INPUT")]
    pub release_or_input: Option<String>,

    /// Edition or language.
    #[arg(value_name = "EDITION_OR_LANGUAGE")]
    pub edition_or_language: Option<String>,
}

#[derive(Debug, Clone, Subcommand)]
pub enum SnapshotAction {
    /// Create a snapshot with TAG.
    Create { tag: String },
    /// Apply a snapshot with TAG.
    Apply { tag: String },
    /// Delete a snapshot with TAG.
    Delete { tag: String },
    /// Show snapshot and disk information.
    Info,
}

#[derive(Debug, Clone, Subcommand)]
pub enum DiskAction {
    /// Show detailed qemu-img information.
    Info,
    /// Resize a stopped VM disk. Growing is allowed by default.
    Resize {
        #[arg(value_name = "SIZE")]
        size: String,
        /// Allow reducing the virtual disk size; requires --yes.
        #[arg(long)]
        shrink: bool,
        /// Confirm a potentially destructive resize.
        #[arg(long)]
        yes: bool,
    },
    /// Check a stopped VM disk for integrity problems.
    Check {
        /// Repair all errors; requires --yes.
        #[arg(long)]
        repair: bool,
        /// Confirm repair changes.
        #[arg(long)]
        yes: bool,
    },
    /// Convert a stopped VM disk to OUTPUT.
    Convert {
        #[arg(value_name = "OUTPUT")]
        destination: PathBuf,
        /// Destination format; defaults to the configured disk format.
        #[arg(long, value_name = "FORMAT")]
        format: Option<String>,
        /// Compress qcow/qcow2 output.
        #[arg(long)]
        compress: bool,
        /// Permit overwriting an existing output file.
        #[arg(long)]
        force: bool,
    },
    /// Rewrite a stopped disk to reclaim sparse/compressed space; requires --yes.
    Compact {
        /// Confirm replacement of the disk image. Internal snapshots are not preserved.
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Debug, Clone, Subcommand)]
pub enum GuestAction {
    /// Check that the guest agent is responding.
    Ping,
    /// Request a clean guest shutdown.
    Shutdown {
        /// Maximum time to wait for QEMU to exit or enter shutdown state.
        #[arg(
            long,
            default_value_t = 10,
            value_parser = clap::value_parser!(u64).range(1..=86_400)
        )]
        timeout: u64,
    },
    /// Query guest network interfaces and addresses.
    Ip,
    /// Execute a program directly inside the guest.
    Exec {
        /// Maximum time to wait for the guest process.
        #[arg(
            long,
            default_value_t = 10,
            value_name = "SECONDS",
            value_parser = clap::value_parser!(u64).range(1..=86_400)
        )]
        timeout: u64,
        #[arg(value_name = "PROGRAM")]
        program: String,
        #[arg(
            value_name = "ARG",
            trailing_var_arg = true,
            allow_hyphen_values = true
        )]
        args: Vec<String>,
    },
}

#[derive(Debug, Clone, Args, Default)]
pub struct LaunchOptions {
    /// Display backend: gtk, sdl, spice, spice-app, none, or cocoa on macOS.
    #[arg(long, value_name = "MODE", help_heading = "Display")]
    pub display: Option<String>,

    /// SPICE viewer: spicy, remote-viewer, or none.
    #[arg(long, value_name = "VIEWER", help_heading = "Display")]
    pub viewer: Option<String>,

    /// Enable SDL braille support.
    #[arg(long, help_heading = "Display")]
    pub braille: bool,

    /// Start the display in fullscreen mode.
    #[arg(long, visible_alias = "full-screen", help_heading = "Display")]
    pub fullscreen: bool,

    /// Enable host-guest clipboard sharing with the GTK display.
    #[arg(long, help_heading = "Display")]
    pub clipboard: bool,

    /// Screen width; requires --height.
    #[arg(long, value_name = "PIXELS", help_heading = "Display")]
    pub width: Option<u32>,

    /// Screen height; requires --width.
    #[arg(long, value_name = "PIXELS", help_heading = "Display")]
    pub height: Option<u32>,

    /// SPICE access: local, remote, or an explicit bind host/address.
    #[arg(long, value_name = "ACCESS", help_heading = "Networking and sharing")]
    pub access: Option<String>,

    /// Permit unauthenticated remote SPICE/Telnet listeners.
    #[arg(long, help_heading = "Networking and sharing")]
    pub allow_insecure_remote: bool,

    /// SSH access: local, remote, or an explicit bind host/address.
    #[arg(long, value_name = "ACCESS", help_heading = "Networking and sharing")]
    pub ssh_access: Option<String>,

    /// Disable guest networking.
    #[arg(long, help_heading = "Networking and sharing")]
    pub offline: bool,

    /// SSH forwarding port.
    #[arg(long, value_name = "PORT", help_heading = "Networking and sharing")]
    pub ssh_port: Option<u16>,

    /// SPICE TCP port.
    #[arg(long, value_name = "PORT", help_heading = "Networking and sharing")]
    pub spice_port: Option<u16>,

    /// Host directory exposed to the guest; use `none` to disable sharing.
    #[arg(long, value_name = "PATH", help_heading = "Networking and sharing")]
    pub public_dir: Option<PathBuf>,

    /// Keyboard mode: usb, ps2, or virtio.
    #[arg(long, value_name = "MODE", help_heading = "Devices")]
    pub keyboard: Option<String>,

    /// Keyboard layout, for example en-us.
    #[arg(
        long,
        visible_alias = "keyboard_layout",
        value_name = "LAYOUT",
        help_heading = "Devices"
    )]
    pub keyboard_layout: Option<String>,

    /// Mouse mode: tablet, ps2, usb, or virtio.
    #[arg(long, value_name = "MODE", help_heading = "Devices")]
    pub mouse: Option<String>,

    /// USB controller: ehci, xhci, or none.
    #[arg(long, value_name = "MODE", help_heading = "Devices")]
    pub usb_controller: Option<String>,

    /// Sound card model.
    #[arg(long, value_name = "MODEL", help_heading = "Devices")]
    pub sound_card: Option<String>,

    /// Sound duplex codec.
    #[arg(long, value_name = "CODEC", help_heading = "Devices")]
    pub sound_duplex: Option<String>,

    /// Do not commit disk writes during this run.
    #[arg(long, help_heading = "Advanced")]
    pub status_quo: bool,

    /// Skip the macOS-on-AMD unstable-TSC safety check.
    #[arg(long, help_heading = "Advanced")]
    pub ignore_tsc_warning: bool,

    /// Pin guest CPUs to comma-separated host CPU IDs.
    #[arg(long, value_name = "CPUS", help_heading = "Advanced")]
    pub cpu_pinning: Option<String>,

    /// Additional arguments passed to the SPICE viewer.
    #[arg(
        long = "viewer-extra-args",
        visible_alias = "viewer_extra_args",
        num_args = 1..,
        allow_hyphen_values = true,
        help_heading = "Advanced"
    )]
    pub viewer_extra_args: Vec<String>,

    /// Legacy QEMU monitor mode: socket, telnet, or none.
    #[arg(long, value_name = "MODE", help_heading = "Advanced")]
    pub monitor: Option<String>,

    /// Send a command to the monitor after startup.
    #[arg(long, value_name = "COMMAND", help_heading = "Advanced")]
    pub monitor_cmd: Option<String>,

    /// Monitor telnet host.
    #[arg(long, value_name = "HOST", help_heading = "Advanced")]
    pub monitor_telnet_host: Option<String>,

    /// Monitor telnet port.
    #[arg(long, value_name = "PORT", help_heading = "Advanced")]
    pub monitor_telnet_port: Option<u16>,

    /// Serial mode: socket, telnet, or none.
    #[arg(long, value_name = "MODE", help_heading = "Advanced")]
    pub serial: Option<String>,

    /// Serial telnet host.
    #[arg(long, value_name = "HOST", help_heading = "Advanced")]
    pub serial_telnet_host: Option<String>,

    /// Serial telnet port.
    #[arg(long, value_name = "PORT", help_heading = "Advanced")]
    pub serial_telnet_port: Option<u16>,

    /// Additional raw arguments passed to QEMU.
    #[arg(
        long = "extra-args",
        visible_alias = "extra_args",
        num_args = 1..,
        allow_hyphen_values = true,
        help_heading = "Advanced"
    )]
    pub extra_args: Vec<String>,
}
