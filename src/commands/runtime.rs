use super::*;

pub(super) fn write_pid(vm: &Vm, pid: i32) -> Result<()> {
    let path = vm.paths.pid_file();
    let identity = qemu::process_identity(pid).map_or_else(
        || format!("{pid}\n"),
        |identity| format!("{pid} {identity}\n"),
    );
    qemu::write_atomic_file(&path, identity.as_bytes())
}

pub(super) fn apply_cpu_pinning(pid: i32, pinning: &str) -> Result<()> {
    let output = ProcessCommand::new("taskset")
        .args(["-acp", pinning, &pid.to_string()])
        .output()
        .map_err(|error| Error::command_unavailable("taskset", error))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(Error::command_failed_status("taskset", output.status))
    }
}

pub(super) fn check_tsc_stability(vm: &Vm, quiet: bool) -> Result<()> {
    let clocksource =
        fs::read_to_string("/sys/devices/system/clocksource/clocksource0/current_clocksource")
            .unwrap_or_default();
    let cmdline = fs::read_to_string("/proc/cmdline").unwrap_or_default();
    let vendor = fs::read_to_string("/proc/cpuinfo")
        .ok()
        .filter(|contents| contents.contains("AuthenticAMD"));
    if !tsc_warning_needed(
        env::consts::OS,
        vendor.is_some(),
        &vm.config.guest_os,
        vm.config.macos_release.as_deref(),
        clocksource.trim(),
        &cmdline,
    ) {
        return Ok(());
    }
    if vm.config.ignore_tsc_warning {
        if quiet {
            return Ok(());
        }
        eprintln!(
            "vmctl: warning: macOS {} may freeze with an unstable TSC (clocksource: {})",
            vm.config
                .macos_release
                .as_deref()
                .unwrap_or("newer release"),
            clocksource.trim()
        );
        return Ok(());
    }
    Err(Error::message(format!(
        "macOS {} may freeze with an unstable AMD TSC (clocksource: {}); fix the host or retry with --ignore-tsc-warning",
        vm.config
            .macos_release
            .as_deref()
            .unwrap_or("newer release"),
        clocksource.trim()
    )))
}

pub(super) fn tsc_warning_needed(
    host_os: &str,
    amd_cpu: bool,
    guest_os: &str,
    release: Option<&str>,
    clocksource: &str,
    cmdline: &str,
) -> bool {
    host_os == "linux"
        && amd_cpu
        && guest_os == "macos"
        && matches!(release, Some("ventura" | "sonoma" | "sequoia" | "tahoe"))
        && !clocksource.is_empty()
        && clocksource != "tsc"
        && !cmdline.split_whitespace().any(|arg| arg == "tsc=reliable")
}

pub(super) fn validate_cpu_pinning(value: &str) -> Result<()> {
    if value.is_empty()
        || value
            .split(',')
            .any(|part| part.is_empty() || part.parse::<u32>().is_err())
    {
        return Err(Error::message(
            "cpu pinning must be a comma-separated list of host CPU IDs",
        ));
    }
    Ok(())
}

pub(super) fn validate_cpu_pinning_for_host(
    value: &str,
    host_os: &str,
    cpu_cores: u32,
) -> Result<()> {
    validate_cpu_pinning(value)?;
    if host_os != "linux" {
        return Err(Error::message(
            "cpu pinning is only supported on Linux hosts",
        ));
    }
    let count = value.split(',').count();
    if count != cpu_cores as usize {
        return Err(Error::message(format!(
            "cpu pinning lists {count} host CPUs but the VM has {cpu_cores} vCPUs"
        )));
    }
    let mut seen = Vec::new();
    for part in value.split(',') {
        let id = part
            .parse::<u32>()
            .expect("validate_cpu_pinning checked CPU IDs");
        if seen.contains(&id) {
            return Err(Error::message(format!(
                "cpu pinning repeats host CPU {id}; use distinct CPU IDs"
            )));
        }
        seen.push(id);
        if !host_cpu_id_available(id) {
            return Err(Error::message(format!(
                "cpu pinning references host CPU {id}, but that CPU is not online or available"
            )));
        }
    }
    Ok(())
}

