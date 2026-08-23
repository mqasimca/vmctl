use super::*;

pub(super) fn doctor(dirs: &Dirs, name: Option<&str>, output: OutputFormat) -> Result<()> {
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
    let iso_builder = ["xorriso", "mkisofs", "genisoimage"]
        .into_iter()
        .find(|command| command_available(command));
    push_doctor_check(
        &mut checks,
        "host.cloud_init_iso_builder",
        if iso_builder.is_some() { "ok" } else { "warn" },
        iso_builder.map_or_else(
            || "cloud VM creation needs xorriso, mkisofs, or genisoimage".to_string(),
            |command| format!("{command} is available for cloud-init seed images"),
        ),
        iso_builder.is_none().then_some(
            "Install xorriso, mkisofs, or genisoimage before using `vmctl get --cloud`.",
        ),
        None,
    );

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
    let passt_backend = qemu_capabilities["network_backends"]
        .as_array()
        .is_some_and(|backends| backends.iter().any(|backend| backend == "passt"));
    let passt_path = find_command("passt");
    let (status, message, hint) = if env::consts::OS != "linux" {
        (
            "skip",
            "passt networking is currently documented for Linux hosts".to_string(),
            None,
        )
    } else if qemu_capabilities["available"] != true {
        (
            "skip",
            "QEMU is unavailable; passt was not checked".to_string(),
            None,
        )
    } else if !passt_backend {
        (
            "warn",
            "QEMU does not support the passt network backend".to_string(),
            Some("Install QEMU 10.1 or newer, or use network=user."),
        )
    } else if let Some(path) = passt_path {
        ("ok", format!("passt is available at {path}"), None)
    } else {
        (
            "warn",
            "QEMU supports passt, but the passt executable is unavailable".to_string(),
            Some("Install passt, or use network=user."),
        )
    };
    push_doctor_check(
        &mut checks,
        "host.network.passt",
        status,
        message,
        hint,
        Some(json!({"qemu_backend": passt_backend})),
    );

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
        if let Some(bridge) = configured_bridge(&vm.config) {
            let (status, message, hint) = if env::consts::OS != "linux" {
                (
                    "skip",
                    format!(
                        "bridge {bridge} was configured; Linux bridge inspection is unavailable"
                    ),
                    None,
                )
            } else if !is_linux_bridge(Path::new("/sys/class/net"), bridge) {
                (
                    "error",
                    format!("configured bridge {bridge} does not exist or is not a Linux bridge"),
                    Some("Create the bridge, attach a host interface, or use network=user."),
                )
            } else if !bridge_helper {
                (
                    "error",
                    format!("qemu-bridge-helper is required by bridge {bridge} but is unavailable"),
                    Some("Install qemu-bridge-helper or use network=user."),
                )
            } else {
                (
                    "warn",
                    format!(
                        "Linux bridge {bridge} is present, but qemu-bridge-helper policy is not verified"
                    ),
                    Some("Ensure the qemu-bridge-helper policy allows this bridge."),
                )
            };
            push_doctor_check(&mut checks, "vm.bridge", status, message, hint, None);
        }
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
        if vm.config.network.eq_ignore_ascii_case("passt") {
            let qemu_passt = vm_qemu_capabilities["network_backends"]
                .as_array()
                .is_some_and(|backends| backends.iter().any(|backend| backend == "passt"));
            let passt_path = find_command("passt");
            let (status, message, hint) = if env::consts::OS != "linux" {
                (
                    "error",
                    "network=passt is currently supported only on Linux hosts".to_string(),
                    Some("Set network=user or run this VM on a Linux host."),
                )
            } else if !qemu_passt {
                (
                    "error",
                    format!("{vm_qemu_binary} does not support network=passt"),
                    Some("Install QEMU 10.1 or newer, or set network=user."),
                )
            } else if let Some(path) = passt_path {
                ("ok", format!("network=passt can use {path}"), None)
            } else {
                (
                    "error",
                    "network=passt requires the passt executable".to_string(),
                    Some("Install passt, or set network=user."),
                )
            };
            push_doctor_check(
                &mut checks,
                "vm.network.passt",
                status,
                message,
                hint,
                Some(json!({"qemu_backend": qemu_passt})),
            );
        }
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
            ("vm.cloud_base_img", vm.config.cloud_base_img.as_ref()),
            ("vm.cloud_init_iso", vm.config.cloud_init_iso.as_ref()),
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
        "ok": errors == 0,
        "scope": {"vm": name},
        "checks": checks,
        "summary": {"errors": errors, "warnings": warnings},
    });

    if output == OutputFormat::Json {
        if errors == 0 {
            print_json_success(report.clone());
        }
    } else {
        print_doctor_human(&report);
    }
    if errors > 0 {
        return Err(Error::doctor_failed(errors, warnings, report));
    }
    Ok(())
}

pub(super) fn is_linux_bridge(network_root: &Path, bridge: &str) -> bool {
    network_root.join(bridge).join("bridge").is_dir()
}

pub(super) fn push_doctor_check(
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

pub(super) fn print_doctor_human(report: &Value) {
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

pub(super) fn read_diagnostic_tail(path: &Path) -> Option<String> {
    let (bytes, _) = read_file_tail(path, 8 * 1024).ok()?;
    let tail = String::from_utf8_lossy(&bytes);
    Some(redact_diagnostic(&tail))
}
