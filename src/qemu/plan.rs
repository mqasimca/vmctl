use super::*;

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

pub(super) fn ipc_endpoints(
    vm: &Vm,
    host: &QemuPlanContext,
) -> Result<(IpcEndpoint, Option<IpcEndpoint>)> {
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
pub(super) fn named_pipe_endpoint(role: &str) -> IpcEndpoint {
    let nonce = next_guest_sync_id().unsigned_abs();
    IpcEndpoint::Pipe(PathBuf::from(format!(
        r"\\.\pipe\vmctl-{role}-{nonce:016x}"
    )))
}

pub(super) fn ephemeral_loopback_endpoint(excluded: &[IpcEndpoint]) -> Result<IpcEndpoint> {
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

pub(super) fn read_ipc_state(paths: &VmPaths) -> Result<(IpcEndpoint, Option<IpcEndpoint>)> {
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

pub(super) fn default_qmp_endpoint(paths: &VmPaths) -> Result<IpcEndpoint> {
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

pub(super) fn default_agent_endpoint(paths: &VmPaths) -> Result<IpcEndpoint> {
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

pub(super) fn qmp_endpoint_for_paths(paths: &VmPaths) -> Result<IpcEndpoint> {
    if paths.ipc_state().is_file() {
        read_ipc_state(paths).map(|state| state.0)
    } else {
        default_qmp_endpoint(paths)
    }
}

pub(super) fn agent_endpoint_for_paths(paths: &VmPaths) -> Result<IpcEndpoint> {
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

pub(super) fn machine_type(config: &VmConfig) -> &'static str {
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

pub(super) fn add_guest_tweaks(args: &mut Vec<String>, config: &VmConfig, machine: &str) {
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

pub(super) fn cpu_model(config: &VmConfig, host: &QemuPlanContext) -> String {
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
