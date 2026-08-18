#[cfg(windows)]
use std::cell::Cell;
use std::env;
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
#[cfg(unix)]
use std::os::unix::fs::{FileTypeExt, OpenOptionsExt, PermissionsExt};
#[cfg(unix)]
use std::os::unix::io::{AsRawFd, FromRawFd, IntoRawFd, OwnedFd};
#[cfg(unix)]
use std::os::unix::net::UnixStream;
#[cfg(windows)]
use std::os::windows::fs::OpenOptionsExt;
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IpcEndpoint {
    #[cfg(unix)]
    Unix(PathBuf),
    Tcp(SocketAddr),
    #[cfg(windows)]
    Pipe(PathBuf),
}

impl IpcEndpoint {
    fn tcp_address(&self) -> Option<SocketAddr> {
        match self {
            Self::Tcp(address) => Some(*address),
            #[cfg(unix)]
            Self::Unix(_) => None,
            #[cfg(windows)]
            Self::Pipe(_) => None,
        }
    }

    fn qmp_argument(&self) -> String {
        match self {
            #[cfg(unix)]
            Self::Unix(path) => format!("unix:{},server=on,wait=off", qemu_path(path)),
            Self::Tcp(address) => format!("tcp:{address},server=on,wait=off"),
            #[cfg(windows)]
            Self::Pipe(path) => format!("pipe:{}", pipe_name(path)),
        }
    }

    fn add_qmp_args(&self, args: &mut Vec<String>) {
        #[cfg(windows)]
        if let Self::Pipe(path) = self {
            args.extend([
                "-chardev".to_string(),
                format!("pipe,id=qmp0,path={}", pipe_name(path)),
                "-mon".to_string(),
                "chardev=qmp0,mode=control".to_string(),
            ]);
            return;
        }
        args.extend(["-qmp".to_string(), self.qmp_argument()]);
    }

    fn guest_agent_argument(&self) -> String {
        match self {
            #[cfg(unix)]
            Self::Unix(path) => {
                format!("socket,id=qga0,path={},server=on,wait=off", qemu_path(path))
            }
            Self::Tcp(address) => format!(
                "socket,id=qga0,host={},port={},server=on,wait=off",
                address.ip(),
                address.port()
            ),
            #[cfg(windows)]
            Self::Pipe(path) => format!("pipe,id=qga0,path={}", pipe_name(path)),
        }
    }

    fn connect(&self, timeout: Duration) -> io::Result<IpcStream> {
        match self {
            #[cfg(unix)]
            Self::Unix(path) => connect_unix_timeout(path, timeout).map(IpcStream::Unix),
            Self::Tcp(address) => TcpStream::connect_timeout(address, timeout).map(IpcStream::Tcp),
            #[cfg(windows)]
            Self::Pipe(path) => {
                use std::os::windows::ffi::OsStrExt;
                use windows_sys::Win32::Foundation::HANDLE;
                use windows_sys::Win32::System::Pipes::{
                    PIPE_NOWAIT, SetNamedPipeHandleState, WaitNamedPipeW,
                };
                let pipe_path = path;
                let path: Vec<u16> = pipe_path.as_os_str().encode_wide().chain([0]).collect();
                let timeout_ms = timeout.as_millis().min(u128::from(u32::MAX)) as u32;
                let ready = unsafe { WaitNamedPipeW(path.as_ptr(), timeout_ms) };
                if ready == 0 {
                    return Err(io::Error::last_os_error());
                }
                let file = std::fs::OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(pipe_path)?;
                let mode = PIPE_NOWAIT;
                let configured = unsafe {
                    SetNamedPipeHandleState(
                        file.as_raw_handle() as HANDLE,
                        &mode,
                        std::ptr::null(),
                        std::ptr::null(),
                    )
                };
                if configured == 0 {
                    return Err(io::Error::last_os_error());
                }
                Ok(IpcStream::Pipe {
                    file,
                    read_timeout: Cell::new(None),
                    write_timeout: Cell::new(None),
                })
            }
        }
    }

    fn display(&self) -> String {
        match self {
            #[cfg(unix)]
            Self::Unix(path) => path.display().to_string(),
            Self::Tcp(address) => format!("tcp://{address}"),
            #[cfg(windows)]
            Self::Pipe(path) => path.display().to_string(),
        }
    }

    fn json_value(&self) -> Value {
        match self {
            #[cfg(unix)]
            Self::Unix(path) => json!({
                "transport": "unix",
                "path": path,
            }),
            Self::Tcp(address) => json!({
                "transport": "tcp",
                "host": address.ip().to_string(),
                "port": address.port(),
            }),
            #[cfg(windows)]
            Self::Pipe(path) => json!({
                "transport": "pipe",
                "path": path,
            }),
        }
    }

    fn from_json(value: &Value) -> Result<Self> {
        let transport = value
            .get("transport")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::message("runtime IPC endpoint has no transport"))?;
        match transport {
            "tcp" => {
                let host = value
                    .get("host")
                    .and_then(Value::as_str)
                    .ok_or_else(|| Error::message("runtime TCP endpoint has no host"))?;
                let port = value
                    .get("port")
                    .and_then(Value::as_u64)
                    .filter(|port| *port <= u16::MAX as u64 && *port != 0)
                    .map(|port| port as u16)
                    .ok_or_else(|| Error::message("runtime TCP endpoint has an invalid port"))?;
                let address = format!("{host}:{port}")
                    .parse::<SocketAddr>()
                    .map_err(|_| {
                        Error::message("runtime TCP endpoint must use a numeric address")
                    })?;
                if !address.ip().is_loopback() {
                    return Err(Error::message(
                        "runtime TCP endpoint must be bound to loopback",
                    ));
                }
                Ok(Self::Tcp(address))
            }
            #[cfg(unix)]
            "unix" => {
                let path = value
                    .get("path")
                    .and_then(Value::as_str)
                    .map(PathBuf::from)
                    .filter(|path| path.is_absolute())
                    .ok_or_else(|| Error::message("runtime Unix endpoint has an invalid path"))?;
                Ok(Self::Unix(path))
            }
            #[cfg(windows)]
            "pipe" => {
                let path = value
                    .get("path")
                    .and_then(Value::as_str)
                    .filter(|path| valid_pipe_path(path))
                    .map(PathBuf::from)
                    .ok_or_else(|| {
                        Error::message("runtime named-pipe endpoint has an invalid path")
                    })?;
                Ok(Self::Pipe(path))
            }
            _ => Err(Error::message(format!(
                "unsupported runtime IPC transport '{transport}'"
            ))),
        }
    }
}

#[cfg(unix)]
fn connect_unix_timeout(path: &Path, timeout: Duration) -> io::Result<UnixStream> {
    use std::os::unix::ffi::OsStrExt;

    let fd = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM | libc::SOCK_NONBLOCK, 0) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    let socket = unsafe { OwnedFd::from_raw_fd(fd) };
    let bytes = path.as_os_str().as_bytes();
    let mut address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    if bytes.len() >= address.sun_path.len() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Unix socket path is too long",
        ));
    }
    address.sun_family = libc::AF_UNIX as _;
    unsafe {
        std::ptr::copy_nonoverlapping(
            bytes.as_ptr(),
            address.sun_path.as_mut_ptr().cast::<u8>(),
            bytes.len(),
        );
    }
    let address_len =
        (std::mem::size_of_val(&address.sun_family) + bytes.len() + 1) as libc::socklen_t;
    let connect_result = unsafe {
        libc::connect(
            socket.as_raw_fd(),
            (&address as *const libc::sockaddr_un).cast::<libc::sockaddr>(),
            address_len,
        )
    };
    if connect_result == 0 {
        let stream = unsafe { UnixStream::from_raw_fd(socket.into_raw_fd()) };
        stream.set_nonblocking(false)?;
        return Ok(stream);
    }
    let error = io::Error::last_os_error();
    if !matches!(
        error.raw_os_error(),
        Some(code) if code == libc::EINPROGRESS || code == libc::EALREADY
    ) {
        return Err(error);
    }

    let deadline = Instant::now()
        .checked_add(timeout)
        .unwrap_or_else(Instant::now);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "Unix socket connection timed out",
            ));
        }
        let timeout_ms = remaining.as_millis().clamp(1, i32::MAX as u128) as i32;
        let mut pollfd = libc::pollfd {
            fd: socket.as_raw_fd(),
            events: libc::POLLOUT,
            revents: 0,
        };
        let polled = unsafe { libc::poll(&mut pollfd, 1, timeout_ms) };
        if polled < 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::EINTR) {
                continue;
            }
            return Err(error);
        }
        if polled == 0 {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "Unix socket connection timed out",
            ));
        }
        let mut socket_error = 0;
        let mut socket_error_len = std::mem::size_of_val(&socket_error) as libc::socklen_t;
        let status = unsafe {
            libc::getsockopt(
                socket.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_ERROR,
                (&mut socket_error as *mut libc::c_int).cast::<libc::c_void>(),
                &mut socket_error_len,
            )
        };
        if status < 0 {
            return Err(io::Error::last_os_error());
        }
        if socket_error != 0 {
            return Err(io::Error::from_raw_os_error(socket_error));
        }
        let stream = unsafe { UnixStream::from_raw_fd(socket.into_raw_fd()) };
        stream.set_nonblocking(false)?;
        return Ok(stream);
    }
}

#[derive(Debug)]
enum IpcStream {
    #[cfg(unix)]
    Unix(UnixStream),
    Tcp(TcpStream),
    #[cfg(windows)]
    Pipe {
        file: File,
        read_timeout: Cell<Option<Duration>>,
        write_timeout: Cell<Option<Duration>>,
    },
}

impl IpcStream {
    fn try_clone(&self) -> io::Result<Self> {
        match self {
            #[cfg(unix)]
            Self::Unix(stream) => stream.try_clone().map(Self::Unix),
            Self::Tcp(stream) => stream.try_clone().map(Self::Tcp),
            #[cfg(windows)]
            Self::Pipe {
                file,
                read_timeout,
                write_timeout,
            } => Ok(Self::Pipe {
                file: file.try_clone()?,
                read_timeout: Cell::new(read_timeout.get()),
                write_timeout: Cell::new(write_timeout.get()),
            }),
        }
    }

    fn set_read_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
        match self {
            #[cfg(unix)]
            Self::Unix(stream) => stream.set_read_timeout(timeout),
            Self::Tcp(stream) => stream.set_read_timeout(timeout),
            #[cfg(windows)]
            Self::Pipe { read_timeout, .. } => {
                read_timeout.set(timeout);
                Ok(())
            }
        }
    }

    fn set_write_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
        match self {
            #[cfg(unix)]
            Self::Unix(stream) => stream.set_write_timeout(timeout),
            Self::Tcp(stream) => stream.set_write_timeout(timeout),
            #[cfg(windows)]
            Self::Pipe { write_timeout, .. } => {
                write_timeout.set(timeout);
                Ok(())
            }
        }
    }
}

impl Read for IpcStream {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        match self {
            #[cfg(unix)]
            Self::Unix(stream) => stream.read(buffer),
            Self::Tcp(stream) => stream.read(buffer),
            #[cfg(windows)]
            Self::Pipe {
                file, read_timeout, ..
            } => read_named_pipe(file, buffer, read_timeout.get()),
        }
    }
}

impl Write for IpcStream {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        match self {
            #[cfg(unix)]
            Self::Unix(stream) => stream.write(buffer),
            Self::Tcp(stream) => stream.write(buffer),
            #[cfg(windows)]
            Self::Pipe {
                file,
                write_timeout,
                ..
            } => write_named_pipe(file, buffer, write_timeout.get()),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            #[cfg(unix)]
            Self::Unix(stream) => stream.flush(),
            Self::Tcp(stream) => stream.flush(),
            #[cfg(windows)]
            Self::Pipe { file, .. } => file.flush(),
        }
    }
}

#[cfg(windows)]
fn pipe_name(path: &Path) -> String {
    let value = path.to_string_lossy();
    value
        .strip_prefix(r"\\.\pipe\")
        .unwrap_or(value.as_ref())
        .to_string()
}

#[cfg(windows)]
fn valid_pipe_path(path: &str) -> bool {
    let Some(name) = path.strip_prefix(r"\\.\pipe\") else {
        return false;
    };
    let Some((prefix, suffix)) = name.rsplit_once('-') else {
        return false;
    };
    matches!(prefix, "vmctl-qmp" | "vmctl-agent")
        && suffix.len() == 16
        && suffix
            .chars()
            .all(|character| character.is_ascii_hexdigit())
}

#[cfg(windows)]
fn read_named_pipe(
    file: &mut File,
    buffer: &mut [u8],
    timeout: Option<Duration>,
) -> io::Result<usize> {
    use windows_sys::Win32::Foundation::HANDLE;
    use windows_sys::Win32::System::Pipes::PeekNamedPipe;

    let Some(timeout) = timeout else {
        return file.read(buffer);
    };
    let deadline = Instant::now()
        .checked_add(timeout)
        .unwrap_or_else(Instant::now);
    loop {
        let mut available = 0_u32;
        let success = unsafe {
            PeekNamedPipe(
                file.as_raw_handle() as HANDLE,
                std::ptr::null_mut(),
                0,
                std::ptr::null_mut(),
                &mut available,
                std::ptr::null_mut(),
            )
        };
        if success == 0 {
            return Err(io::Error::last_os_error());
        }
        if available != 0 {
            return file.read(buffer);
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "named pipe read timed out",
            ));
        }
        thread::sleep(remaining.min(Duration::from_millis(10)));
    }
}

#[cfg(windows)]
fn write_named_pipe(
    file: &mut File,
    buffer: &[u8],
    timeout: Option<Duration>,
) -> io::Result<usize> {
    let Some(timeout) = timeout else {
        return file.write(buffer);
    };
    let deadline = Instant::now()
        .checked_add(timeout)
        .unwrap_or_else(Instant::now);
    loop {
        match file.write(buffer) {
            Ok(written) => return Ok(written),
            Err(error) if named_pipe_write_retryable(&error) => {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "named pipe write timed out",
                    ));
                }
                thread::sleep(remaining.min(Duration::from_millis(5)));
            }
            Err(error) => return Err(error),
        }
    }
}

#[cfg(windows)]
fn named_pipe_write_retryable(error: &io::Error) -> bool {
    use windows_sys::Win32::Foundation::{ERROR_NO_DATA, ERROR_PIPE_BUSY};

    matches!(
        error.kind(),
        io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
    ) || matches!(error.raw_os_error(), Some(code) if code == ERROR_NO_DATA as i32 || code == ERROR_PIPE_BUSY as i32)
}

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
        let ssh_port = if uses_port_forwarding_network(config) {
            Some(match config.ssh_port {
                Some(port) => port,
                None => find_free_port(22220)?,
            })
        } else {
            None
        };
        let spice_port = match config.display.as_str() {
            "none" => Some(match config.spice_port {
                Some(port) => port,
                None => find_free_port(5930)?,
            }),
            "spice" if config.access != "local" || host_os == "windows" => {
                Some(match config.spice_port {
                    Some(port) => port,
                    None => find_free_port(5930)?,
                })
            }
            "spice" | "spice-app" => config.spice_port,
            _ => None,
        };

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
}

#[derive(Debug)]
pub(crate) struct VmOperationLock {
    file: File,
}

impl Drop for VmOperationLock {
    fn drop(&mut self) {
        unlock_operation_file(&self.file);
    }
}

pub(crate) fn acquire_vm_lock(paths: &VmPaths) -> Result<VmOperationLock> {
    fs::create_dir_all(&paths.state_dir)
        .map_err(|error| Error::io(paths.state_dir.display(), error))?;
    let path = paths.state_dir.join("operation.lock");
    let pid = std::process::id() as i32;
    let token = format!(
        "{pid}:{}:{}\n",
        process_identity(pid).unwrap_or_default(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| duration.as_nanos())
    );
    let mut file = open_operation_lock(&path).map_err(|error| Error::io(path.display(), error))?;
    if let Err(error) = lock_operation_file(&file) {
        if error.kind() == io::ErrorKind::WouldBlock {
            return Err(Error::message(format!(
                "another vmctl operation is already using {}; retry after it finishes",
                paths.state_dir.display()
            )));
        }
        return Err(Error::io(path.display(), error));
    }
    file.set_len(0)
        .and_then(|()| file.write_all(token.as_bytes()))
        .and_then(|()| file.sync_all())
        .map_err(|error| Error::io(path.display(), error))?;
    Ok(VmOperationLock { file })
}

fn reject_unsafe_operation_lock(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() {
                Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "refusing to follow a symbolic-link operation lock",
                ))
            } else {
                Ok(())
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn ensure_regular_operation_lock(path: &Path, file: File) -> io::Result<File> {
    if file.metadata()?.file_type().is_file() {
        Ok(file)
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("operation lock {} is not a regular file", path.display()),
        ))
    }
}

#[cfg(unix)]
fn open_operation_lock(path: &Path) -> io::Result<File> {
    reject_unsafe_operation_lock(path)?;
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    ensure_regular_operation_lock(path, file)
}

#[cfg(windows)]
fn open_operation_lock(path: &Path) -> io::Result<File> {
    use windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT;

    reject_unsafe_operation_lock(path)?;
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
    ensure_regular_operation_lock(path, file)
}

#[cfg(not(any(unix, windows)))]
fn open_operation_lock(path: &Path) -> io::Result<File> {
    reject_unsafe_operation_lock(path)?;
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)?;
    ensure_regular_operation_lock(path, file)
}

#[cfg(unix)]
fn lock_operation_file(file: &File) -> io::Result<()> {
    let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(unix)]
fn unlock_operation_file(file: &File) {
    let _ = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_UN) };
}

#[cfg(windows)]
fn lock_operation_file(file: &File) -> io::Result<()> {
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
fn unlock_operation_file(file: &File) {
    use windows_sys::Win32::Storage::FileSystem::UnlockFile;

    let _ = unsafe { UnlockFile(file.as_raw_handle(), 0, 0, 1, 0) };
}

#[cfg(not(any(unix, windows)))]
fn lock_operation_file(_file: &File) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "VM operation locking is unsupported on this platform",
    ))
}

#[cfg(not(any(unix, windows)))]
fn unlock_operation_file(_file: &File) {}

