use super::*;

pub(super) fn logs_vm(
    dirs: &Dirs,
    name: &str,
    max_lines: usize,
    output: OutputFormat,
) -> Result<()> {
    let vm = find(&dirs.vm_dir, &dirs.state_root, name)?;
    let path = vm.paths.state_dir.join("qemu.log");
    let (lines, truncated) = read_log_lines(&path, max_lines)?;
    if output == OutputFormat::Json {
        let returned_lines = lines.len();
        print_json_success(json!({
            "name": vm.config.name,
            "path": path,
            "lines": lines,
            "returned_lines": returned_lines,
            "truncated": truncated,
        }));
    } else if lines.is_empty() {
        println!("{} is empty", path.display());
    } else {
        println!("Last {} line(s) from {}:", lines.len(), path.display());
        for line in lines {
            println!("{line}");
        }
    }
    Ok(())
}

pub(super) fn read_log_lines(path: &Path, max_lines: usize) -> Result<(Vec<String>, bool)> {
    const MAX_LOG_BYTES: usize = 1024 * 1024;
    let bytes = fs::read(path).map_err(|error| Error::io(path.display(), error))?;
    let byte_start = bytes.len().saturating_sub(MAX_LOG_BYTES);
    let text = String::from_utf8_lossy(&bytes[byte_start..]);
    let mut lines: Vec<String> = text.lines().map(redact_diagnostic).collect();
    let line_truncated = lines.len() > max_lines;
    if line_truncated {
        let keep_from = lines.len() - max_lines;
        lines.drain(..keep_from);
    }
    Ok((lines, byte_start != 0 || line_truncated))
}

pub(super) fn validate_usb_devices(vm: &Vm) -> Result<()> {
    if vm.config.usb_devices.is_empty() || env::consts::OS != "linux" {
        return Ok(());
    }
    if find_command("lsusb").is_none() {
        return Err(Error::message(
            "USB pass-through requires lsusb; install the usbutils package and retry",
        ));
    }
    for (vendor, product) in &vm.config.usb_devices {
        let device = format!("{vendor:04x}:{product:04x}");
        let found = ProcessCommand::new("lsusb")
            .args(["-d", &device])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success());
        if !found {
            return Err(Error::message(format!(
                "USB device {device} is not present or accessible; connect it or remove usb_devices"
            )));
        }
    }
    Ok(())
}

pub(super) fn redact_diagnostic(value: &str) -> String {
    let mut redacted = value.to_string();
    for key in ["osk=", "password=", "secret=", "token="] {
        let mut search_from = 0;
        while let Some(relative_start) = redacted[search_from..].find(key) {
            let start = search_from + relative_start;
            let value_start = start + key.len();
            let value_end = redacted[value_start..]
                .find(|character: char| character == ',' || character.is_whitespace())
                .map_or(redacted.len(), |offset| value_start + offset);
            if value_end == value_start {
                search_from = value_start;
                continue;
            }
            redacted.replace_range(value_start..value_end, "<redacted>");
            search_from = value_start + "<redacted>".len();
        }
    }
    redacted
}