pub(super) fn host_cpu_id_available(id: u32) -> bool {
    if let Some(spec) = process_allowed_cpu_spec() {
        return spec.split(',').any(|range| {
            let mut bounds = range.trim().split('-');
            let Some(start) = bounds.next().and_then(|value| value.parse::<u32>().ok()) else {
                return false;
            };
            let end = bounds
                .next()
                .and_then(|value| value.parse::<u32>().ok())
                .unwrap_or(start);
            start <= id && id <= end
        });
    }
    std::thread::available_parallelism()
        .map(|value| id < value.get() as u32)
        .unwrap_or(false)
}

pub(super) fn process_allowed_cpu_spec() -> Option<String> {
    fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|contents| {
            contents.lines().find_map(|line| {
                line.strip_prefix("Cpus_allowed_list:")
                    .map(|value| value.trim().to_string())
            })
        })
        .or_else(|| fs::read_to_string("/sys/devices/system/cpu/online").ok())
}

pub(super) fn command_available(command: &str) -> bool {
    ProcessCommand::new(command)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

pub(super) fn command_version(command: &str) -> Option<String> {
    let output = ProcessCommand::new(command)
        .arg("--version")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let version = text.lines().next()?.trim();
    (!version.is_empty()).then(|| version.to_string())
}

pub(super) fn desktop_quote(path: &Path) -> String {
    quote_desktop_value(path.display().to_string())
}

pub(super) fn desktop_exec_quote(path: &Path) -> String {
    quote_desktop_value(path.display().to_string().replace('%', "%%"))
}

fn quote_desktop_value(value: String) -> String {
    if value
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || "_./:-".contains(character))
    {
        value
    } else {
        format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
    }
}

pub(super) fn launch_viewer(vm: &Vm, plan: &qemu::QemuPlan, quiet: bool) -> bool {
    if !matches!(vm.config.display.as_str(), "none" | "spice" | "spice-app")
        || vm.config.viewer == "none"
    {
        return false;
    }
    match start_viewer(vm, &vm.config.viewer, plan.spice_port) {
        Ok(()) => true,
        Err(error) => {
            if !quiet {
                eprintln!("vmctl: {error}");
            }
            false
        }
    }
}

pub(super) fn start_viewer(vm: &Vm, viewer: &str, port: Option<u16>) -> Result<()> {
    let mut command = ProcessCommand::new(viewer);
    if let Some(port) = port {
        let host = connect_host(spice_address(&vm.config));
        if viewer == "spicy" {
            if host == "127.0.0.1" {
                command.args(["--port", &port.to_string()]);
            } else {
                command.args(["--host", host, "--port", &port.to_string()]);
            }
        } else {
            command.arg(spice_tcp_uri(host, port));
        }
    } else {
        let uri = format!("spice+unix://{}", vm.paths.spice_socket().display());
        if viewer == "spicy" {
            command.arg(format!("--uri={uri}"));
        } else {
            command.arg(uri);
        }
    }
    command
        .arg("--title")
        .arg(&vm.config.name)
        .args(&vm.config.viewer_extra_args)
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command.spawn().map(|_| ()).map_err(|error| {
        Error::message(format!("SPICE viewer `{viewer}` was not started: {error}"))
    })
}

pub(super) fn spice_tcp_uri(host: &str, port: u16) -> String {
    if host.parse::<std::net::Ipv6Addr>().is_ok() {
        format!("spice://[{host}]:{port}")
    } else {
        format!("spice://{host}:{port}")
    }
}

pub(super) fn reconnect_viewer(vm: &Vm, quiet: bool) -> bool {
    if vm.config.viewer == "none" {
        return false;
    }
    let mut vm = vm.clone();
    let Ok((saved, listener)) = runtime_listener(&vm.paths.state_dir.join("ports"), "spice") else {
        return false;
    };
    if saved {
        let port = listener.map(|(port, host)| {
            if let Some(host) = host {
                vm.config.access = connect_host(&host).to_string();
            }
            port
        });
        if port.is_none() && !vm.paths.spice_socket().exists() {
            return false;
        }
        return start_viewer(&vm, &vm.config.viewer, port).is_ok();
    }
    let Ok(host) = HostCapabilities::detect(&vm.config) else {
        return false;
    };
    let Ok(plan) = build_plan(&vm, &host, false) else {
        return false;
    };
    launch_viewer(&vm, &plan, quiet)
}

