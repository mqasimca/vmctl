use super::*;
use clap::{CommandFactory, Parser};
use std::net::TcpListener;

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
fn command_line_parses_schema() {
    let cli = Cli::try_parse_from(["vmctl", "schema"]).unwrap();
    assert!(matches!(cli.command, Some(VmCommand::Schema)));
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
fn command_line_parses_ssh_readiness_wait() {
    let cli = Cli::try_parse_from([
        "vmctl",
        "start",
        "ubuntu",
        "--wait",
        "ssh",
        "--wait-timeout",
        "3",
    ])
    .unwrap();
    assert!(matches!(
        cli.command,
        Some(VmCommand::Start {
            wait: Some(StartWait::Ssh),
            wait_timeout: 3,
            ..
        })
    ));
}

#[test]
fn command_line_parses_cloud_lifecycle_and_cache_commands() {
    let cli = Cli::try_parse_from(["vmctl", "start", "cloud", "--wait", "cloud-init"]).unwrap();
    assert!(matches!(
        cli.command,
        Some(VmCommand::Start {
            wait: Some(StartWait::CloudInit),
            ..
        })
    ));

    let cli = Cli::try_parse_from([
        "vmctl",
        "create",
        "cloud",
        "--from",
        "base.qcow2",
        "--user-data",
        "cloud.yaml",
    ])
    .unwrap();
    assert!(matches!(
        cli.command,
        Some(VmCommand::Create(args))
            if args.user_data == Some(PathBuf::from("cloud.yaml"))
    ));

    let cli = Cli::try_parse_from(["vmctl", "cache", "prune", "--yes"]).unwrap();
    assert!(matches!(
        cli.command,
        Some(VmCommand::Cache {
            action: CacheAction::Prune { yes: true }
        })
    ));
    let cli = Cli::try_parse_from(["vmctl", "backup", "cloud", "backup-dir"]).unwrap();
    assert!(matches!(
        cli.command,
        Some(VmCommand::Backup { vm, destination })
            if vm == "cloud" && destination == *"backup-dir"
    ));
    let cli = Cli::try_parse_from(["vmctl", "reset", "cloud", "--yes"]).unwrap();
    assert!(matches!(
        cli.command,
        Some(VmCommand::Reset { vm, yes: true }) if vm == "cloud"
    ));
    let cli = Cli::try_parse_from(["vmctl", "guest", "cloud", "trim"]).unwrap();
    assert!(matches!(
        cli.command,
        Some(VmCommand::Guest {
            action: GuestAction::Trim,
            ..
        })
    ));
}

#[test]
fn cloud_init_wait_rejects_a_non_cloud_vm_before_starting() {
    let root = tempfile::tempdir().unwrap();
    let dirs = Dirs {
        vm_dir: root.path().join("vms"),
        state_root: root.path().join("state"),
    };
    std::fs::create_dir(&dirs.vm_dir).unwrap();
    std::fs::write(
        dirs.vm_dir.join("vm.conf"),
        "boot=legacy\nnetwork=none\npublic_dir=none\n",
    )
    .unwrap();

    assert!(
        start_vm(
            &dirs,
            "vm",
            &LaunchOptions::default(),
            Some(StartWait::CloudInit),
            1,
            OutputFormat::Human,
        )
        .unwrap_err()
        .to_string()
        .contains("requires a VM configured with cloud_init_iso")
    );
}

#[test]
fn backup_and_reset_keep_their_destructive_boundaries() {
    let root = tempfile::tempdir().unwrap();
    let dirs = Dirs {
        vm_dir: root.path().join("vms"),
        state_root: root.path().join("state"),
    };
    std::fs::create_dir_all(&dirs.vm_dir).unwrap();
    std::fs::write(dirs.vm_dir.join("vm.conf"), "disk_img=\"vm/disk.qcow2\"\n").unwrap();
    let destination = root.path().join("backup");
    std::fs::create_dir(&destination).unwrap();

    assert!(
        backup_vm(&dirs, "vm", &destination, OutputFormat::Human)
            .unwrap_err()
            .to_string()
            .contains("already exists")
    );
    assert!(
        reset_cloud_vm(&dirs, "vm", false, OutputFormat::Human)
            .unwrap_err()
            .to_string()
            .contains("rerun with --yes")
    );
}

#[test]
fn command_line_parses_gtk_clipboard_option() {
    let cli = Cli::try_parse_from(["vmctl", "start", "ubuntu", "--clipboard"]).unwrap();
    assert!(matches!(
        cli.command,
        Some(VmCommand::Start { options, .. }) if options.clipboard
    ));
}

#[test]
fn command_line_parses_ssh_user() {
    let cli = Cli::try_parse_from(["vmctl", "ssh", "freebsd", "--user", "root"]).unwrap();
    assert!(matches!(
        cli.command,
        Some(VmCommand::Ssh { vm, user }) if vm == "freebsd" && user.as_deref() == Some("root")
    ));
}

#[test]
fn command_line_parses_viewer_override() {
    let cli = Cli::try_parse_from(["vmctl", "view", "freebsd", "--viewer", "spicy"]).unwrap();
    assert!(matches!(
        cli.command,
        Some(VmCommand::View { vm, viewer }) if vm == "freebsd" && viewer.as_deref() == Some("spicy")
    ));
}

#[test]
fn set_command_parses_persistent_settings() {
    let cli = Cli::try_parse_from([
        "vmctl",
        "set",
        "ubuntu",
        "--ram",
        "2G",
        "--cpu-cores",
        "2",
        "--disk-size",
        "32G",
        "--cpu-model",
        "host",
        "--cpu-pinning",
        "0,1",
        "--macaddr",
        "52:54:00:12:34:56",
        "--port-forward",
        "8080:80",
        "--boot-menu",
        "on",
        "--boot-once",
        "cdrom",
        "--disk-cache",
        "none",
        "--disk-aio",
        "io_uring",
        "--discard",
        "ignore",
    ])
    .unwrap();
    assert!(matches!(
        cli.command,
        Some(VmCommand::Set {
            vm,
            ram: Some(ram),
            cpu_cores: Some(2),
            disk_size: Some(disk_size),
            cpu_model: Some(cpu_model),
            cpu_pinning: Some(cpu_pinning),
            macaddr: Some(macaddr),
            port_forwards,
            boot_menu: Some(boot_menu),
            boot_once: Some(boot_once),
            disk_cache: Some(disk_cache),
            disk_aio: Some(disk_aio),
            discard: Some(discard),
            ..
        }) if vm == "ubuntu" && ram == "2G" && disk_size == "32G"
            && cpu_model == "host" && cpu_pinning == "0,1"
            && macaddr == "52:54:00:12:34:56" && port_forwards == ["8080:80"]
            && boot_menu == "on" && boot_once == "cdrom" && disk_cache == "none"
            && disk_aio == "io_uring" && discard == "ignore"
    ));
}

#[test]
fn status_command_parses_live() {
    let cli = Cli::try_parse_from(["vmctl", "status", "ubuntu", "--live"]).unwrap();
    assert!(matches!(
        cli.command,
        Some(VmCommand::Status { vm: Some(vm), live: true }) if vm == "ubuntu"
    ));
    assert!(Cli::try_parse_from(["vmctl", "status", "--live"]).is_err());
}

#[test]
fn trailing_commands_do_not_consume_global_options() {
    let cli = Cli::try_parse_from([
        "vmctl",
        "monitor",
        "ubuntu",
        "info status",
        "--output",
        "json",
    ])
    .unwrap();
    assert_eq!(cli.output, OutputFormat::Json);
    assert!(matches!(
        cli.command,
        Some(VmCommand::Monitor { command, .. }) if command == ["info status"]
    ));

    let cli = Cli::try_parse_from([
        "vmctl",
        "guest",
        "ubuntu",
        "exec",
        "/bin/echo",
        "hello",
        "--output",
        "json",
    ])
    .unwrap();
    assert_eq!(cli.output, OutputFormat::Json);
    assert!(matches!(
        cli.command,
        Some(VmCommand::Guest {
            action: GuestAction::Exec { args, .. },
            ..
        }) if args == ["hello"]
    ));
}

#[test]
fn vm_ssh_does_not_persist_host_keys() {
    let options = vm_ssh_options();
    let null_device = if cfg!(windows) { "NUL" } else { "/dev/null" };
    assert!(options.contains(&format!("UserKnownHostsFile={null_device}")));
    assert!(options.contains(&format!("GlobalKnownHostsFile={null_device}")));
    assert!(options.contains(&"StrictHostKeyChecking=no".to_string()));
}

#[test]
fn ssh_readiness_requires_an_ssh_banner() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream.write_all(b"SSH-2.0-vmctl-test\r\n").unwrap();
    });

    assert!(has_ssh_banner(address, Duration::from_secs(1)));
    server.join().unwrap();
}

