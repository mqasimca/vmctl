use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IpcEndpoint {
    #[cfg(unix)]
    Unix(PathBuf),
    Tcp(SocketAddr),
    #[cfg(windows)]
    Pipe(PathBuf),
}

impl IpcEndpoint {
    pub(super) fn tcp_address(&self) -> Option<SocketAddr> {
        match self {
            Self::Tcp(address) => Some(*address),
            #[cfg(unix)]
            Self::Unix(_) => None,
            #[cfg(windows)]
            Self::Pipe(_) => None,
        }
    }

    pub(super) fn qmp_argument(&self) -> String {
        match self {
            #[cfg(unix)]
            Self::Unix(path) => format!("unix:{},server=on,wait=off", qemu_path(path)),
            Self::Tcp(address) => format!("tcp:{address},server=on,wait=off"),
            #[cfg(windows)]
            Self::Pipe(path) => format!("pipe:{}", pipe_name(path)),
        }
    }

    pub(super) fn add_qmp_args(&self, args: &mut Vec<String>) {
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

    pub(super) fn guest_agent_argument(&self) -> String {
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

    pub(super) fn connect(&self, timeout: Duration) -> io::Result<IpcStream> {
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

    pub(super) fn display(&self) -> String {
        match self {
            #[cfg(unix)]
            Self::Unix(path) => path.display().to_string(),
            Self::Tcp(address) => format!("tcp://{address}"),
            #[cfg(windows)]
            Self::Pipe(path) => path.display().to_string(),
        }
    }

    pub(super) fn json_value(&self) -> Value {
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

    pub(super) fn from_json(value: &Value) -> Result<Self> {
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
pub(super) enum IpcStream {
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
    pub(super) fn try_clone(&self) -> io::Result<Self> {
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

    pub(super) fn set_read_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
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

    pub(super) fn set_write_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
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
