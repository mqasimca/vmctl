use super::*;
use tempfile::tempdir;

#[test]
fn parses_config_without_executing_shell() {
    let values = parse_config(
        r##"#!/usr/bin/env vmctl --vm
guest_os="linux"
disk_img="ubuntu/disk.qcow2"
ssh_port="22220"
port_forwards=("8080:80" "8443:443")
"##,
    );

    assert_eq!(values.get("guest_os"), Some(&"linux".to_string()));
    assert_eq!(
        values.get("disk_img"),
        Some(&"ubuntu/disk.qcow2".to_string())
    );
    assert_eq!(
        parse_tokens(values.get("port_forwards")),
        ["8080:80", "8443:443"]
    );
}

#[test]
fn accepts_default_empty_values() {
    let root = tempdir().unwrap();
    let path = root.path().join("ubuntu.conf");
    fs::write(
        &path,
        r#"boot="efi"
cpu_cores=""
disk_img=""
disk_size=""
display=""
guest_os="linux"
iso=""
ram=""
ssh_port=""
spice_port=""
monitor="socket"
serial=""
port_forwards=()
usb_devices=()
secureboot="off"
tpm="off"
"#,
    )
    .unwrap();

    let vm = load_vm(root.path(), root.path(), path).unwrap();
    assert_eq!(vm.config.disk_size, "16G");
    assert_eq!(vm.config.cpu_cores, None);
    assert_eq!(vm.config.ssh_port, None);
    assert_eq!(vm.config.serial, "socket");
    assert_eq!(vm.config.viewer, "remote-viewer");
    assert!(!vm.config.clipboard);
}

#[test]
fn tokenizes_quoted_values_and_escapes() {
    assert_eq!(
        parse_tokens(Some(&"-device 'virtio-rng-pci'".to_string())),
        vec!["-device", "virtio-rng-pci"]
    );
    assert_eq!(
        parse_tokens(Some(&r#"("path with spaces" plain\ value)"#.to_string())),
        vec!["path with spaces", "plain value"]
    );
}

#[test]
fn strips_comments_only_outside_quotes() {
    let values = parse_config("name=one # comment\npath=\"a#b\"\n");
    assert_eq!(values.get("name"), Some(&"one".to_string()));
    assert_eq!(values.get("path"), Some(&"a#b".to_string()));
}

#[test]
fn loads_paths_relative_to_the_config_directory() {
    let root = tempdir().unwrap();
    let config_path = root.path().join("ubuntu.conf");
    fs::write(
        &config_path,
        "boot=legacy\ndisk_img=ubuntu/disk.qcow2\nssh_port=22220\n",
    )
    .unwrap();

    let vm = load_vm(
        root.path(),
        root.path().join("state").as_path(),
        config_path,
    )
    .unwrap();
    assert_eq!(vm.config.disk_img, root.path().join("ubuntu/disk.qcow2"));
    assert_eq!(
        vm.paths.qmp_socket(),
        root.path().join("state/vms/ubuntu/qmp.sock")
    );
}

#[test]
fn rejects_invalid_ports_and_modes() {
    let root = tempdir().unwrap();
    let path = root.path().join("broken.conf");
    fs::write(&path, "ssh_port=0\ndisplay=bad\n").unwrap();
    let error = load_vm(root.path(), root.path(), path).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("ssh_port must be greater than zero")
    );
}

#[test]
fn rejects_clipboard_without_gtk() {
    let root = tempdir().unwrap();
    let path = root.path().join("clipboard.conf");
    fs::write(&path, "display=sdl\nclipboard=on\n").unwrap();
    let error = load_vm(root.path(), root.path(), path).unwrap_err();
    assert!(error.to_string().contains("clipboard requires display=gtk"));
}

#[test]
fn rejects_network_option_injection() {
    let root = tempdir().unwrap();
    let path = root.path().join("broken-network.conf");
    fs::write(&path, "network=br0,helper=/tmp/helper\n").unwrap();
    let error = load_vm(root.path(), root.path(), path).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("network contains QEMU option separators")
    );

    let path = root.path().join("case-sensitive-bridge.conf");
    fs::write(&path, "network=Br0\n").unwrap();
    assert_eq!(
        load_vm(root.path(), root.path(), path)
            .unwrap()
            .config
            .network,
        "Br0"
    );
}

#[test]
fn rejects_disk_and_extra_argument_injection() {
    let root = tempdir().unwrap();
    let disk_path = root.path().join("unsafe-disk.conf");
    fs::write(&disk_path, "disk_format=qcow2,backing_file=/tmp/base\n").unwrap();
    assert!(load_vm(root.path(), root.path(), disk_path).is_err());

    let args_path = root.path().join("unsafe-args.conf");
    fs::write(&args_path, "extra_args=(\"-qmp\" \"tcp:0.0.0.0:4444\")\n").unwrap();
    let error = load_vm(root.path(), root.path(), args_path).unwrap_err();
    assert!(error.to_string().contains("extra_args contains '-qmp'"));
}

#[test]
fn accepts_named_spice_bind_addresses() {
    let root = tempdir().unwrap();
    let path = root.path().join("remote.conf");
    fs::write(&path, "access=vm.example.test\ndisplay=spice\n").unwrap();
    let vm = load_vm(root.path(), root.path(), path).unwrap();
    assert_eq!(vm.config.access, "vm.example.test");
}

#[test]
fn rejects_telnet_option_injection() {
    let root = tempdir().unwrap();
    let path = root.path().join("unsafe-telnet.conf");
    fs::write(&path, "monitor_telnet_host=127.0.0.1,server=off\n").unwrap();
    let error = load_vm(root.path(), root.path(), path).unwrap_err();
    assert!(error.to_string().contains("monitor_telnet_host"));
}

#[test]
fn parses_ssh_bind_and_viewer_arguments() {
    let root = tempdir().unwrap();
    let path = root.path().join("remote.conf");
    fs::write(
        &path,
        "ssh_access=192.0.2.10\nviewer_extra_args=(\"--foo\" \"bar baz\")\n",
    )
    .unwrap();
    let vm = load_vm(root.path(), root.path(), path).unwrap();
    assert_eq!(vm.config.ssh_access, "192.0.2.10");
    assert_eq!(vm.config.viewer_extra_args, ["--foo", "bar baz"]);
}

#[test]
fn parses_usb_devices_as_hex_pairs() {
    let values = parse_config("usb_devices=(\"046d:082d\")\n");
    let root = tempdir().unwrap();
    let path = root.path().join("usb.conf");
    fs::write(&path, "usb_devices=(\"046d:082d\")\n").unwrap();
    let vm = load_vm(root.path(), root.path(), path).unwrap();
    assert_eq!(vm.config.usb_devices, vec![(0x046d, 0x082d)]);
    assert_eq!(parse_tokens(values.get("usb_devices")), vec!["046d:082d"]);
}

#[test]
fn parses_windows_install_media() {
    let root = tempdir().unwrap();
    let path = root.path().join("windows.conf");
    fs::write(
        &path,
        "boot=legacy\niso=windows.iso\nfixed_iso=virtio-win.iso\nunattended_iso=unattended.iso\n",
    )
    .unwrap();
    let vm = load_vm(root.path(), root.path(), path).unwrap();
    assert_eq!(
        vm.config.unattended_iso,
        Some(root.path().join("unattended.iso"))
    );
}