pub fn build_plan(vm: &Vm, host: &QemuPlanContext, prepare_firmware: bool) -> Result<QemuPlan> {
    let (qmp_endpoint, agent_endpoint) = ipc_endpoints(vm, host)?;
    let machine = machine_type(&vm.config);
    let tcg_accel = if host.accelerator == "tcg" {
        let ram_gib = host
            .ram
            .strip_suffix('G')
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(0);
        Some(format!(
            "tcg,tb-size={},thread=multi",
            if ram_gib >= 16 { 512 } else { 256 }
        ))
    } else {
        None
    };
    let smm = if host.host_os == "macos" {
        "off"
    } else if vm.config.secureboot
        || matches!(
            vm.config.guest_os.as_str(),
            "windows" | "windows-server" | "freedos"
        )
    {
        "on"
    } else {
        "off"
    };
    let cpu = cpu_model(&vm.config, host);
    let cores = vm.config.cpu_cores.unwrap_or(host.cpu_cores);
    let ram = vm.config.ram.clone().unwrap_or_else(|| host.ram.clone());
    let arm_bios = arm_monolithic_firmware(&vm.config);
    let mut args = Vec::new();

    let process_name = if host.host_os == "linux" {
        format!(
            "{},process={},debug-threads=on",
            vm.config.name, vm.config.name
        )
    } else if host.host_os == "macos" {
        vm.config.name.clone()
    } else {
        format!("{},process={}", vm.config.name, vm.config.name)
    };
    add(&mut args, "-name", process_name);
    add(
        &mut args,
        "-machine",
        if vm.config.arch == "aarch64" {
            let pflash = if vm.config.boot == "efi" && arm_bios.is_none() {
                ",pflash0=rom,pflash1=efivars"
            } else {
                ""
            };
            format!(
                "{machine},highmem=on{pflash}{}",
                tcg_accel
                    .as_deref()
                    .map_or_else(|| format!(",accel={}", host.accelerator), |_| String::new())
            )
        } else {
            let hpet = if matches!(
                vm.config.guest_os.as_str(),
                "macos" | "windows" | "windows-server"
            ) {
                ",hpet=off"
            } else {
                ""
            };
            format!(
                "{machine}{hpet},smm={smm},vmport=off{}",
                tcg_accel
                    .as_deref()
                    .map_or_else(|| format!(",accel={}", host.accelerator), |_| String::new())
            )
        },
    );
    if let Some(accel) = tcg_accel {
        args.extend(["-accel".to_string(), accel]);
    }
    add(&mut args, "-cpu", cpu);
    add(
        &mut args,
        "-smp",
        format!("cores={cores},threads=1,sockets=1"),
    );
    add(&mut args, "-m", ram);
    if vm.config.guest_os != "macos"
        || matches!(
            vm.config.macos_release.as_deref(),
            Some("big-sur" | "monterey" | "ventura" | "sonoma" | "sequoia" | "tahoe")
        )
    {
        args.extend(["-device".to_string(), "virtio-balloon".to_string()]);
    }
    add(
        &mut args,
        "-rtc",
        if matches!(
            vm.config.guest_os.as_str(),
            "windows" | "windows-server" | "reactos" | "freedos"
        ) {
            "base=localtime,clock=host,driftfix=slew".to_string()
        } else {
            "base=utc,clock=host".to_string()
        },
    );
    add(
        &mut args,
        "-pidfile",
        vm.paths.pid_file().display().to_string(),
    );
    args.extend([
        "-object".to_string(),
        if host.host_os == "windows" {
            "rng-builtin,id=rng0".to_string()
        } else {
            "rng-random,id=rng0,filename=/dev/urandom".to_string()
        },
        "-device".to_string(),
        "virtio-rng-pci,rng=rng0".to_string(),
    ]);

    if vm.config.boot == "efi" {
        let (efi_code, efi_vars) = firmware_paths(vm, prepare_firmware)?;
        if vm.config.arch == "aarch64" {
            if arm_bios.is_some() {
                add(&mut args, "-bios", qemu_path(&efi_code));
            } else {
                add(
                    &mut args,
                    "-blockdev",
                    format!(
                        "driver=file,filename={},node-name=rom,read-only=true",
                        qemu_path(&efi_code)
                    ),
                );
                add(
                    &mut args,
                    "-blockdev",
                    format!(
                        "driver=file,filename={},node-name=efivars",
                        qemu_path(&efi_vars)
                    ),
                );
            }
        } else {
            if vm.config.secureboot {
                add(
                    &mut args,
                    "-global",
                    "driver=cfi.pflash01,property=secure,value=on".to_string(),
                );
            }
            add(
                &mut args,
                "-drive",
                format!(
                    "if=pflash,format={},unit=0,file={},readonly=on",
                    firmware_format(&efi_code),
                    qemu_path(&efi_code)
                ),
            );
            add(
                &mut args,
                "-drive",
                format!(
                    "if=pflash,format={},unit=1,file={}",
                    firmware_format(&efi_vars),
                    qemu_path(&efi_vars)
                ),
            );
        }
    }

    add_storage_args(&mut args, vm)?;

    add_guest_tweaks(&mut args, &vm.config, machine);
    if host.accelerator == "kvm" && vm.config.arch == "x86_64" {
        args.extend([
            "-global".to_string(),
            "kvm-pit.lost_tick_policy=discard".to_string(),
        ]);
    }

    let mut display_config = vm.config.clone();
    if host.host_os == "macos" && display_config.display == "gtk" {
        display_config.display = "cocoa".to_string();
    }
    if display_config.display == "spice-app" {
        display_config.display = "spice".to_string();
        display_config.gl.get_or_insert(false);
    }
    if display_config.display == "cocoa" && host.host_os != "macos" {
        return Err(Error::message(
            "display mode 'cocoa' is only supported on macOS",
        ));
    }
    let display_backends = qemu_display_backends_probe(&host.qemu_binary);
    if let Some(display_backends) = display_backends {
        if display_backends.is_empty() {
            return Err(Error::message(
                "QEMU display capability query returned no backends",
            ));
        }
        let requested = match display_config.display.as_str() {
            "none" | "spice" => "none",
            display => display,
        };
        if !display_backends.iter().any(|backend| backend == requested) {
            if requested == "gtk" && display_backends.iter().any(|backend| backend == "sdl") {
                display_config.display = "sdl".to_string();
            } else {
                return Err(Error::message(format!(
                    "QEMU display backend '{requested}' is unavailable; available backends: {}",
                    display_backends.join(", ")
                )));
            }
        }
    } else {
        return Err(Error::message(
            "could not query QEMU display backends; verify the QEMU binary and retry",
        ));
    }
    if matches!(display_config.display.as_str(), "none" | "spice")
        && !is_loopback_host(spice_address(&display_config))
        && !vm.config.allow_insecure_remote
    {
        return Err(Error::message(
            "remote SPICE is unauthenticated; bind it to localhost or pass --allow-insecure-remote after securing the network",
        ));
    }
    for (mode, host_name) in [
        (
            "monitor",
            (&vm.config.monitor, &vm.config.monitor_telnet_host),
        ),
        ("serial", (&vm.config.serial, &vm.config.serial_telnet_host)),
    ] {
        if host_name.0 == "telnet"
            && !is_loopback_host(host_name.1)
            && !vm.config.allow_insecure_remote
        {
            return Err(Error::message(format!(
                "remote {mode} Telnet is unauthenticated; bind it to localhost or pass --allow-insecure-remote after securing the network"
            )));
        }
    }
    let (display, video, spice_port) = display_args(&display_config, host)?;
    if matches!(display_config.display.as_str(), "none" | "spice") {
        args.extend(["-vga".to_string(), "none".to_string()]);
        if video != "none" {
            args.extend(["-device".to_string(), video]);
        }
    } else if video == "none" {
        args.extend(["-vga".to_string(), "none".to_string()]);
    } else {
        args.extend(["-device".to_string(), video]);
    }
    if vm.config.arch == "aarch64" {
        args.extend(["-device".to_string(), "ramfb".to_string()]);
    }
    add(&mut args, "-display", display);
    match display_config.display.as_str() {
        "none" => {
            if let Some(port) = spice_port {
                add(
                    &mut args,
                    "-spice",
                    format!(
                        "port={port},addr={},disable-ticketing=on",
                        spice_address(&vm.config)
                    ),
                );
            }
        }
        "spice" => add(
            &mut args,
            "-spice",
            host.spice_port.map_or_else(
                || {
                    #[cfg(unix)]
                    {
                        format!(
                            "unix=on,addr={},disable-ticketing=on",
                            qemu_path(&vm.paths.spice_socket())
                        )
                    }
                    #[cfg(windows)]
                    {
                        format!(
                            "port={},addr={},disable-ticketing=on",
                            control_port(&vm.paths.spice_socket()),
                            spice_address(&vm.config)
                        )
                    }
                },
                |port| {
                    format!(
                        "port={port},addr={},disable-ticketing=on",
                        spice_address(&vm.config)
                    )
                },
            ),
        ),
        _ => {}
    }

    add_usb_args(&mut args, &vm.config);
    let audio_driver = if matches!(display_config.display.as_str(), "none" | "spice") {
        Some("spice")
    } else {
        host.audio_driver.as_deref()
    };
    add_audio_args(&mut args, &vm.config, audio_driver);
    add_network_args(
        &mut args,
        vm,
        host.ssh_port,
        host.smbd,
        host.bridge_helper.as_deref(),
    )?;
    add_share_args(&mut args, vm, host);

    let spice = matches!(display_config.display.as_str(), "none" | "spice");
    let gtk_clipboard = display_config.display == "gtk" && display_config.clipboard;
    if vm.config.guest_agent || spice || gtk_clipboard {
        args.extend(["-device".to_string(), "virtio-serial-pci".to_string()]);
    }
    if vm.config.guest_agent {
        add(
            &mut args,
            "-chardev",
            agent_endpoint
                .as_ref()
                .expect("guest agent endpoint exists when guest_agent is enabled")
                .guest_agent_argument(),
        );
        add(
            &mut args,
            "-device",
            "virtserialport,chardev=qga0,name=org.qemu.guest_agent.0".to_string(),
        );
    }
    if spice {
        args.extend([
            "-chardev".to_string(),
            "spicevmc,id=vdagent0,name=vdagent".to_string(),
            "-device".to_string(),
            "virtserialport,chardev=vdagent0,name=com.redhat.spice.0".to_string(),
            "-chardev".to_string(),
            "spiceport,id=webdav0,name=org.spice-space.webdav.0".to_string(),
            "-device".to_string(),
            "virtserialport,chardev=webdav0,name=org.spice-space.webdav.0".to_string(),
        ]);
        if host.usb_redirection {
            args.extend(["-device".to_string(), "qemu-xhci,id=spicepass".to_string()]);
            for index in 1..=3 {
                args.extend([
                    "-chardev".to_string(),
                    format!("spicevmc,id=usbredirchardev{index},name=usbredir"),
                    "-device".to_string(),
                    format!("usb-redir,chardev=usbredirchardev{index},id=usbredirdev{index}"),
                ]);
            }
        }
        if host.smartcard {
            args.extend([
                "-device".to_string(),
                "pci-ohci,id=smartpass".to_string(),
                "-device".to_string(),
                "usb-ccid".to_string(),
                "-chardev".to_string(),
                "spicevmc,id=ccid,name=smartcard".to_string(),
                "-device".to_string(),
                "ccid-card-passthru,chardev=ccid".to_string(),
            ]);
        }
    }
    if gtk_clipboard {
        args.extend([
            "-chardev".to_string(),
            "qemu-vdagent,id=vdagent0,name=vdagent,clipboard=on".to_string(),
            "-device".to_string(),
            "virtserialport,chardev=vdagent0,name=com.redhat.spice.0".to_string(),
        ]);
    }

    if vm.config.tpm {
        add_tpm_args(&mut args, vm, &host.host_os);
    }

    // QMP is vmctl's management channel, so it remains available even when
    // the legacy monitor option is set to "none".
    qmp_endpoint.add_qmp_args(&mut args);

    match vm.config.monitor.as_str() {
        "none" => add(&mut args, "-monitor", "none".to_string()),
        "socket" => add(
            &mut args,
            "-monitor",
            control_endpoint(&vm.paths.monitor_socket(), &host.host_os),
        ),
        "telnet" => add(
            &mut args,
            "-monitor",
            format!(
                "telnet:{}:{},server=on,wait=off",
                qemu_host(&vm.config.monitor_telnet_host),
                vm.config.monitor_telnet_port
            ),
        ),
        monitor => {
            return Err(Error::message(format!(
                "monitor mode '{monitor}' is unsupported"
            )));
        }
    }

    match vm.config.serial.as_str() {
        "none" => args.extend(["-serial".to_string(), "none".to_string()]),
        "socket" => add(
            &mut args,
            "-serial",
            control_endpoint(&vm.paths.serial_socket(), &host.host_os),
        ),
        "telnet" => add(
            &mut args,
            "-serial",
            format!(
                "telnet:{}:{},server=on,wait=off",
                qemu_host(&vm.config.serial_telnet_host),
                vm.config.serial_telnet_port
            ),
        ),
        serial => {
            return Err(Error::message(format!(
                "serial mode '{serial}' is unsupported"
            )));
        }
    }

    if vm.config.status_quo {
        args.push("-snapshot".to_string());
    }
    args.extend(vm.config.extra_args.clone());

    Ok(QemuPlan {
        binary: host.qemu_binary.clone(),
        args,
        qmp_endpoint,
        agent_endpoint,
        ssh_port: host.ssh_port,
        ssh_host: host.ssh_port.map(|_| ssh_address(&vm.config).to_string()),
        spice_port,
        spice_host: spice_port.map(|_| spice_address(&vm.config).to_string()),
        monitor_telnet: (vm.config.monitor == "telnet").then(|| {
            (
                vm.config.monitor_telnet_host.clone(),
                vm.config.monitor_telnet_port,
            )
        }),
        serial_telnet: (vm.config.serial == "telnet").then(|| {
            (
                vm.config.serial_telnet_host.clone(),
                vm.config.serial_telnet_port,
            )
        }),
    })
}

fn ipc_endpoints(vm: &Vm, host: &QemuPlanContext) -> Result<(IpcEndpoint, Option<IpcEndpoint>)> {
    if vm.paths.ipc_state().is_file() {
        let (qmp, agent) = read_ipc_state(&vm.paths)?;
        let agent =
            if vm.config.guest_agent {
                Some(agent.ok_or_else(|| {
                    Error::message("runtime IPC state has no guest-agent endpoint")
                })?)
            } else {
                None
            };
        return Ok((qmp, agent));
    }

    #[cfg(windows)]
    if host.host_os == "windows" {
        let qmp = named_pipe_endpoint("qmp");
        let agent = vm.config.guest_agent.then(|| named_pipe_endpoint("agent"));
        return Ok((qmp, agent));
    }

    #[cfg(not(windows))]
    if host.host_os == "windows" {
        let qmp = ephemeral_loopback_endpoint(&[])?;
        let agent = if vm.config.guest_agent {
            Some(ephemeral_loopback_endpoint(std::slice::from_ref(&qmp))?)
        } else {
            None
        };
        return Ok((qmp, agent));
    }

    #[cfg(unix)]
    {
        Ok((
            IpcEndpoint::Unix(vm.paths.qmp_socket()),
            vm.config
                .guest_agent
                .then(|| IpcEndpoint::Unix(vm.paths.agent_socket())),
        ))
    }
    #[cfg(not(unix))]
    {
        let qmp = ephemeral_loopback_endpoint(&[])?;
        let agent = if vm.config.guest_agent {
            Some(ephemeral_loopback_endpoint(std::slice::from_ref(&qmp))?)
        } else {
            None
        };
        Ok((qmp, agent))
    }
}

#[cfg(windows)]
fn named_pipe_endpoint(role: &str) -> IpcEndpoint {
    let nonce = next_guest_sync_id().unsigned_abs();
    IpcEndpoint::Pipe(PathBuf::from(format!(
        r"\\.\pipe\vmctl-{role}-{nonce:016x}"
    )))
}

fn ephemeral_loopback_endpoint(excluded: &[IpcEndpoint]) -> Result<IpcEndpoint> {
    for _ in 0..8 {
        let listener =
            TcpListener::bind(("127.0.0.1", 0)).map_err(|error| Error::io("127.0.0.1:0", error))?;
        let address = listener
            .local_addr()
            .map_err(|error| Error::io("127.0.0.1:0", error))?;
        let endpoint = IpcEndpoint::Tcp(address);
        if !excluded.contains(&endpoint) {
            return Ok(endpoint);
        }
    }
    Err(Error::message(
        "could not allocate distinct loopback IPC ports",
    ))
}

fn read_ipc_state(paths: &VmPaths) -> Result<(IpcEndpoint, Option<IpcEndpoint>)> {
    let path = paths.ipc_state();
    let contents = fs::read_to_string(&path).map_err(|error| {
        Error::message(format!(
            "runtime IPC state {} is unavailable: {error}",
            path.display()
        ))
    })?;
    let value: Value = serde_json::from_str(&contents).map_err(|error| {
        Error::message(format!(
            "runtime IPC state {} is invalid JSON: {error}",
            path.display()
        ))
    })?;
    if value.get("schema_version").and_then(Value::as_u64) != Some(1) {
        return Err(Error::message(format!(
            "runtime IPC state {} has unsupported schema_version",
            path.display()
        )));
    }
    let qmp = value
        .get("qmp")
        .ok_or_else(|| Error::message("runtime IPC state has no QMP endpoint"))
        .and_then(IpcEndpoint::from_json)?;
    let agent = value
        .get("guest_agent")
        .filter(|value| !value.is_null())
        .map(IpcEndpoint::from_json)
        .transpose()?;
    if agent.as_ref() == Some(&qmp) {
        return Err(Error::message(
            "runtime IPC state reuses the QMP endpoint for the guest agent",
        ));
    }
    Ok((qmp, agent))
}

fn default_qmp_endpoint(paths: &VmPaths) -> Result<IpcEndpoint> {
    #[cfg(unix)]
    {
        Ok(IpcEndpoint::Unix(paths.qmp_socket()))
    }
    #[cfg(not(unix))]
    {
        Err(Error::message(format!(
            "runtime IPC state {} is missing; start the VM again before connecting",
            paths.ipc_state().display()
        )))
    }
}

fn default_agent_endpoint(paths: &VmPaths) -> Result<IpcEndpoint> {
    #[cfg(unix)]
    {
        Ok(IpcEndpoint::Unix(paths.agent_socket()))
    }
    #[cfg(not(unix))]
    {
        Err(Error::message(format!(
            "runtime IPC state {} is missing; start the VM again before using the guest agent",
            paths.ipc_state().display()
        )))
    }
}

fn qmp_endpoint_for_paths(paths: &VmPaths) -> Result<IpcEndpoint> {
    if paths.ipc_state().is_file() {
        read_ipc_state(paths).map(|state| state.0)
    } else {
        default_qmp_endpoint(paths)
    }
}

fn agent_endpoint_for_paths(paths: &VmPaths) -> Result<IpcEndpoint> {
    if paths.ipc_state().is_file() {
        read_ipc_state(paths).and_then(|state| {
            state
                .1
                .ok_or_else(|| Error::message("guest-agent endpoint is not configured"))
        })
    } else {
        default_agent_endpoint(paths)
    }
}

fn machine_type(config: &VmConfig) -> &'static str {
    if config.arch == "aarch64" {
        "virt"
    } else if config.boot == "legacy"
        || matches!(
            config.guest_os.as_str(),
            "batocera" | "freedos" | "haiku" | "kolibrios" | "reactos" | "solaris"
        )
    {
        "pc"
    } else {
        "q35"
    }
}

fn add_guest_tweaks(args: &mut Vec<String>, config: &VmConfig, machine: &str) {
    match config.guest_os.as_str() {
        "macos" if machine == "q35" => args.extend([
            "-global".to_string(),
            "ICH9-LPC.disable_s3=1".to_string(),
            "-global".to_string(),
            "ICH9-LPC.acpi-pci-hotplug-with-bridge-support=off".to_string(),
            "-device".to_string(),
            "isa-applesmc,osk=ourhardworkbythesewordsguardedpleasedontsteal(c)AppleComputerInc"
                .to_string(),
        ]),
        "windows" | "windows-server" if machine == "q35" => {
            args.extend(["-global".to_string(), "ICH9-LPC.disable_s3=1".to_string()])
        }
        _ => {}
    }
}

fn cpu_model(config: &VmConfig, host: &QemuPlanContext) -> String {
    if let Some(cpu) = &config.cpu_model {
        return cpu.clone();
    }
    if config.arch == "aarch64" {
        return "max".to_string();
    }
    match config.guest_os.as_str() {
        "kolibrios" | "reactos" => "qemu32".to_string(),
        "macos" => {
            if host.accelerator == "tcg" {
                "Haswell-v2,vendor=GenuineIntel,-pdpe1gb,+avx,+sse,+sse2,+ssse3,vmware-cpuid-freq=on"
                    .to_string()
            } else {
                "host,-pdpe1gb,+hypervisor,vmware-cpuid-freq=on".to_string()
            }
        }
        "windows" | "windows-server" => {
            let base = if host.accelerator == "kvm" {
                "host"
            } else {
                "qemu64"
            };
            format!("{base},+hypervisor,+invtsc,l3-cache=on")
        }
        _ if host.accelerator == "kvm" || host.accelerator == "hvf" => "host".to_string(),
        _ => "qemu64".to_string(),
    }
}

fn add_storage_args(args: &mut Vec<String>, vm: &Vm) -> Result<()> {
    let config = &vm.config;
    let optimisations = "discard=unmap,detect-zeroes=unmap,cache=writeback,aio=threads";

    if config.guest_os == "macos" {
        let parent = config.disk_img.parent().unwrap_or_else(|| Path::new("."));
        let bootloader = [parent.join("OpenCore.qcow2"), parent.join("ESP.qcow2")]
            .into_iter()
            .find(|path| path.is_file())
            .ok_or_else(|| {
                Error::message(format!(
                    "macOS bootloader not found beside {} (expected OpenCore.qcow2 or ESP.qcow2)",
                    config.disk_img.display()
                ))
            })?;
        args.extend([
            "-device".to_string(),
            "ahci,id=ahci".to_string(),
            "-drive".to_string(),
            format!(
                "id=BootLoader,if=none,format=qcow2,file={}",
                qemu_path(&bootloader)
            ),
            "-device".to_string(),
            "ide-hd,bus=ahci.0,drive=BootLoader,bootindex=0".to_string(),
        ]);
        if let Some(image) = &config.img {
            add_optional_drive_with_id(args, image, "RecoveryImage", "raw", "")?;
            args.extend([
                "-device".to_string(),
                "ide-hd,bus=ahci.1,drive=RecoveryImage".to_string(),
            ]);
        }
        let device = match config.macos_release.as_deref() {
            Some(
                "catalina" | "big-sur" | "monterey" | "ventura" | "sonoma" | "sequoia" | "tahoe",
            ) => "virtio-blk-pci",
            _ => "ide-hd,bus=ahci.2",
        };
        add_system_disk(args, config, device, optimisations)?;
        return Ok(());
    }

    let has_iso =
        config.iso.is_some() || config.fixed_iso.is_some() || config.unattended_iso.is_some();
    if config.arch == "aarch64" && has_iso {
        args.extend([
            "-device".to_string(),
            "virtio-scsi-pci,id=scsi0".to_string(),
        ]);
        if let Some(iso) = &config.iso {
            add_optional_drive_with_id(args, iso, "cd0", "raw", "media=cdrom,readonly=on")?;
            args.extend([
                "-device".to_string(),
                "scsi-cd,drive=cd0,bus=scsi0.0,bootindex=1".to_string(),
            ]);
        }
        if let Some(iso) = &config.fixed_iso {
            add_optional_drive_with_id(args, iso, "cd1", "raw", "media=cdrom,readonly=on")?;
            args.extend([
                "-device".to_string(),
                "scsi-cd,drive=cd1,bus=scsi0.0,bootindex=3".to_string(),
            ]);
        }
        if let Some(iso) = &config.unattended_iso {
            add_optional_drive_with_id(args, iso, "cd2", "raw", "media=cdrom,readonly=on")?;
            args.extend([
                "-device".to_string(),
                "scsi-cd,drive=cd2,bus=scsi0.0,bootindex=4".to_string(),
            ]);
        }
    } else {
        if let Some(iso) = &config.iso {
            let options = if config.guest_os == "reactos" {
                "if=ide,index=2,media=cdrom"
            } else {
                "media=cdrom,index=0,readonly=on"
            };
            add_optional_drive(args, &Some(iso.clone()), options)?;
        }
        if let Some(iso) = &config.fixed_iso {
            add_optional_drive(args, &Some(iso.clone()), "media=cdrom,index=1,readonly=on")?;
        }
        if let Some(iso) = &config.unattended_iso {
            add_optional_drive(args, &Some(iso.clone()), "media=cdrom,index=2,readonly=on")?;
        }
    }
    add_optional_drive(args, &config.floppy, "if=floppy,format=raw")?;

    if config.guest_os == "batocera" {
        let image = config
            .img
            .as_ref()
            .ok_or_else(|| Error::message("batocera requires img"))?;
        add_optional_drive_with_id(args, image, "BootDisk", "raw", "")?;
        args.extend([
            "-device".to_string(),
            "virtio-blk-pci,drive=BootDisk".to_string(),
        ]);
    }
    if config.guest_os == "freedos" && config.iso.is_some() {
        args.extend(["-boot".to_string(), "order=dc".to_string()]);
    }
    if config.guest_os == "kolibrios" && config.iso.is_some() {
        args.extend(["-boot".to_string(), "order=d".to_string()]);
    }

    if config.guest_os == "reactos" {
        add(
            args,
            "-drive",
            format!(
                "if=ide,index=0,media=disk,format={},file={}",
                config.disk_format,
                qemu_path(&config.disk_img)
            ),
        );
        return Ok(());
    }
    if config.guest_os == "kolibrios" {
        args.extend(["-device".to_string(), "ahci,id=ahci".to_string()]);
    }
    let device = match config.guest_os.as_str() {
        "windows-server" => "ide-hd",
        "kolibrios" => "ide-hd,bus=ahci.0",
        "macos" => match config.macos_release.as_deref() {
            Some(
                "catalina" | "big-sur" | "monterey" | "ventura" | "sonoma" | "sequoia" | "tahoe",
            ) => "virtio-blk-pci",
            _ => "ide-hd,bus=ahci.2",
        },
        _ if config.arch == "aarch64" => "virtio-blk-pci,bootindex=2",
        _ => "virtio-blk-pci",
    };
    add_system_disk(args, config, device, optimisations)
}

fn add_system_disk(
    args: &mut Vec<String>,
    config: &VmConfig,
    device: &str,
    optimisations: &str,
) -> Result<()> {
    add(
        args,
        "-drive",
        format!(
            "id=SystemDisk,if=none,format={},file={},{}",
            config.disk_format,
            qemu_path(&config.disk_img),
            optimisations
        ),
    );
    args.extend(["-device".to_string(), format!("{device},drive=SystemDisk")]);
    Ok(())
}

fn add_optional_drive_with_id(
    args: &mut Vec<String>,
    path: &Path,
    id: &str,
    format: &str,
    options: &str,
) -> Result<()> {
    if !path.is_file() {
        return Err(Error::message(format!(
            "configured media file {} does not exist",
            path.display()
        )));
    }
    let options = (!options.is_empty()).then_some(format!(",{options}"));
    add(
        args,
        "-drive",
        format!(
            "id={id},if=none,format={format}{},file={}",
            options.as_deref().unwrap_or_default(),
            qemu_path(path)
        ),
    );
    Ok(())
}

fn add_tpm_args(args: &mut Vec<String>, vm: &Vm, host_os: &str) {
    add(
        args,
        "-chardev",
        socket_chardev(&vm.paths.tpm_socket(), "chrtpm", host_os),
    );
    add(
        args,
        "-tpmdev",
        "emulator,id=tpm0,chardev=chrtpm".to_string(),
    );
    args.extend([
        "-device".to_string(),
        if vm.config.arch == "aarch64" {
            "tpm-tis-device,tpmdev=tpm0".to_string()
        } else {
            "tpm-tis,tpmdev=tpm0".to_string()
        },
    ]);
}

fn display_args(
    config: &VmConfig,
    host: &QemuPlanContext,
) -> Result<(String, String, Option<u16>)> {
    if config.clipboard && config.display != "gtk" {
        return Err(Error::message(
            "clipboard requires the GTK display backend; select --display gtk on a host where GTK is available",
        ));
    }
    if config.clipboard && !qemu_supports_gtk_clipboard(&host.qemu_binary) {
        return Err(Error::message(
            "GTK clipboard sharing requires QEMU 11.1.0 or newer",
        ));
    }
    if config.clipboard && !qemu_supports_vdagent(&host.qemu_binary) {
        return Err(Error::message(
            "GTK clipboard sharing requires QEMU built with qemu-vdagent support; install QEMU's SPICE module",
        ));
    }
    if config.display == "cocoa" && host.host_os != "macos" {
        return Err(Error::message(
            "display mode 'cocoa' is only supported on macOS",
        ));
    }
    let render_node = render_node();
    let local_spice_gl = config.display == "spice"
        && config.access == "local"
        && config.gl.unwrap_or(true)
        && host.virtio_vga_gl
        && qemu_display_backends_probe(&host.qemu_binary)
            .is_some_and(|backends| backends.iter().any(|backend| backend == "egl-headless"))
        && render_node.is_some();
    let requested_gl = config.gl.unwrap_or(true)
        && !matches!(config.display.as_str(), "none")
        && (config.display != "spice" || local_spice_gl);
    let device = match config.guest_os.as_str() {
        guest if guest.ends_with("bsd") => "VGA",
        "linux-old" | "linux_old" | "solaris" => "vmware-svga",
        "macos" => "vmware-svga",
        "windows" | "windows-server" if config.arch == "aarch64" => "virtio-gpu-pci",
        "windows" | "windows-server" if matches!(config.display.as_str(), "none" | "spice") => {
            "qxl-vga"
        }
        "batocera" | "haiku" | "kolibrios" | "reactos" => "qxl-vga",
        _ if config.arch == "aarch64" => "virtio-gpu-pci",
        "linux" if matches!(config.display.as_str(), "none" | "spice" | "spice-app") => {
            "virtio-gpu"
        }
        _ => "virtio-vga",
    };
    let gl_device = match device {
        "virtio-vga" => Some("virtio-vga-gl"),
        "virtio-gpu-pci" => Some("virtio-gpu-gl-pci"),
        "virtio-gpu" => Some("virtio-gpu-gl"),
        _ => None,
    };
    let device =
        if requested_gl && gl_device.is_some_and(|device| gl_device_supported(host, device)) {
            match device {
                "virtio-vga" => "virtio-vga-gl",
                "virtio-gpu-pci" => "virtio-gpu-gl-pci",
                "virtio-gpu" => "virtio-gpu-gl",
                _ => device,
            }
        } else {
            device
        };
    let supports_resolution = device.starts_with("virtio-") || device.starts_with("qxl");
    let resolution = if supports_resolution {
        match (config.width, config.height) {
            (Some(width), Some(height)) => format!(",xres={width},yres={height}"),
            _ => String::new(),
        }
    } else {
        String::new()
    };
    let max_outputs = if device.starts_with("virtio-") || device.starts_with("qxl") {
        config
            .max_outputs
            .map_or_else(String::new, |outputs| format!(",max_outputs={outputs}"))
    } else {
        String::new()
    };
    let spice_port = match config.display.as_str() {
        "none" | "spice" => host.spice_port,
        _ => None,
    };
    let video = if device == "none" {
        "none".to_string()
    } else {
        format!("{device}{resolution}{max_outputs}")
    };
    let fullscreen = if config.fullscreen {
        ",full-screen=on"
    } else {
        ""
    };
    let gl = if requested_gl && (device.contains("-gl") || config.display == "cocoa") {
        "on"
    } else {
        "off"
    };

    let result = match config.display.as_str() {
        "none" => ("none".to_string(), video, spice_port),
        "spice" => (
            render_node.filter(|_| local_spice_gl).map_or_else(
                || "none".to_string(),
                |path| format!("egl-headless,rendernode={}", qemu_path(&path)),
            ),
            video,
            spice_port,
        ),
        "gtk" => (
            format!(
                "gtk{},grab-on-hover=on,zoom-to-fit=off,gl={gl}{fullscreen}",
                if config.clipboard {
                    ",clipboard=on"
                } else {
                    ""
                }
            ),
            video,
            None,
        ),
        "sdl" => (format!("sdl,gl={gl}{fullscreen}"), video, None),
        "cocoa" => (format!("cocoa{fullscreen}"), video, None),
        display => {
            return Err(Error::message(format!(
                "display mode '{display}' is not supported"
            )));
        }
    };

    Ok(result)
}

