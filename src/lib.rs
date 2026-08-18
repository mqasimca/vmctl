pub mod cli;

mod config;
mod error;
mod get;
mod paths;
mod qemu;

use std::collections::BTreeSet;
use std::env;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command as ProcessCommand, Stdio};
use std::time::Duration;

#[cfg(unix)]
use std::os::unix::process::CommandExt;

use serde_json::{Value, json};

use cli::{
    Cli, Command as VmCommand, DiskAction, GuestAction, HostAction, LaunchOptions, OutputFormat,
    SnapshotAction,
};
use config::{discover, find};
use paths::Dirs;
use qemu::{
    HostCapabilities, acquire_vm_lock, build_plan, disk_check, disk_compact, disk_convert,
    disk_info, disk_resize, disk_snapshot, ensure_disk, ensure_ipc_endpoints_available,
    guest_command, guest_exec, guest_shutdown, ipc_report, kill_process, qemu_capability_report,
    qmp_ping, qmp_status, remove_runtime_sockets, render_node, send_monitor_command, shell_join,
    shutdown_via_qmp, spice_address, start_tpm, start_virtiofsd, stop_tpm, stop_virtiofsd,
    virtiofs_requested, virtiofsd_available, wait_for_exit, write_runtime_files,
};

pub use config::{Vm, VmConfig, VmState, parse_config, parse_tokens};
pub use error::{Error, Error as VmctlError, Result};
pub use paths::VmPaths;
pub use qemu::{QemuPlan, QemuPlanContext};

pub fn run(cli: Cli) -> Result<()> {
    let dirs = Dirs::from_cli(&cli)?;
    let output = cli.output;

    if cli.verbose > 0 && output != OutputFormat::Json {
        eprintln!(
            "vmctl: vm-dir={} state-dir={}",
            dirs.vm_dir.display(),
            dirs.state_root.display()
        );
    }

    match cli.command.unwrap_or(VmCommand::List) {
        VmCommand::List => list_vms(&dirs, output),
        VmCommand::Status { vm } => status_vms(&dirs, vm.as_deref(), output),
        VmCommand::Plan {
            vm,
            redact,
            options,
        } => plan_vm(&dirs, &vm, &options, output, redact),
        VmCommand::Start { vm, options } => start_vm(&dirs, &vm, &options, output),
        VmCommand::Stop { vm, timeout, force } => stop_vm(&dirs, &vm, timeout, force, output),
        VmCommand::Kill { vm } => kill_vm(&dirs, &vm, output),
        VmCommand::Logs { vm, lines } => logs_vm(&dirs, &vm, lines as usize, output),
        VmCommand::Restart {
            vm,
            timeout,
            force,
            options,
        } => {
            let mut vm = find(&dirs.vm_dir, &dirs.state_root, &vm)?;
            let _operation_lock = acquire_vm_lock(&vm.paths)?;
            stop_vm_loaded(&vm, timeout, force, output, false)?;
            apply_launch_options(&mut vm, &options)?;
            start_vm_loaded(&vm, output)
        }
        VmCommand::Snapshot { vm, action } => snapshot_vm(&dirs, &vm, action, output),
        VmCommand::Disk { vm, action } => disk_vm(&dirs, &vm, action, output),
        VmCommand::DeleteDisk { vm, yes } => delete_disk(&dirs, &vm, yes, output),
        VmCommand::DeleteVm { vm, yes } => delete_vm(&dirs, &vm, yes, output),
        VmCommand::Monitor { vm, command } => monitor_vm(&dirs, &vm, &command, output),
        VmCommand::Guest { vm, action } => guest_vm(&dirs, &vm, action, output),
        VmCommand::Shortcut { vm, path } => shortcut_vm(&dirs, &vm, path, output),
        VmCommand::Report => report_host(output),
        VmCommand::Doctor { vm } => doctor(&dirs, vm.as_deref(), output),
        VmCommand::Host { action } => host_action(action, output),
        VmCommand::Get(args) => get::run(&args, &dirs, output),
    }
}

fn list_vms(dirs: &Dirs, output: OutputFormat) -> Result<()> {
    let vms = discover(&dirs.vm_dir, &dirs.state_root)?;
    if output == OutputFormat::Json {
        let values: Vec<Value> = vms.iter().map(vm_summary).collect::<Result<_>>()?;
        println!(
            "{}",
            serde_json::to_string_pretty(&values).unwrap_or_default()
        );
        return Ok(());
    }

    if vms.is_empty() {
        println!("No VM configurations found in {}", dirs.vm_dir.display());
        return Ok(());
    }

    println!("{:<28} {:<16} {:<8} CONFIG", "NAME", "STATE", "SSH");
    for vm in vms {
        let ssh = vm
            .config
            .ssh_port
            .map_or_else(|| "auto".to_string(), |port| port.to_string());
        println!(
            "{:<28} {:<16} {:<8} {}",
            vm.config.name,
            state_label(&vm)?,
            ssh,
            vm.config.config_path.display()
        );
    }
    Ok(())
}

fn status_vms(dirs: &Dirs, name: Option<&str>, output: OutputFormat) -> Result<()> {
    if let Some(name) = name {
        let vm = find(&dirs.vm_dir, &dirs.state_root, name)?;
        if output == OutputFormat::Json {
            println!(
                "{}",
                serde_json::to_string_pretty(&vm_status(&vm)?).unwrap_or_default()
            );
        } else {
            print_vm_status(&vm)?;
        }
        Ok(())
    } else {
        list_vms(dirs, output)
    }
}

fn plan_vm(
    dirs: &Dirs,
    name: &str,
    options: &LaunchOptions,
    output: OutputFormat,
    redact: bool,
) -> Result<()> {
    let vm = load_effective_vm(dirs, name, options)?;
    let host = HostCapabilities::detect(&vm.config)?;
    if let Some(pinning) = &vm.config.cpu_pinning {
        validate_cpu_pinning_for_host(
            pinning,
            &host.host_os,
            vm.config.cpu_cores.unwrap_or(host.cpu_cores),
        )?;
    }
    let plan = build_plan(&vm, &host, false)?;
    print_plan(&plan, output, redact);
    Ok(())
}

fn start_vm(dirs: &Dirs, name: &str, options: &LaunchOptions, output: OutputFormat) -> Result<()> {
    let vm = load_effective_vm(dirs, name, options)?;
    let _operation_lock = acquire_vm_lock(&vm.paths)?;
    start_vm_loaded(&vm, output)
}

