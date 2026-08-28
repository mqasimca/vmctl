use super::*;

pub(super) fn qemu_version(output: &[u8]) -> Option<(u32, u32, u32)> {
    String::from_utf8_lossy(output)
        .split_whitespace()
        .find_map(parse_version_token)
}

pub(super) fn parse_version_token(token: &str) -> Option<(u32, u32, u32)> {
    let token =
        token.trim_matches(|character: char| !character.is_ascii_digit() && character != '.');
    let mut parts = token.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next().unwrap_or("0").parse().ok()?;
    Some((major, minor, patch))
}

pub(super) fn qemu_version_supported((major, minor, _patch): (u32, u32, u32)) -> bool {
    major > 6 || (major == 6 && minor >= 1)
}

pub(super) fn qemu_supports_gtk_clipboard(binary: &str) -> bool {
    qemu_help_output(binary, &["-version"])
        .as_deref()
        .and_then(|output| qemu_version(output.as_bytes()))
        .is_some_and(qemu_version_supports_gtk_clipboard)
}

pub(super) fn qemu_version_supports_gtk_clipboard(version: (u32, u32, u32)) -> bool {
    version >= (11, 1, 0)
}

pub(super) fn qemu_supports_vdagent(binary: &str) -> bool {
    qemu_help_output(binary, &["-chardev", "help"])
        .is_some_and(|output| output.contains("qemu-vdagent"))
}

pub(super) use crate::util::{command_available, find_executable};

pub(super) fn find_virtiofsd() -> Option<String> {
    find_executable("virtiofsd").or_else(|| {
        [
            "/usr/lib/virtiofsd",
            "/usr/libexec/virtiofsd",
            "/usr/lib/qemu/virtiofsd",
        ]
        .into_iter()
        .map(PathBuf::from)
        .find(|candidate| candidate.is_file())
        .map(|candidate| candidate.to_string_lossy().into_owned())
    })
}

pub(crate) fn virtiofsd_available() -> bool {
    find_virtiofsd().is_some()
}

pub(super) fn qemu_supports_device(binary: &str, device: &str) -> bool {
    qemu_help_output(binary, &["-device", "help"])
        .is_some_and(|text| qemu_quoted_names(&text).iter().any(|name| name == device))
}

pub(super) fn qemu_supports_gl_devices_in_names(names: &[String], arch: &str) -> bool {
    let devices = if arch == "aarch64" {
        ["virtio-gpu-gl-pci", "virtio-gpu-gl", ""]
    } else {
        ["virtio-vga-gl", "virtio-gpu-gl-pci", "virtio-gpu-gl"]
    };
    devices
        .into_iter()
        .filter(|device| !device.is_empty())
        .any(|device| names.iter().any(|name| name == device))
}

pub(super) fn gl_device_supported(host: &QemuPlanContext, device: &str) -> bool {
    // `virtio_vga_gl` is only a cheap capability gate; the selected device is
    // always queried again here so one GL variant cannot authorize another.
    if !host.virtio_vga_gl {
        return false;
    }
    if command_available(&host.qemu_binary) {
        qemu_supports_device(&host.qemu_binary, device)
    } else {
        false
    }
}

pub(super) fn qemu_quoted_names(text: &str) -> Vec<String> {
    text.lines()
        .flat_map(|line| {
            line.split('"')
                .enumerate()
                .filter(|(index, _)| index % 2 == 1)
                .map(|(_, value)| value.to_string())
                .collect::<Vec<_>>()
        })
        .collect()
}

pub(super) fn qemu_supports_cpu_in_text(text: &str, model: &str) -> bool {
    text.lines().any(|line| {
        line.split_whitespace()
            .next()
            .is_some_and(|candidate| candidate == model)
    })
}

const MAX_PROBE_OUTPUT: usize = 64 * 1024;

pub(super) fn read_limited(mut reader: impl Read) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 8192];
    loop {
        let count = reader.read(&mut chunk)?;
        if count == 0 {
            return Ok(bytes);
        }
        if bytes.len() + count > MAX_PROBE_OUTPUT {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "QEMU probe output exceeded the 64 KiB limit",
            ));
        }
        bytes.extend_from_slice(&chunk[..count]);
    }
}

