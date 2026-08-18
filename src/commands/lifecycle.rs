use super::*;

pub(super) fn start_vm(
    dirs: &Dirs,
    name: &str,
    options: &LaunchOptions,
    wait: Option<StartWait>,
    wait_timeout: u64,
    output: OutputFormat,
) -> Result<()> {
    let vm = load_effective_vm(dirs, name, options)?;
    let _operation_lock = acquire_vm_lock(&vm.paths)?;
    let wait_for_ssh =
        matches!(wait, Some(StartWait::Ssh)).then_some(Duration::from_secs(wait_timeout));
    start_vm_loaded(&vm, output, wait_for_ssh)
}

pub(super) fn ssh_vm(dirs: &Dirs, name: &str, user: Option<&str>) -> Result<()> {
    let vm = find(&dirs.vm_dir, &dirs.state_root, name)?;
    if !matches!(vm.state()?, VmState::Running(_)) {
        return Err(Error::message(format!(
            "{} is not running; start it before opening SSH",
            vm.config.name
        )));
    }
    let port = active_ssh_port(&vm)?;
    let host = ssh_connect_host(&vm.config);
    let mut command = ProcessCommand::new("ssh");
    command
        .args(vm_ssh_options())
        .arg("-p")
        .arg(port.to_string());
    if let Some(user) = user.or(vm.config.ssh_user.as_deref()) {
        command.arg("-l").arg(user);
    }
    let status = command
        .arg(host)
        .status()
        .map_err(|error| Error::command_unavailable("ssh", error))?;
    if status.success() {
        Ok(())
    } else {
        Err(Error::command_failed_status("ssh", status))
    }
}

pub(super) fn view_vm(
    dirs: &Dirs,
    name: &str,
    viewer: Option<&str>,
    output: OutputFormat,
) -> Result<()> {
    let vm = find(&dirs.vm_dir, &dirs.state_root, name)?;
    if !matches!(vm.state()?, VmState::Running(_)) {
        return Err(Error::message(format!(
            "{} is not running; start it before opening its display",
            vm.config.name
        )));
    }
    let port = runtime_port(&vm.paths.state_dir.join("ports"), "spice");
    if port.is_none() && !vm.paths.spice_socket().exists() {
        return Err(Error::message(format!(
            "{} has no active SPICE display; restart it with --display none, --display spice, or --display spice-app",
            vm.config.name
        )));
    }
    let viewer =
        viewer
            .filter(|viewer| !viewer.is_empty())
            .unwrap_or(if vm.config.viewer == "none" {
                "remote-viewer"
            } else {
                &vm.config.viewer
            });
    start_viewer(&vm, viewer, port)?;
    let endpoint = port.map_or_else(
        || format!("spice+unix://{}", vm.paths.spice_socket().display()),
        |port| format!("spice://{}:{port}", spice_address(&vm.config)),
    );
    if output == OutputFormat::Json {
        print_json_success(
            json!({ "name": vm.config.name, "viewer": viewer, "endpoint": endpoint }),
        );
    } else {
        println!("Opened {viewer} for {}", vm.config.name);
    }
    Ok(())
}

pub(super) fn active_ssh_port(vm: &Vm) -> Result<u16> {
    runtime_port(&vm.paths.state_dir.join("ports"), "ssh")
        .or(vm.config.ssh_port)
        .ok_or_else(|| {
            Error::message(format!(
                "{} has no active SSH forward; use network=user or network=passt and restart it",
                vm.config.name
            ))
        })
}

pub(super) fn ssh_connect_host(config: &VmConfig) -> &str {
    match config.ssh_access.as_str() {
        "" | "local" | "remote" => "127.0.0.1",
        host => host,
    }
}

