use super::*;

pub fn write_runtime_files(paths: &VmPaths, plan: &QemuPlan) -> Result<()> {
    ensure_state_directory(&paths.state_dir)?;
    let command_path = paths.state_dir.join("qemu.command");
    write_atomic_file(
        &command_path,
        format!("{}\n", shell_join(&plan.binary, &plan.args)).as_bytes(),
    )?;

    let mut ports = String::new();
    if let Some(port) = plan.ssh_port {
        ports.push_str(&format!(
            "ssh,{port},{}\n",
            plan.ssh_host.as_deref().unwrap_or("127.0.0.1")
        ));
    }
    if let Some(port) = plan.spice_port {
        ports.push_str(&format!(
            "spice,{port},{}\n",
            plan.spice_host.as_deref().unwrap_or("127.0.0.1")
        ));
    }
    let ports_path = paths.state_dir.join("ports");
    write_atomic_file(&ports_path, ports.as_bytes())?;

    let ipc_path = paths.ipc_state();
    let ipc = json!({
        "schema_version": 1,
        "qmp": plan.qmp_endpoint.json_value(),
        "guest_agent": plan.agent_endpoint.as_ref().map(IpcEndpoint::json_value),
    });
    write_atomic_file(&ipc_path, format!("{ipc}\n").as_bytes())?;
    Ok(())
}

pub(crate) fn write_atomic_file(path: &Path, contents: &[u8]) -> Result<()> {
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy())
        .unwrap_or_default();
    let mut temporary = None;
    for _ in 0..8 {
        let candidate = path.with_file_name(format!(
            ".{file_name}.vmctl-{}-{}.tmp",
            std::process::id(),
            next_guest_sync_id().unsigned_abs()
        ));
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(file) => {
                temporary = Some((candidate, file));
                break;
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(Error::io(candidate.display(), error)),
        }
    }
    let (temporary, mut file) = temporary.ok_or_else(|| {
        Error::message(format!(
            "cannot allocate a temporary runtime file beside {}",
            path.display()
        ))
    })?;
    if let Err(error) = file.write_all(contents).and_then(|()| file.sync_all()) {
        let _ = fs::remove_file(&temporary);
        return Err(Error::io(temporary.display(), error));
    }
    drop(file);
    if let Err(error) = replace_file(&temporary, path) {
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }
    Ok(())
}

#[cfg(not(windows))]
pub(crate) fn replace_file(temporary: &Path, destination: &Path) -> Result<()> {
    fs::rename(temporary, destination).map_err(|error| Error::io(destination.display(), error))
}

#[cfg(windows)]
pub(crate) fn replace_file(temporary: &Path, destination: &Path) -> Result<()> {
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

pub(crate) fn qmp_live_resources(paths: &VmPaths) -> Result<Value> {
    let endpoint = qmp_endpoint_for_paths(paths)?;
    let deadline = qmp_deadline()?;
    let mut stream = connect_endpoint_retry(&endpoint, "QMP")?;
    stream
        .set_read_timeout(Some(QMP_TIMEOUT))
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
        "vmctl-live-capabilities",
        None,
        deadline,
    )?;
    let cpus = execute_qmp(
        &mut stream,
        &mut reader,
        "query-cpus-fast",
        "vmctl-live-cpus",
        None,
        deadline,
    )?;
    let memory = execute_qmp(
        &mut stream,
        &mut reader,
        "query-memory-size-summary",
        "vmctl-live-memory",
        None,
        deadline,
    )?;
    let block = execute_qmp(
        &mut stream,
        &mut reader,
        "query-block",
        "vmctl-live-block",
        None,
        deadline,
    )?;
    Ok(json!({"cpus": cpus, "memory": memory, "block": block}))
}

pub(crate) fn ipc_report(paths: &VmPaths, _guest_agent: bool) -> Result<Value> {
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
            "guest_agent": _guest_agent.then(|| IpcEndpoint::Unix(paths.agent_socket()).json_value()),
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
    for (host, port) in &plan.forwarded_ports {
        tcp_endpoints.push(("forwarded TCP", host.clone(), *port));
    }

    let mut seen = Vec::new();
    let mut listeners = Vec::new();
    for (name, host, port) in tcp_endpoints {
        let key = format!("{host}:{port}");
        if seen.iter().any(|seen_key| seen_key == &key) {
            return Err(Error::message(format!(
                "{name} endpoint {key} conflicts with another configured listener; choose unique ports"
            )));
        }
        let listener = TcpListener::bind((host.as_str(), port)).map_err(|error| {
            Error::message(format!(
                "{name} endpoint {key} is unavailable: {error}; choose another port or stop the conflicting service"
            ))
        })?;
        listeners.push(listener);
        seen.push(key);
    }
    let mut udp_sockets = Vec::new();
    for (host, port) in &plan.forwarded_ports {
        let key = format!("{host}:{port}");
        let socket = UdpSocket::bind((host.as_str(), *port)).map_err(|error| {
            Error::message(format!(
                "forwarded UDP endpoint {key} is unavailable: {error}; choose another port or stop the conflicting service"
            ))
        })?;
        udp_sockets.push(socket);
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

pub(super) fn revalidate_vm_process(
    vm: &Vm,
    pid: i32,
    expected_identity: Option<&str>,
) -> Result<bool> {
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
pub(super) fn signal_vm_process(
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
pub(super) fn signal_vm_process(
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

pub(super) fn terminate_pid(pid: i32, force: bool) -> Result<std::process::ExitStatus> {
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
    ensure_state_directory(&vm.paths.state_dir)?;
    let log_path = vm.paths.state_dir.join("swtpm.log");
    let log =
        create_truncated_file(&log_path).map_err(|error| Error::io(log_path.display(), error))?;
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
    if let Err(error) = write_atomic_file(
        &vm.paths.tpm_pid_file(),
        process_record(child.id() as i32).as_bytes(),
    ) {
        let _ = child.kill();
        let _ = child.wait();
        return Err(error);
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
    let log = match create_truncated_file(&log_path) {
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
    if let Err(error) = write_atomic_file(
        &vm.paths.virtiofs_pid_file(),
        process_record(child.id() as i32).as_bytes(),
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