pub(super) fn qemu_help_output(binary: &str, args: &[&str]) -> Option<String> {
    let mut child = Command::new(binary)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;
    let (Some(mut stdout), Some(mut stderr)) = (child.stdout.take(), child.stderr.take()) else {
        let _ = child.kill();
        let _ = child.wait();
        return None;
    };
    let stdout_reader = thread::spawn(move || read_limited(&mut stdout));
    let stderr_reader = thread::spawn(move || read_limited(&mut stderr));
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut probe_failed = false;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
            Ok(None) => {
                let _ = child.kill();
                probe_failed = true;
                break;
            }
            Err(_) => {
                let _ = child.kill();
                probe_failed = true;
                break;
            }
        }
    }
    let status = match child.wait() {
        Ok(status) => Some(status),
        Err(_) => {
            let _ = child.kill();
            let _ = child.wait();
            None
        }
    };
    let stdout = stdout_reader.join().ok()?.ok()?;
    let stderr = stderr_reader.join().ok()?.ok()?;
    if probe_failed || !status.is_some_and(|status| status.success()) {
        return None;
    }
    Some({
        let mut text = String::from_utf8_lossy(&stdout).into_owned();
        text.push_str(&String::from_utf8_lossy(&stderr));
        text
    })
}

pub(super) fn qemu_accelerators_probe(binary: &str) -> Option<Vec<String>> {
    qemu_help_output(binary, &["-accel", "help"]).map(|text| qemu_accelerators_from_text(&text))
}

pub(super) fn qemu_accelerators_from_text(text: &str) -> Vec<String> {
    text.lines()
        .skip_while(|line| !line.contains("Accelerators supported"))
        .skip(1)
        .map(str::trim)
        .take_while(|value| !value.is_empty())
        .filter(|value| {
            value
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || character == '-')
        })
        .map(str::to_string)
        .collect()
}

pub(super) fn qemu_runtime_accelerators(
    binary: &str,
    compiled: &[String],
) -> (Vec<String>, Vec<String>, Vec<String>) {
    let mut usable = Vec::new();
    let mut failures = Vec::new();
    let mut unprobed = Vec::new();
    for accelerator in compiled {
        if accelerator == "tcg" {
            usable.push(accelerator.clone());
        } else if matches!(accelerator.as_str(), "kvm" | "hvf" | "whpx") {
            if qemu_accelerator_usable(binary, accelerator) {
                usable.push(accelerator.clone());
            } else {
                failures.push(accelerator.clone());
            }
        } else {
            unprobed.push(accelerator.clone());
        }
    }
    (usable, failures, unprobed)
}

pub(super) fn read_qmp_greeting(mut reader: impl Read) -> io::Result<bool> {
    let mut greeting = Vec::new();
    let mut byte = [0_u8; 1];
    loop {
        let count = reader.read(&mut byte)?;
        if count == 0 {
            break;
        }
        if greeting.len() == MAX_PROBE_OUTPUT {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "QMP greeting exceeded the 64 KiB limit",
            ));
        }
        greeting.push(byte[0]);
        if byte[0] == b'\n' {
            break;
        }
    }
    if greeting.is_empty() {
        return Ok(false);
    }
    let value: Value = serde_json::from_slice(&greeting).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("invalid QMP greeting: {error}"),
        )
    })?;
    let Some(qmp) = value.get("QMP").and_then(Value::as_object) else {
        return Ok(false);
    };
    Ok(qmp.get("version").is_some_and(Value::is_object)
        && qmp.get("capabilities").is_some_and(Value::is_array))
}