fn start_vm_loaded(vm: &Vm, output: OutputFormat) -> Result<()> {
    if let VmState::Running(pid) = vm.state()? {
        let viewer_reconnected = reconnect_viewer(vm, output == OutputFormat::Json);
        if output == OutputFormat::Json {
            println!(
                "{}",
                json!({
                    "name": vm.config.name,
                    "state": "running",
                    "pid": pid,
                    "viewer_reconnected": viewer_reconnected,
                })
            );
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

    if output == OutputFormat::Json {
        println!(
            "{}",
            json!({
                "name": vm.config.name,
                "state": "running",
                "pid": pid,
                "ssh_port": plan.ssh_port,
                "ssh_host": plan.ssh_host,
                "spice_port": plan.spice_port,
                "spice_host": plan.spice_host,
                "state_dir": vm.paths.state_dir,
                "log": log_path,
                "command": vm.paths.state_dir.join("qemu.command"),
                "viewer_started": viewer_started,
            })
        );
    } else {
        println!("Started {} (pid {pid})", vm.config.name);
        println!("  log:   {}", log_path.display());
        if let Some(port) = plan.ssh_port {
            let user = env::var("USER").unwrap_or_else(|_| "user".to_string());
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
    }
    Ok(())
}

fn stop_vm(dirs: &Dirs, name: &str, timeout: u64, force: bool, output: OutputFormat) -> Result<()> {
    stop_vm_inner(dirs, name, timeout, force, output, true)
}

fn stop_vm_inner(
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

fn stop_vm_loaded(
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
            println!("{}", json!({"name": vm.config.name, "state": "stopped"}));
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
        println!(
            "{}",
            json!({"name": vm.config.name, "state": "stopped", "pid": pid})
        );
    } else {
        println!("Stopped {} (pid {pid})", vm.config.name);
    }
    Ok(())
}

fn kill_vm(dirs: &Dirs, name: &str, output: OutputFormat) -> Result<()> {
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
        println!(
            "{}",
            json!({"name": vm.config.name, "state": "killed", "pid": pid})
        );
    } else {
        println!("Killed {} (pid {pid})", vm.config.name);
    }
    Ok(())
}

fn snapshot_vm(
    dirs: &Dirs,
    name: &str,
    action: SnapshotAction,
    output: OutputFormat,
) -> Result<()> {
    let vm = find(&dirs.vm_dir, &dirs.state_root, name)?;
    let _operation_lock = acquire_vm_lock(&vm.paths)?;
    if matches!(vm.state()?, VmState::Running(_)) {
        return Err(Error::message(
            "disk snapshots require a stopped VM; use the QEMU monitor for live snapshots",
        ));
    }
    let (operation, tag) = match action {
        SnapshotAction::Create { tag } => ("-c", Some(tag)),
        SnapshotAction::Apply { tag } => ("-a", Some(tag)),
        SnapshotAction::Delete { tag } => ("-d", Some(tag)),
        SnapshotAction::Info => ("-l", None),
    };
    let result = disk_snapshot(&vm, operation, tag.as_deref())?;
    if output == OutputFormat::Json {
        println!(
            "{}",
            json!({"name": vm.config.name, "action": operation, "tag": tag, "result": result})
        );
    } else if result.is_empty() {
        println!("Snapshot operation completed for {}", vm.config.name);
    } else {
        println!("{result}");
    }
    Ok(())
}

fn disk_vm(dirs: &Dirs, name: &str, action: DiskAction, output: OutputFormat) -> Result<()> {
    let vm = find(&dirs.vm_dir, &dirs.state_root, name)?;
    let _operation_lock = acquire_vm_lock(&vm.paths)?;
    match action {
        DiskAction::Info => {
            let disk = disk_info(&vm.config.disk_img)?;
            let result = json!({
                "name": vm.config.name,
                "action": "info",
                "disk": disk,
            });
            if output == OutputFormat::Json {
                print_json(&result);
            } else {
                println!("Disk: {}", vm.config.disk_img.display());
                print_disk_info_human(&result["disk"]);
            }
        }
        DiskAction::Resize { size, shrink, yes } => {
            require_stopped_disk(&vm, "resize")?;
            if shrink && !yes {
                return Err(Error::message(
                    "shrinking a disk requires --yes because it can destroy data",
                ));
            }
            let disk = disk_resize(&vm.config.disk_img, &size, shrink)?;
            let result = json!({
                "name": vm.config.name,
                "action": "resize",
                "size": size,
                "shrink": shrink,
                "disk": disk,
            });
            if output == OutputFormat::Json {
                print_json(&result);
            } else {
                println!("Resized {} to {size}", vm.config.name);
                print_disk_info_human(&result["disk"]);
            }
        }
        DiskAction::Check { repair, yes } => {
            require_stopped_disk(&vm, "check")?;
            if repair && !yes {
                return Err(Error::message(
                    "disk repair requires --yes because it changes the image",
                ));
            }
            let check = disk_check(&vm.config.disk_img, repair)?;
            if !check.healthy {
                if output != OutputFormat::Json {
                    print_disk_check_human(&check.report);
                }
                return Err(Error::disk_check_failed(&vm.config.disk_img, check.report));
            }
            let result = json!({
                "name": vm.config.name,
                "action": "check",
                "repair": repair,
                "healthy": true,
                "report": check.report,
            });
            if output == OutputFormat::Json {
                print_json(&result);
            } else {
                println!("Disk check passed for {}", vm.config.name);
                print_disk_check_human(&result["report"]);
            }
        }
        DiskAction::Convert {
            destination,
            format,
            compress,
            force,
        } => {
            require_stopped_disk(&vm, "convert")?;
            let format = format.unwrap_or_else(|| vm.config.disk_format.clone());
            let disk = disk_convert(&vm.config.disk_img, &destination, &format, compress, force)?;
            let result = json!({
                "name": vm.config.name,
                "action": "convert",
                "output": destination,
                "format": format,
                "compressed": compress,
                "disk": disk,
            });
            if output == OutputFormat::Json {
                print_json(&result);
            } else {
                println!("Converted {} to {}", vm.config.name, destination.display());
                print_disk_info_human(&result["disk"]);
            }
        }
        DiskAction::Compact { yes } => {
            require_stopped_disk(&vm, "compact")?;
            if !yes {
                return Err(Error::message(
                    "compacting replaces the disk image and discards internal snapshots; rerun with --yes",
                ));
            }
            let disk = disk_compact(&vm.config.disk_img)?;
            let result = json!({
                "name": vm.config.name,
                "action": "compact",
                "disk": disk,
            });
            if output == OutputFormat::Json {
                print_json(&result);
            } else {
                println!("Compacted disk for {}", vm.config.name);
                print_disk_info_human(&result["disk"]);
            }
        }
    }
    Ok(())
}

fn require_stopped_disk(vm: &Vm, operation: &str) -> Result<()> {
    if let VmState::Running(pid) = vm.state()? {
        return Err(Error::message(format!(
            "disk {operation} requires a stopped VM; {} is running with pid {pid}",
            vm.config.name
        )));
    }
    Ok(())
}

fn print_json(value: &Value) {
    println!(
        "{}",
        serde_json::to_string_pretty(value).unwrap_or_default()
    );
}

fn print_disk_info_human(info: &Value) {
    for (label, key) in [
        ("format", "format"),
        ("virtual size", "virtual-size"),
        ("actual size", "actual-size"),
        ("cluster size", "cluster-size"),
        ("backing file", "backing-filename"),
    ] {
        if let Some(value) = info.get(key) {
            println!("{label}: {}", display_json_value(value));
        }
    }
    if let Some(snapshots) = info.get("snapshots").and_then(Value::as_array) {
        println!("snapshots: {}", snapshots.len());
    }
}

fn print_disk_check_human(report: &Value) {
    println!(
        "{}",
        serde_json::to_string_pretty(report).unwrap_or_default()
    );
}

fn display_json_value(value: &Value) -> String {
    value
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| value.to_string())
}

fn delete_disk(dirs: &Dirs, name: &str, yes: bool, output: OutputFormat) -> Result<()> {
    let vm = find(&dirs.vm_dir, &dirs.state_root, name)?;
    let _operation_lock = acquire_vm_lock(&vm.paths)?;
    ensure_delete_allowed(&vm, yes)?;
    remove_if_present(&vm.config.disk_img)?;
    for path in persistent_efi_vars(&vm) {
        remove_if_present(&path)?;
    }
    if output == OutputFormat::Json {
        println!("{}", json!({"name": vm.config.name, "deleted": "disk"}));
    } else {
        println!("Deleted disk data for {}", vm.config.name);
    }
    Ok(())
}

fn delete_vm(dirs: &Dirs, name: &str, yes: bool, output: OutputFormat) -> Result<()> {
    let vm = find(&dirs.vm_dir, &dirs.state_root, name)?;
    let _operation_lock = acquire_vm_lock(&vm.paths)?;
    ensure_delete_allowed(&vm, yes)?;
    let data_dir = vm
        .config
        .config_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(&vm.config.name);
    if fs::symlink_metadata(&data_dir).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        return Err(Error::message(format!(
            "refusing to remove VM data symlink {}",
            data_dir.display()
        )));
    }
    remove_if_present(&vm.config.disk_img)?;
    for path in persistent_efi_vars(&vm) {
        remove_if_present(&path)?;
    }
    if data_dir.is_dir() {
        fs::remove_dir_all(&data_dir).map_err(|error| Error::io(data_dir.display(), error))?;
    }
    remove_if_present(&vm.config.config_path)?;
    if vm.paths.state_dir.is_dir() {
        fs::remove_dir_all(&vm.paths.state_dir)
            .map_err(|error| Error::io(vm.paths.state_dir.display(), error))?;
    }
    if output == OutputFormat::Json {
        println!("{}", json!({"name": vm.config.name, "deleted": "vm"}));
    } else {
        println!("Deleted VM {}", vm.config.name);
    }
    Ok(())
}

fn monitor_vm(dirs: &Dirs, name: &str, command: &[String], output: OutputFormat) -> Result<()> {
    if command.is_empty() {
        return Err(Error::message("monitor requires a command"));
    }
    let vm = find(&dirs.vm_dir, &dirs.state_root, name)?;
    let _operation_lock = acquire_vm_lock(&vm.paths)?;
    if !matches!(vm.state()?, VmState::Running(_)) {
        return Err(Error::message(format!("{} is not running", vm.config.name)));
    }
    let command = command.join(" ");
    let response = send_monitor_command(&vm, &command)?;
    if output == OutputFormat::Json {
        println!(
            "{}",
            json!({"name": vm.config.name, "command": command, "response": response})
        );
    } else if !response.is_empty() {
        println!("{response}");
    }
    Ok(())
}

