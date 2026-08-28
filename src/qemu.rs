#[cfg(windows)]
use std::cell::Cell;
use std::env;
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream, ToSocketAddrs, UdpSocket};
#[cfg(unix)]
use std::os::unix::fs::{FileTypeExt, OpenOptionsExt, PermissionsExt};
#[cfg(unix)]
use std::os::unix::io::{AsRawFd, FromRawFd, IntoRawFd, OwnedFd};
#[cfg(unix)]
use std::os::unix::net::UnixStream;
#[cfg(windows)]
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
#[cfg(windows)]
use std::os::windows::io::AsRawHandle;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use crate::config::{Vm, VmConfig};
use crate::error::{Error, Result};
use crate::paths::VmPaths;

static NEXT_GUEST_SYNC_ID: AtomicI64 = AtomicI64::new(1);

mod ipc;
pub use ipc::IpcEndpoint;
use ipc::*;

mod capabilities;
mod disk;
use capabilities::*;
mod devices;
use devices::*;
mod display;
use display::*;
mod firmware;
use firmware::*;
mod firmware_helpers;
use firmware_helpers::*;
mod guest;
use guest::*;
mod host;
use host::*;
mod monitor;
use monitor::*;
mod network;
use network::*;
mod plan;
use plan::*;
mod process;
use process::*;
mod qmp;
use qmp::*;
mod runtime;
mod shell;
mod storage;
use storage::*;

pub(crate) use capabilities::{qemu_capability_report, virtiofsd_available};
pub(crate) use devices::{configured_bridge, virtiofs_requested};
pub(crate) use disk::{
    create_cloud_copy, create_cloud_overlay, disk_check, disk_compact, disk_convert, disk_info,
    disk_resize, ensure_disk, qemu_img_failure, require_disk_file, run_qemu_img,
    validate_disk_size,
};
pub(crate) use guest::{
    disk_snapshot, guest_command, guest_exec, guest_fsfreeze_freeze, guest_fsfreeze_status,
    guest_fsfreeze_thaw, guest_fstrim, guest_shutdown,
};
pub(crate) use host::{default_cpu_cores, render_node};
pub(crate) use monitor::send_monitor_command;
pub(crate) use network::spice_address;
pub(crate) use plan::build_plan;
pub(crate) use qmp::{process_identity, process_matches_checked_with_identity};
pub(crate) use runtime::{
    ensure_ipc_endpoints_available, ipc_report, kill_process, qmp_live_resources, qmp_ping,
    qmp_status, remove_runtime_sockets, replace_file, shutdown_via_qmp, start_tpm, start_virtiofsd,
    stop_tpm, stop_virtiofsd, wait_for_exit, write_atomic_file, write_runtime_files,
};
pub(crate) use shell::shell_join;

#[derive(Debug, Clone)]
pub struct QemuPlanContext {
    pub qemu_binary: String,
    pub host_os: String,
    pub accelerator: String,
    pub cpu_cores: u32,
    pub ram: String,
    pub virtio_vga_gl: bool,
    pub usb_redirection: bool,
    pub smartcard: bool,
    pub smbd: bool,
    pub audio_driver: Option<String>,
    pub username: String,
    pub bridge_helper: Option<String>,
    pub virtiofsd: Option<String>,
    pub virtiofs_device: bool,
    pub ssh_port: Option<u16>,
    pub spice_port: Option<u16>,
}

pub type HostCapabilities = QemuPlanContext;