pub(super) fn qemu_runtime_probe(binary: &str, accelerator: &str, cpu: &str) -> Result<()> {
    let machine = format!("accel={accelerator}");
    let mut child = Command::new(binary)
        .args([
            "-nodefaults",
            "-S",
            "-display",
            "none",
            "-machine",
            &machine,
            "-cpu",
            cpu,
            "-qmp",
            "stdio",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| Error::command_unavailable(binary, error))?;
    let Some(mut stdout) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return Err(Error::message(
            "QEMU CPU capability probe did not provide QMP output",
        ));
    };
    let Some(mut stderr) = child.stderr.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return Err(Error::message(
            "QEMU CPU capability probe did not provide stderr",
        ));
    };
    let (ready_tx, ready_rx) = mpsc::channel();
    let stdout_reader = thread::spawn(move || {
        let ready = read_qmp_greeting(&mut stdout);
        let _ = ready_tx.send(ready);
        let mut discarded = [0_u8; 8192];
        while stdout.read(&mut discarded).is_ok_and(|count| count > 0) {}
    });
    let stderr_reader = thread::spawn(move || read_limited(&mut stderr));
    let readiness = ready_rx.recv_timeout(Duration::from_secs(2));
    let mut exited_after_ready = false;
    let mut settle_error = None;
    if matches!(readiness, Ok(Ok(true))) {
        let settle_deadline = Instant::now() + Duration::from_millis(250);
        loop {
            match child.try_wait() {
                Ok(Some(_)) => {
                    exited_after_ready = true;
                    break;
                }
                Ok(None) if Instant::now() < settle_deadline => {
                    thread::sleep(Duration::from_millis(10));
                }
                Ok(None) => break,
                Err(error) => {
                    settle_error = Some(error);
                    break;
                }
            }
        }
    }
    let mut killed_by_us = false;
    if !exited_after_ready {
        match child.try_wait() {
            Ok(Some(_)) => exited_after_ready = true,
            Ok(None) => killed_by_us = child.kill().is_ok(),
            Err(error) => {
                settle_error = Some(error);
                let _ = child.kill();
            }
        }
    }
    let status = match child.wait() {
        Ok(status) => status,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            let _ = stdout_reader.join();
            let _ = stderr_reader.join();
            return Err(Error::io("QEMU CPU capability probe", error));
        }
    };
    let _ = stdout_reader.join();
    let stderr = stderr_reader
        .join()
        .map_err(|_| Error::message("QEMU CPU capability probe reader failed"))?
        .map_err(|error| Error::io("QEMU CPU capability probe", error))?;
    if let Some(error) = settle_error {
        return Err(Error::io("QEMU CPU capability probe", error));
    }
    match readiness {
        Ok(Ok(true)) if !exited_after_ready && killed_by_us => Ok(()),
        Ok(Ok(false)) => Err(Error::message(probe_error_message(
            "QEMU runtime probe failed",
            &stderr,
            status,
        ))),
        Ok(Ok(true)) => Err(Error::message(probe_error_message(
            "QEMU runtime probe exited during initialization",
            &stderr,
            status,
        ))),
        Ok(Err(error)) => Err(Error::io("QEMU CPU capability probe", error)),
        Err(_) => Err(Error::message(
            "QEMU runtime probe timed out after 2 seconds",
        )),
    }
}

pub(super) fn qemu_accelerator_usable(binary: &str, accelerator: &str) -> bool {
    qemu_runtime_probe(binary, accelerator, "max").is_ok()
}

pub(super) fn validate_cpu_spec(binary: &str, cpu: &str, accelerator: &str) -> Result<()> {
    qemu_runtime_probe(binary, accelerator, cpu).map_err(|error| {
        Error::message(format!("QEMU rejected CPU specification '{cpu}': {error}"))
    })
}

pub(super) fn probe_error_message(
    prefix: &str,
    stderr: &[u8],
    status: std::process::ExitStatus,
) -> String {
    let detail = String::from_utf8_lossy(stderr).trim().to_string();
    if detail.is_empty() {
        format!("{prefix} with status {status}")
    } else {
        format!("{prefix}: {detail}")
    }
}

pub(super) fn qemu_display_backends_probe(binary: &str) -> Option<Vec<String>> {
    qemu_help_output(binary, &["-display", "help"])
        .map(|text| qemu_display_backends_from_text(&text))
}

pub(super) fn qemu_display_backends_from_text(text: &str) -> Vec<String> {
    let mut backends = Vec::new();
    let mut in_section = false;
    for line in text.lines() {
        if line.contains("Available display backend types:") {
            in_section = true;
            continue;
        }
        if !in_section {
            continue;
        }
        let value = line.trim();
        if value.is_empty() || value.starts_with("Some ") {
            break;
        }
        if value.chars().all(|character| {
            character.is_ascii_alphanumeric() || character == '-' || character == '_'
        }) {
            backends.push(value.to_string());
        }
    }
    backends
}

pub(super) fn qemu_netdev_backends_probe(binary: &str) -> Option<Vec<String>> {
    qemu_help_output(binary, qemu_netdev_help_args(binary))
        .map(|text| qemu_netdev_backends_from_text(&text))
}

pub(super) fn qemu_netdev_help_args(binary: &str) -> &[&str] {
    if binary.contains("aarch64") {
        &["-machine", "virt", "-netdev", "help"][..]
    } else {
        &["-netdev", "help"][..]
    }
}