pub(super) fn wait_for_ssh_ready(vm: &Vm, timeout: Duration, output: OutputFormat) -> Result<()> {
    let port = active_ssh_port(vm)?;
    let host = ssh_connect_host(&vm.config);
    let endpoint = if host.parse::<std::net::Ipv6Addr>().is_ok() {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    };
    let addresses = endpoint
        .to_socket_addrs()
        .map_err(|error| Error::io(format!("SSH endpoint {endpoint}"), error))?
        .collect::<Vec<_>>();
    if addresses.is_empty() {
        return Err(Error::message(format!(
            "SSH endpoint {endpoint} did not resolve; check ssh_access"
        )));
    }
    if output == OutputFormat::Human {
        eprintln!(
            "vmctl: waiting up to {}s for SSH on {endpoint}",
            timeout.as_secs()
        );
    }
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if !matches!(vm.state()?, VmState::Running(_)) {
            return Err(Error::message(format!(
                "{} stopped before SSH became ready; run `vmctl logs {}`",
                vm.config.name, vm.config.name
            )));
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        let attempt_timeout = remaining.min(Duration::from_millis(500));
        if addresses
            .iter()
            .copied()
            .any(|address| has_ssh_banner(address, attempt_timeout))
        {
            return Ok(());
        }
        thread::sleep(remaining.min(Duration::from_millis(100)));
    }
    Err(Error::message(format!(
        "SSH on {endpoint} was not ready after {}s; the VM is still running. Check `vmctl logs {}` or `vmctl doctor {}`",
        timeout.as_secs(),
        vm.config.name,
        vm.config.name
    )))
}

pub(super) fn has_ssh_banner(address: SocketAddr, timeout: Duration) -> bool {
    let Ok(mut stream) = TcpStream::connect_timeout(&address, timeout) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(timeout));
    let mut banner = [0; 4];
    stream
        .read_exact(&mut banner)
        .is_ok_and(|_| banner == *b"SSH-")
}