fn guest_vm(dirs: &Dirs, name: &str, action: GuestAction, output: OutputFormat) -> Result<()> {
    let vm = find(&dirs.vm_dir, &dirs.state_root, name)?;
    let _operation_lock = acquire_vm_lock(&vm.paths)?;
    let pid = match vm.state()? {
        VmState::Running(pid) => pid,
        VmState::Stopped => {
            return Err(Error::message(format!("{} is not running", vm.config.name)));
        }
    };
    let (command, result) = match action {
        GuestAction::Ping => ("guest-ping", guest_command(&vm, "guest-ping", None)?),
        GuestAction::Shutdown { timeout } => {
            let deadline = std::time::Instant::now()
                .checked_add(Duration::from_secs(timeout))
                .ok_or_else(|| Error::message("guest shutdown timeout is too large"))?;
            guest_shutdown(&vm, deadline)?;
            if wait_for_exit(
                pid,
                &vm.config.name,
                deadline.saturating_duration_since(std::time::Instant::now()),
            ) {
                (
                    "guest-shutdown",
                    json!({"requested": true, "stopped": true, "pid": pid}),
                )
            } else {
                let status = qmp_status(&vm.paths).ok();
                if status.as_deref() != Some("shutdown") {
                    return Err(Error::guest_shutdown_timeout(&vm.config.name, pid, timeout));
                }
                (
                    "guest-shutdown",
                    json!({"requested": true, "stopped": false, "status": status, "pid": pid}),
                )
            }
        }
        GuestAction::Ip => (
            "guest-network-get-interfaces",
            guest_command(&vm, "guest-network-get-interfaces", None)?,
        ),
        GuestAction::Exec {
            timeout,
            program,
            args,
        } => {
            let result = guest_exec(&vm, &program, &args, timeout)?;
            if result.get("signal").and_then(Value::as_i64).is_some()
                || result
                    .get("exitcode")
                    .and_then(Value::as_i64)
                    .is_some_and(|exit_code| exit_code != 0)
            {
                return Err(Error::guest_command_failed(&program, result));
            }
            ("guest-exec", result)
        }
    };
    if output == OutputFormat::Json {
        println!(
            "{}",
            json!({"name": vm.config.name, "command": command, "result": result})
        );
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&result).unwrap_or_default()
        );
    }
    Ok(())
}

fn shortcut_vm(dirs: &Dirs, name: &str, path: Option<PathBuf>, output: OutputFormat) -> Result<()> {
    let vm = find(&dirs.vm_dir, &dirs.state_root, name)?;
    let path = match path {
        Some(path) => path,
        None => paths::home_dir()?
            .join(".local/share/applications")
            .join(format!("{}.desktop", vm.config.name)),
    };
    let executable = env::current_exe().unwrap_or_else(|_| PathBuf::from("vmctl"));
    let config_root = vm
        .config
        .config_path
        .parent()
        .unwrap_or_else(|| Path::new("."));
    let content = format!(
        "[Desktop Entry]\nType=Application\nName={}\nComment=Start {} with vmctl\nTerminal=false\nExec={} --dir {} start {}\nPath={}\nCategories=System;Virtualization;\n",
        vm.config.name,
        vm.config.name,
        desktop_quote(&executable),
        desktop_quote(config_root),
        desktop_quote(Path::new(&vm.config.name)),
        desktop_quote(config_root),
    );
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| Error::io(parent.display(), error))?;
    }
    fs::write(&path, content).map_err(|error| Error::io(path.display(), error))?;
    if output == OutputFormat::Json {
        println!("{}", json!({"name": vm.config.name, "shortcut": path}));
    } else {
        println!("Created {}", path.display());
    }
    Ok(())
}