pub fn spice_address(config: &VmConfig) -> &str {
    match config.access.as_str() {
        "local" | "" => "127.0.0.1",
        "remote" => "0.0.0.0",
        address => address,
    }
}

fn qemu_host(host: &str) -> String {
    if host.parse::<std::net::Ipv6Addr>().is_ok() {
        format!("[{host}]")
    } else {
        host.to_string()
    }
}

fn is_loopback_host(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

fn ssh_address(config: &VmConfig) -> &str {
    match config.ssh_access.as_str() {
        "local" | "" => "127.0.0.1",
        "remote" => "0.0.0.0",
        address => address,
    }
}

fn add_network_args(
    args: &mut Vec<String>,
    vm: &Vm,
    ssh_port: Option<u16>,
    smbd: bool,
    bridge_helper: Option<&str>,
) -> Result<()> {
    if vm.config.offline {
        args.extend(["-nic".to_string(), "none".to_string()]);
        return Ok(());
    }

    let net_device = match vm.config.guest_os.as_str() {
        "freedos" => "pcnet",
        "haiku" | "kolibrios" | "solaris" => "rtl8139",
        "reactos" | "windows-server" => "e1000",
        "macos"
            if matches!(
                vm.config.macos_release.as_deref(),
                Some("big-sur" | "monterey" | "ventura" | "sonoma" | "sequoia" | "tahoe")
            ) =>
        {
            "virtio-net-pci"
        }
        "macos" => "vmxnet3",
        _ => "virtio-net-pci",
    };

    if vm.config.network.eq_ignore_ascii_case("none") {
        args.extend(["-nic".to_string(), "none".to_string()]);
        return Ok(());
    }
    let bridge = configured_bridge(&vm.config);
    if let Some(bridge) = bridge {
        let helper = bridge_helper.ok_or_else(|| {
            Error::message(
                "bridged networking requires qemu-bridge-helper; install it or use network=user",
            )
        })?;
        let mac = vm
            .config
            .macaddr
            .as_deref()
            .map_or_else(String::new, |mac| format!(",mac={mac}"));
        args.extend([
            "-nic".to_string(),
            format!(
                "bridge,br={bridge},helper={},model={net_device}{mac}",
                qemu_path(Path::new(helper))
            ),
        ]);
        return Ok(());
    }

    if uses_passt_network(&vm.config) {
        if smbd
            && matches!(vm.config.guest_os.as_str(), "windows" | "windows-server")
            && vm
                .config
                .public_dir
                .as_ref()
                .is_some_and(|path| path.is_dir())
        {
            return Err(Error::message(
                "Windows SMB sharing requires network=user; passt does not provide QEMU's SMB server",
            ));
        }
        let tcp_ports = vm
            .config
            .port_forwards
            .iter()
            .map(|(host, guest)| format!("{host}:{guest}"))
            .collect::<Vec<_>>();
        let mac = vm
            .config
            .macaddr
            .as_deref()
            .map_or_else(String::new, |mac| format!(",mac={mac}"));
        let mut net = "passt,id=nic,tcp-ports=none,udp-ports=none".to_string();
        if let Some(port) = ssh_port {
            net.push_str(&format!(
                ",param=--tcp-ports={}/{port}:22",
                ssh_address(&vm.config)
            ));
        }
        if !tcp_ports.is_empty() {
            let ports = tcp_ports.join(",,");
            net.push_str(&format!(",param=--tcp-ports=127.0.0.1/{ports}"));
            net.push_str(&format!(",param=--udp-ports=127.0.0.1/{ports}"));
        }
        args.extend([
            "-device".to_string(),
            format!("{net_device},netdev=nic{mac}"),
            "-netdev".to_string(),
            net,
        ]);
        return Ok(());
    }

    let mut net = format!("user,id=nic,hostname={}", vm.config.name);
    if vm.config.network.eq_ignore_ascii_case("restrict") {
        net.push_str(",restrict=on");
    }
    if let Some(port) = ssh_port {
        net.push_str(&format!(
            ",hostfwd=tcp:{}:{port}-:22",
            ssh_address(&vm.config)
        ));
    }
    for (host, guest) in &vm.config.port_forwards {
        net.push_str(&format!(",hostfwd=tcp:127.0.0.1:{host}-:{guest}"));
        net.push_str(&format!(",hostfwd=udp:127.0.0.1:{host}-:{guest}"));
    }
    if smbd
        && matches!(vm.config.guest_os.as_str(), "windows" | "windows-server")
        && let Some(public_dir) = &vm.config.public_dir
        && public_dir.is_dir()
    {
        net.push_str(&format!(",smb={}", qemu_path(public_dir)));
    }
    let mac = vm
        .config
        .macaddr
        .as_deref()
        .map_or_else(String::new, |mac| format!(",mac={mac}"));
    args.extend([
        "-device".to_string(),
        format!("{net_device},netdev=nic{mac}"),
        "-netdev".to_string(),
        net,
    ]);
    Ok(())
}

pub(crate) fn virtiofs_requested(config: &VmConfig, host: &QemuPlanContext) -> bool {
    host.host_os == "linux"
        && host.virtiofsd.is_some()
        && host.virtiofs_device
        && config.guest_os == "linux"
        && config.iso.is_none()
        && config.fixed_iso.is_none()
        && config.unattended_iso.is_none()
        && config.public_dir.as_ref().is_some_and(|path| path.is_dir())
}

fn add_share_args(args: &mut Vec<String>, vm: &Vm, host: &QemuPlanContext) {
    let config = &vm.config;
    if matches!(config.guest_os.as_str(), "windows" | "windows-server")
        || !(config.guest_os.starts_with("linux") || config.guest_os == "macos")
    {
        return;
    }
    let Some(public_dir) = &config.public_dir else {
        return;
    };
    if !public_dir.is_dir() {
        return;
    }

    let username = host
        .username
        .chars()
        .filter(|character| character.is_ascii_alphanumeric() || "-_".contains(*character))
        .collect::<String>();
    let mount_tag = format!(
        "Public-{}",
        if username.is_empty() {
            "user"
        } else {
            username.as_str()
        }
    );
    if virtiofs_requested(config, host) {
        let ram = config.ram.as_deref().unwrap_or(host.ram.as_str());
        args.extend([
            "-object".to_string(),
            format!("memory-backend-file,id=mem,size={ram},mem-path=/dev/shm,share=on"),
            "-numa".to_string(),
            "node,memdev=mem".to_string(),
            "-chardev".to_string(),
            format!(
                "socket,id=char0,path={}",
                qemu_path(&vm.paths.virtiofs_socket())
            ),
            "-device".to_string(),
            format!("vhost-user-fs-pci,queue-size=1024,chardev=char0,tag={mount_tag}"),
        ]);
        return;
    }

    args.extend([
        "-fsdev".to_string(),
        format!(
            "local,id=fsdev0,path={},security_model=mapped-xattr",
            qemu_path(public_dir)
        ),
        "-device".to_string(),
        format!("virtio-9p-pci,fsdev=fsdev0,mount_tag={mount_tag}"),
    ]);
}

fn add_usb_args(args: &mut Vec<String>, config: &VmConfig) {
    match config.usb_controller.as_str() {
        "ehci" => args.extend(["-device".to_string(), "usb-ehci,id=input".to_string()]),
        "xhci" => args.extend(["-device".to_string(), "qemu-xhci,id=input".to_string()]),
        "none" => {}
        _ => {}
    }
    match config.keyboard.as_str() {
        "usb" => args.extend(["-device".to_string(), "usb-kbd,bus=input.0".to_string()]),
        "virtio" => args.extend(["-device".to_string(), "virtio-keyboard".to_string()]),
        _ => {}
    }
    if !config.keyboard_layout.is_empty() {
        args.extend(["-k".to_string(), config.keyboard_layout.clone()]);
    }
    match config.mouse.as_str() {
        "usb" => args.extend(["-device".to_string(), "usb-mouse,bus=input.0".to_string()]),
        "tablet" => args.extend(["-device".to_string(), "usb-tablet,bus=input.0".to_string()]),
        "virtio" => args.extend(["-device".to_string(), "virtio-mouse".to_string()]),
        _ => {}
    }
    if !config.usb_devices.is_empty() {
        args.extend(["-device".to_string(), "qemu-xhci,id=hostpass".to_string()]);
        for (vendor, product) in &config.usb_devices {
            args.extend([
                "-device".to_string(),
                format!(
                    "usb-host,bus=hostpass.0,vendorid=0x{vendor:04x},productid=0x{product:04x}"
                ),
            ]);
        }
    }
    if config.braille {
        args.extend(["-usbdevice".to_string(), "braille".to_string()]);
    }
}

fn add_audio_args(args: &mut Vec<String>, config: &VmConfig, driver: Option<&str>) {
    let Some(driver) = driver else {
        return;
    };
    if config.sound_card == "none" {
        return;
    }

    args.extend([
        "-audiodev".to_string(),
        format!("driver={driver},id=audio0"),
    ]);
    match config.sound_card.as_str() {
        "ich9-intel-hda" | "intel-hda" => args.extend([
            "-device".to_string(),
            config.sound_card.clone(),
            "-device".to_string(),
            format!("{},audiodev=audio0", config.sound_duplex),
        ]),
        "usb-audio" | "virtio-sound-pci" | "ac97" | "es1370" | "sb16" => args.extend([
            "-device".to_string(),
            format!("{},audiodev=audio0", config.sound_card),
        ]),
        _ => {}
    }
}

pub(crate) fn configured_bridge(config: &VmConfig) -> Option<&str> {
    if config.offline || config.network.eq_ignore_ascii_case("none") {
        return None;
    }
    config.bridge.as_deref().or_else(|| {
        (!config.network.is_empty()
            && !config.network.eq_ignore_ascii_case("restrict")
            && !config.network.eq_ignore_ascii_case("user")
            && !uses_passt_network(config))
        .then_some(config.network.as_str())
    })
}

fn uses_user_network(config: &VmConfig) -> bool {
    !config.offline
        && configured_bridge(config).is_none()
        && (config.network.is_empty()
            || config.network.eq_ignore_ascii_case("restrict")
            || config.network.eq_ignore_ascii_case("user"))
}

fn uses_passt_network(config: &VmConfig) -> bool {
    !config.offline && config.network.eq_ignore_ascii_case("passt")
}

fn uses_port_forwarding_network(config: &VmConfig) -> bool {
    uses_user_network(config) || uses_passt_network(config)
}

pub fn ensure_disk(vm: &Vm) -> Result<()> {
    if fs::symlink_metadata(&vm.config.disk_img)
        .is_ok_and(|metadata| metadata.file_type().is_symlink())
    {
        return Err(Error::message(format!(
            "refusing to use disk symlink {}",
            vm.config.disk_img.display()
        )));
    }
    if vm.config.disk_img.exists() {
        let status = Command::new("qemu-img")
            .args(["info", vm.config.disk_img.to_string_lossy().as_ref()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(|error| Error::command_unavailable("qemu-img", error))?;
        if !status.success() {
            return Err(Error::message(format!(
                "qemu-img could not read {}",
                vm.config.disk_img.display()
            )));
        }
        return Ok(());
    }

    if vm.config.iso.is_none()
        && vm.config.fixed_iso.is_none()
        && vm.config.img.is_none()
        && vm.config.guest_os != "macos"
    {
        return Err(Error::message(format!(
            "disk {} does not exist and no ISO was configured",
            vm.config.disk_img.display()
        )));
    }
    validate_disk_size(&vm.config.disk_size)?;
    if let Some(parent) = vm.config.disk_img.parent() {
        fs::create_dir_all(parent).map_err(|error| Error::io(parent.display(), error))?;
    }
    let mut command = Command::new("qemu-img");
    command.args(["create", "-f", &vm.config.disk_format]);
    let options = match vm.config.disk_format.as_str() {
        "qcow2" => format!(
            "lazy_refcounts=on,preallocation={},nocow=on",
            vm.config.preallocation
        ),
        "raw" => format!("preallocation={}", vm.config.preallocation),
        _ => String::new(),
    };
    if !options.is_empty() {
        command.args(["-o", options.as_str()]);
    }
    let status = command
        .args([
            vm.config.disk_img.to_string_lossy().as_ref(),
            &vm.config.disk_size,
        ])
        .status()
        .map_err(|error| Error::command_unavailable("qemu-img", error))?;
    if !status.success() {
        return Err(Error::command_failed("qemu-img create"));
    }
    Ok(())
}

#[derive(Debug, Clone)]
pub(crate) struct DiskCheckResult {
    pub report: Value,
    pub healthy: bool,
}

pub(crate) fn disk_info(path: &Path) -> Result<Value> {
    require_disk_file(path)?;
    let args = vec![
        "info".to_string(),
        "-U".to_string(),
        "--output=json".to_string(),
        path.to_string_lossy().into_owned(),
    ];
    let output = run_qemu_img(&args)?;
    if !output.status.success() {
        return Err(qemu_img_failure("info", output));
    }
    serde_json::from_slice(&output.stdout)
        .map_err(|error| Error::message(format!("qemu-img info returned invalid JSON: {error}")))
}

pub(crate) fn disk_resize(path: &Path, size: &str, shrink: bool) -> Result<Value> {
    require_disk_file(path)?;
    validate_disk_size(size)?;
    let mut args = vec!["resize".to_string()];
    if shrink {
        args.push("--shrink".to_string());
    }
    args.extend([path.to_string_lossy().into_owned(), size.to_string()]);
    let output = run_qemu_img(&args)?;
    if !output.status.success() {
        return Err(qemu_img_failure("resize", output));
    }
    disk_info(path)
}

pub(crate) fn disk_check(path: &Path, repair: bool) -> Result<DiskCheckResult> {
    require_disk_file(path)?;
    let mut args = vec!["check".to_string(), "--output=json".to_string()];
    if repair {
        args.push("--repair=all".to_string());
    }
    args.push(path.to_string_lossy().into_owned());
    let output = run_qemu_img(&args)?;
    let report: Value = serde_json::from_slice(&output.stdout).map_err(|error| {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if detail.is_empty() {
            Error::message(format!("qemu-img check returned invalid JSON: {error}"))
        } else {
            Error::message(format!("qemu-img check failed: {detail}"))
        }
    })?;
    let healthy = output.status.success()
        && ["check-errors", "corruptions", "leaks"]
            .iter()
            .all(|key| report.get(*key).and_then(Value::as_u64).unwrap_or(0) == 0);
    Ok(DiskCheckResult { report, healthy })
}

pub(crate) fn disk_convert(
    source: &Path,
    destination: &Path,
    format: &str,
    compress: bool,
    force: bool,
) -> Result<Value> {
    require_disk_file(source)?;
    validate_disk_format(format)?;
    if same_path(source, destination) {
        return Err(Error::message(
            "disk conversion output must be different from the source disk",
        ));
    }
    prepare_conversion_destination(destination, force)?;
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).map_err(|error| Error::io(parent.display(), error))?;
    }
    let mut args = vec![
        "convert".to_string(),
        "-q".to_string(),
        "-O".to_string(),
        format.to_string(),
    ];
    if compress {
        if !matches!(format, "qcow" | "qcow2") {
            return Err(Error::message(
                "--compress is only supported for qcow and qcow2 output",
            ));
        }
        args.push("-c".to_string());
    }
    args.extend([
        source.to_string_lossy().into_owned(),
        destination.to_string_lossy().into_owned(),
    ]);
    let output = run_qemu_img(&args)?;
    if !output.status.success() {
        return Err(qemu_img_failure("convert", output));
    }
    disk_info(destination)
}

pub(crate) fn disk_compact(path: &Path) -> Result<Value> {
    require_disk_file(path)?;
    let info = disk_info(path)?;
    let format = info
        .get("format")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::message("qemu-img info did not report a disk format"))?;
    validate_disk_format(format)?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| Error::message("disk path has no valid file name"))?;
    let temporary = path.with_file_name(format!(
        ".{file_name}.vmctl-compact-{}.tmp",
        std::process::id()
    ));
    if temporary.exists() {
        return Err(Error::message(format!(
            "temporary compacted disk already exists: {}",
            temporary.display()
        )));
    }
    let mut args = vec![
        "convert".to_string(),
        "-q".to_string(),
        "-O".to_string(),
        format.to_string(),
    ];
    if matches!(format, "qcow" | "qcow2") {
        args.push("-c".to_string());
    }
    args.extend([
        path.to_string_lossy().into_owned(),
        temporary.to_string_lossy().into_owned(),
    ]);
    let output = run_qemu_img(&args)?;
    if !output.status.success() {
        let _ = fs::remove_file(&temporary);
        return Err(qemu_img_failure("compact", output));
    }
    if let Err(error) = replace_runtime_file(&temporary, path) {
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }
    disk_info(path)
}

fn require_disk_file(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            Error::message(format!(
                "disk {} does not exist or is not a regular file",
                path.display()
            ))
        } else {
            Error::io(path.display(), error)
        }
    })?;
    if metadata.file_type().is_symlink() {
        return Err(Error::message(format!(
            "refusing to use disk symlink {}",
            path.display()
        )));
    }
    if !metadata.file_type().is_file() {
        return Err(Error::message(format!(
            "disk {} does not exist or is not a regular file",
            path.display()
        )));
    }
    Ok(())
}

fn validate_disk_size(size: &str) -> Result<()> {
    if size.is_empty()
        || size.starts_with('-')
        || size.chars().any(|character| {
            character.is_ascii_control()
                || character.is_ascii_whitespace()
                || !(character.is_ascii_alphanumeric() || ".+".contains(character))
        })
    {
        return Err(Error::message(format!(
            "invalid disk size '{size}'; use a value such as 20G or +4G"
        )));
    }
    Ok(())
}

fn validate_disk_format(format: &str) -> Result<()> {
    if format.is_empty()
        || format.starts_with('-')
        || format.chars().any(|character| {
            character.is_ascii_control()
                || character.is_ascii_whitespace()
                || !(character.is_ascii_alphanumeric() || ".-_".contains(character))
        })
    {
        return Err(Error::message(format!(
            "invalid disk format '{format}'; use a qemu-img format such as qcow2 or raw"
        )));
    }
    Ok(())
}

fn prepare_conversion_destination(path: &Path, force: bool) -> Result<()> {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return Ok(());
    };
    if metadata.file_type().is_symlink() {
        return Err(Error::message(format!(
            "refusing to write through output symlink {}",
            path.display()
        )));
    }
    if !metadata.file_type().is_file() {
        return Err(Error::message(format!(
            "conversion output is not a regular file: {}",
            path.display()
        )));
    }
    if !force {
        return Err(Error::message(format!(
            "conversion output already exists: {}; rerun with --force to replace it",
            path.display()
        )));
    }
    Ok(())
}

fn same_path(left: &Path, right: &Path) -> bool {
    if left == right {
        return true;
    }
    match (fs::canonicalize(left), fs::canonicalize(right)) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

fn run_qemu_img(args: &[String]) -> Result<Output> {
    Command::new("qemu-img")
        .args(args)
        .output()
        .map_err(|error| Error::command_unavailable("qemu-img", error))
}

fn qemu_img_failure(operation: &str, output: Output) -> Error {
    let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if detail.is_empty() {
        Error::command_failed_status(&format!("qemu-img {operation}"), output.status)
    } else {
        Error::message(format!("qemu-img {operation} failed: {detail}"))
    }
}

fn arm_monolithic_firmware(config: &VmConfig) -> Option<PathBuf> {
    (config.arch == "aarch64" && config.guest_os == "windows" && !config.secureboot).then(|| {
        first_existing(&[
            "/usr/share/edk2/aarch64/QEMU_EFI.fd",
            "/usr/share/qemu-efi-aarch64/QEMU_EFI.fd",
        ])
        .or_else(|| {
            firmware_data_dirs()
                .into_iter()
                .map(|dir| dir.join("qemu-efi-aarch64").join("QEMU_EFI.fd"))
                .find(|path| path.is_file())
        })
    })?
}

fn firmware_data_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(root) = env::var_os("QEMU_HOME") {
        let root = PathBuf::from(root);
        dirs.push(root.join("share"));
        dirs.push(root);
    }
    #[cfg(target_os = "macos")]
    dirs.extend([
        PathBuf::from("/opt/homebrew/share/qemu"),
        PathBuf::from("/usr/local/share/qemu"),
    ]);
    #[cfg(target_os = "windows")]
    {
        for variable in ["ProgramFiles", "ProgramFiles(x86)"] {
            if let Some(root) = env::var_os(variable) {
                dirs.push(PathBuf::from(root).join("qemu").join("share"));
            }
        }
    }
    for binary in ["qemu-system-x86_64", "qemu-system-aarch64"] {
        if let Some(path) = find_executable(binary)
            && let Some(parent) = Path::new(&path).parent()
        {
            dirs.extend([parent.join("../share"), parent.join("../share/qemu")]);
        }
    }
    dirs
}

fn firmware_pair_candidates(pairs: &[(&str, &str)]) -> Vec<(PathBuf, PathBuf)> {
    let mut candidates = pairs
        .iter()
        .map(|(code, vars)| (PathBuf::from(code), PathBuf::from(vars)))
        .collect::<Vec<_>>();
    for dir in firmware_data_dirs() {
        candidates.extend([
            (
                dir.join("edk2-x86_64-code.fd"),
                dir.join("edk2-i386-vars.fd"),
            ),
            (
                dir.join("edk2-x86_64-secure-code.fd"),
                dir.join("edk2-i386-vars.fd"),
            ),
            (
                dir.join("edk2-aarch64-code.fd"),
                dir.join("edk2-arm-vars.fd"),
            ),
            (
                dir.join("edk2").join("x64").join("OVMF_CODE.4m.fd"),
                dir.join("edk2").join("x64").join("OVMF_VARS.4m.fd"),
            ),
        ]);
    }
    candidates
}

