use super::*;
use crate::config::load_vm;
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::symlink;
#[cfg(unix)]
use std::os::unix::net::UnixListener;
use tempfile::tempdir;

#[test]
fn wait_for_exit_treats_missing_process_as_stopped() {
    assert!(wait_for_exit(-1, "vmctl-test", Duration::ZERO));
}

#[test]
fn process_match_treats_missing_process_as_stopped() {
    assert!(!process_matches_checked(i32::MAX, "vmctl-test").unwrap());
}

#[test]
fn operation_lock_prevents_concurrent_acquisition() {
    let root = tempdir().unwrap();
    let paths = VmPaths::new(root.path(), "lock-test");
    let lock = acquire_vm_lock(&paths).unwrap();
    let error = acquire_vm_lock(&paths).err().unwrap();
    assert!(error.to_string().contains("another vmctl operation"));
    drop(lock);
    assert!(acquire_vm_lock(&paths).is_ok());
}

#[cfg(unix)]
#[test]
fn operation_lock_refuses_symbolic_links() {
    let root = tempdir().unwrap();
    let paths = VmPaths::new(root.path(), "lock-test");
    fs::create_dir_all(&paths.state_dir).unwrap();
    let target = root.path().join("target");
    fs::write(&target, "keep").unwrap();
    symlink(&target, paths.state_dir.join("operation.lock")).unwrap();

    let error = acquire_vm_lock(&paths).err().unwrap();

    assert!(error.to_string().contains("symbolic-link"));
    assert_eq!(fs::read_to_string(target).unwrap(), "keep");
}

#[test]
fn monitor_response_has_a_size_limit() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let writer = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream.write_all(&vec![b'x'; MAX_MONITOR_RESPONSE + 1])
    });
    let mut stream = TcpStream::connect(address).unwrap();
    let error = read_monitor_response(
        &mut stream,
        &address.to_string(),
        Instant::now() + Duration::from_secs(1),
    )
    .unwrap_err();

    assert!(error.to_string().contains("safety limit"));
    writer.join().unwrap().unwrap();
}

#[test]
fn monitor_waits_for_the_first_response_byte() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let writer = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        thread::sleep(Duration::from_millis(600));
        stream.write_all(b"ok")
    });
    let mut stream = TcpStream::connect(address).unwrap();
    let response = read_monitor_response(
        &mut stream,
        &address.to_string(),
        Instant::now() + Duration::from_secs(2),
    )
    .unwrap();

    assert_eq!(response, b"ok");
    writer.join().unwrap().unwrap();
}

#[test]
fn monitor_connect_resolves_addresses_within_the_deadline() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let acceptor = thread::spawn(move || listener.accept());

    let stream = connect_monitor(
        &address.to_string(),
        Instant::now() + Duration::from_secs(1),
    )
    .unwrap();

    assert_eq!(stream.peer_addr().unwrap(), address);
    drop(stream);
    acceptor.join().unwrap().unwrap();
}

#[test]
fn ipc_endpoint_json_rejects_non_loopback_addresses() {
    let value = json!({
        "transport": "tcp",
        "host": "0.0.0.0",
        "port": 49152,
    });
    let error = IpcEndpoint::from_json(&value).unwrap_err();
    assert_eq!(
        error.to_string(),
        "runtime TCP endpoint must be bound to loopback"
    );
}

#[test]
fn endpoint_preflight_reports_listener_conflicts() {
    let port = 0;
    let plan = QemuPlan {
        binary: "qemu-system-x86_64".to_string(),
        args: Vec::new(),
        qmp_endpoint: IpcEndpoint::Tcp(format!("127.0.0.1:{port}").parse().unwrap()),
        agent_endpoint: None,
        ssh_port: Some(port),
        ssh_host: Some("127.0.0.1".to_string()),
        spice_port: None,
        spice_host: None,
        monitor_telnet: None,
        serial_telnet: None,
    };
    let error = ensure_ipc_endpoints_available(&plan).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("conflicts with another configured listener")
    );
}

#[test]
fn macos_style_qemu_names_are_recognized() {
    assert!(command_line_has_vm_name(
        "qemu-system-x86_64 -name ubuntu-24.04 -machine q35",
        "ubuntu-24.04"
    ));
    assert!(!command_line_has_vm_name(
        "qemu-system-x86_64 -name other -machine q35",
        "ubuntu-24.04"
    ));
}

#[cfg(unix)]
#[test]
fn disk_operations_reject_symlink_paths() {
    use std::os::unix::fs::symlink;

    let root = tempdir().unwrap();
    let target = root.path().join("disk.qcow2");
    let link = root.path().join("disk-link.qcow2");
    fs::write(&target, []).unwrap();
    symlink(&target, &link).unwrap();
    let error = require_disk_file(&link).unwrap_err();
    assert!(error.to_string().contains("refusing to use disk symlink"));
}

