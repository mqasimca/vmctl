use super::*;

pub(super) fn monitor_vm(
    dirs: &Dirs,
    name: &str,
    command: &[String],
    output: OutputFormat,
) -> Result<()> {
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
        print_json_success(
            json!({"name": vm.config.name, "command": command, "response": response}),
        );
    } else if !response.is_empty() {
        println!("{response}");
    }
    Ok(())
}

pub(super) fn guest_vm(
    dirs: &Dirs,
    name: &str,
    action: GuestAction,
    output: OutputFormat,
) -> Result<()> {
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
                let _ = fs::remove_file(vm.paths.pid_file());
                stop_tpm(&vm.paths);
                stop_virtiofsd(&vm.paths);
                remove_runtime_sockets(&vm.paths);
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
        GuestAction::FreezeStatus => (
            "guest-fsfreeze-status",
            json!({"status": guest_fsfreeze_status(&vm)?}),
        ),
        GuestAction::Freeze => (
            "guest-fsfreeze-freeze",
            json!({"frozen_filesystems": guest_fsfreeze_freeze(&vm)?}),
        ),
        GuestAction::Thaw => (
            "guest-fsfreeze-thaw",
            json!({"thawed_filesystems": guest_fsfreeze_thaw(&vm)?}),
        ),
        GuestAction::Trim => ("guest-fstrim", guest_fstrim(&vm)?),
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
        print_json_success(json!({"name": vm.config.name, "command": command, "result": result}));
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&result).unwrap_or_default()
        );
    }
    Ok(())
}
