use super::*;

pub(super) fn add_tpm_args(args: &mut Vec<String>, vm: &Vm, host_os: &str) {
    add(
        args,
        "-chardev",
        socket_chardev(&vm.paths.tpm_socket(), "chrtpm", host_os),
    );
    add(
        args,
        "-tpmdev",
        "emulator,id=tpm0,chardev=chrtpm".to_string(),
    );
    args.extend([
        "-device".to_string(),
        if vm.config.arch == "aarch64" {
            "tpm-tis-device,tpmdev=tpm0".to_string()
        } else {
            "tpm-tis,tpmdev=tpm0".to_string()
        },
    ]);
}

pub(super) fn display_args(
    config: &VmConfig,
    host: &QemuPlanContext,
) -> Result<(String, String, Option<u16>)> {
    if config.clipboard && config.display != "gtk" {
        return Err(Error::message(
            "clipboard requires the GTK display backend; select --display gtk on a host where GTK is available",
        ));
    }
    if config.clipboard && !qemu_supports_gtk_clipboard(&host.qemu_binary) {
        return Err(Error::message(
            "GTK clipboard sharing requires QEMU 11.1.0 or newer",
        ));
    }
    if config.clipboard && !qemu_supports_vdagent(&host.qemu_binary) {
        return Err(Error::message(
            "GTK clipboard sharing requires QEMU built with qemu-vdagent support; install QEMU's SPICE module",
        ));
    }
    if config.display == "cocoa" && host.host_os != "macos" {
        return Err(Error::message(
            "display mode 'cocoa' is only supported on macOS",
        ));
    }
    let render_node = render_node();
    let local_spice_gl = config.display == "spice"
        && config.access == "local"
        && config.gl.unwrap_or(true)
        && host.virtio_vga_gl
        && qemu_display_backends_probe(&host.qemu_binary)
            .is_some_and(|backends| backends.iter().any(|backend| backend == "egl-headless"))
        && render_node.is_some();
    let requested_gl = config.gl.unwrap_or(true)
        && !matches!(config.display.as_str(), "none")
        && (config.display != "spice" || local_spice_gl);
    let device = match config.guest_os.as_str() {
        guest if guest.ends_with("bsd") => "VGA",
        "linux-old" | "linux_old" | "solaris" => "vmware-svga",
        "macos" => "vmware-svga",
        "windows" | "windows-server" if config.arch == "aarch64" => "virtio-gpu-pci",
        "windows" | "windows-server" if matches!(config.display.as_str(), "none" | "spice") => {
            "qxl-vga"
        }
        "batocera" | "haiku" | "kolibrios" | "reactos" => "qxl-vga",
        _ if config.arch == "aarch64" => "virtio-gpu-pci",
        "linux" if matches!(config.display.as_str(), "none" | "spice" | "spice-app") => {
            "virtio-gpu"
        }
        _ => "virtio-vga",
    };
    let gl_device = match device {
        "virtio-vga" => Some("virtio-vga-gl"),
        "virtio-gpu-pci" => Some("virtio-gpu-gl-pci"),
        "virtio-gpu" => Some("virtio-gpu-gl"),
        _ => None,
    };
    let device =
        if requested_gl && gl_device.is_some_and(|device| gl_device_supported(host, device)) {
            match device {
                "virtio-vga" => "virtio-vga-gl",
                "virtio-gpu-pci" => "virtio-gpu-gl-pci",
                "virtio-gpu" => "virtio-gpu-gl",
                _ => device,
            }
        } else {
            device
        };
    let supports_resolution = device.starts_with("virtio-") || device.starts_with("qxl");
    let resolution = if supports_resolution {
        match (config.width, config.height) {
            (Some(width), Some(height)) => format!(",xres={width},yres={height}"),
            _ => String::new(),
        }
    } else {
        String::new()
    };
    let max_outputs = if device.starts_with("virtio-") || device.starts_with("qxl") {
        config
            .max_outputs
            .map_or_else(String::new, |outputs| format!(",max_outputs={outputs}"))
    } else {
        String::new()
    };
    let spice_port = match config.display.as_str() {
        "none" | "spice" => host.spice_port,
        _ => None,
    };
    let video = if device == "none" {
        "none".to_string()
    } else {
        format!("{device}{resolution}{max_outputs}")
    };
    let fullscreen = if config.fullscreen {
        ",full-screen=on"
    } else {
        ""
    };
    let gl = if requested_gl && (device.contains("-gl") || config.display == "cocoa") {
        "on"
    } else {
        "off"
    };

    let result = match config.display.as_str() {
        "none" => ("none".to_string(), video, spice_port),
        "spice" => (
            render_node.filter(|_| local_spice_gl).map_or_else(
                || "none".to_string(),
                |path| format!("egl-headless,rendernode={}", qemu_path(&path)),
            ),
            video,
            spice_port,
        ),
        "gtk" => (
            format!(
                "gtk{},grab-on-hover=on,zoom-to-fit=off,gl={gl}{fullscreen}",
                if config.clipboard {
                    ",clipboard=on"
                } else {
                    ""
                }
            ),
            video,
            None,
        ),
        "sdl" => (format!("sdl,gl={gl}{fullscreen}"), video, None),
        "cocoa" => (format!("cocoa{fullscreen}"), video, None),
        display => {
            return Err(Error::message(format!(
                "display mode '{display}' is not supported"
            )));
        }
    };

    Ok(result)
}