#[test]
fn spice_tcp_uri_brackets_ipv6_hosts() {
    assert_eq!(spice_tcp_uri("::1", 5930), "spice://[::1]:5930");
    assert_eq!(spice_tcp_uri("127.0.0.1", 5930), "spice://127.0.0.1:5930");
    assert_eq!(connect_host("0.0.0.0"), "127.0.0.1");
    assert_eq!(connect_host("remote"), "127.0.0.1");
    assert_eq!(connect_host("::"), "::1");
}

#[test]
fn desktop_exec_escapes_field_code_percent_signs() {
    assert_eq!(
        desktop_exec_quote(Path::new("/tmp/vm%f config")),
        "\"/tmp/vm%%f config\""
    );
    assert_eq!(
        desktop_quote(Path::new("/tmp/vm%f config")),
        "\"/tmp/vm%f config\""
    );
}

#[cfg(unix)]
#[test]
fn shortcut_refuses_a_symlink_without_touching_its_target() {
    let root = tempfile::tempdir().unwrap();
    fs::write(
        root.path().join("demo.conf"),
        "boot=legacy\nnetwork=none\npublic_dir=none\n",
    )
    .unwrap();
    let victim = root.path().join("victim.txt");
    fs::write(&victim, "keep me\n").unwrap();
    let shortcut = root.path().join("demo.desktop");
    std::os::unix::fs::symlink(&victim, &shortcut).unwrap();
    let dirs = Dirs {
        vm_dir: root.path().to_path_buf(),
        state_root: root.path().join("state"),
    };

    assert!(shortcut_vm(&dirs, "demo", Some(shortcut), OutputFormat::Human).is_err());
    assert_eq!(fs::read_to_string(victim).unwrap(), "keep me\n");
}