impl QemuPlanContext {
    pub fn detect(config: &VmConfig) -> Result<Self> {
        let qemu_binary = format!("qemu-system-{}", config.arch);
        ensure_command(&qemu_binary)?;

        let same_arch = (config.arch == "x86_64" && env::consts::ARCH == "x86_64")
            || (config.arch == "aarch64" && env::consts::ARCH == "aarch64");
        let host_os = env::consts::OS.to_string();
        let accelerators = qemu_accelerators_probe(&qemu_binary)
            .ok_or_else(|| Error::message("could not query QEMU accelerator capabilities"))?;
        let device_help = qemu_help_output(&qemu_binary, &["-device", "help"])
            .ok_or_else(|| Error::message("could not query QEMU device capabilities"))?;
        let device_names = qemu_quoted_names(&device_help);
        let cpu_help = qemu_help_output(&qemu_binary, &["-cpu", "help"])
            .ok_or_else(|| Error::message("could not query QEMU CPU capabilities"))?;
        let accelerator = if same_arch {
            if host_os == "linux"
                && File::open("/dev/kvm").is_ok()
                && accelerators.iter().any(|value| value == "kvm")
                && qemu_accelerator_usable(&qemu_binary, "kvm")
            {
                "kvm"
            } else if host_os == "macos"
                && accelerators.iter().any(|value| value == "hvf")
                && qemu_accelerator_usable(&qemu_binary, "hvf")
            {
                "hvf"
            } else if host_os == "windows"
                && accelerators.iter().any(|value| value == "whpx")
                && qemu_accelerator_usable(&qemu_binary, "whpx")
            {
                "whpx"
            } else {
                "tcg"
            }
        } else {
            "tcg"
        };
        let username = env::var("USER")
            .or_else(|_| env::var("USERNAME"))
            .unwrap_or_else(|_| "user".to_string());
        if uses_passt_network(config) {
            if host_os != "linux" {
                return Err(Error::message(
                    "network=passt is currently supported only on Linux hosts",
                ));
            }
            let netdevs = qemu_netdev_backends_probe(&qemu_binary).ok_or_else(|| {
                Error::message("could not query QEMU network backend capabilities")
            })?;
            if !netdevs.iter().any(|backend| backend == "passt") {
                return Err(Error::message(
                    "network=passt requires QEMU 10.1 or newer with the passt network backend",
                ));
            }
            if find_executable("passt").is_none() {
                return Err(Error::message(
                    "network=passt requires the passt executable; install passt and retry",
                ));
            }
        }
        let (ssh_port, spice_port) = listener_ports(config, &host_os)?;

        let audio_driver = detect_audio_driver(&host_os);
        let virtiofsd = if host_os == "linux" {
            find_virtiofsd()
        } else {
            None
        };
        let context = Self {
            qemu_binary: qemu_binary.clone(),
            host_os,
            accelerator: accelerator.to_string(),
            cpu_cores: default_cpu_cores(),
            ram: default_ram(),
            virtio_vga_gl: qemu_supports_gl_devices_in_names(&device_names, &config.arch),
            usb_redirection: device_names.iter().any(|name| name == "usb-redir"),
            smartcard: device_names.iter().any(|name| name == "ccid-card-passthru"),
            smbd: command_available("smbd"),
            audio_driver,
            username,
            bridge_helper: find_executable("qemu-bridge-helper"),
            virtiofs_device: virtiofsd
                .as_deref()
                .is_some_and(|_| device_names.iter().any(|name| name == "vhost-user-fs-pci")),
            virtiofsd,
            ssh_port,
            spice_port,
        };
        let cpu = cpu_model(config, &context);
        let model = cpu.split(',').next().unwrap_or(&cpu);
        if !qemu_supports_cpu_in_text(&cpu_help, model) {
            return Err(Error::message(format!(
                "QEMU does not support CPU model '{model}' for {}",
                config.arch
            )));
        }
        if model == "host" && accelerator == "tcg" {
            return Err(Error::message(
                "CPU model 'host' requires hardware virtualization; enable KVM/HVF/WHPX or choose a portable CPU model",
            ));
        }
        if cpu.contains(',') {
            validate_cpu_spec(&qemu_binary, &cpu, accelerator)?;
        }
        Ok(context)
    }
}