#[test]
fn windows_plan_uses_local_ipc_transport() {
    let root = tempdir().unwrap();
    let config_path = root.path().join("windows-host.conf");
    fs::write(root.path().join("disk.qcow2"), []).unwrap();
    fs::write(
        &config_path,
        "boot=legacy\ndisk_img=disk.qcow2\ndisplay=none\nnetwork=none\npublic_dir=none\n",
    )
    .unwrap();
    let vm = load_vm(root.path(), root.path(), config_path).unwrap();
    let host = QemuPlanContext {
        qemu_binary: "qemu-system-x86_64".to_string(),
        host_os: "windows".to_string(),
        accelerator: "tcg".to_string(),
        cpu_cores: 2,
        ram: "4G".to_string(),
        virtio_vga_gl: false,
        usb_redirection: false,
        smartcard: false,
        smbd: false,
        audio_driver: Some("dsound".to_string()),
        username: "tester".to_string(),
        bridge_helper: None,
        virtiofsd: None,
        virtiofs_device: false,
        ssh_port: None,
        spice_port: Some(5930),
    };

    let plan = build_plan(&vm, &host, false).unwrap();
    assert!(
        plan.args
            .iter()
            .all(|arg| !arg.contains("filename=/dev/urandom"))
    );
    #[cfg(not(windows))]
    assert!(
        plan.args
            .iter()
            .any(|arg| arg.starts_with("tcp:127.0.0.1:"))
    );
    #[cfg(windows)]
    assert!(
        plan.args
            .windows(2)
            .any(|args| args[0] == "-chardev" && args[1].starts_with("pipe,id=qmp0,"))
    );
    #[cfg(not(windows))]
    assert!(
        plan.args
            .iter()
            .any(|arg| { arg.starts_with("socket,id=qga0,host=127.0.0.1,port=") })
    );
    #[cfg(windows)]
    assert!(
        plan.args
            .iter()
            .any(|arg| arg.starts_with("pipe,id=qga0,path="))
    );
    assert!(!plan.args.iter().any(|arg| arg.starts_with("unix:")));
    #[cfg(not(windows))]
    assert!(matches!(plan.qmp_endpoint, IpcEndpoint::Tcp(_)));
    #[cfg(windows)]
    assert!(matches!(plan.qmp_endpoint, IpcEndpoint::Pipe(_)));
    #[cfg(not(windows))]
    assert!(matches!(plan.agent_endpoint, Some(IpcEndpoint::Tcp(_))));
    #[cfg(windows)]
    assert!(matches!(plan.agent_endpoint, Some(IpcEndpoint::Pipe(_))));
}

#[test]
fn runtime_ipc_state_round_trips_atomically() {
    let root = tempdir().unwrap();
    let config_path = root.path().join("ipc.conf");
    fs::write(root.path().join("disk.qcow2"), []).unwrap();
    fs::write(
        &config_path,
        "boot=legacy\ndisk_img=disk.qcow2\ndisplay=none\nnetwork=none\npublic_dir=none\n",
    )
    .unwrap();
    let vm = load_vm(root.path(), root.path(), config_path).unwrap();
    let host = QemuPlanContext {
        qemu_binary: "qemu-system-x86_64".to_string(),
        host_os: "windows".to_string(),
        accelerator: "tcg".to_string(),
        cpu_cores: 2,
        ram: "4G".to_string(),
        virtio_vga_gl: false,
        usb_redirection: false,
        smartcard: false,
        smbd: false,
        audio_driver: Some("dsound".to_string()),
        username: "tester".to_string(),
        bridge_helper: None,
        virtiofsd: None,
        virtiofs_device: false,
        ssh_port: None,
        spice_port: Some(5930),
    };
    let plan = build_plan(&vm, &host, false).unwrap();
    write_runtime_files(&vm.paths, &plan).unwrap();
    let (qmp, agent) = read_ipc_state(&vm.paths).unwrap();
    assert_eq!(qmp, plan.qmp_endpoint);
    assert_eq!(agent, plan.agent_endpoint);
    assert!(!vm.paths.ipc_state().with_extension("tmp").exists());
}

#[test]
fn shell_quoting_is_safe_for_spaces_and_quotes() {
    assert_eq!(
        shell_join("qemu-system-x86_64", &["path with spaces".to_string()]),
        "qemu-system-x86_64 'path with spaces'"
    );
    assert_eq!(
        shell_join("qemu", &["it's safe".to_string()]),
        "qemu 'it'\\''s safe'"
    );
}

#[test]
fn qemu_version_check_handles_single_and_double_digit_releases() {
    assert_eq!(
        qemu_version(b"QEMU emulator version 6.1.0 (v6.1.0)"),
        Some((6, 1, 0))
    );
    assert_eq!(
        qemu_version(b"QEMU emulator version 10.0.3"),
        Some((10, 0, 3))
    );
    assert!(!qemu_version_supported((6, 0, 9)));
    assert!(qemu_version_supported((6, 1, 0)));
    assert!(qemu_version_supported((10, 0, 0)));
}