fn firmware_paths(vm: &Vm, prepare: bool) -> Result<(PathBuf, PathBuf)> {
    let parent = vm
        .config
        .disk_img
        .parent()
        .unwrap_or_else(|| Path::new("."));

    if vm.config.guest_os == "macos" {
        let code = [parent.join("OVMF_CODE.fd")]
            .into_iter()
            .find(|path| path.is_file())
            .or_else(|| {
                first_existing(&[
                    "/usr/share/edk2/x64/OVMF_CODE.4m.fd",
                    "/usr/share/OVMF/OVMF_CODE_4M.fd",
                    "/usr/share/OVMF/OVMF_CODE.fd",
                    "/usr/share/OVMF/x64/OVMF_CODE.fd",
                ])
            })
            .or_else(|| {
                firmware_data_dirs().into_iter().find_map(|dir| {
                    [
                        dir.join("edk2/x64/OVMF_CODE.4m.fd"),
                        dir.join("OVMF_CODE_4M.fd"),
                        dir.join("OVMF_CODE.fd"),
                    ]
                    .into_iter()
                    .find(|path| path.is_file())
                })
            })
            .ok_or_else(|| Error::message("macOS OVMF_CODE.fd was not found"))?;
        if let Some(vars) = [
            parent.join("OVMF_VARS-1024x768.fd"),
            parent.join("OVMF_VARS-1920x1080.fd"),
            parent.join("OVMF_VARS.fd"),
        ]
        .into_iter()
        .find(|path| path.is_file())
        {
            return Ok((code, vars));
        }
        let vars = parent.join("OVMF_VARS.fd");
        if prepare {
            let template = first_existing(&[
                "/usr/share/edk2/x64/OVMF_VARS.4m.fd",
                "/usr/share/OVMF/OVMF_VARS_4M.fd",
                "/usr/share/OVMF/OVMF_VARS.fd",
                "/usr/share/OVMF/x64/OVMF_VARS.fd",
            ])
            .or_else(|| {
                firmware_data_dirs().into_iter().find_map(|dir| {
                    [
                        dir.join("edk2/x64/OVMF_VARS.4m.fd"),
                        dir.join("OVMF_VARS_4M.fd"),
                        dir.join("OVMF_VARS.fd"),
                    ]
                    .into_iter()
                    .find(|path| path.is_file())
                })
            })
            .ok_or_else(|| Error::message("macOS OVMF variables template was not found"))?;
            fs::create_dir_all(parent).map_err(|error| Error::io(parent.display(), error))?;
            fs::copy(&template, &vars).map_err(|error| {
                Error::message(format!(
                    "cannot copy macOS UEFI variables {} to {}: {error}",
                    template.display(),
                    vars.display()
                ))
            })?;
        }
        return Ok((code, vars));
    }

    if let Some(code) = arm_monolithic_firmware(&vm.config) {
        return Ok((code, parent.join("OVMF_VARS.fd")));
    }

    let static_pairs = if vm.config.arch == "aarch64" {
        vec![
            (
                "/usr/share/AAVMF/AAVMF_CODE.fd",
                "/usr/share/AAVMF/AAVMF_VARS.fd",
            ),
            (
                "/usr/share/edk2/aarch64/QEMU_CODE.fd",
                "/usr/share/edk2/aarch64/QEMU_VARS.fd",
            ),
            (
                "/usr/share/edk2/aarch64/QEMU_EFI-pflash.raw",
                "/usr/share/edk2/aarch64/vars-template-pflash.raw",
            ),
            (
                "/usr/share/qemu/edk2-aarch64-code.fd",
                "/usr/share/qemu/edk2-arm-vars.fd",
            ),
        ]
    } else if vm.config.secureboot {
        vec![
            (
                "/usr/share/edk2/x64/OVMF_CODE.secboot.4m.fd",
                "/usr/share/edk2/x64/OVMF_VARS.4m.fd",
            ),
            (
                "/usr/share/OVMF/OVMF_CODE_4M.secboot.fd",
                "/usr/share/OVMF/OVMF_VARS_4M.ms.fd",
            ),
            (
                "/usr/share/OVMF/OVMF_CODE.secboot.fd",
                "/usr/share/OVMF/OVMF_VARS.fd",
            ),
            (
                "/usr/share/OVMF/x64/OVMF_CODE.secboot.fd",
                "/usr/share/OVMF/x64/OVMF_VARS.fd",
            ),
            (
                "/usr/share/qemu/edk2-x86_64-secure-code.fd",
                "/usr/share/qemu/edk2-i386-vars.fd",
            ),
        ]
    } else {
        vec![
            (
                "/usr/share/edk2/x64/OVMF_CODE.4m.fd",
                "/usr/share/edk2/x64/OVMF_VARS.4m.fd",
            ),
            (
                "/usr/share/OVMF/OVMF_CODE_4M.fd",
                "/usr/share/OVMF/OVMF_VARS_4M.fd",
            ),
            (
                "/usr/share/OVMF/OVMF_CODE.fd",
                "/usr/share/OVMF/OVMF_VARS.fd",
            ),
            (
                "/usr/share/OVMF/x64/OVMF_CODE.fd",
                "/usr/share/OVMF/x64/OVMF_VARS.fd",
            ),
            (
                "/usr/share/qemu/edk2-x86_64-code.fd",
                "/usr/share/qemu/edk2-i386-vars.fd",
            ),
        ]
    };
    let firmware_pairs = firmware_pair_candidates(&static_pairs);
    let (code, template) = firmware_pairs
        .into_iter()
        .find(|(code, vars)| code.is_file() && vars.is_file())
        .ok_or_else(|| Error::message("UEFI firmware pair was not found; install edk2/OVMF"))?;
    let vars = [
        parent.join("OVMF_VARS.fd"),
        parent.join("OVMF_VARS_4M.fd"),
        parent.join(format!("{}-vars.fd", vm.config.name)),
    ]
    .into_iter()
    .find(|path| path.is_file())
    .unwrap_or_else(|| parent.join("OVMF_VARS.fd"));
    if prepare && !vars.is_file() {
        fs::create_dir_all(parent).map_err(|error| Error::io(parent.display(), error))?;
        fs::copy(&template, &vars).map_err(|error| {
            Error::message(format!(
                "cannot copy UEFI variables {} to {}: {error}",
                template.display(),
                vars.display()
            ))
        })?;
    }
    if prepare && !vars.is_file() {
        return Err(Error::message(format!(
            "UEFI variables file {} does not exist",
            vars.display()
        )));
    }
    Ok((code, vars))
}

fn add_optional_drive(args: &mut Vec<String>, path: &Option<PathBuf>, options: &str) -> Result<()> {
    let Some(path) = path else {
        return Ok(());
    };
    if !path.exists() {
        return Err(Error::message(format!(
            "configured media file {} does not exist",
            path.display()
        )));
    }
    add(
        args,
        "-drive",
        format!("{options},file={}", qemu_path(path)),
    );
    Ok(())
}

pub fn write_runtime_files(paths: &VmPaths, plan: &QemuPlan) -> Result<()> {
    fs::create_dir_all(&paths.state_dir)
        .map_err(|error| Error::io(paths.state_dir.display(), error))?;
    #[cfg(unix)]
    fs::set_permissions(&paths.state_dir, fs::Permissions::from_mode(0o700))
        .map_err(|error| Error::io(paths.state_dir.display(), error))?;
    let command_path = paths.state_dir.join("qemu.command");
    fs::write(
        &command_path,
        format!("{}\n", shell_join(&plan.binary, &plan.args)),
    )
    .map_err(|error| Error::io(command_path.display(), error))?;

    let mut ports = String::new();
    if let Some(port) = plan.ssh_port {
        ports.push_str(&format!("ssh,{port}\n"));
    }
    if let Some(port) = plan.spice_port {
        ports.push_str(&format!("spice,{port}\n"));
    }
    let ports_path = paths.state_dir.join("ports");
    fs::write(&ports_path, ports).map_err(|error| Error::io(ports_path.display(), error))?;

    let ipc_path = paths.ipc_state();
    let ipc = json!({
        "schema_version": 1,
        "qmp": plan.qmp_endpoint.json_value(),
        "guest_agent": plan.agent_endpoint.as_ref().map(IpcEndpoint::json_value),
    });
    write_runtime_file(&ipc_path, format!("{ipc}\n").as_bytes())?;
    Ok(())
}

fn write_runtime_file(path: &Path, contents: &[u8]) -> Result<()> {
    let temporary = path.with_extension("tmp");
    fs::write(&temporary, contents).map_err(|error| Error::io(temporary.display(), error))?;
    replace_runtime_file(&temporary, path)
}

#[cfg(not(windows))]
fn replace_runtime_file(temporary: &Path, destination: &Path) -> Result<()> {
    fs::rename(temporary, destination).map_err(|error| Error::io(destination.display(), error))
}

#[cfg(windows)]
fn replace_runtime_file(temporary: &Path, destination: &Path) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };

    let source: Vec<u16> = temporary.as_os_str().encode_wide().chain([0]).collect();
    let target: Vec<u16> = destination.as_os_str().encode_wide().chain([0]).collect();
    let replaced = unsafe {
        MoveFileExW(
            source.as_ptr(),
            target.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if replaced == 0 {
        Err(Error::io(
            destination.display(),
            std::io::Error::last_os_error(),
        ))
    } else {
        Ok(())
    }
}

pub fn remove_runtime_sockets(paths: &VmPaths) {
    for path in [
        paths.qmp_socket(),
        paths.agent_socket(),
        paths.serial_socket(),
        paths.spice_socket(),
        paths.monitor_socket(),
        paths.tpm_socket(),
        paths.virtiofs_socket(),
        paths.virtiofs_socket_pid_file(),
        paths.ipc_state(),
    ] {
        let _ = fs::remove_file(path);
    }
}

pub fn shutdown_via_qmp(paths: &VmPaths) -> Result<()> {
    let endpoint = qmp_endpoint_for_paths(paths)?;
    let deadline = qmp_deadline()?;
    let mut stream = connect_endpoint_retry(&endpoint, "QMP")?;
    stream
        .set_read_timeout(Some(QMP_TIMEOUT))
        .map_err(|error| Error::io(endpoint.display(), error))?;
    stream
        .set_write_timeout(Some(QMP_TIMEOUT))
        .map_err(|error| Error::io(endpoint.display(), error))?;
    let mut reader = BufReader::new(
        stream
            .try_clone()
            .map_err(|error| Error::io(endpoint.display(), error))?,
    );

    read_qmp_greeting_until(&mut reader, deadline)?;
    execute_qmp(
        &mut stream,
        &mut reader,
        "qmp_capabilities",
        "vmctl-capabilities",
        None,
        deadline,
    )?;
    execute_qmp(
        &mut stream,
        &mut reader,
        "system_powerdown",
        "vmctl-shutdown",
        None,
        deadline,
    )?;
    Ok(())
}

pub(crate) fn qmp_ping(paths: &VmPaths) -> Result<Value> {
    let endpoint = qmp_endpoint_for_paths(paths)?;
    let deadline = qmp_deadline()?;
    let stream = connect_endpoint_retry(&endpoint, "QMP")?;
    stream
        .set_read_timeout(Some(QMP_TIMEOUT))
        .map_err(|error| Error::io(endpoint.display(), error))?;
    let mut reader = BufReader::new(
        stream
            .try_clone()
            .map_err(|error| Error::io(endpoint.display(), error))?,
    );
    let greeting = read_qmp_greeting_until(&mut reader, deadline)?;
    Ok(greeting)
}

pub(crate) fn qmp_status(paths: &VmPaths) -> Result<String> {
    let endpoint = qmp_endpoint_for_paths(paths)?;
    let deadline = qmp_deadline()?;
    let mut stream = connect_endpoint_retry(&endpoint, "QMP")?;
    stream
        .set_read_timeout(Some(QMP_TIMEOUT))
        .map_err(|error| Error::io(endpoint.display(), error))?;
    stream
        .set_write_timeout(Some(Duration::from_secs(2)))
        .map_err(|error| Error::io(endpoint.display(), error))?;
    let mut reader = BufReader::new(
        stream
            .try_clone()
            .map_err(|error| Error::io(endpoint.display(), error))?,
    );
    read_qmp_greeting_until(&mut reader, deadline)?;
    execute_qmp(
        &mut stream,
        &mut reader,
        "qmp_capabilities",
        "vmctl-status-capabilities",
        None,
        deadline,
    )?;
    execute_qmp(
        &mut stream,
        &mut reader,
        "query-status",
        "vmctl-status",
        None,
        deadline,
    )?
    .get("status")
    .and_then(Value::as_str)
    .map(str::to_string)
    .ok_or_else(|| Error::Qmp("query-status returned no status".to_string()))
}

pub(crate) fn ipc_report(paths: &VmPaths) -> Result<Value> {
    if paths.ipc_state().is_file() {
        let (qmp, agent) = read_ipc_state(paths)?;
        return Ok(json!({
            "qmp": qmp.json_value(),
            "guest_agent": agent.as_ref().map(IpcEndpoint::json_value),
        }));
    }

    #[cfg(unix)]
    {
        Ok(json!({
            "qmp": IpcEndpoint::Unix(paths.qmp_socket()).json_value(),
            "guest_agent": IpcEndpoint::Unix(paths.agent_socket()).json_value(),
        }))
    }
    #[cfg(not(unix))]
    Ok(json!({"qmp": null, "guest_agent": null}))
}

pub(crate) fn ensure_ipc_endpoints_available(plan: &QemuPlan) -> Result<()> {
    let mut tcp_endpoints = Vec::new();
    for endpoint in std::iter::once(&plan.qmp_endpoint).chain(plan.agent_endpoint.iter()) {
        let Some(address) = endpoint.tcp_address() else {
            continue;
        };
        tcp_endpoints.push(("runtime IPC", address.ip().to_string(), address.port()));
    }
    if let (Some(host), Some(port)) = (&plan.ssh_host, plan.ssh_port) {
        tcp_endpoints.push(("SSH", host.clone(), port));
    }
    if let (Some(host), Some(port)) = (&plan.spice_host, plan.spice_port) {
        tcp_endpoints.push(("SPICE", host.clone(), port));
    }
    if let Some((host, port)) = &plan.monitor_telnet {
        tcp_endpoints.push(("monitor Telnet", host.clone(), *port));
    }
    if let Some((host, port)) = &plan.serial_telnet {
        tcp_endpoints.push(("serial Telnet", host.clone(), *port));
    }

    let mut seen = Vec::new();
    for (name, host, port) in tcp_endpoints {
        let key = format!("{host}:{port}");
        if seen.iter().any(|seen_key| seen_key == &key) {
            return Err(Error::message(format!(
                "{name} endpoint {key} conflicts with another configured listener; choose unique ports"
            )));
        }
        TcpListener::bind((host.as_str(), port)).map_err(|error| {
            Error::message(format!(
                "{name} endpoint {key} is unavailable: {error}; choose another port or stop the conflicting service"
            ))
        })?;
        seen.push(key);
    }
    Ok(())
}

pub(crate) fn wait_for_exit(pid: i32, name: &str, timeout: Duration) -> bool {
    let Some(deadline) = std::time::Instant::now().checked_add(timeout) else {
        return false;
    };
    while std::time::Instant::now() < deadline {
        if !process_matches(pid, name) {
            return true;
        }
        thread::sleep(Duration::from_millis(100));
    }
    !process_matches(pid, name)
}

pub(crate) fn kill_process(vm: &Vm, pid: i32, force: bool) -> Result<()> {
    let Some((recorded_pid, expected_identity)) = read_process_record(&vm.paths.pid_file()) else {
        return Err(Error::message(format!(
            "cannot revalidate PID record for {}; refusing to signal process {pid}",
            vm.config.name
        )));
    };
    if recorded_pid != pid {
        return Err(Error::message(format!(
            "PID record for {} changed before signaling; refusing to signal process {pid}",
            vm.config.name
        )));
    }
    signal_vm_process(vm, pid, expected_identity.as_deref(), force)
}

fn revalidate_vm_process(vm: &Vm, pid: i32, expected_identity: Option<&str>) -> Result<bool> {
    if !process_matches_checked(pid, &vm.config.name)? {
        return Ok(false);
    }
    if !process_matches_checked_with_identity(pid, &vm.config.name, expected_identity)? {
        return Err(Error::message(format!(
            "process {pid} no longer matches {}; refusing to signal it",
            vm.config.name
        )));
    }
    Ok(true)
}

#[cfg(target_os = "linux")]
fn signal_vm_process(
    vm: &Vm,
    pid: i32,
    expected_identity: Option<&str>,
    force: bool,
) -> Result<()> {
    let raw_pidfd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
    if raw_pidfd == -1 {
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::ESRCH) {
            return Ok(());
        }
        if error.raw_os_error() == Some(libc::ENOSYS) {
            return Err(Error::message(
                "the Linux kernel does not support pidfds; refusing to signal an unverified PID",
            ));
        }
        return Err(Error::io(format!("pidfd for process {pid}"), error));
    }
    let pidfd = unsafe { OwnedFd::from_raw_fd(raw_pidfd as i32) };
    if !revalidate_vm_process(vm, pid, expected_identity)? {
        return Ok(());
    }
    let signal = if force { libc::SIGKILL } else { libc::SIGTERM };
    let result = unsafe {
        libc::syscall(
            libc::SYS_pidfd_send_signal,
            pidfd.as_raw_fd(),
            signal,
            std::ptr::null::<libc::siginfo_t>(),
            0,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::ESRCH) {
            Ok(())
        } else {
            Err(Error::io(format!("signal process {pid}"), error))
        }
    }
}

#[cfg(not(target_os = "linux"))]
fn signal_vm_process(
    vm: &Vm,
    pid: i32,
    expected_identity: Option<&str>,
    force: bool,
) -> Result<()> {
    if !revalidate_vm_process(vm, pid, expected_identity)? {
        return Ok(());
    }
    let status = terminate_pid(pid, force)?;
    if status.success() {
        Ok(())
    } else {
        Err(Error::command_failed_status("kill", status))
    }
}

fn terminate_pid(pid: i32, force: bool) -> Result<std::process::ExitStatus> {
    #[cfg(unix)]
    {
        Command::new("kill")
            .args([if force { "-KILL" } else { "-TERM" }, &pid.to_string()])
            .status()
            .map_err(|error| Error::command_unavailable("kill", error))
    }
    #[cfg(windows)]
    {
        let mut command = Command::new("taskkill.exe");
        command.args(["/PID", &pid.to_string(), "/T"]);
        if force {
            command.arg("/F");
        }
        command
            .status()
            .map_err(|error| Error::command_unavailable("taskkill.exe", error))
    }
}

pub(crate) fn start_tpm(vm: &Vm) -> Result<Option<Child>> {
    if !vm.config.tpm {
        return Ok(None);
    }
    fs::create_dir_all(&vm.paths.state_dir)
        .map_err(|error| Error::io(vm.paths.state_dir.display(), error))?;
    let log_path = vm.paths.state_dir.join("swtpm.log");
    let log = File::create(&log_path).map_err(|error| Error::io(log_path.display(), error))?;
    let error_log = log
        .try_clone()
        .map_err(|error| Error::io(log_path.display(), error))?;
    let socket = vm.paths.tpm_socket();
    let state_dir = vm.paths.state_dir.display().to_string();
    let control = if env::consts::OS == "windows" {
        format!("type=tcp,port={}", control_port(&socket))
    } else {
        format!("type=unixio,path={}", socket.display())
    };
    if env::consts::OS == "windows" {
        TcpListener::bind(("127.0.0.1", control_port(&socket))).map_err(|error| {
            Error::message(format!(
                "TPM control port {} is unavailable: {error}",
                control_port(&socket)
            ))
        })?;
    }
    let mut child = Command::new("swtpm")
        .args([
            "socket",
            "--ctrl",
            &control,
            "--tpmstate",
            &format!("dir={state_dir}"),
            "--tpm2",
            "--terminate",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(error_log))
        .spawn()
        .map_err(|error| Error::command_unavailable("swtpm", error))?;
    if let Err(error) = fs::write(vm.paths.tpm_pid_file(), process_record(child.id() as i32)) {
        let _ = child.kill();
        let _ = child.wait();
        return Err(Error::io(vm.paths.tpm_pid_file().display(), error));
    }
    for _ in 0..20 {
        let ready = if env::consts::OS == "windows" {
            TcpStream::connect(("127.0.0.1", control_port(&socket))).is_ok()
        } else {
            socket.exists()
        };
        if ready {
            return Ok(Some(child));
        }
        if child
            .try_wait()
            .map_err(|error| Error::io(log_path.display(), error))?
            .is_some()
        {
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }
    let ready = if env::consts::OS == "windows" {
        TcpStream::connect(("127.0.0.1", control_port(&socket))).is_ok()
    } else {
        socket.exists()
    };
    if !ready {
        let _ = child.kill();
        let _ = child.wait();
        let _ = fs::remove_file(vm.paths.tpm_pid_file());
        return Err(Error::message(format!(
            "swtpm did not create {} (see {})",
            socket.display(),
            log_path.display()
        )));
    }
    Ok(Some(child))
}

pub(crate) fn stop_tpm(paths: &VmPaths) {
    let pid_file = paths.tpm_pid_file();
    let Some((pid, identity)) = read_process_record(&pid_file) else {
        let _ = fs::remove_file(pid_file);
        return;
    };
    if helper_process_matches(pid, "swtpm", identity.as_deref()) {
        let _ = terminate_pid(pid, true);
    }
    let _ = fs::remove_file(pid_file);
}

pub(crate) fn start_virtiofsd(vm: &Vm, host: &QemuPlanContext, quiet: bool) -> bool {
    let Some(binary) = host.virtiofsd.as_deref() else {
        return false;
    };
    if !virtiofs_requested(&vm.config, host) {
        return false;
    }

    stop_virtiofsd(&vm.paths);
    let Some(public_dir) = vm.config.public_dir.as_deref() else {
        return false;
    };
    let log_path = vm.paths.state_dir.join("virtiofsd.log");
    let log = match File::create(&log_path) {
        Ok(log) => log,
        Err(error) => {
            if !quiet {
                eprintln!(
                    "vmctl: warning: cannot create virtiofsd log {}: {error}; using 9p",
                    log_path.display()
                );
            }
            return false;
        }
    };
    let error_log = match log.try_clone() {
        Ok(log) => log,
        Err(error) => {
            if !quiet {
                eprintln!(
                    "vmctl: warning: cannot prepare virtiofsd log {}: {error}; using 9p",
                    log_path.display()
                );
            }
            return false;
        }
    };
    let socket = vm.paths.virtiofs_socket();
    let mut child = match Command::new(binary)
        .args([
            format!("--socket-path={}", socket.display()),
            format!("--shared-dir={}", public_dir.display()),
            "--announce-submounts".to_string(),
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(error_log))
        .spawn()
    {
        Ok(child) => child,
        Err(error) => {
            if !quiet {
                eprintln!("vmctl: warning: cannot start virtiofsd ({error}); using 9p");
            }
            return false;
        }
    };
    if let Err(error) = fs::write(
        vm.paths.virtiofs_pid_file(),
        process_record(child.id() as i32),
    ) {
        let _ = child.kill();
        let _ = child.wait();
        stop_virtiofsd(&vm.paths);
        if !quiet {
            eprintln!(
                "vmctl: warning: cannot record virtiofsd PID {}: {error}; using 9p",
                vm.paths.virtiofs_pid_file().display()
            );
        }
        return false;
    }

    for _ in 0..40 {
        if is_unix_socket(&socket) {
            return true;
        }
        if child.try_wait().ok().flatten().is_some() {
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }

    let _ = child.kill();
    let _ = child.wait();
    stop_virtiofsd(&vm.paths);
    let detail = fs::read_to_string(&log_path)
        .ok()
        .filter(|log| !log.trim().is_empty())
        .map_or_else(String::new, |log| format!(": {}", log.trim()));
    if !quiet {
        eprintln!(
            "vmctl: warning: virtiofsd did not create {}; using 9p{detail}",
            socket.display()
        );
    }
    false
}

pub(crate) fn stop_virtiofsd(paths: &VmPaths) {
    let pid_file = paths.virtiofs_pid_file();
    if let Some((pid, identity)) = read_process_record(&pid_file)
        && helper_process_matches(pid, "virtiofsd", identity.as_deref())
    {
        let _ = terminate_pid(pid, false);
        for _ in 0..20 {
            if !helper_process_matches(pid, "virtiofsd", identity.as_deref()) {
                break;
            }
            thread::sleep(Duration::from_millis(50));
        }
        if helper_process_matches(pid, "virtiofsd", identity.as_deref()) {
            let _ = terminate_pid(pid, true);
        }
    }
    let _ = fs::remove_file(pid_file);
    let _ = fs::remove_file(paths.virtiofs_socket());
    let _ = fs::remove_file(paths.virtiofs_socket_pid_file());
}

#[cfg(unix)]
fn is_unix_socket(path: &Path) -> bool {
    fs::metadata(path).is_ok_and(|metadata| metadata.file_type().is_socket())
}

#[cfg(windows)]
fn is_unix_socket(path: &Path) -> bool {
    path.is_file()
}

pub(crate) fn send_monitor_command(vm: &Vm, command: &str) -> Result<String> {
    if vm.config.monitor == "none" {
        return Err(Error::message("the QEMU monitor is disabled"));
    }
    let response = match vm.config.monitor.as_str() {
        "socket" => {
            return send_qmp_human_monitor_command(vm, command);
        }
        "telnet" => {
            let address = format!(
                "{}:{}",
                qemu_host(&vm.config.monitor_telnet_host),
                vm.config.monitor_telnet_port
            );
            let deadline = qmp_deadline()?;
            let mut stream = connect_monitor(&address, deadline)?;
            stream
                .set_write_timeout(Some(QMP_TIMEOUT))
                .map_err(|error| {
                    Error::message(format!("cannot configure monitor {address}: {error}"))
                })?;
            stream
                .write_all(format!("{command}\n").as_bytes())
                .map_err(|error| Error::message(format!("cannot send monitor command: {error}")))?;
            read_monitor_response(&mut stream, &address, deadline)?
        }
        mode => {
            return Err(Error::message(format!(
                "monitor mode '{mode}' is not supported"
            )));
        }
    };
    Ok(clean_monitor_output(&response))
}

fn connect_monitor(address: &str, deadline: Instant) -> Result<TcpStream> {
    let addresses = resolve_monitor_addresses(address, deadline)?;
    let mut last_error = None;
    for socket_address in addresses {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        match TcpStream::connect_timeout(&socket_address, remaining) {
            Ok(stream) => return Ok(stream),
            Err(error) => last_error = Some(error),
        }
    }
    let error = last_error
        .unwrap_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "connection timed out"));
    Err(Error::message(format!(
        "cannot connect to monitor {address}: {error}"
    )))
}

fn resolve_monitor_addresses(address: &str, deadline: Instant) -> Result<Vec<SocketAddr>> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Err(Error::message(format!(
            "monitor address resolution timed out for {address}"
        )));
    }
    let (sender, receiver) = mpsc::sync_channel(1);
    let address = address.to_string();
    let resolve_address = address.clone();
    thread::Builder::new()
        .spawn(move || {
            let resolved = resolve_address
                .to_socket_addrs()
                .map(|addresses| addresses.collect::<Vec<_>>());
            let _ = sender.send(resolved);
        })
        .map_err(|error| Error::message(format!("cannot resolve monitor {address}: {error}")))?;
    match receiver.recv_timeout(remaining) {
        Ok(Ok(addresses)) if !addresses.is_empty() => Ok(addresses),
        Ok(Ok(_)) => Err(Error::message(format!(
            "monitor {address} did not resolve to an IP address"
        ))),
        Ok(Err(error)) => Err(Error::message(format!(
            "cannot resolve monitor {address}: {error}"
        ))),
        Err(mpsc::RecvTimeoutError::Timeout) => Err(Error::message(format!(
            "monitor address resolution timed out for {address}"
        ))),
        Err(mpsc::RecvTimeoutError::Disconnected) => Err(Error::message(format!(
            "monitor address resolution failed for {address}"
        ))),
    }
}