fn listener_ports(config: &VmConfig, host_os: &str) -> Result<(Option<u16>, Option<u16>)> {
    let uses_port_forwarding = uses_port_forwarding_network(config);
    let mut reserved = if uses_port_forwarding {
        config.port_forwards.iter().map(|(host, _)| *host).collect()
    } else {
        Vec::new()
    };
    for (mode, port, service) in [
        (
            &config.monitor,
            config.monitor_telnet_port,
            "monitor Telnet",
        ),
        (&config.serial, config.serial_telnet_port, "serial Telnet"),
    ] {
        if mode == "telnet" {
            reserve_listener_port(&mut reserved, port, service)?;
        }
    }
    let ssh = if uses_port_forwarding {
        let port = config
            .ssh_port
            .map_or_else(|| find_free_port(22220, &reserved), Ok)?;
        reserve_listener_port(&mut reserved, port, "SSH")?;
        Some(port)
    } else {
        None
    };
    let spice = match config.display.as_str() {
        "none" => Some(
            config
                .spice_port
                .map_or_else(|| find_free_port(5930, &reserved), Ok)?,
        ),
        "spice" | "spice-app" if host_os == "windows" => Some(
            config
                .spice_port
                .map_or_else(|| find_free_port(5930, &reserved), Ok)?,
        ),
        "spice" if config.access != "local" => Some(
            config
                .spice_port
                .map_or_else(|| find_free_port(5930, &reserved), Ok)?,
        ),
        "spice" | "spice-app" => config.spice_port,
        _ => None,
    };
    if let Some(port) = spice {
        reserve_listener_port(&mut reserved, port, "SPICE")?;
    }
    Ok((ssh, spice))
}

fn reserve_listener_port(ports: &mut Vec<u16>, port: u16, service: &str) -> Result<()> {
    if ports.contains(&port) {
        return Err(Error::message(format!(
            "{service} port {port} conflicts with another configured listener"
        )));
    }
    ports.push(port);
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QemuPlan {
    pub binary: String,
    pub args: Vec<String>,
    pub qmp_endpoint: IpcEndpoint,
    pub agent_endpoint: Option<IpcEndpoint>,
    pub ssh_port: Option<u16>,
    pub ssh_host: Option<String>,
    pub spice_port: Option<u16>,
    pub spice_host: Option<String>,
    pub monitor_telnet: Option<(String, u16)>,
    pub serial_telnet: Option<(String, u16)>,
    pub forwarded_ports: Vec<(String, u16)>,
}

#[derive(Debug)]
pub(crate) struct FileLock {
    file: File,
}

impl Drop for FileLock {
    fn drop(&mut self) {
        unlock_file(&self.file);
    }
}

pub(crate) fn acquire_file_lock(path: &Path) -> io::Result<FileLock> {
    let file = open_lock_file(path)?;
    try_lock_file(&file)?;
    Ok(FileLock { file })
}

fn ensure_state_directory(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if unsafe_file_metadata(&metadata) => {
            return Err(Error::message(format!(
                "refusing to use state directory symlink {}",
                path.display()
            )));
        }
        Ok(metadata) if !metadata.is_dir() => {
            return Err(Error::message(format!(
                "state path {} is not a directory",
                path.display()
            )));
        }
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            fs::create_dir_all(path).map_err(|error| Error::io(path.display(), error))?;
        }
        Err(error) => return Err(Error::io(path.display(), error)),
    }
    let metadata = fs::symlink_metadata(path).map_err(|error| Error::io(path.display(), error))?;
    if unsafe_file_metadata(&metadata) || !metadata.is_dir() {
        return Err(Error::message(format!(
            "state path {} is not a real directory",
            path.display()
        )));
    }
    #[cfg(unix)]
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(|error| Error::io(path.display(), error))?;
    Ok(())
}