#[cfg(unix)]
#[test]
fn executable_lookup_rejects_non_executable_files() {
    let root = tempdir().unwrap();
    let command = root.path().join("qemu-system-test");
    fs::write(&command, "#!/bin/sh\n").unwrap();
    assert!(!is_executable_file(&command));
    fs::set_permissions(&command, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(is_executable_file(&command));
}

#[test]
fn gtk_clipboard_requires_qemu_11_1() {
    assert!(qemu_version_supports_gtk_clipboard((11, 1, 0)));
    assert!(!qemu_version_supports_gtk_clipboard((11, 0, 9)));
}

#[test]
fn monitor_output_drops_terminal_control_sequences() {
    assert_eq!(
        clean_monitor_output(b"\x1b[Kinfo status\x1b[D\n(qemu)"),
        "info status\n(qemu)"
    );
}

#[test]
fn cocoa_display_is_rejected_on_linux() {
    let root = tempdir().unwrap();
    let config_path = root.path().join("cocoa.conf");
    fs::write(root.path().join("disk.qcow2"), []).unwrap();
    fs::write(
        &config_path,
        "boot=legacy\ndisk_img=disk.qcow2\ndisplay=cocoa\nnetwork=none\npublic_dir=none\n",
    )
    .unwrap();
    let vm = load_vm(root.path(), root.path(), config_path).unwrap();
    let host = QemuPlanContext {
        qemu_binary: "qemu-system-x86_64".to_string(),
        host_os: "linux".to_string(),
        accelerator: "tcg".to_string(),
        cpu_cores: 2,
        ram: "4G".to_string(),
        virtio_vga_gl: false,
        usb_redirection: false,
        smartcard: false,
        smbd: false,
        audio_driver: None,
        username: "tester".to_string(),
        bridge_helper: None,
        virtiofsd: None,
        virtiofs_device: false,
        ssh_port: None,
        spice_port: None,
    };

    let error = build_plan(&vm, &host, false).unwrap_err();
    assert_eq!(
        error.to_string(),
        "display mode 'cocoa' is only supported on macOS"
    );
}

#[test]
fn spice_app_uses_managed_software_rendering() {
    let root = tempdir().unwrap();
    let config_path = root.path().join("spice-app.conf");
    fs::write(root.path().join("disk.qcow2"), []).unwrap();
    fs::write(
        &config_path,
        "boot=legacy\ndisk_img=disk.qcow2\ndisplay=spice-app\nnetwork=none\npublic_dir=none\n",
    )
    .unwrap();
    let vm = load_vm(root.path(), root.path(), config_path).unwrap();
    let host = QemuPlanContext {
        qemu_binary: "qemu-system-x86_64".to_string(),
        host_os: "linux".to_string(),
        accelerator: "tcg".to_string(),
        cpu_cores: 2,
        ram: "4G".to_string(),
        virtio_vga_gl: true,
        usb_redirection: false,
        smartcard: false,
        smbd: false,
        audio_driver: None,
        username: "tester".to_string(),
        bridge_helper: None,
        virtiofsd: None,
        virtiofs_device: false,
        ssh_port: None,
        spice_port: None,
    };

    let plan = build_plan(&vm, &host, false).unwrap();
    assert!(
        plan.args
            .windows(2)
            .any(|args| args == ["-display", "none"])
    );
    assert!(
        plan.args
            .windows(2)
            .any(|args| args == ["-device", "virtio-gpu"])
    );
    assert!(!plan.args.iter().any(|arg| arg == "virtio-gpu-gl"));
    assert!(
        plan.args
            .windows(2)
            .any(|args| { args[0] == "-spice" && args[1].contains("disable-ticketing=on") })
    );
    assert!(!plan.args.iter().any(|arg| arg == "spice-app,gl=off"));
}

#[test]
fn plan_builder_is_deterministic_with_injected_host_capabilities() {
    let root = tempdir().unwrap();
    let config_path = root.path().join("test.conf");
    let disk = root.path().join("disk.qcow2");
    fs::write(&disk, []).unwrap();
    fs::write(
        &config_path,
        "boot=legacy\ndisk_img=disk.qcow2\ndisplay=none\nnetwork=none\npublic_dir=none\nguest_agent=false\n",
    )
    .unwrap();
    let vm = load_vm(root.path(), root.path(), config_path).unwrap();
    let host = QemuPlanContext {
        qemu_binary: "qemu-system-x86_64".to_string(),
        host_os: "linux".to_string(),
        accelerator: "tcg".to_string(),
        cpu_cores: 2,
        ram: "4G".to_string(),
        virtio_vga_gl: false,
        usb_redirection: false,
        smartcard: false,
        smbd: false,
        audio_driver: None,
        username: "tester".to_string(),
        bridge_helper: None,
        virtiofsd: None,
        virtiofs_device: false,
        ssh_port: None,
        spice_port: Some(5930),
    };

    let plan = build_plan(&vm, &host, false).unwrap();
    assert_eq!(plan.ssh_port, None);
    let qmp_value = format!(
        "unix:{},server=on,wait=off",
        vm.paths.qmp_socket().display()
    );
    assert!(
        plan.args
            .windows(2)
            .any(|args| args[0] == "-qmp" && args[1] == qmp_value)
    );
    assert!(plan.args.windows(2).any(|args| args == ["-nic", "none"]));
    assert!(plan.args.windows(2).any(|args| args == ["-vga", "none"]));
    assert!(plan.args.windows(2).any(|args| {
        args[0] == "-spice" && args[1] == "port=5930,addr=127.0.0.1,disable-ticketing=on"
    }));
    assert!(
        plan.args
            .windows(2)
            .any(|args| args == ["-display", "none"])
    );
}

#[test]
fn linux_public_share_uses_virtiofs_when_available() {
    let root = tempdir().unwrap();
    let config_path = root.path().join("virtiofs.conf");
    fs::write(root.path().join("disk.qcow2"), []).unwrap();
    fs::create_dir(root.path().join("public")).unwrap();
    fs::write(
        &config_path,
        "boot=legacy\ndisk_img=disk.qcow2\ndisplay=none\nnetwork=none\npublic_dir=public\n",
    )
    .unwrap();
    let vm = load_vm(root.path(), root.path(), config_path).unwrap();
    let host = QemuPlanContext {
        qemu_binary: "qemu-system-x86_64".to_string(),
        host_os: "linux".to_string(),
        accelerator: "tcg".to_string(),
        cpu_cores: 2,
        ram: "4G".to_string(),
        virtio_vga_gl: false,
        usb_redirection: false,
        smartcard: false,
        smbd: false,
        audio_driver: None,
        username: "tester".to_string(),
        bridge_helper: None,
        virtiofsd: Some("/usr/bin/virtiofsd".to_string()),
        virtiofs_device: true,
        ssh_port: None,
        spice_port: Some(5930),
    };

    let plan = build_plan(&vm, &host, false).unwrap();
    assert!(virtiofs_requested(&vm.config, &host));
    assert!(
        plan.args
            .iter()
            .any(|arg| arg == "vhost-user-fs-pci,queue-size=1024,chardev=char0,tag=Public-tester")
    );
    assert!(!plan.args.iter().any(|arg| arg == "virtio-9p-pci"));
    assert!(
        plan.args
            .iter()
            .any(|arg| arg.starts_with("memory-backend-file,id=mem,"))
    );
}

#[test]
fn macos_public_share_uses_9p() {
    let root = tempdir().unwrap();
    let config_path = root.path().join("macos.conf");
    fs::write(
        &config_path,
        "guest_os=macos\nboot=efi\ndisplay=none\nnetwork=none\npublic_dir=public\n",
    )
    .unwrap();
    fs::create_dir(root.path().join("public")).unwrap();
    let vm = load_vm(root.path(), root.path(), config_path).unwrap();
    let host = QemuPlanContext {
        qemu_binary: "qemu-system-x86_64".to_string(),
        host_os: "linux".to_string(),
        accelerator: "tcg".to_string(),
        cpu_cores: 2,
        ram: "4G".to_string(),
        virtio_vga_gl: false,
        usb_redirection: false,
        smartcard: false,
        smbd: false,
        audio_driver: None,
        username: "tester".to_string(),
        bridge_helper: None,
        virtiofsd: None,
        virtiofs_device: false,
        ssh_port: None,
        spice_port: Some(5930),
    };
    let mut args = Vec::new();
    add_share_args(&mut args, &vm, &host);
    assert!(args.iter().any(|arg| arg.starts_with("local,id=fsdev0,")));
    assert!(
        args.iter()
            .any(|arg| arg == "virtio-9p-pci,fsdev=fsdev0,mount_tag=Public-tester")
    );
}

#[test]
fn unsupported_guest_does_not_receive_public_share() {
    let root = tempdir().unwrap();
    let config_path = root.path().join("freebsd.conf");
    fs::write(
        &config_path,
        "guest_os=freebsd\nboot=efi\ndisplay=none\nnetwork=none\npublic_dir=public\n",
    )
    .unwrap();
    fs::create_dir(root.path().join("public")).unwrap();
    let vm = load_vm(root.path(), root.path(), config_path).unwrap();
    let host = QemuPlanContext {
        qemu_binary: "qemu-system-x86_64".to_string(),
        host_os: "linux".to_string(),
        accelerator: "tcg".to_string(),
        cpu_cores: 2,
        ram: "4G".to_string(),
        virtio_vga_gl: false,
        usb_redirection: false,
        smartcard: false,
        smbd: false,
        audio_driver: None,
        username: "tester".to_string(),
        bridge_helper: None,
        virtiofsd: None,
        virtiofs_device: false,
        ssh_port: None,
        spice_port: Some(5930),
    };
    let mut args = Vec::new();
    add_share_args(&mut args, &vm, &host);
    assert!(args.is_empty());
}

#[test]
fn windows_server_uses_smb_but_not_guest_filesystem_shares() {
    let root = tempdir().unwrap();
    let config_path = root.path().join("windows-server.conf");
    fs::write(
        &config_path,
        "guest_os=windows-server\nboot=legacy\ndisplay=none\nnetwork=user\npublic_dir=public\n",
    )
    .unwrap();
    fs::create_dir(root.path().join("public")).unwrap();
    let vm = load_vm(root.path(), root.path(), config_path).unwrap();
    let host = QemuPlanContext {
        qemu_binary: "qemu-system-x86_64".to_string(),
        host_os: "linux".to_string(),
        accelerator: "tcg".to_string(),
        cpu_cores: 2,
        ram: "4G".to_string(),
        virtio_vga_gl: false,
        usb_redirection: false,
        smartcard: false,
        smbd: true,
        audio_driver: None,
        username: "tester".to_string(),
        bridge_helper: None,
        virtiofsd: None,
        virtiofs_device: false,
        ssh_port: None,
        spice_port: Some(5930),
    };
    let mut args = Vec::new();
    add_share_args(&mut args, &vm, &host);
    assert!(args.is_empty());
    let mut network_args = Vec::new();
    add_network_args(&mut network_args, &vm, Some(22444), true, None).unwrap();
    assert!(network_args.iter().any(|arg| arg.contains("smb=")));
}

#[test]
fn qemu_option_paths_double_commas_without_rewriting_backslashes() {
    let path = Path::new("/tmp/disk,one\\two.qcow2");
    assert_eq!(qemu_path(path), "/tmp/disk,,one\\two.qcow2");
}

#[test]
fn unsafe_share_username_becomes_a_safe_mount_tag() {
    let root = tempdir().unwrap();
    let config_path = root.path().join("linux.conf");
    fs::create_dir(root.path().join("public")).unwrap();
    fs::write(
        &config_path,
        "boot=legacy\ndisplay=none\nnetwork=none\npublic_dir=public\n",
    )
    .unwrap();
    let vm = load_vm(root.path(), root.path(), config_path).unwrap();
    let host = QemuPlanContext {
        qemu_binary: "qemu-system-x86_64".to_string(),
        host_os: "linux".to_string(),
        accelerator: "tcg".to_string(),
        cpu_cores: 2,
        ram: "4G".to_string(),
        virtio_vga_gl: false,
        usb_redirection: false,
        smartcard: false,
        audio_driver: None,
        smbd: false,
        username: "bad,user=tag".to_string(),
        bridge_helper: None,
        virtiofsd: None,
        virtiofs_device: false,
        ssh_port: None,
        spice_port: None,
    };
    let mut args = Vec::new();
    add_share_args(&mut args, &vm, &host);
    assert!(args.iter().all(|arg| !arg.contains("bad,user=tag")));
    assert!(args.iter().any(|arg| arg.contains("tag=Public-badusertag")));
}

#[test]
fn usb_audio_selects_xhci_controller() {
    let root = tempdir().unwrap();
    let config_path = root.path().join("audio.conf");
    fs::write(
        &config_path,
        "sound_card=usb-audio\nusb_controller=none\nboot=legacy\ndisplay=none\nnetwork=none\n",
    )
    .unwrap();
    let vm = load_vm(root.path(), root.path(), config_path).unwrap();
    assert_eq!(vm.config.usb_controller, "xhci");
}

#[test]
fn plan_reports_and_uses_the_ssh_bind_address() {
    let root = tempdir().unwrap();
    let config_path = root.path().join("remote.conf");
    let disk = root.path().join("disk.qcow2");
    fs::write(&disk, []).unwrap();
    fs::write(
        &config_path,
        "boot=legacy\ndisk_img=disk.qcow2\ndisplay=none\nssh_access=remote\npublic_dir=none\n",
    )
    .unwrap();
    let vm = load_vm(root.path(), root.path(), config_path).unwrap();
    let host = QemuPlanContext {
        qemu_binary: "qemu-system-x86_64".to_string(),
        host_os: "linux".to_string(),
        accelerator: "tcg".to_string(),
        cpu_cores: 2,
        ram: "4G".to_string(),
        virtio_vga_gl: false,
        usb_redirection: false,
        smartcard: false,
        smbd: false,
        audio_driver: None,
        username: "tester".to_string(),
        bridge_helper: None,
        virtiofsd: None,
        virtiofs_device: false,
        ssh_port: Some(22444),
        spice_port: Some(5930),
    };

    let plan = build_plan(&vm, &host, false).unwrap();
    assert_eq!(plan.ssh_host.as_deref(), Some("0.0.0.0"));
    assert_eq!(plan.spice_host.as_deref(), Some("127.0.0.1"));
    assert!(
        plan.args
            .iter()
            .any(|arg| arg.contains("hostfwd=tcp:0.0.0.0:22444-:22"))
    );
    assert!(
        plan.args
            .windows(2)
            .any(|args| args == ["-accel", "tcg,tb-size=256,thread=multi"])
    );
}

#[test]
fn user_network_is_not_treated_as_a_bridge() {
    let root = tempdir().unwrap();
    let config_path = root.path().join("user-network.conf");
    fs::write(
        &config_path,
        "boot=legacy\ndisplay=none\nnetwork=user\npublic_dir=none\n",
    )
    .unwrap();
    let vm = load_vm(root.path(), root.path(), config_path).unwrap();
    let host = QemuPlanContext {
        qemu_binary: "qemu-system-x86_64".to_string(),
        host_os: "linux".to_string(),
        accelerator: "tcg".to_string(),
        cpu_cores: 2,
        ram: "4G".to_string(),
        virtio_vga_gl: false,
        usb_redirection: false,
        smartcard: false,
        smbd: false,
        audio_driver: None,
        username: "tester".to_string(),
        bridge_helper: None,
        virtiofsd: None,
        virtiofs_device: false,
        ssh_port: Some(22444),
        spice_port: Some(5930),
    };
    let plan = build_plan(&vm, &host, false).unwrap();
    assert!(uses_user_network(&vm.config));
    assert_eq!(configured_bridge(&vm.config), None);
    assert_eq!(plan.ssh_port, Some(22444));
    assert!(
        plan.args
            .windows(2)
            .any(|args| { args[0] == "-netdev" && args[1].starts_with("user,id=nic,") })
    );
    assert!(!plan.args.iter().any(|arg| arg == "bridge,br=user"));
}

#[test]
fn passt_network_scopes_forwarded_ports() {
    let root = tempdir().unwrap();
    let config_path = root.path().join("passt-network.conf");
    fs::write(
        &config_path,
        "boot=legacy\ndisplay=none\nnetwork=passt\nssh_access=remote\nport_forwards=(\"8080:80\" \"8443:443\")\npublic_dir=none\n",
    )
    .unwrap();
    let vm = load_vm(root.path(), root.path(), config_path).unwrap();
    let host = QemuPlanContext {
        qemu_binary: "qemu-system-x86_64".to_string(),
        host_os: "linux".to_string(),
        accelerator: "tcg".to_string(),
        cpu_cores: 2,
        ram: "4G".to_string(),
        virtio_vga_gl: false,
        usb_redirection: false,
        smartcard: false,
        smbd: false,
        audio_driver: None,
        username: "tester".to_string(),
        bridge_helper: None,
        virtiofsd: None,
        virtiofs_device: false,
        ssh_port: Some(22444),
        spice_port: Some(5930),
    };

    let plan = build_plan(&vm, &host, false).unwrap();

    assert!(uses_passt_network(&vm.config));
    assert_eq!(configured_bridge(&vm.config), None);
    assert_eq!(plan.ssh_port, Some(22444));
    assert!(plan.args.windows(2).any(|args| {
        args[0] == "-netdev"
            && args[1]
                == "passt,id=nic,tcp-ports=none,udp-ports=none,param=--tcp-ports=0.0.0.0/22444:22,param=--tcp-ports=127.0.0.1/8080:80,,8443:443,param=--udp-ports=127.0.0.1/8080:80,,8443:443"
    }));
}

#[test]
fn bridge_plan_includes_the_detected_qemu_helper() {
    let root = tempdir().unwrap();
    let config_path = root.path().join("bridge.conf");
    let disk = root.path().join("disk.qcow2");
    fs::write(&disk, []).unwrap();
    fs::write(
        &config_path,
        "boot=legacy\ndisk_img=disk.qcow2\nnetwork=br0\ndisplay=none\npublic_dir=none\n",
    )
    .unwrap();
    let vm = load_vm(root.path(), root.path(), config_path).unwrap();
    let host = QemuPlanContext {
        qemu_binary: "qemu-system-x86_64".to_string(),
        host_os: "linux".to_string(),
        accelerator: "tcg".to_string(),
        cpu_cores: 2,
        ram: "4G".to_string(),
        virtio_vga_gl: false,
        usb_redirection: false,
        smartcard: false,
        smbd: false,
        audio_driver: None,
        username: "tester".to_string(),
        bridge_helper: Some("/usr/lib/qemu/qemu-bridge-helper".to_string()),
        virtiofsd: None,
        virtiofs_device: false,
        ssh_port: None,
        spice_port: Some(5930),
    };

    let plan = build_plan(&vm, &host, false).unwrap();
    assert_eq!(configured_bridge(&vm.config), Some("br0"));
    assert!(plan.args.iter().any(|arg| {
        arg == "bridge,br=br0,helper=/usr/lib/qemu/qemu-bridge-helper,model=virtio-net-pci"
    }));
    let mut no_helper = host.clone();
    no_helper.bridge_helper = None;
    assert!(
        build_plan(&vm, &no_helper, false)
            .unwrap_err()
            .to_string()
            .contains("bridged networking requires qemu-bridge-helper")
    );
}

#[test]
fn gtk_clipboard_is_explicit_in_the_display_plan() {
    let root = tempdir().unwrap();
    let config_path = root.path().join("clipboard.conf");
    fs::write(
        &config_path,
        "boot=legacy\ndisplay=gtk\nclipboard=on\nnetwork=none\npublic_dir=none\n",
    )
    .unwrap();
    let vm = load_vm(root.path(), root.path(), config_path).unwrap();
    let host = QemuPlanContext {
        qemu_binary: "qemu-system-x86_64".to_string(),
        host_os: "linux".to_string(),
        accelerator: "tcg".to_string(),
        cpu_cores: 2,
        ram: "4G".to_string(),
        virtio_vga_gl: false,
        usb_redirection: false,
        smartcard: false,
        smbd: false,
        audio_driver: None,
        username: "tester".to_string(),
        bridge_helper: None,
        virtiofsd: None,
        virtiofs_device: false,
        ssh_port: None,
        spice_port: Some(5930),
    };

    match build_plan(&vm, &host, false) {
        Ok(plan) => {
            assert!(plan.args.iter().any(|arg| arg.contains("clipboard=on")));
            assert!(plan.args.iter().any(|arg| arg.contains("qemu-vdagent")));
        }
        Err(error) => assert!(
            error.to_string().contains("QEMU 11.1.0") || error.to_string().contains("qemu-vdagent")
        ),
    }
}

#[test]
fn arm_windows_uses_virtio_graphics() {
    let root = tempdir().unwrap();
    let config_path = root.path().join("arm-windows.conf");
    let disk = root.path().join("disk.qcow2");
    fs::write(&disk, []).unwrap();
    fs::write(
        &config_path,
        "arch=aarch64\nguest_os=windows\nboot=legacy\ndisk_img=disk.qcow2\ndisplay=gtk\nnetwork=none\npublic_dir=none\n",
    )
    .unwrap();
    let vm = load_vm(root.path(), root.path(), config_path).unwrap();
    let host = QemuPlanContext {
        qemu_binary: "qemu-system-aarch64".to_string(),
        host_os: "linux".to_string(),
        accelerator: "tcg".to_string(),
        cpu_cores: 2,
        ram: "4G".to_string(),
        virtio_vga_gl: false,
        usb_redirection: false,
        smartcard: false,
        smbd: false,
        audio_driver: None,
        username: "tester".to_string(),
        bridge_helper: None,
        virtiofsd: None,
        virtiofs_device: false,
        ssh_port: None,
        spice_port: None,
    };

    let plan = build_plan(&vm, &host, false).unwrap();
    assert!(plan.args.iter().any(|arg| arg == "virtio-gpu-pci"));
    assert!(!plan.args.iter().any(|arg| arg == "qxl-vga"));
}

#[test]
fn firmware_pair_selection_skips_incomplete_entries() {
    let root = tempdir().unwrap();
    let first_code = root.path().join("first-code");
    let first_vars = root.path().join("first-vars");
    let second_code = root.path().join("second-code");
    let second_vars = root.path().join("second-vars");
    fs::write(&first_code, []).unwrap();
    fs::write(&second_code, []).unwrap();
    fs::write(&second_vars, []).unwrap();
    let pairs = [
        (first_code.to_str().unwrap(), first_vars.to_str().unwrap()),
        (second_code.to_str().unwrap(), second_vars.to_str().unwrap()),
    ];

    assert_eq!(first_complete_pair(&pairs).unwrap().0, second_code);
}

#[test]
fn plan_attaches_windows_install_media_in_stable_order() {
    let root = tempdir().unwrap();
    for name in [
        "disk.qcow2",
        "windows.iso",
        "virtio-win.iso",
        "unattended.iso",
    ] {
        fs::write(root.path().join(name), []).unwrap();
    }
    let config_path = root.path().join("windows.conf");
    fs::write(
        &config_path,
        "boot=legacy\ndisk_img=disk.qcow2\niso=windows.iso\nfixed_iso=virtio-win.iso\nunattended_iso=unattended.iso\ndisplay=none\nnetwork=none\npublic_dir=none\nguest_agent=false\n",
    )
    .unwrap();
    let vm = load_vm(root.path(), root.path(), config_path).unwrap();
    let host = QemuPlanContext {
        qemu_binary: "qemu-system-x86_64".to_string(),
        host_os: "linux".to_string(),
        accelerator: "tcg".to_string(),
        cpu_cores: 2,
        ram: "4G".to_string(),
        virtio_vga_gl: false,
        usb_redirection: false,
        smartcard: false,
        smbd: false,
        audio_driver: None,
        username: "tester".to_string(),
        bridge_helper: None,
        virtiofsd: None,
        virtiofs_device: false,
        ssh_port: None,
        spice_port: Some(5930),
    };

    let plan = build_plan(&vm, &host, false).unwrap();
    assert!(plan.args.windows(2).any(|args| {
        args[0] == "-drive"
            && args[1].contains("media=cdrom,index=2,readonly=on")
            && args[1].contains("unattended.iso")
    }));
}

#[test]
fn arm_plan_avoids_x86_machine_flags_and_wires_tpm() {
    let root = tempdir().unwrap();
    let config_path = root.path().join("arm.conf");
    let disk = root.path().join("disk.qcow2");
    fs::write(&disk, []).unwrap();
    fs::write(
        &config_path,
        "arch=aarch64\nboot=legacy\ndisk_img=disk.qcow2\ndisplay=none\nnetwork=none\npublic_dir=none\ntpm=on\n",
    )
    .unwrap();
    let vm = load_vm(root.path(), root.path(), config_path).unwrap();
    let host = QemuPlanContext {
        qemu_binary: "qemu-system-aarch64".to_string(),
        host_os: "linux".to_string(),
        accelerator: "tcg".to_string(),
        cpu_cores: 2,
        ram: "4G".to_string(),
        virtio_vga_gl: false,
        usb_redirection: false,
        smartcard: false,
        smbd: false,
        audio_driver: None,
        username: "tester".to_string(),
        bridge_helper: None,
        virtiofsd: None,
        virtiofs_device: false,
        ssh_port: None,
        spice_port: Some(5930),
    };

    let plan = build_plan(&vm, &host, false).unwrap();
    let machine = plan
        .args
        .windows(2)
        .find(|args| args[0] == "-machine")
        .map(|args| args[1].as_str())
        .unwrap();
    assert!(machine.starts_with("virt,"));
    assert!(!machine.contains("smm=") && !machine.contains("vmport="));
    assert!(
        plan.args
            .windows(2)
            .any(|args| args == ["-device", "ramfb"])
    );
    assert!(plan.args.iter().any(|arg| arg.contains("tpm-tis-device")));
}

#[test]
fn efi_plan_can_preview_before_variables_are_created() {
    if first_existing(&[
        "/usr/share/edk2/x64/OVMF_CODE.4m.fd",
        "/usr/share/OVMF/OVMF_CODE_4M.fd",
        "/usr/share/OVMF/OVMF_CODE.fd",
    ])
    .is_none()
    {
        return;
    }
    let root = tempdir().unwrap();
    let config_path = root.path().join("efi.conf");
    let disk = root.path().join("disk.qcow2");
    fs::write(&disk, []).unwrap();
    fs::write(
        &config_path,
        "boot=efi\ndisk_img=disk.qcow2\ndisplay=none\nnetwork=none\npublic_dir=none\n",
    )
    .unwrap();
    let vm = load_vm(root.path(), root.path(), config_path).unwrap();
    let host = QemuPlanContext {
        qemu_binary: "qemu-system-x86_64".to_string(),
        host_os: "linux".to_string(),
        accelerator: "tcg".to_string(),
        cpu_cores: 2,
        ram: "4G".to_string(),
        virtio_vga_gl: false,
        usb_redirection: false,
        smartcard: false,
        smbd: false,
        audio_driver: None,
        username: "tester".to_string(),
        bridge_helper: None,
        virtiofsd: None,
        virtiofs_device: false,
        ssh_port: None,
        spice_port: Some(5930),
    };

    let plan = build_plan(&vm, &host, false).unwrap();
    assert!(plan.args.iter().any(|arg| arg.ends_with("OVMF_VARS.fd")));
    assert!(!root.path().join("OVMF_VARS.fd").exists());
}

#[test]
fn disk_argument_validation_rejects_option_injection() {
    assert!(validate_disk_size("20G").is_ok());
    assert!(validate_disk_size("+4G").is_ok());
    assert!(validate_disk_size("--shrink").is_err());
    assert!(validate_disk_size("20 G").is_err());
    assert!(validate_disk_format("qcow2").is_ok());
    assert!(validate_disk_format("-raw").is_err());
    assert!(validate_disk_format("raw image").is_err());
}

#[test]
fn qemu_help_parsers_extract_display_devices_and_cpu_models() {
    let display = qemu_display_backends_from_text(
        "Available display backend types:\nnone\ngtk\nspice-app\n\nSome display backends support options",
    );
    assert_eq!(display, ["none", "gtk", "spice-app"]);

    let devices =
        qemu_quoted_names("name \"virtio-vga-gl\", bus PCI\nname \"usb-redir\", bus usb-bus");
    assert_eq!(devices, ["virtio-vga-gl", "usb-redir"]);

    let cpus =
        "Available CPUs:\n  host                  host CPU\n  max                   all features\n";
    assert!(qemu_supports_cpu_in_text(cpus, "host"));
    assert!(!qemu_supports_cpu_in_text(cpus, "unknown"));

    let accelerators =
        qemu_accelerators_from_text("Accelerators supported in QEMU binary:\ntcg\nkvm\n\n");
    assert_eq!(accelerators, ["tcg", "kvm"]);

    let netdevs =
        qemu_netdev_backends_from_text("Available netdev backend types:\nsocket\npasst\nuser\n\n");
    assert_eq!(netdevs, ["socket", "passt", "user"]);
    assert_eq!(
        qemu_netdev_help_args("qemu-system-aarch64"),
        ["-machine", "virt", "-netdev", "help"]
    );
}

#[test]
fn unavailable_qemu_capabilities_are_explained() {
    let report = qemu_capability_report("vmctl-qemu-does-not-exist");
    assert_eq!(report["available"], false);
    assert!(report["probe_error"].as_str().is_some());
}

#[test]
fn cpu_probe_rejects_unknown_features_when_qemu_is_installed() {
    if !command_available("qemu-system-x86_64") {
        return;
    }
    let error = validate_cpu_spec("qemu-system-x86_64", "qemu64,+vmctl-unknown-feature", "tcg")
        .unwrap_err();
    assert!(error.to_string().contains("rejected CPU specification"));
}

#[test]
fn qmp_probe_requires_a_valid_greeting() {
    assert!(
        read_qmp_greeting(io::Cursor::new(
            b"{\"QMP\":{\"version\":{},\"capabilities\":[]}}\r\n" as &[u8],
        ))
        .unwrap()
    );
    assert!(!read_qmp_greeting(io::Cursor::new(b"{\"QMP\":null}\n" as &[u8])).unwrap());
    assert!(!read_qmp_greeting(io::Cursor::new(b"{\"not_qmp\":{}}\n" as &[u8])).unwrap());
    assert!(read_qmp_greeting(io::Cursor::new(b"not-json\n" as &[u8])).is_err());
}

#[test]
fn guest_exec_output_is_decoded_without_discarding_raw_data() {
    let result = normalize_guest_exec_result(json!({
        "exited": true,
        "exitcode": 0,
        "out-data": "SGVsbG8h",
        "err-data": "d29w",
    }))
    .unwrap();
    assert_eq!(result["stdout"], "Hello!");
    assert_eq!(result["stderr"], "wop");
    assert_eq!(result["out-data"], "SGVsbG8h");
}

#[test]
fn base64_decoder_rejects_invalid_data() {
    assert!(decode_base64("SGVsbG8").is_ok());
    assert!(decode_base64("SGVsbG8$").is_err());
    assert!(decode_base64("A===").is_err());
    assert!(decode_base64("AA=").is_err());
    assert!(decode_base64("AB==").is_err());
}

#[test]
fn guest_agent_commands_synchronize_each_connection() {
    #[cfg(unix)]
    {
        let root = tempdir().unwrap();
        let config_path = root.path().join("guest-agent.conf");
        fs::write(&config_path, "guest_agent=true\n").unwrap();
        let vm = load_vm(root.path(), root.path(), config_path).unwrap();
        fs::create_dir_all(&vm.paths.state_dir).unwrap();
        let listener = UnixListener::bind(vm.paths.agent_socket()).unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut marker = [0_u8; 1];
            reader.read_exact(&mut marker).unwrap();
            assert_eq!(marker, [0xff]);
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let request: Value = serde_json::from_str(line.trim()).unwrap();
            assert_eq!(request["execute"], "guest-sync-delimited");
            let id = request["arguments"]["id"].as_i64().unwrap();
            stream.write_all(&[0xff]).unwrap();
            stream
                .write_all(format!("{{\"return\":{}}}\n", id + 1).as_bytes())
                .unwrap();
            stream.write_all(&[0xff]).unwrap();
            stream
                .write_all(format!("{{\"return\":{id}}}\n").as_bytes())
                .unwrap();
            line.clear();
            reader.read_line(&mut line).unwrap();
            let request: Value = serde_json::from_str(line.trim()).unwrap();
            assert_eq!(request["execute"], "guest-ping");
            stream.write_all(b"{\"return\":{}}\n").unwrap();
        });
        assert_eq!(guest_command(&vm, "guest-ping", None).unwrap(), json!({}));
        server.join().unwrap();
    }
}

#[test]
fn guest_agent_response_limit_is_enforced_while_reading() {
    let mut reader = BufReader::new(io::Cursor::new(b"12345\n"));
    let error = read_bounded_line(&mut reader, 4).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
}