const MAX_MONITOR_RESPONSE: usize = 1024 * 1024;

fn read_monitor_response(
    stream: &mut TcpStream,
    address: &str,
    deadline: Instant,
) -> Result<Vec<u8>> {
    let mut response = Vec::new();
    let mut buffer = [0_u8; 8192];
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(Error::message(format!(
                "monitor {address} did not become idle within {} seconds",
                QMP_TIMEOUT.as_secs()
            )));
        }
        stream
            .set_read_timeout(Some(remaining.min(Duration::from_millis(500))))
            .map_err(|error| {
                Error::message(format!("cannot configure monitor {address}: {error}"))
            })?;
        match stream.read(&mut buffer) {
            Ok(0) => return Ok(response),
            Ok(count) => {
                if response.len() + count > MAX_MONITOR_RESPONSE {
                    return Err(Error::message(format!(
                        "monitor {address} response exceeds the {} byte safety limit",
                        MAX_MONITOR_RESPONSE
                    )));
                }
                response.extend_from_slice(&buffer[..count]);
            }
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
                ) =>
            {
                if response.is_empty() {
                    continue;
                }
                return Ok(response);
            }
            Err(error) => {
                return Err(Error::message(format!(
                    "cannot read monitor {address}: {error}"
                )));
            }
        }
    }
}

fn send_qmp_human_monitor_command(vm: &Vm, command: &str) -> Result<String> {
    let endpoint = qmp_endpoint_for_paths(&vm.paths)?;
    let deadline = qmp_deadline()?;
    let mut stream = connect_endpoint_retry(&endpoint, "QMP")?;
    stream
        .set_read_timeout(Some(QMP_TIMEOUT))
        .map_err(|error| Error::io(endpoint.display(), error))?;
    stream
        .set_write_timeout(Some(QMP_TIMEOUT))
        .map_err(|error| Error::io(endpoint.display(), error))?;
    let mut reader = BufReader::new(
        stream
            .try_clone()
            .map_err(|error| Error::io(endpoint.display(), error))?,
    );
    read_qmp_greeting_until(&mut reader, deadline)?;
    execute_qmp(
        &mut stream,
        &mut reader,
        "qmp_capabilities",
        "vmctl-monitor-capabilities",
        None,
        deadline,
    )?;
    let response = execute_qmp(
        &mut stream,
        &mut reader,
        "human-monitor-command",
        "vmctl-monitor-command",
        Some(json!({"command-line": command})),
        deadline,
    )?;
    response
        .as_str()
        .map(str::trim)
        .map(str::to_string)
        .ok_or_else(|| Error::Qmp("human-monitor-command returned no text".to_string()))
}

fn clean_monitor_output(response: &[u8]) -> String {
    let mut output = String::new();
    let mut escape = false;
    let mut csi = false;
    for byte in response {
        if csi {
            if (0x40..=0x7e).contains(byte) {
                csi = false;
            }
            continue;
        }
        if escape {
            escape = false;
            csi = *byte == b'[';
            continue;
        }
        if *byte == 0x1b {
            escape = true;
        } else if *byte == b'\n' || *byte == b'\t' || !byte.is_ascii_control() {
            output.push(*byte as char);
        }
    }
    output.trim().to_string()
}

pub(crate) fn guest_command(vm: &Vm, command: &str, arguments: Option<Value>) -> Result<Value> {
    guest_command_with_timeout(vm, command, arguments, Duration::from_secs(2), true)
}

pub(crate) fn guest_shutdown(vm: &Vm, deadline: Instant) -> Result<Value> {
    guest_command_until(vm, "guest-shutdown", None, deadline, false)
}

fn guest_command_with_timeout(
    vm: &Vm,
    command: &str,
    arguments: Option<Value>,
    read_timeout: Duration,
    expect_response: bool,
) -> Result<Value> {
    let deadline = Instant::now()
        .checked_add(read_timeout)
        .ok_or_else(|| Error::message("guest-agent timeout is too large"))?;
    guest_command_until(vm, command, arguments, deadline, expect_response)
}

fn guest_command_until(
    vm: &Vm,
    command: &str,
    arguments: Option<Value>,
    deadline: Instant,
    expect_response: bool,
) -> Result<Value> {
    let endpoint = agent_endpoint_for_paths(&vm.paths)?;
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Err(Error::guest_agent_unavailable(
            command,
            "timeout expired before connecting",
        ));
    }
    let stream = connect_endpoint_retry_with_timeout(&endpoint, "guest-agent", remaining)
        .map_err(|error| Error::guest_agent_unavailable(command, error.to_string()))?;
    stream
        .set_read_timeout(Some(remaining))
        .map_err(|error| Error::io(endpoint.display(), error))?;
    stream
        .set_write_timeout(Some(remaining))
        .map_err(|error| Error::io(endpoint.display(), error))?;
    let mut reader = BufReader::new(stream);
    let sync_id = next_guest_sync_id();
    sync_guest_agent(&mut reader, sync_id, deadline).map_err(|error| match error.kind() {
        io::ErrorKind::TimedOut
        | io::ErrorKind::WouldBlock
        | io::ErrorKind::UnexpectedEof
        | io::ErrorKind::ConnectionReset
        | io::ErrorKind::BrokenPipe
        | io::ErrorKind::NotConnected
        | io::ErrorKind::ConnectionAborted => {
            Error::guest_agent_unavailable(command, "did not respond during synchronization")
        }
        io::ErrorKind::InvalidData => Error::guest_agent_protocol(command, error.to_string()),
        _ => Error::io(endpoint.display(), error),
    })?;
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Err(Error::guest_agent_unavailable(
            command,
            "timeout expired during synchronization",
        ));
    }
    reader
        .get_mut()
        .set_read_timeout(Some(remaining))
        .map_err(|error| Error::io(endpoint.display(), error))?;
    reader
        .get_mut()
        .set_write_timeout(Some(remaining))
        .map_err(|error| Error::io(endpoint.display(), error))?;

    let request = match arguments {
        Some(arguments) => json!({"execute": command, "arguments": arguments}),
        None => json!({"execute": command}),
    };
    write_all_until(
        reader.get_mut(),
        format!("{request}\n").as_bytes(),
        deadline,
    )
    .map_err(|error| Error::io(endpoint.display(), error))?;
    if !expect_response {
        let shutdown_deadline = Instant::now()
            .checked_add(Duration::from_millis(250))
            .map_or(deadline, |candidate| candidate.min(deadline));
        reader
            .get_mut()
            .set_read_timeout(Some(
                shutdown_deadline.saturating_duration_since(Instant::now()),
            ))
            .map_err(|error| Error::io(endpoint.display(), error))?;
        let line =
            match read_bounded_line_until(&mut reader, MAX_GUEST_AGENT_RESPONSE, shutdown_deadline)
            {
                Ok(line) if line.is_empty() => return Ok(Value::Null),
                Ok(line) if line.trim().is_empty() => {
                    return Err(Error::guest_agent_protocol(
                        command,
                        "response was an empty JSON line",
                    ));
                }
                Ok(line) => line,
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
                    ) =>
                {
                    return Ok(Value::Null);
                }
                Err(error) => return Err(Error::io(endpoint.display(), error)),
            };
        let response: Value = serde_json::from_str(line.trim()).map_err(|error| {
            Error::guest_agent_protocol(command, format!("invalid JSON: {error}"))
        })?;
        let object = response
            .as_object()
            .ok_or_else(|| Error::guest_agent_protocol(command, "response is not a JSON object"))?;
        if let Some(error) = object.get("error") {
            return Err(Error::guest_agent_protocol(
                command,
                format!("command rejected: {error}"),
            ));
        }
        return object.get("return").cloned().ok_or_else(|| {
            Error::guest_agent_protocol(command, "response has neither return nor error")
        });
    }

    let line = read_bounded_line_until(&mut reader, MAX_GUEST_AGENT_RESPONSE, deadline).map_err(
        |error| {
            if matches!(
                error.kind(),
                io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
            ) {
                Error::guest_agent_unavailable(
                    command,
                    "did not respond within the configured timeout",
                )
            } else if error.kind() == io::ErrorKind::InvalidData {
                Error::guest_agent_protocol(command, error.to_string())
            } else {
                Error::io(endpoint.display(), error)
            }
        },
    )?;
    if line.trim().is_empty() {
        return Err(Error::guest_agent_unavailable(
            command,
            "closed the connection without responding",
        ));
    }
    let response: Value = serde_json::from_str(line.trim())
        .map_err(|error| Error::guest_agent_protocol(command, format!("invalid JSON: {error}")))?;
    let object = response
        .as_object()
        .ok_or_else(|| Error::guest_agent_protocol(command, "response is not a JSON object"))?;
    if let Some(error) = object.get("error") {
        return Err(Error::guest_agent_protocol(
            command,
            format!("command rejected: {error}"),
        ));
    }
    object.get("return").cloned().ok_or_else(|| {
        Error::guest_agent_protocol(command, "response has neither return nor error")
    })
}

fn next_guest_sync_id() -> i64 {
    let sequence = NEXT_GUEST_SYNC_ID.fetch_add(1, Ordering::Relaxed) as u64;
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos() as u64);
    let process = u64::from(std::process::id());
    let id = nanos ^ process.rotate_left(17) ^ sequence.rotate_left(31);
    (id & i64::MAX as u64).max(1) as i64
}

fn sync_guest_agent(
    reader: &mut BufReader<IpcStream>,
    id: i64,
    deadline: Instant,
) -> io::Result<()> {
    let mut consumed = 0_usize;
    let mut sync_request = Vec::new();
    let request = json!({
        "execute": "guest-sync-delimited",
        "arguments": {"id": id},
    });
    sync_request.push(0xff);
    sync_request.extend_from_slice(format!("{request}\n").as_bytes());
    write_all_until(reader.get_mut(), &sync_request, deadline)?;

    let mut byte = [0_u8; 1];
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "guest agent synchronization timed out",
            ));
        }
        reader.get_mut().set_read_timeout(Some(remaining))?;
        if reader.read(&mut byte)? == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "guest agent closed during synchronization",
            ));
        }
        consumed += 1;
        if consumed > MAX_GUEST_AGENT_RESPONSE {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "guest agent synchronization exceeded the safety limit",
            ));
        }
        if byte[0] == 0xff {
            let remaining_limit = MAX_GUEST_AGENT_RESPONSE.saturating_sub(consumed);
            let line = read_bounded_line_until(reader, remaining_limit, deadline)?;
            consumed = consumed.saturating_add(line.len());
            if line.is_empty() {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "guest agent closed after synchronization sentinel",
                ));
            }
            let response: Value = serde_json::from_str(line.trim()).map_err(|error| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("invalid guest-sync-delimited response: {error}"),
                )
            })?;
            if response.get("return").and_then(Value::as_i64) == Some(id) {
                return Ok(());
            }
            continue;
        }
    }
}

fn read_bounded_line_until(
    reader: &mut BufReader<IpcStream>,
    limit: usize,
    deadline: Instant,
) -> io::Result<String> {
    let mut bytes = Vec::new();
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "bounded read timed out",
            ));
        }
        reader.get_mut().set_read_timeout(Some(remaining))?;
        let chunk = reader.fill_buf()?;
        if chunk.is_empty() {
            break;
        }
        let newline = chunk.iter().position(|byte| *byte == b'\n');
        let count = newline.map_or(chunk.len(), |index| index + 1);
        if bytes.len() + count > limit {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("IPC response exceeded the {limit}-byte safety limit"),
            ));
        }
        bytes.extend_from_slice(&chunk[..count]);
        reader.consume(count);
        if newline.is_some() {
            break;
        }
    }
    String::from_utf8(bytes)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))
}

#[cfg(test)]
fn read_bounded_line(reader: &mut impl BufRead, limit: usize) -> io::Result<String> {
    let mut bytes = Vec::new();
    loop {
        let chunk = reader.fill_buf()?;
        if chunk.is_empty() {
            break;
        }
        let newline = chunk.iter().position(|byte| *byte == b'\n');
        let count = newline.map_or(chunk.len(), |index| index + 1);
        if bytes.len() + count > limit {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("guest-agent response exceeded the {limit}-byte safety limit"),
            ));
        }
        bytes.extend_from_slice(&chunk[..count]);
        reader.consume(count);
        if newline.is_some() {
            break;
        }
    }
    String::from_utf8(bytes)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))
}

pub(crate) fn guest_exec(
    vm: &Vm,
    program: &str,
    args: &[String],
    timeout_secs: u64,
) -> Result<Value> {
    if timeout_secs == 0 {
        return Err(Error::message(
            "guest command timeout must be greater than zero",
        ));
    }
    let deadline = Instant::now()
        .checked_add(Duration::from_secs(timeout_secs))
        .ok_or_else(|| Error::message("guest command timeout is too large"))?;
    let pid = guest_command_until(
        vm,
        "guest-exec",
        Some(json!({
            "path": program,
            "arg": args,
            "capture-output": true,
        })),
        deadline,
        true,
    )?
    .get("pid")
    .and_then(Value::as_u64)
    .filter(|pid| *pid > 0)
    .ok_or_else(|| {
        Error::guest_agent_protocol("guest-exec", "response has no positive integer process id")
    })?;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(Error::guest_command_timeout(program, pid, timeout_secs));
        }
        let result = match guest_command_until(
            vm,
            "guest-exec-status",
            Some(json!({"pid": pid})),
            deadline,
            true,
        ) {
            Ok(result) => result,
            Err(_error) if Instant::now() >= deadline => {
                return Err(Error::guest_command_timeout(program, pid, timeout_secs));
            }
            Err(error) => return Err(error),
        };
        let exited = result
            .get("exited")
            .and_then(Value::as_bool)
            .ok_or_else(|| {
                Error::guest_agent_protocol(
                    "guest-exec-status",
                    "response has no boolean exited field",
                )
            })?;
        let exit_code = guest_status_integer(&result, "exitcode")?;
        let signal = guest_status_integer(&result, "signal")?;
        if exited {
            if exit_code.is_some() == signal.is_some() {
                return Err(Error::guest_agent_protocol(
                    "guest-exec-status",
                    "exited response must contain exactly one non-negative exitcode or positive signal",
                ));
            }
            return normalize_guest_exec_result(result);
        }
        if exit_code.is_some() || signal.is_some() {
            return Err(Error::guest_agent_protocol(
                "guest-exec-status",
                "running response must not contain exitcode or signal",
            ));
        }
        let sleep_for = deadline
            .saturating_duration_since(Instant::now())
            .min(Duration::from_millis(100));
        if sleep_for.is_zero() {
            return Err(Error::guest_command_timeout(program, pid, timeout_secs));
        }
        thread::sleep(sleep_for);
    }
}

const MAX_GUEST_AGENT_RESPONSE: usize = 8 * 1024 * 1024;

fn guest_status_integer(result: &Value, key: &str) -> Result<Option<i64>> {
    let Some(value) = result.get(key) else {
        return Ok(None);
    };
    let number = value.as_i64().ok_or_else(|| {
        Error::guest_agent_protocol("guest-exec-status", format!("{key} must be an integer"))
    })?;
    let valid = if key == "signal" {
        number > 0
    } else {
        number >= 0
    };
    if !valid {
        return Err(Error::guest_agent_protocol(
            "guest-exec-status",
            format!("{key} has an invalid value"),
        ));
    }
    Ok(Some(number))
}

fn normalize_guest_exec_result(mut result: Value) -> Result<Value> {
    let object = result.as_object_mut().ok_or_else(|| {
        Error::guest_agent_protocol("guest-exec-status", "response is not a JSON object")
    })?;
    for (encoded_key, text_key) in [("out-data", "stdout"), ("err-data", "stderr")] {
        let Some(value) = object.get(encoded_key) else {
            continue;
        };
        let encoded = value.as_str().map(str::to_owned).ok_or_else(|| {
            Error::guest_agent_protocol(
                "guest-exec-status",
                format!("{encoded_key} is not a base64 string"),
            )
        })?;
        let bytes = decode_base64(&encoded).map_err(|error| {
            Error::guest_agent_protocol(
                "guest-exec-status",
                format!("invalid {encoded_key}: {error}"),
            )
        })?;
        let utf8 = String::from_utf8(bytes).ok();
        object.insert(
            text_key.to_string(),
            utf8.map_or(Value::Null, Value::String),
        );
        object.insert(format!("{text_key}_base64"), Value::String(encoded));
        object.insert(
            format!("{text_key}_encoding"),
            Value::String(
                if object[text_key].is_null() {
                    "base64"
                } else {
                    "utf-8"
                }
                .to_string(),
            ),
        );
    }
    Ok(result)
}

fn decode_base64(value: &str) -> Result<Vec<u8>> {
    let bytes = value
        .bytes()
        .filter(|byte| !byte.is_ascii_whitespace())
        .collect::<Vec<_>>();
    if bytes.is_empty() {
        return Ok(Vec::new());
    }
    if bytes.len() % 4 == 1 {
        return Err(Error::message("base64 data has an invalid length"));
    }
    let mut output = Vec::with_capacity(bytes.len() * 3 / 4);
    for (chunk_index, chunk) in bytes.chunks(4).enumerate() {
        let last = chunk_index == bytes.len().div_ceil(4) - 1;
        let mut digits = [0_u8; 4];
        for (index, byte) in chunk.iter().copied().enumerate() {
            if byte == b'=' {
                if !last || index < 2 {
                    return Err(Error::message("base64 padding is in an invalid position"));
                }
                continue;
            }
            digits[index] = base64_digit(byte)
                .ok_or_else(|| Error::message("base64 data contains an invalid character"))?;
        }
        if chunk.len() < 4 && chunk.contains(&b'=') {
            return Err(Error::message("base64 padding is incomplete"));
        }
        if chunk.len() == 2 {
            if digits[1] & 0x0f != 0 {
                return Err(Error::message("base64 data has non-zero trailing bits"));
            }
            output.push((digits[0] << 2) | (digits[1] >> 4));
        } else if chunk.len() == 3 {
            if digits[2] & 0x03 != 0 {
                return Err(Error::message("base64 data has non-zero trailing bits"));
            }
            output.extend([
                (digits[0] << 2) | (digits[1] >> 4),
                (digits[1] << 4) | (digits[2] >> 2),
            ]);
        } else if chunk[2] == b'=' {
            if chunk[3] != b'=' || digits[1] & 0x0f != 0 {
                return Err(Error::message(
                    "base64 padding or trailing bits are invalid",
                ));
            }
            output.push((digits[0] << 2) | (digits[1] >> 4));
        } else if chunk[3] == b'=' {
            if digits[2] & 0x03 != 0 {
                return Err(Error::message("base64 data has non-zero trailing bits"));
            }
            output.extend([
                (digits[0] << 2) | (digits[1] >> 4),
                (digits[1] << 4) | (digits[2] >> 2),
            ]);
        } else if chunk.len() == 4 {
            output.extend([
                (digits[0] << 2) | (digits[1] >> 4),
                (digits[1] << 4) | (digits[2] >> 2),
                (digits[2] << 6) | digits[3],
            ]);
        }
    }
    Ok(output)
}

fn base64_digit(byte: u8) -> Option<u8> {
    match byte {
        b'A'..=b'Z' => Some(byte - b'A'),
        b'a'..=b'z' => Some(byte - b'a' + 26),
        b'0'..=b'9' => Some(byte - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}

pub(crate) fn disk_snapshot(vm: &Vm, action: &str, tag: Option<&str>) -> Result<String> {
    require_disk_file(&vm.config.disk_img)?;
    let mut args = vec!["snapshot".to_string()];
    if let Some(tag) = tag {
        args.extend([action.to_string(), tag.to_string()]);
    } else {
        args.push(action.to_string());
    }
    args.push(vm.config.disk_img.to_string_lossy().into_owned());
    let output = Command::new("qemu-img")
        .args(args)
        .output()
        .map_err(|error| Error::command_unavailable("qemu-img", error))?;
    if !output.status.success() {
        return Err(Error::command_failed_status(
            "qemu-img snapshot",
            output.status,
        ));
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok(if text.is_empty() {
        String::from_utf8_lossy(&output.stderr).trim().to_string()
    } else {
        text
    })
}

#[cfg(target_os = "linux")]
fn pid_matches(pid: i32, needle: &str) -> bool {
    fs::read(format!("/proc/{pid}/cmdline"))
        .is_ok_and(|command_line| String::from_utf8_lossy(&command_line).contains(needle))
}

#[cfg(windows)]
fn pid_matches(pid: i32, needle: &str) -> bool {
    if let Ok(output) = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            &format!("(Get-CimInstance Win32_Process -Filter 'ProcessId = {pid}').CommandLine"),
        ])
        .output()
        && output.status.success()
    {
        return String::from_utf8_lossy(&output.stdout)
            .to_ascii_lowercase()
            .contains(&needle.to_ascii_lowercase());
    }
    Command::new("tasklist.exe")
        .args(["/FI", &format!("PID eq {pid}"), "/NH"])
        .output()
        .is_ok_and(|output| {
            output.status.success()
                && String::from_utf8_lossy(&output.stdout)
                    .to_ascii_lowercase()
                    .contains(&needle.to_ascii_lowercase())
        })
}

#[cfg(all(unix, not(target_os = "linux")))]
fn pid_matches(pid: i32, needle: &str) -> bool {
    Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "command="])
        .output()
        .is_ok_and(|output| String::from_utf8_lossy(&output.stdout).contains(needle))
}

#[cfg(not(any(unix, windows)))]
fn pid_matches(_pid: i32, _needle: &str) -> bool {
    false
}

fn execute_qmp(
    stream: &mut IpcStream,
    reader: &mut BufReader<IpcStream>,
    command: &str,
    id: &str,
    arguments: Option<Value>,
    deadline: Instant,
) -> Result<Value> {
    let request = match arguments {
        Some(arguments) => json!({
            "execute": command,
            "arguments": arguments,
            "id": id,
        }),
        None => json!({ "execute": command, "id": id }),
    };
    write_all_until(stream, format!("{request}\n").as_bytes(), deadline)
        .map_err(|error| Error::Qmp(format!("cannot send {command}: {error}")))?;

    loop {
        let response = read_qmp_message_until(reader, deadline)?;
        let object = response
            .as_object()
            .ok_or_else(|| Error::Qmp(format!("{command} returned a non-object response")))?;
        if object.get("event").is_some() {
            continue;
        }
        if let Some(error) = object.get("error")
            && object.get("id").is_none()
        {
            return Err(Error::Qmp(format!("{command} rejected: {error}")));
        }
        if object.get("id").and_then(Value::as_str) != Some(id) {
            continue;
        }
        if let Some(error) = object.get("error") {
            return Err(Error::Qmp(format!("{command} rejected: {error}")));
        }
        return object
            .get("return")
            .cloned()
            .ok_or_else(|| Error::Qmp(format!("{command} response has neither return nor error")));
    }
}

const QMP_TIMEOUT: Duration = Duration::from_secs(2);

fn qmp_deadline() -> Result<Instant> {
    Instant::now()
        .checked_add(QMP_TIMEOUT)
        .ok_or_else(|| Error::message("QMP timeout is too large"))
}

fn read_qmp_greeting_until(reader: &mut BufReader<IpcStream>, deadline: Instant) -> Result<Value> {
    let greeting = read_qmp_message_until(reader, deadline)?;
    let Some(qmp) = greeting.get("QMP").and_then(Value::as_object) else {
        return Err(Error::Qmp("QEMU greeting has no QMP object".to_string()));
    };
    if !qmp.get("version").is_some_and(Value::is_object)
        || !qmp.get("capabilities").is_some_and(Value::is_array)
    {
        return Err(Error::Qmp(
            "QEMU greeting is missing version or capabilities".to_string(),
        ));
    }
    Ok(greeting)
}

