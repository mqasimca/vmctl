use super::*;

pub(super) fn detect_audio_driver(host_os: &str) -> Option<String> {
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

pub(crate) fn default_cpu_cores() -> u32 {
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

pub(super) fn default_ram() -> String {
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
                .map(|bytes| bytes / 1024)
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

pub(super) fn find_free_port(start: u16, reserved: &[u16]) -> Result<u16> {
    let end = start.saturating_add(9);
    for port in start..=end {
        if reserved.contains(&port) {
            continue;
        }
        if TcpListener::bind(("127.0.0.1", port)).is_ok() {
            return Ok(port);
        }
    }
    Err(Error::message(format!(
        "no free port found in {start}-{end}",
    )))
}

pub(super) fn ensure_command(command: &str) -> Result<()> {
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
