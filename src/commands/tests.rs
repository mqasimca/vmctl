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
fn runtime_port_reads_saved_ssh_port() {
    let root = tempfile::tempdir().unwrap();
    let ports = root.path().join("ports");
    fs::write(&ports, "spice,5930\nssh,22220\n").unwrap();
    assert_eq!(runtime_port(&ports, "ssh"), Some(22220));
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
    ])
    .unwrap();
    assert!(matches!(cli.command, Some(VmCommand::Create(_))));
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