fn read_qmp_message_until(reader: &mut BufReader<IpcStream>, deadline: Instant) -> Result<Value> {
    let line = read_bounded_line_until(reader, MAX_QMP_MESSAGE, deadline)
        .map_err(|error| Error::Qmp(format!("cannot read QMP response: {error}")))?;
    if line.trim().is_empty() {
        return Err(Error::Qmp("QEMU closed the QMP socket".to_string()));
    }
    serde_json::from_str(line.trim())
        .map_err(|error| Error::Qmp(format!("invalid QMP response: {error}")))
}

fn write_all_until(stream: &mut IpcStream, bytes: &[u8], deadline: Instant) -> io::Result<()> {
    let mut offset = 0;
    while offset < bytes.len() {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "IPC write timed out",
            ));
        }
        stream.set_write_timeout(Some(remaining))?;
        let written = stream.write(&bytes[offset..])?;
        if written == 0 {
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "IPC peer accepted no data",
            ));
        }
        offset += written;
    }
    Ok(())
}

const MAX_QMP_MESSAGE: usize = 8 * 1024 * 1024;

fn connect_endpoint_retry(endpoint: &IpcEndpoint, service: &str) -> Result<IpcStream> {
    connect_endpoint_retry_with_timeout(endpoint, service, Duration::from_secs(1))
}

fn connect_endpoint_retry_with_timeout(
    endpoint: &IpcEndpoint,
    service: &str,
    timeout: Duration,
) -> Result<IpcStream> {
    let deadline = Instant::now()
        .checked_add(timeout)
        .ok_or_else(|| Error::message("IPC connection timeout is too large"))?;
    let last_error = loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break io::Error::new(io::ErrorKind::TimedOut, "IPC connection timed out");
        }
        match endpoint.connect(remaining) {
            Ok(stream) => return Ok(stream),
            Err(error) => {
                if Instant::now() >= deadline {
                    break error;
                }
                thread::sleep(
                    Duration::from_millis(50)
                        .min(deadline.saturating_duration_since(Instant::now())),
                );
            }
        }
    };
    Err(Error::message(format!(
        "cannot connect to {service} endpoint {}: {}",
        endpoint.display(),
        last_error
    )))
}

pub(crate) fn process_matches(pid: i32, name: &str) -> bool {
    process_matches_checked(pid, name).unwrap_or(false)
}

pub(crate) fn process_matches_checked_with_identity(
    pid: i32,
    name: &str,
    expected_identity: Option<&str>,
) -> Result<bool> {
    if !process_matches_checked(pid, name)? {
        return Ok(false);
    }
    Ok(expected_identity
        .is_none_or(|expected| process_identity(pid).is_some_and(|actual| actual == expected)))
}

pub(crate) fn process_matches_checked(pid: i32, name: &str) -> Result<bool> {
    if pid <= 0 {
        return Ok(false);
    }

    #[cfg(target_os = "linux")]
    {
        let command_line = match fs::read(format!("/proc/{pid}/cmdline")) {
            Ok(command_line) => command_line,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(Error::io(format!("/proc/{pid}/cmdline"), error)),
        };
        let fields: Vec<&[u8]> = command_line.split(|byte| *byte == 0).collect();
        let executable = fields
            .first()
            .copied()
            .and_then(|value| std::str::from_utf8(value).ok())
            .and_then(|value| Path::new(value).file_name())
            .and_then(|value| value.to_str());
        if !executable.is_some_and(|value| value.starts_with("qemu-system-")) {
            return Ok(false);
        }
        if name.is_empty() {
            return Ok(true);
        }
        let expected = format!("{name},process={name}");
        let arguments: Vec<&str> = fields
            .iter()
            .filter_map(|field| std::str::from_utf8(field).ok())
            .collect();
        let command = String::from_utf8_lossy(&command_line);
        Ok(arguments
            .iter()
            .any(|value| value.starts_with(&expected) || *value == format!("process={name}"))
            || command_line_has_vm_name(&command, name))
    }

    #[cfg(windows)]
    {
        let powershell = Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                &format!("(Get-CimInstance Win32_Process -Filter 'ProcessId = {pid}').CommandLine"),
            ])
            .output();
        if let Ok(output) = powershell
            && output.status.success()
        {
            let command_line = String::from_utf8_lossy(&output.stdout);
            if command_line.trim().is_empty() {
                return Ok(false);
            }
            let qemu = command_line.to_ascii_lowercase().contains("qemu-system-");
            let expected = format!("process={name}");
            return Ok(qemu
                && (name.is_empty()
                    || command_line.contains(&expected)
                    || command_line_has_vm_name(&command_line, name)));
        }

        let tasklist = Command::new("tasklist.exe")
            .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
            .output()
            .map_err(|error| Error::command_unavailable("tasklist.exe", error))?;
        if !tasklist.status.success() {
            return Err(Error::message(format!(
                "cannot inspect process {pid}; PowerShell and tasklist both failed"
            )));
        }
        let task = String::from_utf8_lossy(&tasklist.stdout).to_ascii_lowercase();
        if task.trim().is_empty() || task.contains("no tasks are running") {
            return Ok(false);
        }
        if !task.contains("qemu-system-") {
            return Ok(false);
        }
        if name.is_empty() {
            return Ok(true);
        }
        return Err(Error::message(format!(
            "cannot verify VM process {pid} identity without PowerShell"
        )));
    }

    #[cfg(all(unix, not(target_os = "linux")))]
    {
        let output = Command::new("ps")
            .args(["-p", &pid.to_string(), "-o", "command="])
            .output()
            .map_err(|error| Error::command_unavailable("ps", error))?;
        if !output.status.success() {
            return Ok(false);
        }
        let command = String::from_utf8_lossy(&output.stdout);
        let expected = format!("process={name}");
        Ok(command.contains("qemu-system-")
            && (name.is_empty()
                || command.contains(&expected)
                || command_line_has_vm_name(&command, name)))
    }
}

fn command_line_has_vm_name(command: &str, name: &str) -> bool {
    command
        .split_whitespace()
        .collect::<Vec<_>>()
        .windows(2)
        .any(|pair| pair == ["-name", name])
}

#[cfg(target_os = "linux")]
pub(crate) fn process_identity(pid: i32) -> Option<String> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let rest = stat.rsplit_once(") ")?.1;
    rest.split_whitespace().nth(19).map(str::to_string)
}

#[cfg(windows)]
pub(crate) fn process_identity(pid: i32) -> Option<String> {
    let output = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            &format!("(Get-CimInstance Win32_Process -Filter 'ProcessId = {pid}').CreationDate"),
        ])
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .filter(|value| !value.is_empty())
}

#[cfg(all(unix, not(target_os = "linux")))]
pub(crate) fn process_identity(pid: i32) -> Option<String> {
    Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "lstart="])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| {
            let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
            (!value.is_empty()).then(|| value.split_whitespace().collect::<Vec<_>>().join("_"))
        })
}

#[cfg(not(any(unix, windows)))]
pub(crate) fn process_identity(_pid: i32) -> Option<String> {
    None
}

fn process_record(pid: i32) -> String {
    process_identity(pid).map_or_else(
        || format!("{pid}\n"),
        |identity| format!("{pid} {identity}\n"),
    )
}

fn read_process_record(path: &Path) -> Option<(i32, Option<String>)> {
    let contents = fs::read_to_string(path).ok()?;
    let mut fields = contents.split_whitespace();
    let pid = fields.next()?.parse().ok()?;
    Some((pid, fields.next().map(str::to_string)))
}

fn helper_process_matches(pid: i32, name: &str, expected_identity: Option<&str>) -> bool {
    pid_matches(pid, name)
        && expected_identity
            .is_none_or(|expected| process_identity(pid).is_some_and(|actual| actual == expected))
}

fn detect_audio_driver(host_os: &str) -> Option<String> {
    if host_os == "macos" {
        return Some("coreaudio".to_string());
    }
    if host_os == "windows" {
        return Some("dsound".to_string());
    }
    if host_os == "freebsd" {
        return Some("oss".to_string());
    }
    let runtime = env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from);
    if runtime
        .as_ref()
        .is_some_and(|path| path.join("pipewire-0").exists())
    {
        Some("pipewire".to_string())
    } else if runtime
        .as_ref()
        .is_some_and(|path| path.join("pulse/native").exists())
    {
        Some("pa".to_string())
    } else {
        Some("alsa".to_string())
    }
}

pub(crate) fn render_node() -> Option<PathBuf> {
    let mut nodes = fs::read_dir("/dev/dri")
        .ok()?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter_map(|path| {
            let name = path.file_name()?.to_str()?;
            let number = name.strip_prefix("renderD")?.parse::<u32>().ok()?;
            File::open(&path).ok().map(|_| (number, path))
        })
        .collect::<Vec<_>>();
    nodes.sort_by_key(|(number, _)| *number);
    nodes.into_iter().next().map(|(_, path)| path)
}

fn default_cpu_cores() -> u32 {
    let host = std::thread::available_parallelism()
        .map(|parallelism| parallelism.get() as u32)
        .unwrap_or(2);
    if host >= 32 {
        16
    } else if host >= 16 {
        8
    } else if host >= 8 {
        4
    } else if host >= 4 {
        2
    } else {
        1
    }
}

fn default_ram() -> String {
    let gib = fs::read_to_string("/proc/meminfo")
        .ok()
        .and_then(|contents| {
            contents.lines().find_map(|line| {
                let value = line.strip_prefix("MemTotal:")?.split_whitespace().next()?;
                value.parse::<u64>().ok()
            })
        })
        .or_else(|| {
            Command::new("sysctl")
                .args(["-n", "hw.memsize"])
                .output()
                .ok()
                .filter(|output| output.status.success())
                .and_then(|output| String::from_utf8(output.stdout).ok())
                .and_then(|value| value.trim().parse::<u64>().ok())
                .map(|bytes| bytes / 1024 / 1024 / 1024)
        })
        .map(|kib| kib / 1024 / 1024)
        .unwrap_or(4);
    if gib >= 128 {
        "32G".to_string()
    } else if gib >= 64 {
        "16G".to_string()
    } else if gib >= 16 {
        "8G".to_string()
    } else {
        "4G".to_string()
    }
}

fn find_free_port(start: u16) -> Result<u16> {
    for port in start..=start.saturating_add(9) {
        if TcpListener::bind(("127.0.0.1", port)).is_ok() {
            return Ok(port);
        }
    }
    Err(Error::message(format!(
        "no free port found in {start}-{}",
        start + 9
    )))
}

fn ensure_command(command: &str) -> Result<()> {
    let output = Command::new(command)
        .arg("--version")
        .output()
        .map_err(|error| Error::command_unavailable(command, error))?;
    if !output.status.success() {
        return Err(Error::command_failed_status(command, output.status));
    }
    if let Some(version @ (major, minor, patch)) = qemu_version(&output.stdout)
        && !qemu_version_supported(version)
    {
        return Err(Error::message(format!(
            "{command} 6.1.0 or newer is required, detected {major}.{minor}.{patch}. Upgrade QEMU and retry."
        )));
    }
    Ok(())
}

fn qemu_version(output: &[u8]) -> Option<(u32, u32, u32)> {
    String::from_utf8_lossy(output)
        .split_whitespace()
        .find_map(parse_version_token)
}

fn parse_version_token(token: &str) -> Option<(u32, u32, u32)> {
    let token =
        token.trim_matches(|character: char| !character.is_ascii_digit() && character != '.');
    let mut parts = token.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next().unwrap_or("0").parse().ok()?;
    Some((major, minor, patch))
}

fn qemu_version_supported((major, minor, _patch): (u32, u32, u32)) -> bool {
    major > 6 || (major == 6 && minor >= 1)
}

fn qemu_supports_gtk_clipboard(binary: &str) -> bool {
    qemu_help_output(binary, &["-version"])
        .as_deref()
        .and_then(|output| qemu_version(output.as_bytes()))
        .is_some_and(qemu_version_supports_gtk_clipboard)
}

fn qemu_version_supports_gtk_clipboard(version: (u32, u32, u32)) -> bool {
    version >= (11, 1, 0)
}

fn qemu_supports_vdagent(binary: &str) -> bool {
    qemu_help_output(binary, &["-chardev", "help"])
        .is_some_and(|output| output.contains("qemu-vdagent"))
}

fn command_available(command: &str) -> bool {
    find_executable(command)
        .map(Command::new)
        .unwrap_or_else(|| Command::new(command))
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn find_executable(command: &str) -> Option<String> {
    let path = env::var_os("PATH")?;
    let names = executable_names(command);
    env::split_paths(&path).find_map(|directory| {
        names
            .iter()
            .map(|name| directory.join(name))
            .find(|candidate| is_executable_file(candidate))
            .map(|candidate| candidate.to_string_lossy().into_owned())
    })
}

fn is_executable_file(path: &Path) -> bool {
    let Ok(metadata) = fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

fn executable_names(command: &str) -> Vec<String> {
    #[cfg(windows)]
    {
        let mut names = vec![command.to_string()];
        if Path::new(command).extension().is_none() {
            let extensions =
                env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_string());
            names.extend(
                extensions
                    .split(';')
                    .filter(|extension| !extension.is_empty())
                    .map(|extension| format!("{command}{extension}")),
            );
        }
        names
    }
    #[cfg(not(windows))]
    {
        vec![command.to_string()]
    }
}

fn find_virtiofsd() -> Option<String> {
    find_executable("virtiofsd").or_else(|| {
        [
            "/usr/lib/virtiofsd",
            "/usr/libexec/virtiofsd",
            "/usr/lib/qemu/virtiofsd",
        ]
        .into_iter()
        .map(PathBuf::from)
        .find(|candidate| candidate.is_file())
        .map(|candidate| candidate.to_string_lossy().into_owned())
    })
}

pub(crate) fn virtiofsd_available() -> bool {
    find_virtiofsd().is_some()
}

fn qemu_supports_device(binary: &str, device: &str) -> bool {
    qemu_help_output(binary, &["-device", "help"])
        .is_some_and(|text| qemu_quoted_names(&text).iter().any(|name| name == device))
}

fn qemu_supports_gl_devices_in_names(names: &[String], arch: &str) -> bool {
    let devices = if arch == "aarch64" {
        ["virtio-gpu-gl-pci", "virtio-gpu-gl", ""]
    } else {
        ["virtio-vga-gl", "virtio-gpu-gl-pci", "virtio-gpu-gl"]
    };
    devices
        .into_iter()
        .filter(|device| !device.is_empty())
        .any(|device| names.iter().any(|name| name == device))
}

fn gl_device_supported(host: &QemuPlanContext, device: &str) -> bool {
    // `virtio_vga_gl` is only a cheap capability gate; the selected device is
    // always queried again here so one GL variant cannot authorize another.
    if !host.virtio_vga_gl {
        return false;
    }
    if command_available(&host.qemu_binary) {
        qemu_supports_device(&host.qemu_binary, device)
    } else {
        false
    }
}

fn qemu_quoted_names(text: &str) -> Vec<String> {
    text.lines()
        .flat_map(|line| {
            line.split('"')
                .enumerate()
                .filter(|(index, _)| index % 2 == 1)
                .map(|(_, value)| value.to_string())
                .collect::<Vec<_>>()
        })
        .collect()
}

fn qemu_supports_cpu_in_text(text: &str, model: &str) -> bool {
    text.lines().any(|line| {
        line.split_whitespace()
            .next()
            .is_some_and(|candidate| candidate == model)
    })
}

const MAX_PROBE_OUTPUT: usize = 64 * 1024;

fn read_limited(mut reader: impl Read) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 8192];
    loop {
        let count = reader.read(&mut chunk)?;
        if count == 0 {
            return Ok(bytes);
        }
        if bytes.len() + count > MAX_PROBE_OUTPUT {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "QEMU probe output exceeded the 64 KiB limit",
            ));
        }
        bytes.extend_from_slice(&chunk[..count]);
    }
}

fn qemu_help_output(binary: &str, args: &[&str]) -> Option<String> {
    let mut child = Command::new(binary)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;
    let (Some(mut stdout), Some(mut stderr)) = (child.stdout.take(), child.stderr.take()) else {
        let _ = child.kill();
        let _ = child.wait();
        return None;
    };
    let stdout_reader = thread::spawn(move || read_limited(&mut stdout));
    let stderr_reader = thread::spawn(move || read_limited(&mut stderr));
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut probe_failed = false;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
            Ok(None) => {
                let _ = child.kill();
                probe_failed = true;
                break;
            }
            Err(_) => {
                let _ = child.kill();
                probe_failed = true;
                break;
            }
        }
    }
    let status = match child.wait() {
        Ok(status) => Some(status),
        Err(_) => {
            let _ = child.kill();
            let _ = child.wait();
            None
        }
    };
    let stdout = stdout_reader.join().ok()?.ok()?;
    let stderr = stderr_reader.join().ok()?.ok()?;
    if probe_failed || !status.is_some_and(|status| status.success()) {
        return None;
    }
    Some({
        let mut text = String::from_utf8_lossy(&stdout).into_owned();
        text.push_str(&String::from_utf8_lossy(&stderr));
        text
    })
}

fn qemu_accelerators_probe(binary: &str) -> Option<Vec<String>> {
    qemu_help_output(binary, &["-accel", "help"]).map(|text| qemu_accelerators_from_text(&text))
}

fn qemu_accelerators_from_text(text: &str) -> Vec<String> {
    text.lines()
        .skip_while(|line| !line.contains("Accelerators supported"))
        .skip(1)
        .map(str::trim)
        .take_while(|value| !value.is_empty())
        .filter(|value| {
            value
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || character == '-')
        })
        .map(str::to_string)
        .collect()
}

fn qemu_runtime_accelerators(
    binary: &str,
    compiled: &[String],
) -> (Vec<String>, Vec<String>, Vec<String>) {
    let mut usable = Vec::new();
    let mut failures = Vec::new();
    let mut unprobed = Vec::new();
    for accelerator in compiled {
        if accelerator == "tcg" {
            usable.push(accelerator.clone());
        } else if matches!(accelerator.as_str(), "kvm" | "hvf" | "whpx") {
            if qemu_accelerator_usable(binary, accelerator) {
                usable.push(accelerator.clone());
            } else {
                failures.push(accelerator.clone());
            }
        } else {
            unprobed.push(accelerator.clone());
        }
    }
    (usable, failures, unprobed)
}

fn read_qmp_greeting(mut reader: impl Read) -> io::Result<bool> {
    let mut greeting = Vec::new();
    let mut byte = [0_u8; 1];
    loop {
        let count = reader.read(&mut byte)?;
        if count == 0 {
            break;
        }
        if greeting.len() == MAX_PROBE_OUTPUT {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "QMP greeting exceeded the 64 KiB limit",
            ));
        }
        greeting.push(byte[0]);
        if byte[0] == b'\n' {
            break;
        }
    }
    if greeting.is_empty() {
        return Ok(false);
    }
    let value: Value = serde_json::from_slice(&greeting).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("invalid QMP greeting: {error}"),
        )
    })?;
    let Some(qmp) = value.get("QMP").and_then(Value::as_object) else {
        return Ok(false);
    };
    Ok(qmp.get("version").is_some_and(Value::is_object)
        && qmp.get("capabilities").is_some_and(Value::is_array))
}

fn qemu_runtime_probe(binary: &str, accelerator: &str, cpu: &str) -> Result<()> {
    let machine = format!("accel={accelerator}");
    let mut child = Command::new(binary)
        .args([
            "-nodefaults",
            "-S",
            "-display",
            "none",
            "-machine",
            &machine,
            "-cpu",
            cpu,
            "-qmp",
            "stdio",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| Error::command_unavailable(binary, error))?;
    let Some(mut stdout) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return Err(Error::message(
            "QEMU CPU capability probe did not provide QMP output",
        ));
    };
    let Some(mut stderr) = child.stderr.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return Err(Error::message(
            "QEMU CPU capability probe did not provide stderr",
        ));
    };
    let (ready_tx, ready_rx) = mpsc::channel();
    let stdout_reader = thread::spawn(move || {
        let ready = read_qmp_greeting(&mut stdout);
        let _ = ready_tx.send(ready);
        let mut discarded = [0_u8; 8192];
        while stdout.read(&mut discarded).is_ok_and(|count| count > 0) {}
    });
    let stderr_reader = thread::spawn(move || read_limited(&mut stderr));
    let readiness = ready_rx.recv_timeout(Duration::from_secs(2));
    let mut exited_after_ready = false;
    let mut settle_error = None;
    if matches!(readiness, Ok(Ok(true))) {
        let settle_deadline = Instant::now() + Duration::from_millis(250);
        loop {
            match child.try_wait() {
                Ok(Some(_)) => {
                    exited_after_ready = true;
                    break;
                }
                Ok(None) if Instant::now() < settle_deadline => {
                    thread::sleep(Duration::from_millis(10));
                }
                Ok(None) => break,
                Err(error) => {
                    settle_error = Some(error);
                    break;
                }
            }
        }
    }
    let mut killed_by_us = false;
    if !exited_after_ready {
        match child.try_wait() {
            Ok(Some(_)) => exited_after_ready = true,
            Ok(None) => killed_by_us = child.kill().is_ok(),
            Err(error) => {
                settle_error = Some(error);
                let _ = child.kill();
            }
        }
    }
    let status = match child.wait() {
        Ok(status) => status,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            let _ = stdout_reader.join();
            let _ = stderr_reader.join();
            return Err(Error::io("QEMU CPU capability probe", error));
        }
    };
    let _ = stdout_reader.join();
    let stderr = stderr_reader
        .join()
        .map_err(|_| Error::message("QEMU CPU capability probe reader failed"))?
        .map_err(|error| Error::io("QEMU CPU capability probe", error))?;
    if let Some(error) = settle_error {
        return Err(Error::io("QEMU CPU capability probe", error));
    }
    match readiness {
        Ok(Ok(true)) if !exited_after_ready && killed_by_us => Ok(()),
        Ok(Ok(false)) => Err(Error::message(probe_error_message(
            "QEMU runtime probe failed",
            &stderr,
            status,
        ))),
        Ok(Ok(true)) => Err(Error::message(probe_error_message(
            "QEMU runtime probe exited during initialization",
            &stderr,
            status,
        ))),
        Ok(Err(error)) => Err(Error::io("QEMU CPU capability probe", error)),
        Err(_) => Err(Error::message(
            "QEMU runtime probe timed out after 2 seconds",
        )),
    }
}

fn qemu_accelerator_usable(binary: &str, accelerator: &str) -> bool {
    qemu_runtime_probe(binary, accelerator, "max").is_ok()
}

fn validate_cpu_spec(binary: &str, cpu: &str, accelerator: &str) -> Result<()> {
    qemu_runtime_probe(binary, accelerator, cpu).map_err(|error| {
        Error::message(format!("QEMU rejected CPU specification '{cpu}': {error}"))
    })
}

fn probe_error_message(prefix: &str, stderr: &[u8], status: std::process::ExitStatus) -> String {
    let detail = String::from_utf8_lossy(stderr).trim().to_string();
    if detail.is_empty() {
        format!("{prefix} with status {status}")
    } else {
        format!("{prefix}: {detail}")
    }
}

fn qemu_display_backends_probe(binary: &str) -> Option<Vec<String>> {
    qemu_help_output(binary, &["-display", "help"])
        .map(|text| qemu_display_backends_from_text(&text))
}

fn qemu_display_backends_from_text(text: &str) -> Vec<String> {
    let mut backends = Vec::new();
    let mut in_section = false;
    for line in text.lines() {
        if line.contains("Available display backend types:") {
            in_section = true;
            continue;
        }
        if !in_section {
            continue;
        }
        let value = line.trim();
        if value.is_empty() || value.starts_with("Some ") {
            break;
        }
        if value.chars().all(|character| {
            character.is_ascii_alphanumeric() || character == '-' || character == '_'
        }) {
            backends.push(value.to_string());
        }
    }
    backends
}

fn qemu_netdev_backends_probe(binary: &str) -> Option<Vec<String>> {
    qemu_help_output(binary, qemu_netdev_help_args(binary))
        .map(|text| qemu_netdev_backends_from_text(&text))
}

fn qemu_netdev_help_args(binary: &str) -> &[&str] {
    if binary.contains("aarch64") {
        &["-machine", "virt", "-netdev", "help"][..]
    } else {
        &["-netdev", "help"][..]
    }
}

fn qemu_netdev_backends_from_text(text: &str) -> Vec<String> {
    let mut backends = Vec::new();
    let mut in_section = false;
    for line in text.lines() {
        if line.contains("Available netdev backend types:") {
            in_section = true;
            continue;
        }
        if !in_section {
            continue;
        }
        let value = line.trim();
        if value.is_empty() || value.starts_with("Some ") {
            break;
        }
        if value.chars().all(|character| {
            character.is_ascii_alphanumeric() || character == '-' || character == '_'
        }) {
            backends.push(value.to_string());
        }
    }
    backends
}