pub(super) fn start_vm_loaded(
    vm: &Vm,
    output: OutputFormat,
    wait_for_ssh: Option<Duration>,
) -> Result<()> {
    if let VmState::Running(pid) = vm.state()? {
        let viewer_reconnected = reconnect_viewer(vm, output == OutputFormat::Json);
        if let Some(timeout) = wait_for_ssh {
            wait_for_ssh_ready(vm, timeout, output)?;
        }
        if output == OutputFormat::Json {
            print_json_success(json!({
                "name": vm.config.name,
                "state": "running",
                "pid": pid,
                "viewer_reconnected": viewer_reconnected,
                "waited_for_ssh": wait_for_ssh.is_some(),
            }));
        } else {
            println!("{} is already running (pid {pid})", vm.config.name);
            if viewer_reconnected {
                println!("Reconnected the configured viewer");
            }
        }
        return Ok(());
    }

    check_tsc_stability(vm, output == OutputFormat::Json)?;
    validate_usb_devices(vm)?;

    fs::create_dir_all(&vm.paths.state_dir)
        .map_err(|error| Error::io(vm.paths.state_dir.display(), error))?;
    remove_runtime_sockets(&vm.paths);
    stop_tpm(&vm.paths);
    stop_virtiofsd(&vm.paths);
    let _ = fs::remove_file(vm.paths.pid_file());
    ensure_disk(vm)?;

    let log_path = vm.paths.state_dir.join("qemu.log");
    let log = File::create(&log_path).map_err(|error| Error::io(log_path.display(), error))?;
    let error_log = log
        .try_clone()
        .map_err(|error| Error::io(log_path.display(), error))?;
    let mut host = HostCapabilities::detect(&vm.config)?;
    if let Some(pinning) = &vm.config.cpu_pinning {
        validate_cpu_pinning_for_host(
            pinning,
            &host.host_os,
            vm.config.cpu_cores.unwrap_or(host.cpu_cores),
        )?;
    }
    let mut plan = build_plan(vm, &host, true)?;
    write_runtime_files(&vm.paths, &plan)?;
    if virtiofs_requested(&vm.config, &host)
        && !start_virtiofsd(vm, &host, output == OutputFormat::Json)
    {
        host.virtiofsd = None;
        host.virtiofs_device = false;
        plan = build_plan(vm, &host, true)?;
        write_runtime_files(&vm.paths, &plan)?;
    }
    let tpm = match start_tpm(vm) {
        Ok(tpm) => tpm,
        Err(error) => {
            stop_virtiofsd(&vm.paths);
            return Err(error);
        }
    };
    if let Err(error) = ensure_ipc_endpoints_available(&plan) {
        drop(tpm);
        stop_tpm(&vm.paths);
        stop_virtiofsd(&vm.paths);
        remove_runtime_sockets(&vm.paths);
        return Err(error);
    }
    let mut qemu = ProcessCommand::new(&plan.binary);
    qemu.args(&plan.args);
    #[cfg(unix)]
    // Keep QEMU independent from the terminal/session that launched vmctl.
    unsafe {
        qemu.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = match qemu
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(error_log))
        .spawn()
    {
        Ok(child) => child,
        Err(error) => {
            drop(tpm);
            stop_tpm(&vm.paths);
            stop_virtiofsd(&vm.paths);
            return Err(Error::command_unavailable(&plan.binary, error));
        }
    };
    let pid = child.id() as i32;
    if let Err(error) = write_pid(vm, pid) {
        let _ = child.kill();
        let _ = child.wait();
        stop_tpm(&vm.paths);
        stop_virtiofsd(&vm.paths);
        remove_runtime_sockets(&vm.paths);
        return Err(error);
    }

    if wait_for_exit(pid, &vm.config.name, Duration::from_secs(2)) {
        let _ = child.kill();
        let _ = child.wait();
        stop_tpm(&vm.paths);
        stop_virtiofsd(&vm.paths);
        let log = fs::read_to_string(&log_path).unwrap_or_default();
        let _ = fs::remove_file(vm.paths.pid_file());
        let detail = if log.trim().is_empty() {
            format!(
                "QEMU exited during startup without diagnostic output; see {} and {}",
                log_path.display(),
                vm.paths.state_dir.join("qemu.command").display()
            )
        } else {
            format!(
                "QEMU exited during startup; see {}\n{log}",
                log_path.display()
            )
        };
        remove_runtime_sockets(&vm.paths);
        return Err(Error::message(detail));
    }

    if let Some(pinning) = &vm.config.cpu_pinning
        && let Err(error) = apply_cpu_pinning(pid, pinning)
    {
        let _ = kill_process(vm, pid, true);
        let _ = wait_for_exit(pid, &vm.config.name, Duration::from_secs(2));
        stop_tpm(&vm.paths);
        stop_virtiofsd(&vm.paths);
        let _ = fs::remove_file(vm.paths.pid_file());
        remove_runtime_sockets(&vm.paths);
        return Err(error);
    }
    if let Some(command) = &vm.config.monitor_cmd
        && let Err(error) = send_monitor_command(vm, command)
    {
        let _ = kill_process(vm, pid, true);
        let _ = wait_for_exit(pid, &vm.config.name, Duration::from_secs(2));
        stop_tpm(&vm.paths);
        stop_virtiofsd(&vm.paths);
        let _ = fs::remove_file(vm.paths.pid_file());
        remove_runtime_sockets(&vm.paths);
        return Err(error);
    }
    let viewer_started = launch_viewer(vm, &plan, output == OutputFormat::Json);

    if let Some(timeout) = wait_for_ssh {
        wait_for_ssh_ready(vm, timeout, output)?;
    }

    if output == OutputFormat::Json {
        print_json_success(json!({
            "name": vm.config.name,
            "state": "running",
            "pid": pid,
            "ssh_port": plan.ssh_port,
            "ssh_host": plan.ssh_host,
            "ssh_user": vm.config.ssh_user,
            "spice_port": plan.spice_port,
            "spice_host": plan.spice_host,
            "state_dir": vm.paths.state_dir,
            "log": log_path,
            "command": vm.paths.state_dir.join("qemu.command"),
            "viewer_started": viewer_started,
            "waited_for_ssh": wait_for_ssh.is_some(),
        }));
    } else {
        println!("Started {} (pid {pid})", vm.config.name);
        println!("  log:   {}", log_path.display());
        if let Some(port) = plan.ssh_port {
            let user = vm
                .config
                .ssh_user
                .clone()
                .unwrap_or_else(|| env::var("USER").unwrap_or_else(|_| "user".to_string()));
            println!(
                "  ssh:   ssh -p {port} {user}@{}",
                plan.ssh_host.as_deref().unwrap_or("127.0.0.1")
            );
        }
        if let Some(port) = plan.spice_port {
            println!(
                "  spice: {}:{port}",
                plan.spice_host.as_deref().unwrap_or("127.0.0.1")
            );
        }
        if wait_for_ssh.is_some() {
            println!("  ssh:   ready");
        }
    }
    Ok(())
}