fn report_host(output: OutputFormat) -> Result<()> {
    let native_qemu = format!("qemu-system-{}", env::consts::ARCH);
    let native_capabilities = qemu_capability_report(&native_qemu);
    let kvm_readable = env::consts::OS == "linux" && File::open("/dev/kvm").is_ok();
    let qemu_supports_accelerator = |name: &str| {
        native_capabilities["runtime_accelerators"]
            .as_array()
            .is_some_and(|values| values.iter().any(|value| value.as_str() == Some(name)))
    };
    let report = json!({
        "os": env::consts::OS,
        "arch": env::consts::ARCH,
        "cpu_cores": std::thread::available_parallelism().map(|value| value.get()).ok(),
        "kvm": kvm_readable,
        "accelerators": {
            "kvm": kvm_readable && qemu_supports_accelerator("kvm"),
            "hvf": env::consts::OS == "macos" && qemu_supports_accelerator("hvf"),
            "whpx": env::consts::OS == "windows" && qemu_supports_accelerator("whpx"),
        },
        "graphics": {
            "render_node": render_node(),
        },
        "commands": {
            "qemu-system-x86_64": command_available("qemu-system-x86_64"),
            "qemu-system-aarch64": command_available("qemu-system-aarch64"),
            "qemu-img": command_available("qemu-img"),
            "swtpm": command_available("swtpm"),
            "qemu-bridge-helper": find_command("qemu-bridge-helper").is_some(),
            "virtiofsd": virtiofsd_available(),
        },
        "versions": {
            "qemu-system-x86_64": command_version("qemu-system-x86_64"),
            "qemu-system-aarch64": command_version("qemu-system-aarch64"),
            "qemu-img": command_version("qemu-img"),
        },
        "qemu": {
            "x86_64": qemu_capability_report("qemu-system-x86_64"),
            "aarch64": qemu_capability_report("qemu-system-aarch64"),
        },
    });
    if output == OutputFormat::Json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report).unwrap_or_default()
        );
    } else {
        println!(
            "host: {} {}",
            report["os"].as_str().unwrap_or("unknown"),
            report["arch"].as_str().unwrap_or("unknown")
        );
        println!("cpu cores: {}", report["cpu_cores"]);
        println!("kvm: {}", report["kvm"]);
        println!("qemu-img: {}", report["commands"]["qemu-img"]);
        println!(
            "qemu version: {}",
            report["versions"][native_qemu.as_str()]
                .as_str()
                .unwrap_or("unavailable")
        );
        println!("swtpm: {}", report["commands"]["swtpm"]);
        println!(
            "qemu-bridge-helper: {}",
            report["commands"]["qemu-bridge-helper"]
        );
        println!("virtiofsd: {}", report["commands"]["virtiofsd"]);
        for arch in ["x86_64", "aarch64"] {
            let qemu = &report["qemu"][arch];
            let backends = qemu["display_backends"]
                .as_array()
                .map(|values| {
                    values
                        .iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            println!(
                "qemu-system-{arch}: {} (display: {})",
                qemu["version"].as_str().unwrap_or("unavailable"),
                if backends.is_empty() {
                    "unavailable"
                } else {
                    &backends
                }
            );
        }
    }
    Ok(())
}

fn doctor(dirs: &Dirs, name: Option<&str>, output: OutputFormat) -> Result<()> {
    let mut checks = Vec::new();
    push_doctor_check(
        &mut checks,
        "host.platform",
        "ok",
        format!("{} {}", env::consts::OS, env::consts::ARCH),
        None,
        None,
    );

    let native_qemu = format!("qemu-system-{}", env::consts::ARCH);
    for command in [native_qemu.as_str(), "qemu-system-aarch64", "qemu-img"] {
        if command == "qemu-system-aarch64" && native_qemu == command {
            continue;
        }
        let required = command == native_qemu || command == "qemu-img";
        let path = find_command(command);
        let status = if path.as_deref().is_some_and(|_| command_available(command)) {
            "ok"
        } else if required {
            "error"
        } else {
            "warn"
        };
        let message = match (status, path) {
            ("ok", Some(path)) => command_version(command).map_or_else(
                || format!("{command} is available at {path}"),
                |version| format!("{command} {version} is available at {path}"),
            ),
            ("error", _) => format!("{command} is required but unavailable"),
            _ => format!("{command} is unavailable; this architecture is optional"),
        };
        push_doctor_check(
            &mut checks,
            &format!("host.command.{command}"),
            status,
            message,
            (status == "error")
                .then_some("Install the QEMU package matching the host architecture."),
            None,
        );
    }

    let qemu_capabilities = qemu_capability_report(&native_qemu);
    let runtime_failures = qemu_capabilities["runtime_probe_failures"]
        .as_array()
        .is_some_and(|values| !values.is_empty());
    let runtime_unprobed = qemu_capabilities["runtime_unprobed"]
        .as_array()
        .is_some_and(|values| !values.is_empty());
    if qemu_capabilities["available"] == true && (runtime_failures || runtime_unprobed) {
        push_doctor_check(
            &mut checks,
            "host.accelerator.runtime",
            if runtime_failures { "warn" } else { "skip" },
            format!(
                "runtime accelerator probes incomplete (failed: {}, unprobed: {})",
                qemu_capabilities["runtime_probe_failures"], qemu_capabilities["runtime_unprobed"]
            ),
            runtime_failures.then_some(
                "vmctl will fall back to TCG when hardware acceleration cannot be initialized.",
            ),
            Some(qemu_capabilities.clone()),
        );
    }
    if qemu_capabilities["available"] == true && qemu_capabilities["complete"] != true {
        push_doctor_check(
            &mut checks,
            "host.qemu_capabilities",
            "error",
            qemu_capabilities["probe_error"]
                .as_str()
                .unwrap_or("QEMU capability probes are incomplete"),
            Some("Verify the QEMU installation and retry the read-only capability check."),
            Some(qemu_capabilities.clone()),
        );
    }
    if qemu_capabilities["available"] == true && qemu_capabilities["complete"] == true {
        let backends = qemu_capabilities["display_backends"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect::<BTreeSet<_>>();
        for backend in ["gtk", "sdl", "spice-app"] {
            let available = backends.contains(backend);
            push_doctor_check(
                &mut checks,
                &format!("host.display.{backend}"),
                if available { "ok" } else { "warn" },
                if available {
                    format!("QEMU display backend '{backend}' is available")
                } else {
                    format!("QEMU display backend '{backend}' is unavailable")
                },
                (!available).then_some(
                    "Install a QEMU GUI/display backend package or choose another display mode.",
                ),
                None,
            );
        }
    }

    if env::consts::OS == "linux" {
        let kvm = Path::new("/dev/kvm");
        let (status, message, hint) = if !kvm.exists() {
            (
                "warn",
                "/dev/kvm is not present; QEMU will use software emulation",
                Some("Enable virtualization in firmware or continue with slower TCG emulation."),
            )
        } else if File::open(kvm).is_ok() {
            ("ok", "/dev/kvm is readable", None)
        } else {
            (
                "error",
                "/dev/kvm exists but is not readable",
                Some("Check the kvm group membership and device permissions."),
            )
        };
        push_doctor_check(&mut checks, "host.kvm", status, message, hint, None);
    } else {
        push_doctor_check(
            &mut checks,
            "host.kvm",
            "skip",
            "KVM device check is Linux-specific",
            None,
            None,
        );
    }

    for (id, command, message) in [
        (
            "host.viewer.remote_viewer",
            "remote-viewer",
            "SPICE remote-viewer",
        ),
        ("host.viewer.spicy", "spicy", "SPICE spicy viewer"),
        ("host.swtpm", "swtpm", "TPM 2.0 helper"),
        ("host.smbd", "smbd", "Samba file sharing"),
    ] {
        let status = if command_available(command) {
            "ok"
        } else {
            "warn"
        };
        push_doctor_check(
            &mut checks,
            id,
            status,
            if status == "ok" {
                format!("{message} is available")
            } else {
                format!("{message} is unavailable; dependent features will not work")
            },
            None,
            None,
        );
    }
    let virtiofsd = virtiofsd_available();
    push_doctor_check(
        &mut checks,
        "host.virtiofsd",
        if virtiofsd { "ok" } else { "warn" },
        if virtiofsd {
            "virtiofsd is available"
        } else {
            "virtiofsd is unavailable; Linux shares will use 9p"
        },
        None,
        None,
    );
    let bridge_helper = find_command("qemu-bridge-helper").is_some();
    push_doctor_check(
        &mut checks,
        "host.qemu_bridge_helper",
        if bridge_helper { "ok" } else { "warn" },
        if bridge_helper {
            "qemu-bridge-helper is available"
        } else {
            "qemu-bridge-helper is unavailable; bridged networking will not work"
        },
        None,
        None,
    );

    if let Some(name) = name {
        let vm = find(&dirs.vm_dir, &dirs.state_root, name)?;
        push_doctor_check(
            &mut checks,
            "vm.config",
            "ok",
            format!("configuration parsed: {}", vm.config.config_path.display()),
            None,
            None,
        );
        let vm_qemu_binary = format!("qemu-system-{}", vm.config.arch);
        let vm_qemu_capabilities = qemu_capability_report(&vm_qemu_binary);
        let vm_qemu_available =
            vm_qemu_capabilities["available"] == true && vm_qemu_capabilities["complete"] == true;
        push_doctor_check(
            &mut checks,
            "vm.qemu_capabilities",
            if vm_qemu_available { "ok" } else { "error" },
            if vm_qemu_available {
                format!(
                    "{} is available for the configured {} guest",
                    vm_qemu_binary, vm.config.arch
                )
            } else {
                format!(
                    "{} is unavailable or its capability probes are incomplete for the configured {} guest",
                    vm_qemu_binary, vm.config.arch
                )
            },
            (!vm_qemu_available).then_some(
                "Install the QEMU system package matching the VM architecture, then retry.",
            ),
            Some(vm_qemu_capabilities.clone()),
        );
        let vm_runtime_failures = vm_qemu_capabilities["runtime_probe_failures"]
            .as_array()
            .is_some_and(|values| !values.is_empty());
        let vm_runtime_unprobed = vm_qemu_capabilities["runtime_unprobed"]
            .as_array()
            .is_some_and(|values| !values.is_empty());
        if vm_qemu_capabilities["available"] == true && (vm_runtime_failures || vm_runtime_unprobed)
        {
            push_doctor_check(
                &mut checks,
                "vm.accelerator.runtime",
                if vm_runtime_failures { "warn" } else { "skip" },
                format!(
                    "runtime accelerator probes incomplete (failed: {}, unprobed: {})",
                    vm_qemu_capabilities["runtime_probe_failures"],
                    vm_qemu_capabilities["runtime_unprobed"]
                ),
                vm_runtime_failures
                    .then_some("vmctl will choose a usable accelerator or fall back to TCG."),
                Some(vm_qemu_capabilities.clone()),
            );
        }
        let pid = match vm.state()? {
            VmState::Running(pid) => Some(pid),
            VmState::Stopped => None,
        };
        push_doctor_check(
            &mut checks,
            "vm.state",
            "ok",
            pid.map_or_else(
                || format!("{name} is stopped"),
                |pid| format!("{name} is running with pid {pid}"),
            ),
            None,
            None,
        );

        let disk_status = if vm.config.disk_img.is_file() {
            "ok"
        } else {
            "warn"
        };
        push_doctor_check(
            &mut checks,
            "vm.disk",
            disk_status,
            if disk_status == "ok" {
                format!("disk exists: {}", vm.config.disk_img.display())
            } else {
                format!(
                    "disk will be created on start: {}",
                    vm.config.disk_img.display()
                )
            },
            None,
            None,
        );

        for (id, path) in [
            ("vm.iso", vm.config.iso.as_ref()),
            ("vm.fixed_iso", vm.config.fixed_iso.as_ref()),
            ("vm.unattended_iso", vm.config.unattended_iso.as_ref()),
            ("vm.floppy", vm.config.floppy.as_ref()),
            ("vm.img", vm.config.img.as_ref()),
        ] {
            if let Some(path) = path {
                let status = if path.is_file() { "ok" } else { "error" };
                push_doctor_check(
                    &mut checks,
                    id,
                    status,
                    if status == "ok" {
                        format!("media exists: {}", path.display())
                    } else {
                        format!("configured media is missing: {}", path.display())
                    },
                    (status == "error")
                        .then_some("Fix the path or remove the stale media setting."),
                    None,
                );
            }
        }

        if let Some(public_dir) = &vm.config.public_dir {
            let status = if public_dir.is_dir() { "ok" } else { "error" };
            push_doctor_check(
                &mut checks,
                "vm.public_dir",
                status,
                if status == "ok" {
                    format!("share directory exists: {}", public_dir.display())
                } else {
                    format!("share directory is missing: {}", public_dir.display())
                },
                (status == "error").then_some("Create the directory or set public_dir=none."),
                None,
            );
        }

        if !vm.config.usb_devices.is_empty() {
            if env::consts::OS != "linux" {
                push_doctor_check(
                    &mut checks,
                    "vm.usb_devices",
                    "skip",
                    "USB pass-through preflight is only implemented on Linux",
                    None,
                    None,
                );
            } else if find_command("lsusb").is_none() {
                push_doctor_check(
                    &mut checks,
                    "vm.usb_devices",
                    "error",
                    "lsusb is required to verify configured USB devices",
                    Some("Install usbutils before starting a VM with USB pass-through."),
                    None,
                );
            } else {
                for (vendor, product) in &vm.config.usb_devices {
                    let device = format!("{vendor:04x}:{product:04x}");
                    let found = ProcessCommand::new("lsusb")
                        .args(["-d", &device])
                        .stdout(Stdio::null())
                        .stderr(Stdio::null())
                        .status()
                        .is_ok_and(|status| status.success());
                    push_doctor_check(
                        &mut checks,
                        &format!("vm.usb.{device}"),
                        if found { "ok" } else { "error" },
                        if found {
                            format!("USB device {device} is present")
                        } else {
                            format!("USB device {device} is missing or inaccessible")
                        },
                        Some("Connect the device and check host permissions before retrying."),
                        None,
                    );
                }
            }
        }

        match HostCapabilities::detect(&vm.config) {
            Err(error) => push_doctor_check(
                &mut checks,
                "vm.plan",
                "error",
                error.to_string(),
                Some("Fix the reported host dependency, firmware, or VM configuration issue."),
                None,
            ),
            Ok(host) => {
                if let Some(pinning) = &vm.config.cpu_pinning {
                    let vcpus = vm.config.cpu_cores.unwrap_or(host.cpu_cores);
                    match validate_cpu_pinning_for_host(pinning, &host.host_os, vcpus) {
                        Ok(()) => push_doctor_check(
                            &mut checks,
                            "vm.cpu_pinning",
                            "ok",
                            format!("CPU pinning is valid for {vcpus} vCPUs"),
                            None,
                            None,
                        ),
                        Err(error) => push_doctor_check(
                            &mut checks,
                            "vm.cpu_pinning",
                            "error",
                            error.to_string(),
                            Some("Fix cpu_pinning or remove it before starting the VM."),
                            None,
                        ),
                    }
                }
                match build_plan(&vm, &host, false) {
                    Ok(_) => {
                        push_doctor_check(
                            &mut checks,
                            "vm.plan",
                            "ok",
                            "QEMU command plan can be built",
                            None,
                            None,
                        );
                        push_doctor_check(
                            &mut checks,
                            "vm.accelerator",
                            if host.accelerator == "tcg" { "warn" } else { "ok" },
                            if host.accelerator == "tcg" {
                                "using TCG software emulation".to_string()
                            } else {
                                format!("using {} hardware acceleration", host.accelerator)
                            },
                            (host.accelerator == "tcg").then_some(
                                "Enable a usable hardware accelerator for better performance when available.",
                            ),
                            Some(json!({"accelerator": host.accelerator})),
                        );
                    }
                    Err(error) => push_doctor_check(
                        &mut checks,
                        "vm.plan",
                        "error",
                        error.to_string(),
                        Some(
                            "Fix the reported host dependency, firmware, or VM configuration issue.",
                        ),
                        None,
                    ),
                }
            }
        }

        let log_path = vm.paths.state_dir.join("qemu.log");
        if log_path.is_file() {
            let tail = read_diagnostic_tail(&log_path);
            push_doctor_check(
                &mut checks,
                "vm.qemu_log",
                "ok",
                format!("QEMU log: {}", log_path.display()),
                None,
                tail.map(|tail| json!({"tail": tail})),
            );
        } else {
            push_doctor_check(
                &mut checks,
                "vm.qemu_log",
                "warn",
                format!("QEMU log does not exist yet: {}", log_path.display()),
                Some("Start the VM once; startup failures will be recorded here."),
                None,
            );
        }
        let command_path = vm.paths.state_dir.join("qemu.command");
        push_doctor_check(
            &mut checks,
            "vm.qemu_command",
            if command_path.is_file() { "ok" } else { "warn" },
            format!("saved command: {}", command_path.display()),
            None,
            None,
        );

        if let Some(pid) = pid {
            let qmp_result = qmp_ping(&vm.paths);
            push_doctor_check(
                &mut checks,
                "vm.qmp",
                if qmp_result.is_ok() { "ok" } else { "warn" },
                qmp_result.as_ref().map_or_else(
                    |error| format!("QMP endpoint is unavailable: {error}"),
                    |_| "QMP endpoint is responding".to_string(),
                ),
                Some("Check qemu.log and the saved command if monitor operations fail."),
                Some(json!({"pid": pid, "ipc_state": vm.paths.ipc_state()})),
            );
            if vm.config.guest_agent {
                let agent_result = guest_command(&vm, "guest-ping", None);
                let agent_ok = agent_result.is_ok();
                let agent_message = agent_result.map_or_else(
                    |error| format!("guest-agent endpoint is unavailable: {error}"),
                    |_| "guest-agent is responding".to_string(),
                );
                push_doctor_check(
                    &mut checks,
                    "vm.guest_agent",
                    if agent_ok { "ok" } else { "warn" },
                    agent_message,
                    Some("Install and start the guest agent inside the VM."),
                    None,
                );
            }
        }
        if matches!(vm.config.display.as_str(), "none" | "spice" | "spice-app")
            && vm.config.viewer != "none"
        {
            let available = command_available(&vm.config.viewer);
            push_doctor_check(
                &mut checks,
                "vm.viewer",
                if available { "ok" } else { "error" },
                if available {
                    format!("viewer command {} is available", vm.config.viewer)
                } else {
                    format!("viewer command {} is unavailable", vm.config.viewer)
                },
                Some("Install the configured SPICE viewer or set viewer=none."),
                None,
            );
        }
    }

    let errors = checks
        .iter()
        .filter(|check| check["status"] == "error")
        .count();
    let warnings = checks
        .iter()
        .filter(|check| check["status"] == "warn")
        .count();
    let report = json!({
        "schema_version": 1,
        "ok": errors == 0,
        "scope": {"vm": name},
        "checks": checks,
        "summary": {"errors": errors, "warnings": warnings},
    });

    if output == OutputFormat::Json {
        if errors == 0 {
            println!(
                "{}",
                serde_json::to_string_pretty(&report).unwrap_or_default()
            );
        }
    } else {
        print_doctor_human(&report);
    }
    if errors > 0 {
        return Err(Error::doctor_failed(errors, warnings, report));
    }
    Ok(())
}

fn push_doctor_check(
    checks: &mut Vec<Value>,
    id: &str,
    status: &str,
    message: impl Into<String>,
    hint: Option<&str>,
    evidence: Option<Value>,
) {
    let mut check = json!({
        "id": id,
        "status": status,
        "message": message.into(),
    });
    let object = check.as_object_mut().expect("doctor check object");
    if let Some(hint) = hint {
        object.insert("hint".to_string(), json!(hint));
    }
    if let Some(evidence) = evidence {
        object.insert("evidence".to_string(), evidence);
    }
    checks.push(check);
}

fn print_doctor_human(report: &Value) {
    println!(
        "doctor: {}",
        if report["ok"].as_bool().unwrap_or(false) {
            "ready"
        } else {
            "issues found"
        }
    );
    if let Some(checks) = report["checks"].as_array() {
        for check in checks {
            let marker = match check["status"].as_str().unwrap_or("error") {
                "ok" => "OK",
                "warn" => "WARN",
                "skip" => "SKIP",
                _ => "ERROR",
            };
            println!(
                "[{marker}] {}: {}",
                check["id"].as_str().unwrap_or("check"),
                check["message"].as_str().unwrap_or_default()
            );
            if let Some(hint) = check["hint"].as_str() {
                println!("      hint: {hint}");
            }
        }
    }
    println!(
        "summary: {} error(s), {} warning(s)",
        report["summary"]["errors"], report["summary"]["warnings"]
    );
}

fn read_diagnostic_tail(path: &Path) -> Option<String> {
    let bytes = fs::read(path).ok()?;
    let start = bytes.len().saturating_sub(8 * 1024);
    let tail = String::from_utf8_lossy(&bytes[start..]);
    Some(redact_diagnostic(&tail))
}

fn logs_vm(dirs: &Dirs, name: &str, max_lines: usize, output: OutputFormat) -> Result<()> {
    let vm = find(&dirs.vm_dir, &dirs.state_root, name)?;
    let path = vm.paths.state_dir.join("qemu.log");
    let (lines, truncated) = read_log_lines(&path, max_lines)?;
    if output == OutputFormat::Json {
        let returned_lines = lines.len();
        println!(
            "{}",
            json!({
                "name": vm.config.name,
                "path": path,
                "lines": lines,
                "returned_lines": returned_lines,
                "truncated": truncated,
            })
        );
    } else if lines.is_empty() {
        println!("{} is empty", path.display());
    } else {
        println!("Last {} line(s) from {}:", lines.len(), path.display());
        for line in lines {
            println!("{line}");
        }
    }
    Ok(())
}

fn read_log_lines(path: &Path, max_lines: usize) -> Result<(Vec<String>, bool)> {
    const MAX_LOG_BYTES: usize = 1024 * 1024;
    let bytes = fs::read(path).map_err(|error| Error::io(path.display(), error))?;
    let byte_start = bytes.len().saturating_sub(MAX_LOG_BYTES);
    let text = String::from_utf8_lossy(&bytes[byte_start..]);
    let mut lines: Vec<String> = text.lines().map(redact_diagnostic).collect();
    let line_truncated = lines.len() > max_lines;
    if line_truncated {
        let keep_from = lines.len() - max_lines;
        lines.drain(..keep_from);
    }
    Ok((lines, byte_start != 0 || line_truncated))
}

fn validate_usb_devices(vm: &Vm) -> Result<()> {
    if vm.config.usb_devices.is_empty() || env::consts::OS != "linux" {
        return Ok(());
    }
    if find_command("lsusb").is_none() {
        return Err(Error::message(
            "USB pass-through requires lsusb; install the usbutils package and retry",
        ));
    }
    for (vendor, product) in &vm.config.usb_devices {
        let device = format!("{vendor:04x}:{product:04x}");
        let found = ProcessCommand::new("lsusb")
            .args(["-d", &device])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success());
        if !found {
            return Err(Error::message(format!(
                "USB device {device} is not present or accessible; connect it or remove usb_devices"
            )));
        }
    }
    Ok(())
}

fn redact_diagnostic(value: &str) -> String {
    let mut redacted = value.to_string();
    for key in ["osk=", "password=", "secret=", "token="] {
        let mut search_from = 0;
        while let Some(relative_start) = redacted[search_from..].find(key) {
            let start = search_from + relative_start;
            let value_start = start + key.len();
            let value_end = redacted[value_start..]
                .find(|character: char| character == ',' || character.is_whitespace())
                .map_or(redacted.len(), |offset| value_start + offset);
            if value_end == value_start {
                search_from = value_start;
                continue;
            }
            redacted.replace_range(value_start..value_end, "<redacted>");
            search_from = value_start + "<redacted>".len();
        }
    }
    redacted
}

fn host_action(action: HostAction, output: OutputFormat) -> Result<()> {
    match action {
        HostAction::IgnoreMsrsAlways => configure_ignore_msrs(output, true),
    }
}

fn configure_ignore_msrs(output: OutputFormat, report: bool) -> Result<()> {
    if env::consts::OS != "linux" {
        return Err(Error::message(
            "persistent KVM MSR settings are only supported on Linux",
        ));
    }
    let path = Path::new("/etc/modprobe.d/vmctl-kvm.conf");
    if path
        .symlink_metadata()
        .ok()
        .is_some_and(|metadata| metadata.file_type().is_symlink())
    {
        return Err(Error::message(format!(
            "refusing to write through symlink {}",
            path.display()
        )));
    }
    let existing = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(Error::io(path.display(), error)),
    };
    let setting = "options kvm ignore_msrs=Y";
    let already_configured = existing.lines().any(|line| line.trim() == setting);
    let used_sudo = if already_configured {
        false
    } else {
        let contents = if existing.is_empty() {
            format!("{setting}\n")
        } else {
            format!("{}\n{setting}\n", existing.trim_end())
        };
        write_host_file(path, &contents)?
    };

    let initramfs = if already_configured {
        "already configured"
    } else if let Some(command) = find_command("update-initramfs") {
        let mut process = if used_sudo {
            let mut process = ProcessCommand::new("sudo");
            process.arg(&command);
            process
        } else {
            ProcessCommand::new(&command)
        };
        let status = process
            .args(["-k", "all", "-u"])
            .status()
            .map_err(|error| Error::command_unavailable(&command, error))?;
        if !status.success() {
            return Err(Error::command_failed_status(&command, status));
        }
        "rebuilt"
    } else if let Some(command) = find_command("mkinitcpio") {
        let mut process = if used_sudo {
            let mut process = ProcessCommand::new("sudo");
            process.arg(&command);
            process
        } else {
            ProcessCommand::new(&command)
        };
        let status = process
            .arg("-P")
            .status()
            .map_err(|error| Error::command_unavailable(&command, error))?;
        if !status.success() {
            return Err(Error::command_failed_status(&command, status));
        }
        "rebuilt with mkinitcpio"
    } else {
        "not available; reboot or rebuild initramfs manually"
    };

    if !report {
        return Ok(());
    }
    if output == OutputFormat::Json {
        println!(
            "{}",
            json!({
                "path": path,
                "configured": true,
                "initramfs": initramfs,
            })
        );
    } else {
        println!("Configured {}", path.display());
        println!("initramfs: {initramfs}");
    }
    Ok(())
}