pub(crate) fn qemu_capability_report(binary: &str) -> Value {
    let version = qemu_help_output(binary, &["-version"]).and_then(|text| {
        qemu_version(text.as_bytes())
            .map(|(major, minor, patch)| format!("{major}.{minor}.{patch}"))
    });
    let available = version.is_some();
    if !available {
        return json!({
            "available": false,
            "complete": false,
            "version": Value::Null,
            "probe_error": format!(
                "could not execute '{binary}' or its capability query failed"
            ),
            "accelerators": [],
            "runtime_accelerators": [],
            "runtime_probe_failures": [],
            "runtime_unprobed": [],
            "runtime_complete": false,
            "display_backends": [],
            "network_backends": [],
            "devices": {},
            "cpu_models": {},
        });
    }
    let display_probe = qemu_help_output(binary, &["-display", "help"]);
    let display = display_probe
        .as_deref()
        .map_or_else(Vec::new, qemu_display_backends_from_text);
    let network_backends = qemu_netdev_backends_probe(binary);
    let accelerator_probe = qemu_help_output(binary, &["-accel", "help"]);
    let accelerators = accelerator_probe
        .as_deref()
        .map_or_else(Vec::new, qemu_accelerators_from_text);
    let (runtime_accelerators, runtime_probe_failures, runtime_unprobed) =
        qemu_runtime_accelerators(binary, &accelerators);
    let device_probe = qemu_help_output(binary, &["-device", "help"]);
    let device_names = device_probe
        .as_deref()
        .map_or_else(Vec::new, qemu_quoted_names);
    let cpu_probe = qemu_help_output(binary, &["-cpu", "help"]);
    let complete = display_probe.is_some()
        && network_backends.is_some()
        && accelerator_probe.is_some()
        && device_probe.is_some()
        && cpu_probe.is_some();
    let devices = [
        "virtio-vga-gl",
        "virtio-gpu-gl",
        "virtio-gpu-gl-pci",
        "usb-redir",
        "usb-ccid",
        "ccid-card-passthru",
        "vhost-user-fs-pci",
        "virtio-sound-pci",
    ];
    let device_support = devices
        .into_iter()
        .map(|device| {
            (
                device.to_string(),
                json!(device_probe.is_some() && device_names.iter().any(|name| name == device)),
            )
        })
        .collect::<serde_json::Map<_, _>>();
    let cpu_models = ["host", "max", "qemu64", "qemu32", "Haswell-v2"]
        .into_iter()
        .map(|model| {
            (
                model.to_string(),
                json!(
                    cpu_probe
                        .as_deref()
                        .is_some_and(|text| qemu_supports_cpu_in_text(text, model))
                ),
            )
        })
        .collect::<serde_json::Map<_, _>>();
    let failed_probes = [
        ("display", display_probe.is_none()),
        ("network", network_backends.is_none()),
        ("accelerator", accelerator_probe.is_none()),
        ("device", device_probe.is_none()),
        ("cpu", cpu_probe.is_none()),
    ]
    .into_iter()
    .filter_map(|(name, failed)| failed.then_some(name))
    .collect::<Vec<_>>();
    json!({
        "available": available,
        "complete": complete,
        "version": version,
        "probe_error": (!complete).then(|| {
            format!("capability probes failed: {}", failed_probes.join(", "))
        }),
        "accelerators": accelerators,
        "runtime_accelerators": runtime_accelerators,
        "runtime_probe_failures": runtime_probe_failures,
        "runtime_unprobed": runtime_unprobed,
        "runtime_complete": accelerator_probe.is_some()
            && runtime_probe_failures.is_empty()
            && runtime_unprobed.is_empty(),
        "display_backends": display,
        "network_backends": network_backends.unwrap_or_default(),
        "devices": device_support,
        "cpu_models": cpu_models,
    })
}

fn first_existing(paths: &[&str]) -> Option<PathBuf> {
    paths.iter().map(PathBuf::from).find(|path| path.is_file())
}

#[cfg(test)]
fn first_complete_pair(pairs: &[(&str, &str)]) -> Option<(PathBuf, PathBuf)> {
    pairs.iter().find_map(|(code, vars)| {
        let code = Path::new(code);
        let vars = Path::new(vars);
        (code.is_file() && vars.is_file()).then(|| (code.to_path_buf(), vars.to_path_buf()))
    })
}

fn firmware_format(path: &Path) -> &'static str {
    let mut magic = [0; 4];
    if File::open(path)
        .and_then(|mut file| file.read_exact(&mut magic))
        .is_ok()
        && magic == [0x51, 0x46, 0x49, 0xfb]
    {
        "qcow2"
    } else {
        "raw"
    }
}

fn add(args: &mut Vec<String>, flag: &str, value: String) {
    args.push(flag.to_string());
    args.push(value);
}

fn qemu_path(path: &Path) -> String {
    path.display().to_string().replace(',', ",,")
}

fn control_endpoint(path: &Path, host_os: &str) -> String {
    if host_os == "windows" {
        #[cfg(windows)]
        return format!("pipe:{}", control_pipe_name(path));
        #[cfg(not(windows))]
        return format!("tcp:127.0.0.1:{},server=on,wait=off", control_port(path));
    }
    #[cfg(unix)]
    {
        format!("unix:{},server=on,wait=off", qemu_path(path))
    }
    #[cfg(not(unix))]
    {
        format!("tcp:127.0.0.1:{},server=on,wait=off", control_port(path))
    }
}

#[cfg(windows)]
fn control_pipe_name(path: &Path) -> String {
    let mut hash = 2_166_136_261u32;
    for byte in path.to_string_lossy().bytes() {
        hash ^= u32::from(byte);
        hash = hash.wrapping_mul(16_777_619);
    }
    format!("vmctl-control-{hash:08x}")
}

fn socket_chardev(path: &Path, id: &str, host_os: &str) -> String {
    if host_os == "windows" {
        return format!(
            "socket,id={id},host=127.0.0.1,port={},server=off,wait=off",
            control_port(path)
        );
    }
    #[cfg(unix)]
    {
        format!(
            "socket,id={id},path={},server=off,wait=off",
            qemu_path(path)
        )
    }
    #[cfg(not(unix))]
    {
        format!(
            "socket,id={id},host=127.0.0.1,port={},server=off,wait=off",
            control_port(path)
        )
    }
}

fn control_port(path: &Path) -> u16 {
    let mut hash = 2_166_136_261u32;
    for byte in path.to_string_lossy().bytes() {
        hash ^= u32::from(byte);
        hash = hash.wrapping_mul(16_777_619);
    }
    40_000 + (hash % 20_000) as u16
}

pub fn shell_join(binary: &str, args: &[String]) -> String {
    let mut command = String::new();
    write_shell_quoted(binary, &mut command);
    for argument in args {
        command.push(' ');
        write_shell_quoted(argument, &mut command);
    }
    command
}