pub(super) fn stop_vm(
    dirs: &Dirs,
    name: &str,
    timeout: u64,
    force: bool,
    output: OutputFormat,
) -> Result<()> {
    stop_vm_inner(dirs, name, timeout, force, output, true)
}

pub(super) fn stop_vm_inner(
    dirs: &Dirs,
    name: &str,
    timeout: u64,
    force: bool,
    output: OutputFormat,
    report: bool,
) -> Result<()> {
    let vm = find(&dirs.vm_dir, &dirs.state_root, name)?;
    let _operation_lock = acquire_vm_lock(&vm.paths)?;
    stop_vm_loaded(&vm, timeout, force, output, report)
}

pub(super) fn stop_vm_loaded(
    vm: &Vm,
    timeout: u64,
    force: bool,
    output: OutputFormat,
    report: bool,
) -> Result<()> {
    let VmState::Running(pid) = vm.state()? else {
        let _ = fs::remove_file(vm.paths.pid_file());
        stop_tpm(&vm.paths);
        stop_virtiofsd(&vm.paths);
        remove_runtime_sockets(&vm.paths);
        if !report {
            return Ok(());
        }
        if output == OutputFormat::Json {
            print_json_success(json!({"name": vm.config.name, "state": "stopped"}));
        } else {
            println!("{} is already stopped", vm.config.name);
        }
        return Ok(());
    };

    let graceful = shutdown_via_qmp(&vm.paths);
    if graceful.is_err() && !force {
        return graceful;
    }
    if graceful.is_err() {
        kill_process(vm, pid, true)?;
    }
    if !wait_for_exit(pid, &vm.config.name, Duration::from_secs(timeout)) {
        if !force {
            return Err(Error::stop_timeout(&vm.config.name, pid, timeout, false));
        }
        kill_process(vm, pid, true)?;
        if !wait_for_exit(pid, &vm.config.name, Duration::from_secs(2)) {
            return Err(Error::stop_timeout(&vm.config.name, pid, timeout, true));
        }
    }
    let _ = fs::remove_file(vm.paths.pid_file());
    stop_tpm(&vm.paths);
    stop_virtiofsd(&vm.paths);
    remove_runtime_sockets(&vm.paths);

    if !report {
        return Ok(());
    }
    if output == OutputFormat::Json {
        print_json_success(json!({"name": vm.config.name, "state": "stopped", "pid": pid}));
    } else {
        println!("Stopped {} (pid {pid})", vm.config.name);
    }
    Ok(())
}

pub(super) fn kill_vm(dirs: &Dirs, name: &str, output: OutputFormat) -> Result<()> {
    let vm = find(&dirs.vm_dir, &dirs.state_root, name)?;
    let _operation_lock = acquire_vm_lock(&vm.paths)?;
    let VmState::Running(pid) = vm.state()? else {
        return Err(Error::message(format!("{} is not running", vm.config.name)));
    };
    kill_process(&vm, pid, true)?;
    if !wait_for_exit(pid, &vm.config.name, Duration::from_secs(2)) {
        return Err(Error::message(format!(
            "{} is still running after kill (pid {pid})",
            vm.config.name
        )));
    }
    let _ = fs::remove_file(vm.paths.pid_file());
    stop_tpm(&vm.paths);
    stop_virtiofsd(&vm.paths);
    remove_runtime_sockets(&vm.paths);
    if output == OutputFormat::Json {
        print_json_success(json!({"name": vm.config.name, "state": "killed", "pid": pid}));
    } else {
        println!("Killed {} (pid {pid})", vm.config.name);
    }
    Ok(())
}