pub(super) fn qemu_netdev_backends_from_text(text: &str) -> Vec<String> {
    let mut backends = Vec::new();
    let mut in_section = false;
    for line in text.lines() {
        if line.contains("Available netdev backend types:") {
            in_section = true;
            continue;
        }
        if !in_section {
            continue;
        }
        let value = line.trim();
        if value.is_empty() || value.starts_with("Some ") {
            break;
        }
        if value.chars().all(|character| {
            character.is_ascii_alphanumeric() || character == '-' || character == '_'
        }) {
            backends.push(value.to_string());
        }
    }
    backends
}

pub(crate) fn qemu_capability_report(binary: &str) -> Value {
    let version = qemu_help_output(binary, &["-version"]).and_then(|text| {
        qemu_version(text.as_bytes())
            .map(|(major, minor, patch)| format!("{major}.{minor}.{patch}"))
    });
    let available = version.is_some();
    if !available {
        return json!({
            "available": false,
            "complete": false,
            "version": Value::Null,
            "probe_error": format!(
                "could not execute '{binary}' or its capability query failed"
            ),
            "accelerators": [],
            "runtime_accelerators": [],
            "runtime_probe_failures": [],
            "runtime_unprobed": [],
            "runtime_complete": false,
            "display_backends": [],
            "network_backends": [],
            "devices": {},
            "cpu_models": {},
        });
    }
    let display_probe = qemu_help_output(binary, &["-display", "help"]);
    let display = display_probe
        .as_deref()
        .map_or_else(Vec::new, qemu_display_backends_from_text);
    let network_backends = qemu_netdev_backends_probe(binary);
    let accelerator_probe = qemu_help_output(binary, &["-accel", "help"]);
    let accelerators = accelerator_probe
        .as_deref()
        .map_or_else(Vec::new, qemu_accelerators_from_text);
    let (runtime_accelerators, runtime_probe_failures, runtime_unprobed) =
        qemu_runtime_accelerators(binary, &accelerators);
    let device_probe = qemu_help_output(binary, &["-device", "help"]);
    let device_names = device_probe
        .as_deref()
        .map_or_else(Vec::new, qemu_quoted_names);
    let cpu_probe = qemu_help_output(binary, &["-cpu", "help"]);
    let complete = display_probe.is_some()
        && network_backends.is_some()
        && accelerator_probe.is_some()
        && device_probe.is_some()
        && cpu_probe.is_some();
    let devices = [
        "virtio-vga-gl",
        "virtio-gpu-gl",
        "virtio-gpu-gl-pci",
        "usb-redir",
        "usb-ccid",
        "ccid-card-passthru",
        "vhost-user-fs-pci",
        "virtio-sound-pci",
    ];
    let device_support = devices
        .into_iter()
        .map(|device| {
            (
                device.to_string(),
                json!(device_probe.is_some() && device_names.iter().any(|name| name == device)),
            )
        })
        .collect::<serde_json::Map<_, _>>();
    let cpu_models = ["host", "max", "qemu64", "qemu32", "Haswell-v2"]
        .into_iter()
        .map(|model| {
            (
                model.to_string(),
                json!(
                    cpu_probe
                        .as_deref()
                        .is_some_and(|text| qemu_supports_cpu_in_text(text, model))
                ),
            )
        })
        .collect::<serde_json::Map<_, _>>();
    let failed_probes = [
        ("display", display_probe.is_none()),
        ("network", network_backends.is_none()),
        ("accelerator", accelerator_probe.is_none()),
        ("device", device_probe.is_none()),
        ("cpu", cpu_probe.is_none()),
    ]
    .into_iter()
    .filter_map(|(name, failed)| failed.then_some(name))
    .collect::<Vec<_>>();
    json!({
        "available": available,
        "complete": complete,
        "version": version,
        "probe_error": (!complete).then(|| {
            format!("capability probes failed: {}", failed_probes.join(", "))
        }),
        "accelerators": accelerators,
        "runtime_accelerators": runtime_accelerators,
        "runtime_probe_failures": runtime_probe_failures,
        "runtime_unprobed": runtime_unprobed,
        "runtime_complete": accelerator_probe.is_some()
            && runtime_probe_failures.is_empty()
            && runtime_unprobed.is_empty(),
        "display_backends": display,
        "network_backends": network_backends.unwrap_or_default(),
        "devices": device_support,
        "cpu_models": cpu_models,
    })
}