#[cfg(unix)]
#[test]
fn runtime_file_writes_do_not_follow_symlinks() {
    let root = tempfile::tempdir().unwrap();
    fs::write(
        root.path().join("demo.conf"),
        "boot=legacy\nnetwork=none\npublic_dir=none\n",
    )
    .unwrap();
    let vm = find(root.path(), &root.path().join("state"), "demo").unwrap();
    fs::create_dir_all(&vm.paths.state_dir).unwrap();
    let victim = root.path().join("victim.txt");
    fs::write(&victim, "keep me\n").unwrap();
    let log = vm.paths.state_dir.join("qemu.log");
    std::os::unix::fs::symlink(&victim, &log).unwrap();

    assert!(qemu::create_truncated_file(&log).is_err());
    let pid_file = vm.paths.pid_file();
    std::os::unix::fs::symlink(&victim, &pid_file).unwrap();
    write_pid(&vm, i32::MAX).unwrap();

    assert_eq!(fs::read_to_string(victim).unwrap(), "keep me\n");
    assert!(
        !fs::symlink_metadata(pid_file)
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

#[test]
fn runtime_listeners_are_authoritative_and_preserve_hosts() {
    let root = tempfile::tempdir().unwrap();
    let ports = root.path().join("ports");
    fs::write(&ports, "spice,5930\nssh,22220,192.0.2.10\n").unwrap();
    assert_eq!(
        runtime_listener(&ports, "ssh").unwrap(),
        (true, Some((22220, Some("192.0.2.10".to_string()))))
    );
    assert_eq!(
        runtime_listener(&ports, "spice").unwrap(),
        (true, Some((5930, None)))
    );

    fs::write(&ports, "").unwrap();
    assert_eq!(runtime_listener(&ports, "ssh").unwrap(), (true, None));
}

#[test]
fn active_ssh_uses_runtime_absence_and_host_overrides() {
    let root = tempfile::tempdir().unwrap();
    fs::write(
        root.path().join("demo.conf"),
        "boot=legacy\nnetwork=user\nssh_port=22220\nssh_access=local\npublic_dir=none\n",
    )
    .unwrap();
    let vm = find(root.path(), &root.path().join("state"), "demo").unwrap();
    fs::create_dir_all(&vm.paths.state_dir).unwrap();
    let ports = vm.paths.state_dir.join("ports");

    fs::write(&ports, "").unwrap();
    assert!(active_ssh_endpoint(&vm).is_err());

    fs::write(&ports, "ssh,22333,192.0.2.10\n").unwrap();
    assert_eq!(
        active_ssh_endpoint(&vm).unwrap(),
        ("192.0.2.10".to_string(), 22333)
    );
    assert_eq!(
        runtime_ssh_host(&vm).unwrap(),
        Some("192.0.2.10".to_string())
    );
}

#[test]
fn launch_options_enable_gtk_clipboard() {
    let root = tempfile::tempdir().unwrap();
    let config_path = root.path().join("clipboard.conf");
    fs::write(
        &config_path,
        "boot=legacy\ndisplay=gtk\nnetwork=none\npublic_dir=none\n",
    )
    .unwrap();
    let mut vm = find(root.path(), root.path(), "clipboard").unwrap();

    apply_launch_options(
        &mut vm,
        &LaunchOptions {
            clipboard: true,
            ..LaunchOptions::default()
        },
    )
    .unwrap();

    assert!(vm.config.clipboard);
}

#[test]
fn launch_options_override_resources() {
    let root = tempfile::tempdir().unwrap();
    let config_path = root.path().join("resources.conf");
    fs::write(
        &config_path,
        "boot=legacy\nram=1G\ncpu_cores=1\nnetwork=none\npublic_dir=none\n",
    )
    .unwrap();
    let mut vm = find(root.path(), root.path(), "resources").unwrap();

    apply_launch_options(
        &mut vm,
        &LaunchOptions {
            ram: Some("2G".to_string()),
            cpu_cores: Some(2),
            ..LaunchOptions::default()
        },
    )
    .unwrap();

    assert_eq!(vm.config.ram.as_deref(), Some("2G"));
    assert_eq!(vm.config.cpu_cores, Some(2));
}

#[test]
fn launch_options_reject_global_flags_consumed_by_raw_arguments() {
    let root = tempfile::tempdir().unwrap();
    fs::write(
        root.path().join("raw.conf"),
        "boot=legacy\nnetwork=none\npublic_dir=none\n",
    )
    .unwrap();
    let mut vm = find(root.path(), root.path(), "raw").unwrap();

    let error = apply_launch_options(
        &mut vm,
        &LaunchOptions {
            extra_args: vec!["--output=json".to_string()],
            ..LaunchOptions::default()
        },
    )
    .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("global options before --extra-args")
    );
}

#[test]
fn start_preflight_preserves_disk_and_log_on_host_validation_failure() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("installer.iso"), []).unwrap();
    fs::write(
        root.path().join("preflight.conf"),
        "boot=legacy\niso=installer.iso\ndisk_size=1M\nnetwork=none\npublic_dir=none\n",
    )
    .unwrap();
    let mut vm = find(root.path(), &root.path().join("state"), "preflight").unwrap();
    vm.config.arch = "vmctl-test-missing".to_string();
    fs::create_dir_all(&vm.paths.state_dir).unwrap();
    let log = vm.paths.state_dir.join("qemu.log");
    fs::write(&log, "previous startup diagnostics\n").unwrap();
    fs::write(vm.paths.ipc_state(), "stale runtime state").unwrap();

    let error = start_vm_loaded(&vm, OutputFormat::Human, None).unwrap_err();

    assert!(matches!(
        error,
        Error::CommandUnavailable { command, .. }
            if command == "qemu-system-vmctl-test-missing"
    ));
    assert!(!vm.config.disk_img.exists());
    assert_eq!(
        fs::read_to_string(log).unwrap(),
        "previous startup diagnostics\n"
    );
    assert!(!vm.paths.ipc_state().exists());
}

