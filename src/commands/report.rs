use super::*;

pub(super) fn report_host(output: OutputFormat) -> Result<()> {
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
            "passt": command_available("passt"),
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
        println!("passt: {}", report["commands"]["passt"]);
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
