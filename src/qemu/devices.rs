use super::*;

pub(crate) fn virtiofs_requested(config: &VmConfig, host: &QemuPlanContext) -> bool {
    host.host_os == "linux"
        && host.virtiofsd.is_some()
        && host.virtiofs_device
        && config.guest_os == "linux"
        && config.iso.is_none()
        && config.fixed_iso.is_none()
        && config.unattended_iso.is_none()
        && config.public_dir.as_ref().is_some_and(|path| path.is_dir())
}

pub(super) fn add_share_args(args: &mut Vec<String>, vm: &Vm, host: &QemuPlanContext) {
    let config = &vm.config;
    if matches!(config.guest_os.as_str(), "windows" | "windows-server")
        || !(config.guest_os.starts_with("linux") || config.guest_os == "macos")
    {
        return;
    }
    let Some(public_dir) = &config.public_dir else {
        return;
    };
    if !public_dir.is_dir() {
        return;
    }

    let username = host
        .username
        .chars()
        .filter(|character| character.is_ascii_alphanumeric() || "-_".contains(*character))
        .collect::<String>();
    let mount_tag = format!(
        "Public-{}",
        if username.is_empty() {
            "user"
        } else {
            username.as_str()
        }
    );
    if virtiofs_requested(config, host) {
        let ram = config.ram.as_deref().unwrap_or(host.ram.as_str());
        args.extend([
            "-object".to_string(),
            format!("memory-backend-file,id=mem,size={ram},mem-path=/dev/shm,share=on"),
            "-numa".to_string(),
            "node,memdev=mem".to_string(),
            "-chardev".to_string(),
            format!(
                "socket,id=char0,path={}",
                qemu_path(&vm.paths.virtiofs_socket())
            ),
            "-device".to_string(),
            format!("vhost-user-fs-pci,queue-size=1024,chardev=char0,tag={mount_tag}"),
        ]);
        return;
    }

    args.extend([
        "-fsdev".to_string(),
        format!(
            "local,id=fsdev0,path={},security_model=mapped-xattr",
            qemu_path(public_dir)
        ),
        "-device".to_string(),
        format!("virtio-9p-pci,fsdev=fsdev0,mount_tag={mount_tag}"),
    ]);
}

pub(super) fn add_usb_args(args: &mut Vec<String>, config: &VmConfig) {
    match config.usb_controller.as_str() {
        "ehci" => args.extend(["-device".to_string(), "usb-ehci,id=input".to_string()]),
        "xhci" => args.extend(["-device".to_string(), "qemu-xhci,id=input".to_string()]),
        "none" => {}
        _ => {}
    }
    match config.keyboard.as_str() {
        "usb" => args.extend(["-device".to_string(), "usb-kbd,bus=input.0".to_string()]),
        "virtio" => args.extend(["-device".to_string(), "virtio-keyboard".to_string()]),
        _ => {}
    }
    if !config.keyboard_layout.is_empty() {
        args.extend(["-k".to_string(), config.keyboard_layout.clone()]);
    }
    match config.mouse.as_str() {
        "usb" => args.extend(["-device".to_string(), "usb-mouse,bus=input.0".to_string()]),
        "tablet" => args.extend(["-device".to_string(), "usb-tablet,bus=input.0".to_string()]),
        "virtio" => args.extend(["-device".to_string(), "virtio-mouse".to_string()]),
        _ => {}
    }
    if !config.usb_devices.is_empty() {
        args.extend(["-device".to_string(), "qemu-xhci,id=hostpass".to_string()]);
        for (vendor, product) in &config.usb_devices {
            args.extend([
                "-device".to_string(),
                format!(
                    "usb-host,bus=hostpass.0,vendorid=0x{vendor:04x},productid=0x{product:04x}"
                ),
            ]);
        }
    }
    if config.braille {
        args.extend(["-usbdevice".to_string(), "braille".to_string()]);
    }
}

pub(super) fn add_audio_args(args: &mut Vec<String>, config: &VmConfig, driver: Option<&str>) {
    let Some(driver) = driver else {
        return;
    };
    if config.sound_card == "none" {
        return;
    }

    args.extend([
        "-audiodev".to_string(),
        format!("driver={driver},id=audio0"),
    ]);
    match config.sound_card.as_str() {
        "ich9-intel-hda" | "intel-hda" => args.extend([
            "-device".to_string(),
            config.sound_card.clone(),
            "-device".to_string(),
            format!("{},audiodev=audio0", config.sound_duplex),
        ]),
        "usb-audio" | "virtio-sound-pci" | "ac97" | "es1370" | "sb16" => args.extend([
            "-device".to_string(),
            format!("{},audiodev=audio0", config.sound_card),
        ]),
        _ => {}
    }
}

pub(crate) fn configured_bridge(config: &VmConfig) -> Option<&str> {
    if config.offline || config.network.eq_ignore_ascii_case("none") {
        return None;
    }
    config.bridge.as_deref().or_else(|| {
        (!config.network.is_empty()
            && !config.network.eq_ignore_ascii_case("restrict")
            && !config.network.eq_ignore_ascii_case("user")
            && !uses_passt_network(config))
        .then_some(config.network.as_str())
    })
}

pub(super) fn uses_user_network(config: &VmConfig) -> bool {
    !config.offline
        && configured_bridge(config).is_none()
        && (config.network.is_empty()
            || config.network.eq_ignore_ascii_case("restrict")
            || config.network.eq_ignore_ascii_case("user"))
}

pub(super) fn uses_passt_network(config: &VmConfig) -> bool {
    !config.offline && config.network.eq_ignore_ascii_case("passt")
}

pub(super) fn uses_port_forwarding_network(config: &VmConfig) -> bool {
    uses_user_network(config) || uses_passt_network(config)
}