fn write_host_file(path: &Path, contents: &str) -> Result<bool> {
    match fs::write(path, contents) {
        Ok(()) => Ok(false),
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
            let mut child = ProcessCommand::new("sudo")
                .args(["tee", path.to_string_lossy().as_ref()])
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::inherit())
                .spawn()
                .map_err(|error| Error::command_unavailable("sudo", error))?;
            child
                .stdin
                .take()
                .ok_or_else(|| Error::message("sudo did not provide stdin"))?
                .write_all(contents.as_bytes())
                .map_err(|error| Error::io(path.display(), error))?;
            let status = child
                .wait()
                .map_err(|error| Error::command_unavailable("sudo", error))?;
            if status.success() {
                Ok(true)
            } else {
                Err(Error::command_failed_status("sudo tee", status))
            }
        }
        Err(error) => Err(Error::io(path.display(), error)),
    }
}

fn find_command(command: &str) -> Option<String> {
    let path = env::var_os("PATH")?;
    #[cfg(windows)]
    let names = {
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
    };
    #[cfg(not(windows))]
    let names = [command.to_string()];
    env::split_paths(&path).find_map(|directory| {
        names
            .iter()
            .map(|name| directory.join(name))
            .find(|candidate| candidate.is_file())
            .map(|candidate| candidate.to_string_lossy().into_owned())
    })
}