pub(super) type RuntimeEndpoint = (u16, Option<String>);

pub(super) fn runtime_listener(
    path: &Path,
    wanted: &str,
) -> Result<(bool, Option<RuntimeEndpoint>)> {
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok((false, None)),
        Err(error) => return Err(Error::io(path.display(), error)),
    };
    let listener = contents.lines().find_map(|line| {
        let mut fields = line.splitn(3, ',');
        let name = fields.next()?;
        let port = fields.next()?.parse().ok()?;
        (name == wanted).then(|| {
            (
                port,
                fields
                    .next()
                    .filter(|host| !host.is_empty())
                    .map(str::to_string),
            )
        })
    });
    Ok((true, listener))
}

pub(super) fn connect_host(host: &str) -> &str {
    match host {
        "::" => "::1",
        "" | "local" | "remote" | "0.0.0.0" => "127.0.0.1",
        host => host,
    }
}

pub(super) fn effective_ssh_port(vm: &Vm) -> Result<Option<u16>> {
    if matches!(vm.state()?, VmState::Running(_)) {
        let (saved, listener) = runtime_listener(&vm.paths.state_dir.join("ports"), "ssh")?;
        Ok(if saved {
            listener.map(|(port, _)| port)
        } else {
            vm.config.ssh_port
        })
    } else {
        Ok(vm.config.ssh_port)
    }
}

pub(super) fn runtime_ssh_host(vm: &Vm) -> Result<Option<String>> {
    let (saved, listener) = runtime_listener(&vm.paths.state_dir.join("ports"), "ssh")?;
    if !saved {
        return Ok(None);
    }
    Ok(listener.map(|(_, host)| {
        host.as_deref()
            .map(connect_host)
            .unwrap_or_else(|| ssh_connect_host(&vm.config))
            .to_string()
    }))
}

pub(super) fn vm_ssh_options() -> [String; 8] {
    let null_device = if cfg!(windows) { "NUL" } else { "/dev/null" };
    [
        "-o".to_string(),
        format!("UserKnownHostsFile={null_device}"),
        "-o".to_string(),
        format!("GlobalKnownHostsFile={null_device}"),
        "-o".to_string(),
        "StrictHostKeyChecking=no".to_string(),
        "-o".to_string(),
        "LogLevel=ERROR".to_string(),
    ]
}

pub(super) fn ensure_delete_allowed(vm: &Vm, yes: bool) -> Result<()> {
    if matches!(vm.state()?, VmState::Running(_)) {
        return Err(Error::message(format!(
            "{} is running; stop it before deleting data",
            vm.config.name
        )));
    }
    if !yes {
        return Err(Error::message("deletion is irreversible; rerun with --yes"));
    }
    Ok(())
}

pub(super) fn persistent_efi_vars(vm: &Vm) -> Vec<PathBuf> {
    let parent = vm
        .config
        .disk_img
        .parent()
        .unwrap_or_else(|| Path::new("."));
    let vm_vars = parent.join(format!("{}-vars.fd", vm.config.name));
    let data_dir = vm
        .config
        .config_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(&vm.config.name);
    if parent != data_dir {
        return vec![vm_vars];
    }
    vec![
        parent.join("OVMF_VARS.fd"),
        parent.join("OVMF_VARS_4M.fd"),
        parent.join("OVMF_VARS-1024x768.fd"),
        parent.join("OVMF_VARS-1920x1080.fd"),
        vm_vars,
    ]
}

pub(super) fn remove_if_present(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(Error::io(path.display(), error)),
    }
}

pub(super) fn cli_path(path: &Path) -> Result<PathBuf> {
    if path == Path::new("~") {
        return paths::home_dir();
    }
    if let Some(relative) = path.to_str().and_then(|value| value.strip_prefix("~/")) {
        return Ok(paths::home_dir()?.join(relative));
    }
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        env::current_dir()
            .map(|current| current.join(path))
            .map_err(|error| Error::io("current directory", error))
    }
}