pub(crate) fn acquire_vm_lock(paths: &VmPaths) -> Result<FileLock> {
    ensure_state_directory(&paths.state_dir)?;
    let path = paths.state_dir.join("operation.lock");
    let pid = std::process::id() as i32;
    let token = format!(
        "{pid}:{}:{}\n",
        process_identity(pid).unwrap_or_default(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| duration.as_nanos())
    );
    let mut lock = acquire_file_lock(&path).map_err(|error| {
        if error.kind() == io::ErrorKind::WouldBlock {
            Error::message(format!(
                "another vmctl operation is already using {}; retry after it finishes",
                paths.state_dir.display()
            ))
        } else {
            Error::io(path.display(), error)
        }
    })?;
    lock.file
        .set_len(0)
        .and_then(|()| lock.file.write_all(token.as_bytes()))
        .and_then(|()| lock.file.sync_all())
        .map_err(|error| Error::io(path.display(), error))?;
    Ok(lock)
}

fn reject_unsafe_file(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if unsafe_file_metadata(&metadata) {
                Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "refusing to follow a symbolic-link file",
                ))
            } else if !metadata.file_type().is_file() {
                Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("{} is not a regular file", path.display()),
                ))
            } else {
                Ok(())
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn ensure_regular_file(path: &Path, file: File) -> io::Result<File> {
    let metadata = file.metadata()?;
    if metadata.file_type().is_file() && !unsafe_file_metadata(&metadata) {
        Ok(file)
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{} is not a regular file", path.display()),
        ))
    }
}

#[cfg(windows)]
fn unsafe_file_metadata(metadata: &fs::Metadata) -> bool {
    metadata.file_attributes()
        & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
        != 0
}

#[cfg(not(windows))]
fn unsafe_file_metadata(metadata: &fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}

#[cfg(unix)]
fn open_lock_file(path: &Path) -> io::Result<File> {
    reject_unsafe_file(path)?;
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    ensure_regular_file(path, file)
}

#[cfg(windows)]
fn open_lock_file(path: &Path) -> io::Result<File> {
    use windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT;

    reject_unsafe_file(path)?;
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
    ensure_regular_file(path, file)
}

#[cfg(not(any(unix, windows)))]
fn open_lock_file(path: &Path) -> io::Result<File> {
    reject_unsafe_file(path)?;
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)?;
    ensure_regular_file(path, file)
}

#[cfg(unix)]
pub(crate) fn create_truncated_file(path: &Path) -> io::Result<File> {
    reject_unsafe_file(path)?;
    let file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    ensure_regular_file(path, file)
}

#[cfg(windows)]
pub(crate) fn create_truncated_file(path: &Path) -> io::Result<File> {
    use windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT;

    reject_unsafe_file(path)?;
    let file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
    ensure_regular_file(path, file)
}

#[cfg(not(any(unix, windows)))]
pub(crate) fn create_truncated_file(path: &Path) -> io::Result<File> {
    reject_unsafe_file(path)?;
    let file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)?;
    ensure_regular_file(path, file)
}

#[cfg(unix)]
fn try_lock_file(file: &File) -> io::Result<()> {
    let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(unix)]
fn unlock_file(file: &File) {
    let _ = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_UN) };
}

#[cfg(windows)]
fn try_lock_file(file: &File) -> io::Result<()> {
    use windows_sys::Win32::Foundation::ERROR_LOCK_VIOLATION;
    use windows_sys::Win32::Storage::FileSystem::LockFile;

    let locked = unsafe { LockFile(file.as_raw_handle(), 0, 0, 1, 0) };
    if locked != 0 {
        return Ok(());
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() == Some(ERROR_LOCK_VIOLATION as i32) {
        Err(io::Error::from(io::ErrorKind::WouldBlock))
    } else {
        Err(error)
    }
}

#[cfg(windows)]
fn unlock_file(file: &File) {
    use windows_sys::Win32::Storage::FileSystem::UnlockFile;

    let _ = unsafe { UnlockFile(file.as_raw_handle(), 0, 0, 1, 0) };
}

#[cfg(not(any(unix, windows)))]
fn try_lock_file(_file: &File) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "VM operation locking is unsupported on this platform",
    ))
}

#[cfg(not(any(unix, windows)))]
fn unlock_file(_file: &File) {}

#[cfg(test)]
mod tests;