fn write_shell_quoted(value: &str, output: &mut String) {
    if !value.is_empty()
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "_./:=,-".contains(character))
    {
        output.push_str(value);
        return;
    }
    output.push('\'');
    output.push_str(&value.replace('\'', "'\\''"));
    output.push('\'');
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::load_vm;
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::symlink;
    #[cfg(unix)]
    use std::os::unix::net::UnixListener;
    use tempfile::tempdir;

    #[test]
    fn wait_for_exit_treats_missing_process_as_stopped() {
        assert!(wait_for_exit(-1, "vmctl-test", Duration::ZERO));
    }

    #[test]
    fn process_match_treats_missing_process_as_stopped() {
        assert!(!process_matches_checked(i32::MAX, "vmctl-test").unwrap());
    }

    #[test]
    fn operation_lock_prevents_concurrent_acquisition() {
        let root = tempdir().unwrap();
        let paths = VmPaths::new(root.path(), "lock-test");
        let lock = acquire_vm_lock(&paths).unwrap();
        let error = acquire_vm_lock(&paths).err().unwrap();
        assert!(error.to_string().contains("another vmctl operation"));
        drop(lock);
        assert!(acquire_vm_lock(&paths).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn operation_lock_refuses_symbolic_links() {
        let root = tempdir().unwrap();
        let paths = VmPaths::new(root.path(), "lock-test");
        fs::create_dir_all(&paths.state_dir).unwrap();
        let target = root.path().join("target");
        fs::write(&target, "keep").unwrap();
        symlink(&target, paths.state_dir.join("operation.lock")).unwrap();

        let error = acquire_vm_lock(&paths).err().unwrap();

        assert!(error.to_string().contains("symbolic-link"));
        assert_eq!(fs::read_to_string(target).unwrap(), "keep");
    }

    #[test]
    fn monitor_response_has_a_size_limit() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let writer = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream.write_all(&vec![b'x'; MAX_MONITOR_RESPONSE + 1])
        });
        let mut stream = TcpStream::connect(address).unwrap();
        let error = read_monitor_response(
            &mut stream,
            &address.to_string(),
            Instant::now() + Duration::from_secs(1),
        )
        .unwrap_err();

        assert!(error.to_string().contains("safety limit"));
        writer.join().unwrap().unwrap();
    }

    #[test]
    fn monitor_waits_for_the_first_response_byte() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let writer = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            thread::sleep(Duration::from_millis(600));
            stream.write_all(b"ok")
        });
        let mut stream = TcpStream::connect(address).unwrap();
        let response = read_monitor_response(
            &mut stream,
            &address.to_string(),
            Instant::now() + Duration::from_secs(2),
        )
        .unwrap();

        assert_eq!(response, b"ok");
        writer.join().unwrap().unwrap();
    }

    #[test]
    fn monitor_connect_resolves_addresses_within_the_deadline() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let acceptor = thread::spawn(move || listener.accept());

        let stream = connect_monitor(
            &address.to_string(),
            Instant::now() + Duration::from_secs(1),
        )
        .unwrap();

        assert_eq!(stream.peer_addr().unwrap(), address);
        drop(stream);
        acceptor.join().unwrap().unwrap();
    }

    #[test]
    fn ipc_endpoint_json_rejects_non_loopback_addresses() {
        let value = json!({
            "transport": "tcp",
            "host": "0.0.0.0",
            "port": 49152,
        });
        let error = IpcEndpoint::from_json(&value).unwrap_err();
        assert_eq!(
            error.to_string(),
            "runtime TCP endpoint must be bound to loopback"
        );
    }

    #[test]
    fn endpoint_preflight_reports_listener_conflicts() {
        let port = 0;
        let plan = QemuPlan {
            binary: "qemu-system-x86_64".to_string(),
            args: Vec::new(),
            qmp_endpoint: IpcEndpoint::Tcp(format!("127.0.0.1:{port}").parse().unwrap()),
            agent_endpoint: None,
            ssh_port: Some(port),
            ssh_host: Some("127.0.0.1".to_string()),
            spice_port: None,
            spice_host: None,
            monitor_telnet: None,
            serial_telnet: None,
        };
        let error = ensure_ipc_endpoints_available(&plan).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("conflicts with another configured listener")
        );
    }

    #[test]
    fn macos_style_qemu_names_are_recognized() {
        assert!(command_line_has_vm_name(
            "qemu-system-x86_64 -name ubuntu-24.04 -machine q35",
            "ubuntu-24.04"
        ));
        assert!(!command_line_has_vm_name(
            "qemu-system-x86_64 -name other -machine q35",
            "ubuntu-24.04"
        ));
    }

    #[cfg(unix)]
    #[test]
    fn disk_operations_reject_symlink_paths() {
        use std::os::unix::fs::symlink;

        let root = tempdir().unwrap();
        let target = root.path().join("disk.qcow2");
        let link = root.path().join("disk-link.qcow2");
        fs::write(&target, []).unwrap();
        symlink(&target, &link).unwrap();
        let error = require_disk_file(&link).unwrap_err();
        assert!(error.to_string().contains("refusing to use disk symlink"));
    }

    #[test]
    fn windows_plan_uses_local_ipc_transport() {
        let root = tempdir().unwrap();
        let config_path = root.path().join("windows-host.conf");
        fs::write(root.path().join("disk.qcow2"), []).unwrap();
        fs::write(
            &config_path,
            "boot=legacy\ndisk_img=disk.qcow2\ndisplay=none\nnetwork=none\npublic_dir=none\n",
        )
        .unwrap();
        let vm = load_vm(root.path(), root.path(), config_path).unwrap();
        let host = QemuPlanContext {
            qemu_binary: "qemu-system-x86_64".to_string(),
            host_os: "windows".to_string(),
            accelerator: "tcg".to_string(),
            cpu_cores: 2,
            ram: "4G".to_string(),
            virtio_vga_gl: false,
            usb_redirection: false,
            smartcard: false,
            smbd: false,
            audio_driver: Some("dsound".to_string()),
            username: "tester".to_string(),
            bridge_helper: None,
            virtiofsd: None,
            virtiofs_device: false,
            ssh_port: None,
            spice_port: Some(5930),
        };

        let plan = build_plan(&vm, &host, false).unwrap();
        assert!(
            plan.args
                .iter()
                .all(|arg| !arg.contains("filename=/dev/urandom"))
        );
        #[cfg(not(windows))]
        assert!(
            plan.args
                .iter()
                .any(|arg| arg.starts_with("tcp:127.0.0.1:"))
        );
        #[cfg(windows)]
        assert!(
            plan.args
                .windows(2)
                .any(|args| args[0] == "-chardev" && args[1].starts_with("pipe,id=qmp0,"))
        );
        #[cfg(not(windows))]
        assert!(
            plan.args
                .iter()
                .any(|arg| { arg.starts_with("socket,id=qga0,host=127.0.0.1,port=") })
        );
        #[cfg(windows)]
        assert!(
            plan.args
                .iter()
                .any(|arg| arg.starts_with("pipe,id=qga0,path="))
        );
        assert!(!plan.args.iter().any(|arg| arg.starts_with("unix:")));
        #[cfg(not(windows))]
        assert!(matches!(plan.qmp_endpoint, IpcEndpoint::Tcp(_)));
        #[cfg(windows)]
        assert!(matches!(plan.qmp_endpoint, IpcEndpoint::Pipe(_)));
        #[cfg(not(windows))]
        assert!(matches!(plan.agent_endpoint, Some(IpcEndpoint::Tcp(_))));
        #[cfg(windows)]
        assert!(matches!(plan.agent_endpoint, Some(IpcEndpoint::Pipe(_))));
    }

    #[test]
    fn runtime_ipc_state_round_trips_atomically() {
        let root = tempdir().unwrap();
        let config_path = root.path().join("ipc.conf");
        fs::write(root.path().join("disk.qcow2"), []).unwrap();
        fs::write(
            &config_path,
            "boot=legacy\ndisk_img=disk.qcow2\ndisplay=none\nnetwork=none\npublic_dir=none\n",
        )
        .unwrap();
        let vm = load_vm(root.path(), root.path(), config_path).unwrap();
        let host = QemuPlanContext {
            qemu_binary: "qemu-system-x86_64".to_string(),
            host_os: "windows".to_string(),
            accelerator: "tcg".to_string(),
            cpu_cores: 2,
            ram: "4G".to_string(),
            virtio_vga_gl: false,
            usb_redirection: false,
            smartcard: false,
            smbd: false,
            audio_driver: Some("dsound".to_string()),
            username: "tester".to_string(),
            bridge_helper: None,
            virtiofsd: None,
            virtiofs_device: false,
            ssh_port: None,
            spice_port: Some(5930),
        };
        let plan = build_plan(&vm, &host, false).unwrap();
        write_runtime_files(&vm.paths, &plan).unwrap();
        let (qmp, agent) = read_ipc_state(&vm.paths).unwrap();
        assert_eq!(qmp, plan.qmp_endpoint);
        assert_eq!(agent, plan.agent_endpoint);
        assert!(!vm.paths.ipc_state().with_extension("tmp").exists());
    }

    #[test]
    fn shell_quoting_is_safe_for_spaces_and_quotes() {
        assert_eq!(
            shell_join("qemu-system-x86_64", &["path with spaces".to_string()]),
            "qemu-system-x86_64 'path with spaces'"
        );
        assert_eq!(
            shell_join("qemu", &["it's safe".to_string()]),
            "qemu 'it'\\''s safe'"
        );
    }

    #[test]
    fn qemu_version_check_handles_single_and_double_digit_releases() {
        assert_eq!(
            qemu_version(b"QEMU emulator version 6.1.0 (v6.1.0)"),
            Some((6, 1, 0))
        );
        assert_eq!(
            qemu_version(b"QEMU emulator version 10.0.3"),
            Some((10, 0, 3))
        );
        assert!(!qemu_version_supported((6, 0, 9)));
        assert!(qemu_version_supported((6, 1, 0)));
        assert!(qemu_version_supported((10, 0, 0)));
    }

    #[cfg(unix)]
    #[test]
    fn executable_lookup_rejects_non_executable_files() {
        let root = tempdir().unwrap();
        let command = root.path().join("qemu-system-test");
        fs::write(&command, "#!/bin/sh\n").unwrap();
        assert!(!is_executable_file(&command));
        fs::set_permissions(&command, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(is_executable_file(&command));
    }

    #[test]
    fn gtk_clipboard_requires_qemu_11_1() {
        assert!(qemu_version_supports_gtk_clipboard((11, 1, 0)));
        assert!(!qemu_version_supports_gtk_clipboard((11, 0, 9)));
    }

    #[test]
    fn monitor_output_drops_terminal_control_sequences() {
        assert_eq!(
            clean_monitor_output(b"\x1b[Kinfo status\x1b[D\n(qemu)"),
            "info status\n(qemu)"
        );
    }

    #[test]
    fn cocoa_display_is_rejected_on_linux() {
        let root = tempdir().unwrap();
        let config_path = root.path().join("cocoa.conf");
        fs::write(root.path().join("disk.qcow2"), []).unwrap();
        fs::write(
            &config_path,
            "boot=legacy\ndisk_img=disk.qcow2\ndisplay=cocoa\nnetwork=none\npublic_dir=none\n",
        )
        .unwrap();
        let vm = load_vm(root.path(), root.path(), config_path).unwrap();
        let host = QemuPlanContext {
            qemu_binary: "qemu-system-x86_64".to_string(),
            host_os: "linux".to_string(),
            accelerator: "tcg".to_string(),
            cpu_cores: 2,
            ram: "4G".to_string(),
            virtio_vga_gl: false,
            usb_redirection: false,
            smartcard: false,
            smbd: false,
            audio_driver: None,
            username: "tester".to_string(),
            bridge_helper: None,
            virtiofsd: None,
            virtiofs_device: false,
            ssh_port: None,
            spice_port: None,
        };

        let error = build_plan(&vm, &host, false).unwrap_err();
        assert_eq!(
            error.to_string(),
            "display mode 'cocoa' is only supported on macOS"
        );
    }

    #[test]
    fn spice_app_uses_managed_software_rendering() {
        let root = tempdir().unwrap();
        let config_path = root.path().join("spice-app.conf");
        fs::write(root.path().join("disk.qcow2"), []).unwrap();
        fs::write(
            &config_path,
            "boot=legacy\ndisk_img=disk.qcow2\ndisplay=spice-app\nnetwork=none\npublic_dir=none\n",
        )
        .unwrap();
        let vm = load_vm(root.path(), root.path(), config_path).unwrap();
        let host = QemuPlanContext {
            qemu_binary: "qemu-system-x86_64".to_string(),
            host_os: "linux".to_string(),
            accelerator: "tcg".to_string(),
            cpu_cores: 2,
            ram: "4G".to_string(),
            virtio_vga_gl: true,
            usb_redirection: false,
            smartcard: false,
            smbd: false,
            audio_driver: None,
            username: "tester".to_string(),
            bridge_helper: None,
            virtiofsd: None,
            virtiofs_device: false,
            ssh_port: None,
            spice_port: None,
        };

        let plan = build_plan(&vm, &host, false).unwrap();
        assert!(
            plan.args
                .windows(2)
                .any(|args| args == ["-display", "none"])
        );
        assert!(
            plan.args
                .windows(2)
                .any(|args| args == ["-device", "virtio-gpu"])
        );
        assert!(!plan.args.iter().any(|arg| arg == "virtio-gpu-gl"));
        assert!(
            plan.args
                .windows(2)
                .any(|args| { args[0] == "-spice" && args[1].contains("disable-ticketing=on") })
        );
        assert!(!plan.args.iter().any(|arg| arg == "spice-app,gl=off"));
    }

    #[test]
    fn plan_builder_is_deterministic_with_injected_host_capabilities() {
        let root = tempdir().unwrap();
        let config_path = root.path().join("test.conf");
        let disk = root.path().join("disk.qcow2");
        fs::write(&disk, []).unwrap();
        fs::write(
            &config_path,
            "boot=legacy\ndisk_img=disk.qcow2\ndisplay=none\nnetwork=none\npublic_dir=none\nguest_agent=false\n",
        )
        .unwrap();
        let vm = load_vm(root.path(), root.path(), config_path).unwrap();
        let host = QemuPlanContext {
            qemu_binary: "qemu-system-x86_64".to_string(),
            host_os: "linux".to_string(),
            accelerator: "tcg".to_string(),
            cpu_cores: 2,
            ram: "4G".to_string(),
            virtio_vga_gl: false,
            usb_redirection: false,
            smartcard: false,
            smbd: false,
            audio_driver: None,
            username: "tester".to_string(),
            bridge_helper: None,
            virtiofsd: None,
            virtiofs_device: false,
            ssh_port: None,
            spice_port: Some(5930),
        };

        let plan = build_plan(&vm, &host, false).unwrap();
        assert_eq!(plan.ssh_port, None);
        let qmp_value = format!(
            "unix:{},server=on,wait=off",
            vm.paths.qmp_socket().display()
        );
        assert!(
            plan.args
                .windows(2)
                .any(|args| args[0] == "-qmp" && args[1] == qmp_value)
        );
        assert!(plan.args.windows(2).any(|args| args == ["-nic", "none"]));
        assert!(plan.args.windows(2).any(|args| args == ["-vga", "none"]));
        assert!(plan.args.windows(2).any(|args| {
            args[0] == "-spice" && args[1] == "port=5930,addr=127.0.0.1,disable-ticketing=on"
        }));
        assert!(
            plan.args
                .windows(2)
                .any(|args| args == ["-display", "none"])
        );
    }

    #[test]
    fn linux_public_share_uses_virtiofs_when_available() {
        let root = tempdir().unwrap();
        let config_path = root.path().join("virtiofs.conf");
        fs::write(root.path().join("disk.qcow2"), []).unwrap();
        fs::create_dir(root.path().join("public")).unwrap();
        fs::write(
            &config_path,
            "boot=legacy\ndisk_img=disk.qcow2\ndisplay=none\nnetwork=none\npublic_dir=public\n",
        )
        .unwrap();
        let vm = load_vm(root.path(), root.path(), config_path).unwrap();
        let host = QemuPlanContext {
            qemu_binary: "qemu-system-x86_64".to_string(),
            host_os: "linux".to_string(),
            accelerator: "tcg".to_string(),
            cpu_cores: 2,
            ram: "4G".to_string(),
            virtio_vga_gl: false,
            usb_redirection: false,
            smartcard: false,
            smbd: false,
            audio_driver: None,
            username: "tester".to_string(),
            bridge_helper: None,
            virtiofsd: Some("/usr/bin/virtiofsd".to_string()),
            virtiofs_device: true,
            ssh_port: None,
            spice_port: Some(5930),
        };

        let plan = build_plan(&vm, &host, false).unwrap();
        assert!(virtiofs_requested(&vm.config, &host));
        assert!(
            plan.args
                .iter()
                .any(|arg| arg
                    == "vhost-user-fs-pci,queue-size=1024,chardev=char0,tag=Public-tester")
        );
        assert!(!plan.args.iter().any(|arg| arg == "virtio-9p-pci"));
        assert!(
            plan.args
                .iter()
                .any(|arg| arg.starts_with("memory-backend-file,id=mem,"))
        );
    }

    #[test]
    fn macos_public_share_uses_9p() {
        let root = tempdir().unwrap();
        let config_path = root.path().join("macos.conf");
        fs::write(
            &config_path,
            "guest_os=macos\nboot=efi\ndisplay=none\nnetwork=none\npublic_dir=public\n",
        )
        .unwrap();
        fs::create_dir(root.path().join("public")).unwrap();
        let vm = load_vm(root.path(), root.path(), config_path).unwrap();
        let host = QemuPlanContext {
            qemu_binary: "qemu-system-x86_64".to_string(),
            host_os: "linux".to_string(),
            accelerator: "tcg".to_string(),
            cpu_cores: 2,
            ram: "4G".to_string(),
            virtio_vga_gl: false,
            usb_redirection: false,
            smartcard: false,
            smbd: false,
            audio_driver: None,
            username: "tester".to_string(),
            bridge_helper: None,
            virtiofsd: None,
            virtiofs_device: false,
            ssh_port: None,
            spice_port: Some(5930),
        };
        let mut args = Vec::new();
        add_share_args(&mut args, &vm, &host);
        assert!(args.iter().any(|arg| arg.starts_with("local,id=fsdev0,")));
        assert!(
            args.iter()
                .any(|arg| arg == "virtio-9p-pci,fsdev=fsdev0,mount_tag=Public-tester")
        );
    }

    #[test]
    fn unsupported_guest_does_not_receive_public_share() {
        let root = tempdir().unwrap();
        let config_path = root.path().join("freebsd.conf");
        fs::write(
            &config_path,
            "guest_os=freebsd\nboot=efi\ndisplay=none\nnetwork=none\npublic_dir=public\n",
        )
        .unwrap();
        fs::create_dir(root.path().join("public")).unwrap();
        let vm = load_vm(root.path(), root.path(), config_path).unwrap();
        let host = QemuPlanContext {
            qemu_binary: "qemu-system-x86_64".to_string(),
            host_os: "linux".to_string(),
            accelerator: "tcg".to_string(),
            cpu_cores: 2,
            ram: "4G".to_string(),
            virtio_vga_gl: false,
            usb_redirection: false,
            smartcard: false,
            smbd: false,
            audio_driver: None,
            username: "tester".to_string(),
            bridge_helper: None,
            virtiofsd: None,
            virtiofs_device: false,
            ssh_port: None,
            spice_port: Some(5930),
        };
        let mut args = Vec::new();
        add_share_args(&mut args, &vm, &host);
        assert!(args.is_empty());
    }

    #[test]
    fn windows_server_uses_smb_but_not_guest_filesystem_shares() {
        let root = tempdir().unwrap();
        let config_path = root.path().join("windows-server.conf");
        fs::write(
            &config_path,
            "guest_os=windows-server\nboot=legacy\ndisplay=none\nnetwork=user\npublic_dir=public\n",
        )
        .unwrap();
        fs::create_dir(root.path().join("public")).unwrap();
        let vm = load_vm(root.path(), root.path(), config_path).unwrap();
        let host = QemuPlanContext {
            qemu_binary: "qemu-system-x86_64".to_string(),
            host_os: "linux".to_string(),
            accelerator: "tcg".to_string(),
            cpu_cores: 2,
            ram: "4G".to_string(),
            virtio_vga_gl: false,
            usb_redirection: false,
            smartcard: false,
            smbd: true,
            audio_driver: None,
            username: "tester".to_string(),
            bridge_helper: None,
            virtiofsd: None,
            virtiofs_device: false,
            ssh_port: None,
            spice_port: Some(5930),
        };
        let mut args = Vec::new();
        add_share_args(&mut args, &vm, &host);
        assert!(args.is_empty());
        let mut network_args = Vec::new();
        add_network_args(&mut network_args, &vm, Some(22444), true, None).unwrap();
        assert!(network_args.iter().any(|arg| arg.contains("smb=")));
    }

    #[test]
    fn qemu_option_paths_double_commas_without_rewriting_backslashes() {
        let path = Path::new("/tmp/disk,one\\two.qcow2");
        assert_eq!(qemu_path(path), "/tmp/disk,,one\\two.qcow2");
    }

    #[test]
    fn unsafe_share_username_becomes_a_safe_mount_tag() {
        let root = tempdir().unwrap();
        let config_path = root.path().join("linux.conf");
        fs::create_dir(root.path().join("public")).unwrap();
        fs::write(
            &config_path,
            "boot=legacy\ndisplay=none\nnetwork=none\npublic_dir=public\n",
        )
        .unwrap();
        let vm = load_vm(root.path(), root.path(), config_path).unwrap();
        let host = QemuPlanContext {
            qemu_binary: "qemu-system-x86_64".to_string(),
            host_os: "linux".to_string(),
            accelerator: "tcg".to_string(),
            cpu_cores: 2,
            ram: "4G".to_string(),
            virtio_vga_gl: false,
            usb_redirection: false,
            smartcard: false,
            audio_driver: None,
            smbd: false,
            username: "bad,user=tag".to_string(),
            bridge_helper: None,
            virtiofsd: None,
            virtiofs_device: false,
            ssh_port: None,
            spice_port: None,
        };
        let mut args = Vec::new();
        add_share_args(&mut args, &vm, &host);
        assert!(args.iter().all(|arg| !arg.contains("bad,user=tag")));
        assert!(args.iter().any(|arg| arg.contains("tag=Public-badusertag")));
    }

    #[test]
    fn usb_audio_selects_xhci_controller() {
        let root = tempdir().unwrap();
        let config_path = root.path().join("audio.conf");
        fs::write(
            &config_path,
            "sound_card=usb-audio\nusb_controller=none\nboot=legacy\ndisplay=none\nnetwork=none\n",
        )
        .unwrap();
        let vm = load_vm(root.path(), root.path(), config_path).unwrap();
        assert_eq!(vm.config.usb_controller, "xhci");
    }

    #[test]
    fn plan_reports_and_uses_the_ssh_bind_address() {
        let root = tempdir().unwrap();
        let config_path = root.path().join("remote.conf");
        let disk = root.path().join("disk.qcow2");
        fs::write(&disk, []).unwrap();
        fs::write(
            &config_path,
            "boot=legacy\ndisk_img=disk.qcow2\ndisplay=none\nssh_access=remote\npublic_dir=none\n",
        )
        .unwrap();
        let vm = load_vm(root.path(), root.path(), config_path).unwrap();
        let host = QemuPlanContext {
            qemu_binary: "qemu-system-x86_64".to_string(),
            host_os: "linux".to_string(),
            accelerator: "tcg".to_string(),
            cpu_cores: 2,
            ram: "4G".to_string(),
            virtio_vga_gl: false,
            usb_redirection: false,
            smartcard: false,
            smbd: false,
            audio_driver: None,
            username: "tester".to_string(),
            bridge_helper: None,
            virtiofsd: None,
            virtiofs_device: false,
            ssh_port: Some(22444),
            spice_port: Some(5930),
        };

        let plan = build_plan(&vm, &host, false).unwrap();
        assert_eq!(plan.ssh_host.as_deref(), Some("0.0.0.0"));
        assert_eq!(plan.spice_host.as_deref(), Some("127.0.0.1"));
        assert!(
            plan.args
                .iter()
                .any(|arg| arg.contains("hostfwd=tcp:0.0.0.0:22444-:22"))
        );
        assert!(
            plan.args
                .windows(2)
                .any(|args| args == ["-accel", "tcg,tb-size=256,thread=multi"])
        );
    }

    #[test]
    fn user_network_is_not_treated_as_a_bridge() {
        let root = tempdir().unwrap();
        let config_path = root.path().join("user-network.conf");
        fs::write(
            &config_path,
            "boot=legacy\ndisplay=none\nnetwork=user\npublic_dir=none\n",
        )
        .unwrap();
        let vm = load_vm(root.path(), root.path(), config_path).unwrap();
        let host = QemuPlanContext {
            qemu_binary: "qemu-system-x86_64".to_string(),
            host_os: "linux".to_string(),
            accelerator: "tcg".to_string(),
            cpu_cores: 2,
            ram: "4G".to_string(),
            virtio_vga_gl: false,
            usb_redirection: false,
            smartcard: false,
            smbd: false,
            audio_driver: None,
            username: "tester".to_string(),
            bridge_helper: None,
            virtiofsd: None,
            virtiofs_device: false,
            ssh_port: Some(22444),
            spice_port: Some(5930),
        };
        let plan = build_plan(&vm, &host, false).unwrap();
        assert!(uses_user_network(&vm.config));
        assert_eq!(configured_bridge(&vm.config), None);
        assert_eq!(plan.ssh_port, Some(22444));
        assert!(
            plan.args
                .windows(2)
                .any(|args| { args[0] == "-netdev" && args[1].starts_with("user,id=nic,") })
        );
        assert!(!plan.args.iter().any(|arg| arg == "bridge,br=user"));
    }

    #[test]
    fn passt_network_scopes_forwarded_ports() {
        let root = tempdir().unwrap();
        let config_path = root.path().join("passt-network.conf");
        fs::write(
            &config_path,
            "boot=legacy\ndisplay=none\nnetwork=passt\nssh_access=remote\nport_forwards=(\"8080:80\" \"8443:443\")\npublic_dir=none\n",
        )
        .unwrap();
        let vm = load_vm(root.path(), root.path(), config_path).unwrap();
        let host = QemuPlanContext {
            qemu_binary: "qemu-system-x86_64".to_string(),
            host_os: "linux".to_string(),
            accelerator: "tcg".to_string(),
            cpu_cores: 2,
            ram: "4G".to_string(),
            virtio_vga_gl: false,
            usb_redirection: false,
            smartcard: false,
            smbd: false,
            audio_driver: None,
            username: "tester".to_string(),
            bridge_helper: None,
            virtiofsd: None,
            virtiofs_device: false,
            ssh_port: Some(22444),
            spice_port: Some(5930),
        };

        let plan = build_plan(&vm, &host, false).unwrap();

        assert!(uses_passt_network(&vm.config));
        assert_eq!(configured_bridge(&vm.config), None);
        assert_eq!(plan.ssh_port, Some(22444));
        assert!(plan.args.windows(2).any(|args| {
            args[0] == "-netdev"
                && args[1]
                    == "passt,id=nic,tcp-ports=none,udp-ports=none,param=--tcp-ports=0.0.0.0/22444:22,param=--tcp-ports=127.0.0.1/8080:80,,8443:443,param=--udp-ports=127.0.0.1/8080:80,,8443:443"
        }));
    }

    #[test]
    fn bridge_plan_includes_the_detected_qemu_helper() {
        let root = tempdir().unwrap();
        let config_path = root.path().join("bridge.conf");
        let disk = root.path().join("disk.qcow2");
        fs::write(&disk, []).unwrap();
        fs::write(
            &config_path,
            "boot=legacy\ndisk_img=disk.qcow2\nnetwork=br0\ndisplay=none\npublic_dir=none\n",
        )
        .unwrap();
        let vm = load_vm(root.path(), root.path(), config_path).unwrap();
        let host = QemuPlanContext {
            qemu_binary: "qemu-system-x86_64".to_string(),
            host_os: "linux".to_string(),
            accelerator: "tcg".to_string(),
            cpu_cores: 2,
            ram: "4G".to_string(),
            virtio_vga_gl: false,
            usb_redirection: false,
            smartcard: false,
            smbd: false,
            audio_driver: None,
            username: "tester".to_string(),
            bridge_helper: Some("/usr/lib/qemu/qemu-bridge-helper".to_string()),
            virtiofsd: None,
            virtiofs_device: false,
            ssh_port: None,
            spice_port: Some(5930),
        };

        let plan = build_plan(&vm, &host, false).unwrap();
        assert_eq!(configured_bridge(&vm.config), Some("br0"));
        assert!(plan.args.iter().any(|arg| {
            arg == "bridge,br=br0,helper=/usr/lib/qemu/qemu-bridge-helper,model=virtio-net-pci"
        }));
        let mut no_helper = host.clone();
        no_helper.bridge_helper = None;
        assert!(
            build_plan(&vm, &no_helper, false)
                .unwrap_err()
                .to_string()
                .contains("bridged networking requires qemu-bridge-helper")
        );
    }

    #[test]
    fn gtk_clipboard_is_explicit_in_the_display_plan() {
        let root = tempdir().unwrap();
        let config_path = root.path().join("clipboard.conf");
        fs::write(
            &config_path,
            "boot=legacy\ndisplay=gtk\nclipboard=on\nnetwork=none\npublic_dir=none\n",
        )
        .unwrap();
        let vm = load_vm(root.path(), root.path(), config_path).unwrap();
        let host = QemuPlanContext {
            qemu_binary: "qemu-system-x86_64".to_string(),
            host_os: "linux".to_string(),
            accelerator: "tcg".to_string(),
            cpu_cores: 2,
            ram: "4G".to_string(),
            virtio_vga_gl: false,
            usb_redirection: false,
            smartcard: false,
            smbd: false,
            audio_driver: None,
            username: "tester".to_string(),
            bridge_helper: None,
            virtiofsd: None,
            virtiofs_device: false,
            ssh_port: None,
            spice_port: Some(5930),
        };

        match build_plan(&vm, &host, false) {
            Ok(plan) => {
                assert!(plan.args.iter().any(|arg| arg.contains("clipboard=on")));
                assert!(plan.args.iter().any(|arg| arg.contains("qemu-vdagent")));
            }
            Err(error) => assert!(
                error.to_string().contains("QEMU 11.1.0")
                    || error.to_string().contains("qemu-vdagent")
            ),
        }
    }

    #[test]
    fn arm_windows_uses_virtio_graphics() {
        let root = tempdir().unwrap();
        let config_path = root.path().join("arm-windows.conf");
        let disk = root.path().join("disk.qcow2");
        fs::write(&disk, []).unwrap();
        fs::write(
            &config_path,
            "arch=aarch64\nguest_os=windows\nboot=legacy\ndisk_img=disk.qcow2\ndisplay=gtk\nnetwork=none\npublic_dir=none\n",
        )
        .unwrap();
        let vm = load_vm(root.path(), root.path(), config_path).unwrap();
        let host = QemuPlanContext {
            qemu_binary: "qemu-system-aarch64".to_string(),
            host_os: "linux".to_string(),
            accelerator: "tcg".to_string(),
            cpu_cores: 2,
            ram: "4G".to_string(),
            virtio_vga_gl: false,
            usb_redirection: false,
            smartcard: false,
            smbd: false,
            audio_driver: None,
            username: "tester".to_string(),
            bridge_helper: None,
            virtiofsd: None,
            virtiofs_device: false,
            ssh_port: None,
            spice_port: None,
        };

        let plan = build_plan(&vm, &host, false).unwrap();
        assert!(plan.args.iter().any(|arg| arg == "virtio-gpu-pci"));
        assert!(!plan.args.iter().any(|arg| arg == "qxl-vga"));
    }

    #[test]
    fn firmware_pair_selection_skips_incomplete_entries() {
        let root = tempdir().unwrap();
        let first_code = root.path().join("first-code");
        let first_vars = root.path().join("first-vars");
        let second_code = root.path().join("second-code");
        let second_vars = root.path().join("second-vars");
        fs::write(&first_code, []).unwrap();
        fs::write(&second_code, []).unwrap();
        fs::write(&second_vars, []).unwrap();
        let pairs = [
            (first_code.to_str().unwrap(), first_vars.to_str().unwrap()),
            (second_code.to_str().unwrap(), second_vars.to_str().unwrap()),
        ];

        assert_eq!(first_complete_pair(&pairs).unwrap().0, second_code);
    }

    #[test]
    fn plan_attaches_windows_install_media_in_stable_order() {
        let root = tempdir().unwrap();
        for name in [
            "disk.qcow2",
            "windows.iso",
            "virtio-win.iso",
            "unattended.iso",
        ] {
            fs::write(root.path().join(name), []).unwrap();
        }
        let config_path = root.path().join("windows.conf");
        fs::write(
            &config_path,
            "boot=legacy\ndisk_img=disk.qcow2\niso=windows.iso\nfixed_iso=virtio-win.iso\nunattended_iso=unattended.iso\ndisplay=none\nnetwork=none\npublic_dir=none\nguest_agent=false\n",
        )
        .unwrap();
        let vm = load_vm(root.path(), root.path(), config_path).unwrap();
        let host = QemuPlanContext {
            qemu_binary: "qemu-system-x86_64".to_string(),
            host_os: "linux".to_string(),
            accelerator: "tcg".to_string(),
            cpu_cores: 2,
            ram: "4G".to_string(),
            virtio_vga_gl: false,
            usb_redirection: false,
            smartcard: false,
            smbd: false,
            audio_driver: None,
            username: "tester".to_string(),
            bridge_helper: None,
            virtiofsd: None,
            virtiofs_device: false,
            ssh_port: None,
            spice_port: Some(5930),
        };

        let plan = build_plan(&vm, &host, false).unwrap();
        assert!(plan.args.windows(2).any(|args| {
            args[0] == "-drive"
                && args[1].contains("media=cdrom,index=2,readonly=on")
                && args[1].contains("unattended.iso")
        }));
    }

    #[test]
    fn arm_plan_avoids_x86_machine_flags_and_wires_tpm() {
        let root = tempdir().unwrap();
        let config_path = root.path().join("arm.conf");
        let disk = root.path().join("disk.qcow2");
        fs::write(&disk, []).unwrap();
        fs::write(
            &config_path,
            "arch=aarch64\nboot=legacy\ndisk_img=disk.qcow2\ndisplay=none\nnetwork=none\npublic_dir=none\ntpm=on\n",
        )
        .unwrap();
        let vm = load_vm(root.path(), root.path(), config_path).unwrap();
        let host = QemuPlanContext {
            qemu_binary: "qemu-system-aarch64".to_string(),
            host_os: "linux".to_string(),
            accelerator: "tcg".to_string(),
            cpu_cores: 2,
            ram: "4G".to_string(),
            virtio_vga_gl: false,
            usb_redirection: false,
            smartcard: false,
            smbd: false,
            audio_driver: None,
            username: "tester".to_string(),
            bridge_helper: None,
            virtiofsd: None,
            virtiofs_device: false,
            ssh_port: None,
            spice_port: Some(5930),
        };

        let plan = build_plan(&vm, &host, false).unwrap();
        let machine = plan
            .args
            .windows(2)
            .find(|args| args[0] == "-machine")
            .map(|args| args[1].as_str())
            .unwrap();
        assert!(machine.starts_with("virt,"));
        assert!(!machine.contains("smm=") && !machine.contains("vmport="));
        assert!(
            plan.args
                .windows(2)
                .any(|args| args == ["-device", "ramfb"])
        );
        assert!(plan.args.iter().any(|arg| arg.contains("tpm-tis-device")));
    }

    #[test]
    fn efi_plan_can_preview_before_variables_are_created() {
        if first_existing(&[
            "/usr/share/edk2/x64/OVMF_CODE.4m.fd",
            "/usr/share/OVMF/OVMF_CODE_4M.fd",
            "/usr/share/OVMF/OVMF_CODE.fd",
        ])
        .is_none()
        {
            return;
        }
        let root = tempdir().unwrap();
        let config_path = root.path().join("efi.conf");
        let disk = root.path().join("disk.qcow2");
        fs::write(&disk, []).unwrap();
        fs::write(
            &config_path,
            "boot=efi\ndisk_img=disk.qcow2\ndisplay=none\nnetwork=none\npublic_dir=none\n",
        )
        .unwrap();
        let vm = load_vm(root.path(), root.path(), config_path).unwrap();
        let host = QemuPlanContext {
            qemu_binary: "qemu-system-x86_64".to_string(),
            host_os: "linux".to_string(),
            accelerator: "tcg".to_string(),
            cpu_cores: 2,
            ram: "4G".to_string(),
            virtio_vga_gl: false,
            usb_redirection: false,
            smartcard: false,
            smbd: false,
            audio_driver: None,
            username: "tester".to_string(),
            bridge_helper: None,
            virtiofsd: None,
            virtiofs_device: false,
            ssh_port: None,
            spice_port: Some(5930),
        };

        let plan = build_plan(&vm, &host, false).unwrap();
        assert!(plan.args.iter().any(|arg| arg.ends_with("OVMF_VARS.fd")));
        assert!(!root.path().join("OVMF_VARS.fd").exists());
    }

    #[test]
    fn disk_argument_validation_rejects_option_injection() {
        assert!(validate_disk_size("20G").is_ok());
        assert!(validate_disk_size("+4G").is_ok());
        assert!(validate_disk_size("--shrink").is_err());
        assert!(validate_disk_size("20 G").is_err());
        assert!(validate_disk_format("qcow2").is_ok());
        assert!(validate_disk_format("-raw").is_err());
        assert!(validate_disk_format("raw image").is_err());
    }

    #[test]
    fn qemu_help_parsers_extract_display_devices_and_cpu_models() {
        let display = qemu_display_backends_from_text(
            "Available display backend types:\nnone\ngtk\nspice-app\n\nSome display backends support options",
        );
        assert_eq!(display, ["none", "gtk", "spice-app"]);

        let devices =
            qemu_quoted_names("name \"virtio-vga-gl\", bus PCI\nname \"usb-redir\", bus usb-bus");
        assert_eq!(devices, ["virtio-vga-gl", "usb-redir"]);

        let cpus = "Available CPUs:\n  host                  host CPU\n  max                   all features\n";
        assert!(qemu_supports_cpu_in_text(cpus, "host"));
        assert!(!qemu_supports_cpu_in_text(cpus, "unknown"));

        let accelerators =
            qemu_accelerators_from_text("Accelerators supported in QEMU binary:\ntcg\nkvm\n\n");
        assert_eq!(accelerators, ["tcg", "kvm"]);

        let netdevs = qemu_netdev_backends_from_text(
            "Available netdev backend types:\nsocket\npasst\nuser\n\n",
        );
        assert_eq!(netdevs, ["socket", "passt", "user"]);
        assert_eq!(
            qemu_netdev_help_args("qemu-system-aarch64"),
            ["-machine", "virt", "-netdev", "help"]
        );
    }

    #[test]
    fn unavailable_qemu_capabilities_are_explained() {
        let report = qemu_capability_report("vmctl-qemu-does-not-exist");
        assert_eq!(report["available"], false);
        assert!(report["probe_error"].as_str().is_some());
    }

    #[test]
    fn cpu_probe_rejects_unknown_features_when_qemu_is_installed() {
        if !command_available("qemu-system-x86_64") {
            return;
        }
        let error = validate_cpu_spec("qemu-system-x86_64", "qemu64,+vmctl-unknown-feature", "tcg")
            .unwrap_err();
        assert!(error.to_string().contains("rejected CPU specification"));
    }

    #[test]
    fn qmp_probe_requires_a_valid_greeting() {
        assert!(
            read_qmp_greeting(io::Cursor::new(
                b"{\"QMP\":{\"version\":{},\"capabilities\":[]}}\r\n" as &[u8],
            ))
            .unwrap()
        );
        assert!(!read_qmp_greeting(io::Cursor::new(b"{\"QMP\":null}\n" as &[u8])).unwrap());
        assert!(!read_qmp_greeting(io::Cursor::new(b"{\"not_qmp\":{}}\n" as &[u8])).unwrap());
        assert!(read_qmp_greeting(io::Cursor::new(b"not-json\n" as &[u8])).is_err());
    }

    #[test]
    fn guest_exec_output_is_decoded_without_discarding_raw_data() {
        let result = normalize_guest_exec_result(json!({
            "exited": true,
            "exitcode": 0,
            "out-data": "SGVsbG8h",
            "err-data": "d29w",
        }))
        .unwrap();
        assert_eq!(result["stdout"], "Hello!");
        assert_eq!(result["stderr"], "wop");
        assert_eq!(result["out-data"], "SGVsbG8h");
    }

    #[test]
    fn base64_decoder_rejects_invalid_data() {
        assert!(decode_base64("SGVsbG8").is_ok());
        assert!(decode_base64("SGVsbG8$").is_err());
        assert!(decode_base64("A===").is_err());
        assert!(decode_base64("AA=").is_err());
        assert!(decode_base64("AB==").is_err());
    }

    #[test]
    fn guest_agent_commands_synchronize_each_connection() {
        #[cfg(unix)]
        {
            let root = tempdir().unwrap();
            let config_path = root.path().join("guest-agent.conf");
            fs::write(&config_path, "guest_agent=true\n").unwrap();
            let vm = load_vm(root.path(), root.path(), config_path).unwrap();
            fs::create_dir_all(&vm.paths.state_dir).unwrap();
            let listener = UnixListener::bind(vm.paths.agent_socket()).unwrap();
            let server = thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut marker = [0_u8; 1];
                reader.read_exact(&mut marker).unwrap();
                assert_eq!(marker, [0xff]);
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let request: Value = serde_json::from_str(line.trim()).unwrap();
                assert_eq!(request["execute"], "guest-sync-delimited");
                let id = request["arguments"]["id"].as_i64().unwrap();
                stream.write_all(&[0xff]).unwrap();
                stream
                    .write_all(format!("{{\"return\":{}}}\n", id + 1).as_bytes())
                    .unwrap();
                stream.write_all(&[0xff]).unwrap();
                stream
                    .write_all(format!("{{\"return\":{id}}}\n").as_bytes())
                    .unwrap();
                line.clear();
                reader.read_line(&mut line).unwrap();
                let request: Value = serde_json::from_str(line.trim()).unwrap();
                assert_eq!(request["execute"], "guest-ping");
                stream.write_all(b"{\"return\":{}}\n").unwrap();
            });
            assert_eq!(guest_command(&vm, "guest-ping", None).unwrap(), json!({}));
            server.join().unwrap();
        }
    }

    #[test]
    fn guest_agent_response_limit_is_enforced_while_reading() {
        let mut reader = BufReader::new(io::Cursor::new(b"12345\n"));
        let error = read_bounded_line(&mut reader, 4).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }
}