fn load_effective_vm(dirs: &Dirs, name: &str, options: &LaunchOptions) -> Result<Vm> {
    let mut vm = find(&dirs.vm_dir, &dirs.state_root, name)?;
    apply_launch_options(&mut vm, options)?;
    Ok(vm)
}

fn apply_launch_options(vm: &mut Vm, options: &LaunchOptions) -> Result<()> {
    let config = &mut vm.config;
    if let Some(value) = &options.display {
        config.display = value.to_ascii_lowercase();
    }
    if let Some(value) = &options.viewer {
        config.viewer = value.to_ascii_lowercase();
    }
    if let Some(value) = &options.access {
        config.access = value.to_ascii_lowercase();
    }
    config.allow_insecure_remote |= options.allow_insecure_remote;
    if let Some(value) = &options.ssh_access {
        config.ssh_access = value.to_ascii_lowercase();
    }
    if options.braille {
        config.braille = true;
        config.display = "sdl".to_string();
        config.usb_controller = "xhci".to_string();
    }
    config.fullscreen |= options.fullscreen;
    config.offline |= options.offline;
    config.status_quo |= options.status_quo;
    config.ignore_tsc_warning |= options.ignore_tsc_warning;
    if let Some(value) = &options.cpu_pinning {
        validate_cpu_pinning(value)?;
        config.cpu_pinning = Some(value.clone());
    }
    if options.width.is_some() || options.height.is_some() {
        config.width = options.width.or(config.width);
        config.height = options.height.or(config.height);
    }
    if let Some(value) = options.ssh_port {
        config.ssh_port = Some(value);
    }
    if let Some(value) = options.spice_port {
        config.spice_port = Some(value);
    }
    config
        .viewer_extra_args
        .extend(options.viewer_extra_args.clone());
    if let Some(value) = &options.public_dir {
        config.public_dir = if value == Path::new("none") {
            None
        } else {
            Some(cli_path(value)?)
        };
    }
    if let Some(value) = &options.monitor {
        config.monitor = value.to_ascii_lowercase();
    }
    if let Some(value) = &options.monitor_cmd {
        config.monitor_cmd = Some(value.clone());
    }
    if let Some(value) = &options.monitor_telnet_host {
        config.monitor_telnet_host = value.clone();
    }
    if let Some(value) = options.monitor_telnet_port {
        config.monitor_telnet_port = value;
    }
    if let Some(value) = &options.serial {
        config.serial = value.to_ascii_lowercase();
    }
    if let Some(value) = &options.serial_telnet_host {
        config.serial_telnet_host = value.clone();
    }
    if let Some(value) = options.serial_telnet_port {
        config.serial_telnet_port = value;
    }
    if let Some(value) = &options.keyboard {
        config.keyboard = value.to_ascii_lowercase();
    }
    if let Some(value) = &options.keyboard_layout {
        config.keyboard_layout = value.clone();
    }
    if let Some(value) = &options.mouse {
        config.mouse = value.to_ascii_lowercase();
    }
    if let Some(value) = &options.usb_controller {
        config.usb_controller = value.to_ascii_lowercase();
    }
    if let Some(value) = &options.sound_card {
        config.sound_card = value.to_ascii_lowercase();
    }
    if let Some(value) = &options.sound_duplex {
        config.sound_duplex = value.to_ascii_lowercase();
    }
    if config.sound_card == "usb-audio" {
        config.usb_controller = "xhci".to_string();
    }
    config.extra_args.extend(options.extra_args.clone());
    config.validate()
}