#[test]
fn set_vm_persists_cpu_and_ram() {
    let root = tempfile::tempdir().unwrap();
    fs::write(
        root.path().join("resources.conf"),
        "boot=legacy\nram=1G\ncpu_cores=1\nnetwork=none\npublic_dir=none\n",
    )
    .unwrap();
    let dirs = Dirs {
        vm_dir: root.path().to_path_buf(),
        state_root: root.path().join("state"),
    };

    set_vm(
        &dirs,
        "resources",
        Some("2G"),
        Some(2),
        None,
        Some("host"),
        Some("0,1"),
        Some("52:54:00:12:34:56"),
        Some("none"),
        &["8080:80".to_string()],
        Some("on"),
        Some("cdrom"),
        Some("none"),
        Some("io_uring"),
        Some("ignore"),
        OutputFormat::Json,
    )
    .unwrap();

    let vm = find(&dirs.vm_dir, &dirs.state_root, "resources").unwrap();
    assert_eq!(vm.config.ram.as_deref(), Some("2G"));
    assert_eq!(vm.config.cpu_cores, Some(2));
    assert_eq!(vm.config.cpu_model.as_deref(), Some("host"));
    assert_eq!(vm.config.cpu_pinning.as_deref(), Some("0,1"));
    assert_eq!(vm.config.macaddr.as_deref(), Some("52:54:00:12:34:56"));
    assert_eq!(vm.config.port_forwards, [(8080, 80)]);
    assert!(vm.config.boot_menu);
    assert_eq!(vm.config.boot_once.as_deref(), Some("cdrom"));
    assert_eq!(vm.config.disk_cache, "none");
    assert_eq!(vm.config.disk_aio, "io_uring");
    assert_eq!(vm.config.discard, "ignore");
}

