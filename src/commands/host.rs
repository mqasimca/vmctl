use super::*;

pub(super) fn host_action(action: HostAction, output: OutputFormat) -> Result<()> {
    match action {
        HostAction::IgnoreMsrsAlways => configure_ignore_msrs(output, true),
    }
}

pub(super) fn configure_ignore_msrs(output: OutputFormat, report: bool) -> Result<()> {
    if env::consts::OS != "linux" {
        return Err(Error::message(
            "persistent KVM MSR settings are only supported on Linux",
        ));
    }
    let path = Path::new("/etc/modprobe.d/vmctl-kvm.conf");
    crate::util::ensure_not_symlink(path, "write through")?;
    let existing = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(Error::io(path.display(), error)),
    };
    let setting = "options kvm ignore_msrs=Y";
    let already_configured = existing.lines().any(|line| line.trim() == setting);
    if !already_configured {
        let contents = if existing.is_empty() {
            format!("{setting}\n")
        } else {
            format!("{}\n{setting}\n", existing.trim_end())
        };
        write_host_file(path, &contents)?;
    }

    let initramfs = if already_configured {
        "already configured"
    } else if let Some(command) = find_command("update-initramfs") {
        let mut process = if host_needs_sudo() {
            let mut process = ProcessCommand::new("sudo");
            process.arg(&command);
            process
        } else {
            ProcessCommand::new(&command)
        };
        let status = process
            .args(["-k", "all", "-u"])
            .status()
            .map_err(|error| Error::command_unavailable(&command, error))?;
        if !status.success() {
            return Err(Error::command_failed_status(&command, status));
        }
        "rebuilt"
    } else if let Some(command) = find_command("mkinitcpio") {
        let mut process = if host_needs_sudo() {
            let mut process = ProcessCommand::new("sudo");
            process.arg(&command);
            process
        } else {
            ProcessCommand::new(&command)
        };
        let status = process
            .arg("-P")
            .status()
            .map_err(|error| Error::command_unavailable(&command, error))?;
        if !status.success() {
            return Err(Error::command_failed_status(&command, status));
        }
        "rebuilt with mkinitcpio"
    } else {
        "not available; reboot or rebuild initramfs manually"
    };

    if !report {
        return Ok(());
    }
    if output == OutputFormat::Json {
        print_json_success(json!({
            "path": path,
            "configured": true,
            "initramfs": initramfs,
        }));
    } else {
        println!("Configured {}", path.display());
        println!("initramfs: {initramfs}");
    }
    Ok(())
}

#[cfg(unix)]
fn host_needs_sudo() -> bool {
    // Safe: querying the effective UID has no side effects.
    needs_sudo_for_uid(unsafe { libc::geteuid() })
}

#[cfg(not(unix))]
fn host_needs_sudo() -> bool {
    false
}

#[cfg(any(unix, test))]
fn needs_sudo_for_uid(effective_uid: u32) -> bool {
    effective_uid != 0
}

pub(super) fn write_host_file(path: &Path, contents: &str) -> Result<bool> {
    match fs::write(path, contents) {
        Ok(()) => Ok(false),
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
            let mut child = ProcessCommand::new("sudo")
                .args(["tee", path.to_string_lossy().as_ref()])
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::inherit())
                .spawn()
                .map_err(|error| Error::command_unavailable("sudo", error))?;
            child
                .stdin
                .take()
                .ok_or_else(|| Error::message("sudo did not provide stdin"))?
                .write_all(contents.as_bytes())
                .map_err(|error| Error::io(path.display(), error))?;
            let status = child
                .wait()
                .map_err(|error| Error::command_unavailable("sudo", error))?;
            if status.success() {
                Ok(true)
            } else {
                Err(Error::command_failed_status("sudo tee", status))
            }
        }
        Err(error) => Err(Error::io(path.display(), error)),
    }
}

pub(super) use crate::util::find_executable as find_command;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_root_host_actions_use_sudo() {
        assert!(!needs_sudo_for_uid(0));
        assert!(needs_sudo_for_uid(1));
    }
}