fn print_plan(plan: &qemu::QemuPlan, output: OutputFormat, redact: bool) {
    let args = redact_plan_args(&plan.args, redact);
    if output == OutputFormat::Json {
        println!(
            "{}",
            json!({
                "binary": plan.binary,
                "args": args,
                "command": shell_join(&plan.binary, &args),
                "ssh_port": plan.ssh_port,
                "ssh_host": plan.ssh_host,
                "spice_port": plan.spice_port,
                "spice_host": plan.spice_host,
                "monitor_telnet": plan.monitor_telnet.as_ref().map(|(host, port)| json!({"host": host, "port": port})),
                "serial_telnet": plan.serial_telnet.as_ref().map(|(host, port)| json!({"host": host, "port": port})),
                "redacted": redact,
            })
        );
        return;
    }
    println!("{}", shell_join(&plan.binary, &args));
    if let Some(port) = plan.ssh_port {
        println!("ssh_port={port}");
    }
    if let Some(port) = plan.spice_port {
        println!("spice_port={port}");
    }
    if let Some((host, port)) = &plan.monitor_telnet {
        println!("monitor_telnet={host}:{port}");
    }
    if let Some((host, port)) = &plan.serial_telnet {
        println!("serial_telnet={host}:{port}");
    }
}

fn redact_plan_args(args: &[String], redact: bool) -> Vec<String> {
    if !redact {
        return args.to_vec();
    }
    let mut redact_next = false;
    args.iter()
        .map(|arg| {
            if redact_next {
                redact_next = false;
                return "<redacted>".to_string();
            }
            if matches!(arg.as_str(), "--password" | "--secret" | "--token") {
                redact_next = true;
                return arg.clone();
            }
            redact_inline_value(arg)
        })
        .collect()
}

fn redact_inline_value(value: &str) -> String {
    for key in ["osk=", "password=", "secret=", "token="] {
        if let Some(start) = value.find(key) {
            let end = value[start + key.len()..]
                .find(',')
                .map_or(value.len(), |offset| start + key.len() + offset);
            return format!("{}<redacted>{}", &value[..start + key.len()], &value[end..]);
        }
    }
    value.to_string()
}

fn vm_summary(vm: &Vm) -> Result<Value> {
    let (state, pid) = match vm.state()? {
        VmState::Running(pid) => ("running", Some(pid)),
        VmState::Stopped => ("stopped", None),
    };
    Ok(json!({
        "name": vm.config.name,
        "state": state,
        "pid": pid,
        "config": vm.config.config_path,
        "ssh_port": vm.config.ssh_port,
        "guest_os": vm.config.guest_os,
        "arch": vm.config.arch,
        "ssh_access": vm.config.ssh_access,
    }))
}

fn vm_status(vm: &Vm) -> Result<Value> {
    let summary = vm_summary(vm)?;
    let ipc = ipc_report(&vm.paths)?;
    let qmp_status = if summary["state"] == "running" {
        match qmp_status(&vm.paths) {
            Ok(status) => json!({"reachable": true, "status": status}),
            Err(error) => json!({
                "reachable": false,
                "status": null,
                "error": error.to_string(),
            }),
        }
    } else {
        json!({"reachable": false, "status": "stopped"})
    };
    Ok(json!({
        "name": vm.config.name,
        "state": summary["state"].clone(),
        "pid": summary["pid"].clone(),
        "config": vm.config.config_path,
        "state_dir": vm.paths.state_dir,
        "guest_os": vm.config.guest_os,
        "arch": vm.config.arch,
        "display": vm.config.display,
        "disk": vm.config.disk_img,
        "disk_size": vm.config.disk_size,
        "boot": vm.config.boot,
        "ssh_port": vm.config.ssh_port,
        "ssh_access": vm.config.ssh_access,
        "ipc": ipc,
        "qmp_status": qmp_status,
        "monitor": vm.paths.monitor_socket(),
        "serial": vm.paths.serial_socket(),
    }))
}

fn state_label(vm: &Vm) -> Result<String> {
    Ok(match vm.state()? {
        VmState::Running(pid) => format!("running({pid})"),
        VmState::Stopped => "stopped".to_string(),
    })
}

fn print_vm_status(vm: &Vm) -> Result<()> {
    let ipc = ipc_report(&vm.paths)?;
    let guest_agent = if ipc["guest_agent"].is_null() {
        "disabled".to_string()
    } else {
        ipc_endpoint_label(&ipc["guest_agent"])
    };
    println!("name:        {}", vm.config.name);
    println!("state:       {}", state_label(vm)?);
    println!("config:      {}", vm.config.config_path.display());
    println!("state dir:   {}", vm.paths.state_dir.display());
    println!("guest os:    {}", vm.config.guest_os);
    println!("arch:        {}", vm.config.arch);
    println!("display:     {}", vm.config.display);
    println!("disk:        {}", vm.config.disk_img.display());
    println!("disk size:   {}", vm.config.disk_size);
    println!("boot:        {}", vm.config.boot);
    println!(
        "ssh port:    {}",
        vm.config
            .ssh_port
            .map_or_else(|| "auto".to_string(), |port| port.to_string())
    );
    println!("qmp:         {}", ipc_endpoint_label(&ipc["qmp"]));
    let qmp_state = match vm.state()? {
        VmState::Stopped => "stopped".to_string(),
        VmState::Running(_) => qmp_status(&vm.paths).unwrap_or_else(|_| "unavailable".to_string()),
    };
    println!("qmp state:   {qmp_state}");
    println!("monitor:     {}", vm.paths.monitor_socket().display());
    println!("guest agent: {guest_agent}");
    println!("serial:      {}", vm.paths.serial_socket().display());
    println!("runtime:     {}", vm.paths.state_dir.display());
    Ok(())
}

fn ipc_endpoint_label(value: &Value) -> String {
    match value.get("transport").and_then(Value::as_str) {
        Some("tcp") => format!(
            "tcp://{}:{}",
            value
                .get("host")
                .and_then(Value::as_str)
                .unwrap_or("127.0.0.1"),
            value
                .get("port")
                .and_then(Value::as_u64)
                .unwrap_or_default()
        ),
        Some("unix") => value
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or("unavailable")
            .to_string(),
        _ => "unavailable".to_string(),
    }
}

fn write_pid(vm: &Vm, pid: i32) -> Result<()> {
    let path = vm.paths.pid_file();
    let temporary = path.with_extension("pid.tmp");
    let identity = qemu::process_identity(pid).map_or_else(
        || format!("{pid}\n"),
        |identity| format!("{pid} {identity}\n"),
    );
    fs::write(&temporary, identity).map_err(|error| Error::io(temporary.display(), error))?;
    fs::rename(&temporary, &path).map_err(|error| Error::io(path.display(), error))
}

fn apply_cpu_pinning(pid: i32, pinning: &str) -> Result<()> {
    let status = ProcessCommand::new("taskset")
        .args(["-cp", pinning, &pid.to_string()])
        .status()
        .map_err(|error| Error::command_unavailable("taskset", error))?;
    if status.success() {
        Ok(())
    } else {
        Err(Error::command_failed_status("taskset", status))
    }
}