#[test]
fn set_vm_can_clear_port_forwards() {
    let root = tempfile::tempdir().unwrap();
    fs::write(
        root.path().join("clear-forwards.conf"),
        "boot=legacy\nport_forwards=(\"8080:80\")\nnetwork=none\npublic_dir=none\n",
    )
    .unwrap();
    let dirs = Dirs {
        vm_dir: root.path().to_path_buf(),
        state_root: root.path().join("state"),
    };

    set_vm(
        &dirs,
        "clear-forwards",
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        &["none".to_string()],
        None,
        None,
        None,
        None,
        None,
        OutputFormat::Json,
    )
    .unwrap();

    assert!(
        find(&dirs.vm_dir, &dirs.state_root, "clear-forwards")
            .unwrap()
            .config
            .port_forwards
            .is_empty()
    );
}

#[test]
fn set_vm_rejects_all_updates_before_writing_any() {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("unchanged.conf");
    let original = "boot=legacy\nram=1G\nnetwork=none\npublic_dir=none\n";
    fs::write(&config, original).unwrap();
    let dirs = Dirs {
        vm_dir: root.path().to_path_buf(),
        state_root: root.path().join("state"),
    };

    assert!(
        set_vm(
            &dirs,
            "unchanged",
            Some("2G"),
            None,
            None,
            Some("host\ncpu_cores=99"),
            None,
            None,
            None,
            &[],
            None,
            None,
            None,
            None,
            None,
            OutputFormat::Human,
        )
        .is_err()
    );
    assert_eq!(fs::read_to_string(config).unwrap(), original);
}

#[cfg(unix)]
#[test]
fn set_vm_refuses_a_symlinked_config_without_touching_its_target() {
    let root = tempfile::tempdir().unwrap();
    let victim = root.path().join("victim.txt");
    fs::write(&victim, "keep me\n").unwrap();
    let config = root.path().join("linked.conf");
    std::os::unix::fs::symlink(&victim, &config).unwrap();
    let dirs = Dirs {
        vm_dir: root.path().to_path_buf(),
        state_root: root.path().join("state"),
    };

    assert!(find(root.path(), &dirs.state_root, config.to_str().unwrap()).is_err());
    assert!(
        set_vm(
            &dirs,
            "linked",
            Some("2G"),
            None,
            None,
            None,
            None,
            None,
            None,
            &[],
            None,
            None,
            None,
            None,
            None,
            OutputFormat::Human,
        )
        .is_err()
    );
    assert_eq!(fs::read_to_string(victim).unwrap(), "keep me\n");
}

