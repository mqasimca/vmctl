use super::*;

#[cfg(target_os = "linux")]
pub(super) fn pid_matches(pid: i32, needle: &str) -> bool {
    fs::read(format!("/proc/{pid}/cmdline"))
        .is_ok_and(|command_line| String::from_utf8_lossy(&command_line).contains(needle))
}

#[cfg(windows)]
pub(super) fn pid_matches(pid: i32, needle: &str) -> bool {
    if let Ok(output) = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            &format!("(Get-CimInstance Win32_Process -Filter 'ProcessId = {pid}').CommandLine"),
        ])
        .output()
        && output.status.success()
    {
        return String::from_utf8_lossy(&output.stdout)
            .to_ascii_lowercase()
            .contains(&needle.to_ascii_lowercase());
    }
    Command::new("tasklist.exe")
        .args(["/FI", &format!("PID eq {pid}"), "/NH"])
        .output()
        .is_ok_and(|output| {
            output.status.success()
                && String::from_utf8_lossy(&output.stdout)
                    .to_ascii_lowercase()
                    .contains(&needle.to_ascii_lowercase())
        })
}

#[cfg(all(unix, not(target_os = "linux")))]
pub(super) fn pid_matches(pid: i32, needle: &str) -> bool {
    Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "command="])
        .output()
        .is_ok_and(|output| String::from_utf8_lossy(&output.stdout).contains(needle))
}

#[cfg(not(any(unix, windows)))]
pub(super) fn pid_matches(_pid: i32, _needle: &str) -> bool {
    false
}