fn check_tsc_stability(vm: &Vm, quiet: bool) -> Result<()> {
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

fn tsc_warning_needed(
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

fn validate_cpu_pinning(value: &str) -> Result<()> {
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

fn validate_cpu_pinning_for_host(value: &str, host_os: &str, cpu_cores: u32) -> Result<()> {
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

fn host_cpu_id_available(id: u32) -> bool {
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

fn process_allowed_cpu_spec() -> Option<String> {
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

fn command_available(command: &str) -> bool {
    ProcessCommand::new(command)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn command_version(command: &str) -> Option<String> {
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

fn desktop_quote(path: &Path) -> String {
    let value = path.display().to_string();
    if value
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || "_./:-".contains(character))
    {
        value
    } else {
        format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
    }
}

fn launch_viewer(vm: &Vm, plan: &qemu::QemuPlan, quiet: bool) -> bool {
    if !matches!(vm.config.display.as_str(), "none" | "spice" | "spice-app")
        || vm.config.viewer == "none"
    {
        return false;
    }
    let mut command = ProcessCommand::new(&vm.config.viewer);
    if let Some(port) = plan.spice_port {
        if vm.config.viewer == "spicy" {
            if spice_address(&vm.config) == "127.0.0.1" {
                command.args(["--port", &port.to_string()]);
            } else {
                command.args([
                    "--host",
                    spice_address(&vm.config),
                    "--port",
                    &port.to_string(),
                ]);
            }
        } else {
            command.arg(format!("spice://{}:{port}", spice_address(&vm.config)));
        }
    } else {
        let uri = format!("spice+unix://{}", vm.paths.spice_socket().display());
        if vm.config.viewer == "spicy" {
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
    match command.spawn() {
        Ok(_) => true,
        Err(error) => {
            if !quiet {
                eprintln!(
                    "vmctl: viewer `{}` was not started: {error}",
                    vm.config.viewer
                );
            }
            false
        }
    }
}

fn reconnect_viewer(vm: &Vm, quiet: bool) -> bool {
    if !matches!(vm.config.display.as_str(), "none" | "spice" | "spice-app")
        || vm.config.viewer == "none"
    {
        return false;
    }
    let mut vm = vm.clone();
    if let Some(port) = runtime_port(&vm.paths.state_dir.join("ports"), "spice") {
        vm.config.spice_port = Some(port);
    }
    let Ok(host) = HostCapabilities::detect(&vm.config) else {
        return false;
    };
    let Ok(plan) = build_plan(&vm, &host, false) else {
        return false;
    };
    launch_viewer(&vm, &plan, quiet)
}

fn runtime_port(path: &Path, wanted: &str) -> Option<u16> {
    fs::read_to_string(path).ok()?.lines().find_map(|line| {
        let (name, port) = line.split_once(',')?;
        (name == wanted).then(|| port.parse().ok()).flatten()
    })
}

fn ensure_delete_allowed(vm: &Vm, yes: bool) -> Result<()> {
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

fn persistent_efi_vars(vm: &Vm) -> Vec<PathBuf> {
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

fn remove_if_present(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(Error::io(path.display(), error)),
    }
}

fn cli_path(path: &Path) -> Result<PathBuf> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use clap::{CommandFactory, Parser};

    #[test]
    fn command_line_parsing_keeps_vm_names_and_options() {
        let cli = Cli::try_parse_from([
            "vmctl",
            "plan",
            "ubuntu",
            "--display",
            "none",
            "--output",
            "json",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Some(VmCommand::Plan {
                vm,
                redact: false,
                options
            })
                if vm == "ubuntu" && options.display.as_deref() == Some("none")
        ));
        assert_eq!(cli.output, OutputFormat::Json);
    }

    #[test]
    fn command_line_parses_network_and_viewer_overrides() {
        let cli = Cli::try_parse_from([
            "vmctl",
            "start",
            "ubuntu",
            "--ssh-access",
            "remote",
            "--viewer-extra-args",
            "--foo",
            "bar",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Some(VmCommand::Start { options, .. })
                if options.ssh_access.as_deref() == Some("remote")
                    && options.viewer_extra_args == ["--foo", "bar"]
        ));
    }

    #[test]
    fn guest_exec_accepts_hyphenated_guest_arguments() {
        let cli = Cli::try_parse_from([
            "vmctl",
            "guest",
            "ubuntu",
            "exec",
            "/bin/sh",
            "-c",
            "echo hello",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Some(VmCommand::Guest {
                action: GuestAction::Exec { args, .. },
                ..
            }) if args == ["-c", "echo hello"]
        ));
    }

    #[test]
    fn guest_exec_rejects_zero_timeout_at_parse_time() {
        assert!(
            Cli::try_parse_from([
                "vmctl",
                "guest",
                "ubuntu",
                "exec",
                "--timeout",
                "0",
                "/bin/true",
            ])
            .is_err()
        );
    }

    #[test]
    fn stop_and_restart_reject_invalid_timeouts_and_preserve_force() {
        assert!(Cli::try_parse_from(["vmctl", "stop", "ubuntu", "--timeout", "0"]).is_err());
        assert!(Cli::try_parse_from(["vmctl", "restart", "ubuntu", "--timeout", "86401"]).is_err());
        let cli = Cli::try_parse_from(["vmctl", "restart", "ubuntu", "--timeout", "30", "--force"])
            .unwrap();
        assert!(matches!(
            cli.command,
            Some(VmCommand::Restart {
                timeout: 30,
                force: true,
                ..
            })
        ));
    }

    #[test]
    fn logs_command_bounds_lines() {
        assert!(Cli::try_parse_from(["vmctl", "logs", "ubuntu", "--lines", "0"]).is_err());
        let cli = Cli::try_parse_from(["vmctl", "logs", "ubuntu", "--lines", "2"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(VmCommand::Logs { vm, lines }) if vm == "ubuntu" && lines == 2
        ));
    }

    #[test]
    fn log_tail_is_bounded_and_redacted() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("qemu.log");
        fs::write(
            &path,
            "first\npassword=secret secret=hidden\nlast token=private\n",
        )
        .unwrap();

        let (lines, truncated) = read_log_lines(&path, 2).unwrap();

        assert!(truncated);
        assert_eq!(
            lines,
            [
                "password=<redacted> secret=<redacted>",
                "last token=<redacted>"
            ]
        );
    }

    #[test]
    fn efi_vars_outside_vm_data_are_not_owned_by_the_vm() {
        let root = tempfile::tempdir().unwrap();
        let config = root.path().join("demo.conf");
        fs::write(&config, "disk_img=\"shared/disk.qcow2\"\n").unwrap();
        let vm = find(root.path(), root.path(), "demo").unwrap();

        assert_eq!(
            persistent_efi_vars(&vm),
            vec![root.path().join("shared/demo-vars.fd")]
        );
    }

    #[test]
    fn no_command_defaults_to_list() {
        let cli = Cli::try_parse_from(["vmctl"]).unwrap();
        assert!(cli.command.is_none());
    }

    #[test]
    fn help_guides_first_use_and_groups_launch_options() {
        let mut command = Cli::command();
        let root_help = command.render_long_help().to_string();
        assert!(root_help.contains("Examples:"));
        assert!(root_help.contains("vmctl get ubuntu 24.04"));
        assert!(!root_help.contains("--redact"));
        assert!(!root_help.contains("--ignore-msrs-always"));

        let start_help = command
            .find_subcommand_mut("start")
            .unwrap()
            .render_long_help()
            .to_string();
        let headings = [
            "Display:",
            "Networking and sharing:",
            "Devices:",
            "Advanced:",
        ];
        for heading in headings {
            assert!(start_help.contains(heading));
        }
        let positions = headings.map(|heading| start_help.find(heading).unwrap());
        assert!(positions.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(start_help.contains("gtk, sdl, spice, spice-app, none"));
    }

    #[test]
    fn get_and_host_commands_are_typed() {
        let cli = Cli::try_parse_from(["vmctl", "get", "--url", "ubuntu", "24.04"]).unwrap();
        assert!(matches!(cli.command, Some(VmCommand::Get(_))));

        let cli = Cli::try_parse_from([
            "vmctl",
            "get",
            "--insecure",
            "--download",
            "ubuntu",
            "24.04",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Some(VmCommand::Get(args)) if args.insecure && args.download
        ));

        let cli = Cli::try_parse_from(["vmctl", "host", "ignore-msrs-always"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(VmCommand::Host {
                action: HostAction::IgnoreMsrsAlways
            })
        ));

        let cli = Cli::try_parse_from([
            "vmctl",
            "disk",
            "ubuntu",
            "convert",
            "ubuntu.raw",
            "--format",
            "raw",
            "--force",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Some(VmCommand::Disk {
                vm,
                action: DiskAction::Convert {
                    destination,
                    format: Some(format),
                    force: true,
                    ..
                }
            }) if vm == "ubuntu"
                && destination == Path::new("ubuntu.raw")
                && format == "raw"
        ));
    }

    #[test]
    fn cpu_pinning_validation_rejects_shell_text() {
        assert!(validate_cpu_pinning("0,2,4").is_ok());
        assert!(validate_cpu_pinning("0; reboot").is_err());
    }

    #[test]
    fn cpu_pinning_matches_vcpu_count_and_host() {
        if !host_cpu_id_available(1) {
            return;
        }
        assert!(validate_cpu_pinning_for_host("0,1", "linux", 2).is_ok());
        assert!(validate_cpu_pinning_for_host("0,0", "linux", 2).is_err());
        assert!(validate_cpu_pinning_for_host("0", "linux", 2).is_err());
        assert!(validate_cpu_pinning_for_host("999999", "linux", 1).is_err());
        assert!(validate_cpu_pinning_for_host("0,1", "macos", 2).is_err());
    }

    #[test]
    fn plan_redaction_removes_inline_and_next_argument_secrets() {
        let args = vec![
            "isa-applesmc,osk=private-key,other=value".to_string(),
            "--token".to_string(),
            "private-token".to_string(),
        ];
        assert_eq!(
            redact_plan_args(&args, true),
            [
                "isa-applesmc,osk=<redacted>,other=value",
                "--token",
                "<redacted>"
            ]
        );
    }

    #[test]
    fn tsc_warning_only_applies_to_risky_macos_hosts() {
        assert!(tsc_warning_needed(
            "linux",
            true,
            "macos",
            Some("ventura"),
            "hpet",
            "quiet"
        ));
        assert!(!tsc_warning_needed(
            "linux",
            true,
            "macos",
            Some("ventura"),
            "tsc",
            "quiet"
        ));
        assert!(!tsc_warning_needed(
            "linux",
            false,
            "macos",
            Some("ventura"),
            "hpet",
            "quiet"
        ));
    }
}