#[test]
fn set_vm_rejects_cpu_pinning_mismatches_before_writing() {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("pinned.conf");
    let dirs = Dirs {
        vm_dir: root.path().to_path_buf(),
        state_root: root.path().join("state"),
    };
    let set_cores = |pinning| {
        set_vm(
            &dirs,
            "pinned",
            None,
            Some(2),
            None,
            None,
            pinning,
            None,
            None,
            &[],
            None,
            None,
            None,
            None,
            None,
            OutputFormat::Human,
        )
    };

    let original = "boot=legacy\nnetwork=none\npublic_dir=none\n";
    fs::write(&config, original).unwrap();
    assert!(set_cores(Some("0")).is_err());
    assert_eq!(fs::read_to_string(&config).unwrap(), original);

    let original = "boot=legacy\ncpu_cores=1\ncpu_pinning=0\nnetwork=none\npublic_dir=none\n";
    fs::write(&config, original).unwrap();
    assert!(set_cores(None).is_err());
    assert_eq!(fs::read_to_string(config).unwrap(), original);
}

#[cfg(target_os = "linux")]
#[test]
fn set_vm_pinning_uses_the_qemu_default_core_count() {
    let cores = qemu::default_cpu_cores() as usize;
    let mut ids = Vec::with_capacity(cores);
    for range in process_allowed_cpu_spec().unwrap().split(',') {
        let (start, end) = range
            .split_once('-')
            .map_or((range, range), |(start, end)| (start, end));
        for id in start.trim().parse::<u32>().unwrap()..=end.trim().parse::<u32>().unwrap() {
            ids.push(id);
            if ids.len() == cores {
                break;
            }
        }
        if ids.len() == cores {
            break;
        }
    }
    assert_eq!(ids.len(), cores);
    let pinning = ids.iter().map(u32::to_string).collect::<Vec<_>>().join(",");

    let root = tempfile::tempdir().unwrap();
    fs::write(
        root.path().join("default-pinning.conf"),
        "boot=legacy\nnetwork=none\npublic_dir=none\n",
    )
    .unwrap();
    let dirs = Dirs {
        vm_dir: root.path().to_path_buf(),
        state_root: root.path().join("state"),
    };
    set_vm(
        &dirs,
        "default-pinning",
        None,
        None,
        None,
        None,
        Some(&pinning),
        None,
        None,
        &[],
        None,
        None,
        None,
        None,
        None,
        OutputFormat::Human,
    )
    .unwrap();
    assert_eq!(
        find(&dirs.vm_dir, &dirs.state_root, "default-pinning")
            .unwrap()
            .config
            .cpu_pinning
            .as_deref(),
        Some(pinning.as_str())
    );
}

#[test]
fn guest_exec_accepts_hyphenated_guest_arguments() {
    let cli = Cli::try_parse_from([
        "vmctl",
        "guest",
        "ubuntu",
        "exec",
        "/bin/sh",
        "--",
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
    let cli =
        Cli::try_parse_from(["vmctl", "restart", "ubuntu", "--timeout", "30", "--force"]).unwrap();
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
fn file_tail_limits_bytes_read() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("large.log");
    fs::write(&path, b"0123456789").unwrap();

    assert_eq!(read_file_tail(&path, 4).unwrap(), (b"6789".to_vec(), true));
}

#[test]
fn recognizes_linux_bridge_directories() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir_all(root.path().join("br0/bridge")).unwrap();
    assert!(is_linux_bridge(root.path(), "br0"));
    assert!(!is_linux_bridge(root.path(), "eth0"));
}

#[cfg(unix)]
#[test]
fn command_lookup_requires_an_executable_file() {
    let root = tempfile::tempdir().unwrap();
    let command = root.path().join("helper");
    fs::write(&command, "#!/bin/sh\n").unwrap();
    assert!(!is_executable_file(&command));
    fs::set_permissions(&command, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(is_executable_file(&command));
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
fn create_command_is_typed() {
    let cli = Cli::try_parse_from([
        "vmctl",
        "create",
        "ubuntu-lab",
        "--from",
        "ubuntu-24.04-desktop-amd64--sha256-123456789abc.iso",
        "--ram",
        "4G",
        "--cpu-cores",
        "2",
        "--disk-size",
        "32G",
    ])
    .unwrap();
    let Some(VmCommand::Create(args)) = cli.command else {
        panic!("expected create command");
    };
    assert_eq!(args.ram.as_deref(), Some("4G"));
    assert_eq!(args.cpu_cores, Some(2));
    assert_eq!(args.disk_size.as_deref(), Some("32G"));
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
