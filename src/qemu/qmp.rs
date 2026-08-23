use super::*;

pub(super) fn execute_qmp(
    stream: &mut IpcStream,
    reader: &mut BufReader<IpcStream>,
    command: &str,
    id: &str,
    arguments: Option<Value>,
    deadline: Instant,
) -> Result<Value> {
    let request = match arguments {
        Some(arguments) => json!({
            "execute": command,
            "arguments": arguments,
            "id": id,
        }),
        None => json!({ "execute": command, "id": id }),
    };
    write_all_until(stream, format!("{request}\n").as_bytes(), deadline)
        .map_err(|error| Error::Qmp(format!("cannot send {command}: {error}")))?;

    loop {
        let response = read_qmp_message_until(reader, deadline)?;
        let object = response
            .as_object()
            .ok_or_else(|| Error::Qmp(format!("{command} returned a non-object response")))?;
        if object.get("event").is_some() {
            continue;
        }
        if let Some(error) = object.get("error")
            && object.get("id").is_none()
        {
            return Err(Error::Qmp(format!("{command} rejected: {error}")));
        }
        if object.get("id").and_then(Value::as_str) != Some(id) {
            continue;
        }
        if let Some(error) = object.get("error") {
            return Err(Error::Qmp(format!("{command} rejected: {error}")));
        }
        return object
            .get("return")
            .cloned()
            .ok_or_else(|| Error::Qmp(format!("{command} response has neither return nor error")));
    }
}

pub(super) const QMP_TIMEOUT: Duration = Duration::from_secs(2);

pub(super) fn qmp_deadline() -> Result<Instant> {
    Instant::now()
        .checked_add(QMP_TIMEOUT)
        .ok_or_else(|| Error::message("QMP timeout is too large"))
}

pub(super) fn read_qmp_greeting_until(
    reader: &mut BufReader<IpcStream>,
    deadline: Instant,
) -> Result<Value> {
    let greeting = read_qmp_message_until(reader, deadline)?;
    let Some(qmp) = greeting.get("QMP").and_then(Value::as_object) else {
        return Err(Error::Qmp("QEMU greeting has no QMP object".to_string()));
    };
    if !qmp.get("version").is_some_and(Value::is_object)
        || !qmp.get("capabilities").is_some_and(Value::is_array)
    {
        return Err(Error::Qmp(
            "QEMU greeting is missing version or capabilities".to_string(),
        ));
    }
    Ok(greeting)
}

pub(super) fn read_qmp_message_until(
    reader: &mut BufReader<IpcStream>,
    deadline: Instant,
) -> Result<Value> {
    let line = read_bounded_line_until(reader, MAX_QMP_MESSAGE, deadline)
        .map_err(|error| Error::Qmp(format!("cannot read QMP response: {error}")))?;
    if line.trim().is_empty() {
        return Err(Error::Qmp("QEMU closed the QMP socket".to_string()));
    }
    serde_json::from_str(line.trim())
        .map_err(|error| Error::Qmp(format!("invalid QMP response: {error}")))
}

pub(super) fn write_all_until(
    stream: &mut IpcStream,
    bytes: &[u8],
    deadline: Instant,
) -> io::Result<()> {
    let mut offset = 0;
    while offset < bytes.len() {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "IPC write timed out",
            ));
        }
        stream.set_write_timeout(Some(remaining))?;
        let written = stream.write(&bytes[offset..])?;
        if written == 0 {
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "IPC peer accepted no data",
            ));
        }
        offset += written;
    }
    Ok(())
}

const MAX_QMP_MESSAGE: usize = 8 * 1024 * 1024;

pub(super) fn connect_endpoint_retry(endpoint: &IpcEndpoint, service: &str) -> Result<IpcStream> {
    connect_endpoint_retry_with_timeout(endpoint, service, Duration::from_secs(1))
}

pub(super) fn connect_endpoint_retry_with_timeout(
    endpoint: &IpcEndpoint,
    service: &str,
    timeout: Duration,
) -> Result<IpcStream> {
    let deadline = Instant::now()
        .checked_add(timeout)
        .ok_or_else(|| Error::message("IPC connection timeout is too large"))?;
    let last_error = loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break io::Error::new(io::ErrorKind::TimedOut, "IPC connection timed out");
        }
        match endpoint.connect(remaining) {
            Ok(stream) => return Ok(stream),
            Err(error) => {
                if Instant::now() >= deadline {
                    break error;
                }
                thread::sleep(
                    Duration::from_millis(50)
                        .min(deadline.saturating_duration_since(Instant::now())),
                );
            }
        }
    };
    Err(Error::message(format!(
        "cannot connect to {service} endpoint {}: {}",
        endpoint.display(),
        last_error
    )))
}

pub(crate) fn process_matches(pid: i32, name: &str) -> bool {
    process_matches_checked(pid, name).unwrap_or(false)
}

pub(crate) fn process_matches_checked_with_identity(
    pid: i32,
    name: &str,
    expected_identity: Option<&str>,
) -> Result<bool> {
    if !process_matches_checked(pid, name)? {
        return Ok(false);
    }
    Ok(expected_identity
        .is_none_or(|expected| process_identity(pid).is_some_and(|actual| actual == expected)))
}

pub(crate) fn process_matches_checked(pid: i32, name: &str) -> Result<bool> {
    if pid <= 0 {
        return Ok(false);
    }

    #[cfg(target_os = "linux")]
    {
        let command_line = match fs::read(format!("/proc/{pid}/cmdline")) {
            Ok(command_line) => command_line,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(Error::io(format!("/proc/{pid}/cmdline"), error)),
        };
        let fields: Vec<&[u8]> = command_line.split(|byte| *byte == 0).collect();
        let executable = fields
            .first()
            .copied()
            .and_then(|value| std::str::from_utf8(value).ok())
            .and_then(|value| Path::new(value).file_name())
            .and_then(|value| value.to_str());
        if !executable.is_some_and(|value| value.starts_with("qemu-system-")) {
            return Ok(false);
        }
        if name.is_empty() {
            return Ok(true);
        }
        let command = String::from_utf8_lossy(&command_line);
        Ok(command_line_has_process_name(&command, name)
            || command_line_has_vm_name(&command, name))
    }

    #[cfg(windows)]
    {
        let powershell = Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                &format!("(Get-CimInstance Win32_Process -Filter 'ProcessId = {pid}').CommandLine"),
            ])
            .output();
        if let Ok(output) = powershell
            && output.status.success()
        {
            let command_line = String::from_utf8_lossy(&output.stdout);
            if command_line.trim().is_empty() {
                return Ok(false);
            }
            let qemu = command_line.to_ascii_lowercase().contains("qemu-system-");
            return Ok(qemu
                && (name.is_empty()
                    || command_line_has_process_name(&command_line, name)
                    || command_line_has_vm_name(&command_line, name)));
        }

        let tasklist = Command::new("tasklist.exe")
            .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
            .output()
            .map_err(|error| Error::command_unavailable("tasklist.exe", error))?;
        if !tasklist.status.success() {
            return Err(Error::message(format!(
                "cannot inspect process {pid}; PowerShell and tasklist both failed"
            )));
        }
        let task = String::from_utf8_lossy(&tasklist.stdout).to_ascii_lowercase();
        if task.trim().is_empty() || task.contains("no tasks are running") {
            return Ok(false);
        }
        if !task.contains("qemu-system-") {
            return Ok(false);
        }
        if name.is_empty() {
            return Ok(true);
        }
        return Err(Error::message(format!(
            "cannot verify VM process {pid} identity without PowerShell"
        )));
    }

    #[cfg(all(unix, not(target_os = "linux")))]
    {
        let output = Command::new("ps")
            .args(["-p", &pid.to_string(), "-o", "command="])
            .output()
            .map_err(|error| Error::command_unavailable("ps", error))?;
        if !output.status.success() {
            return Ok(false);
        }
        let command = String::from_utf8_lossy(&output.stdout);
        Ok(command.contains("qemu-system-")
            && (name.is_empty()
                || command_line_has_process_name(&command, name)
                || command_line_has_vm_name(&command, name)))
    }
}

pub(super) fn process_name_argument_matches(value: &str, name: &str) -> bool {
    let expected = format!("process={name}");
    value
        .trim_matches(['\'', '"'])
        .split(',')
        .any(|option| option == expected)
}

pub(super) fn command_line_has_process_name(command: &str, name: &str) -> bool {
    command
        .split(|character: char| character.is_whitespace() || character == '\0')
        .any(|argument| process_name_argument_matches(argument, name))
}

pub(super) fn command_line_has_vm_name(command: &str, name: &str) -> bool {
    command
        .split_whitespace()
        .collect::<Vec<_>>()
        .windows(2)
        .any(|pair| {
            if pair[0].trim_matches(['\'', '"']) != "-name" {
                return false;
            }
            let value = pair[1].trim_matches(['\'', '"']);
            let mut options = value.split(',');
            if options.next() != Some(name) {
                return false;
            }
            options
                .find_map(|option| option.strip_prefix("process="))
                .is_none_or(|process_name| process_name == name)
        })
}

#[cfg(target_os = "linux")]
pub(crate) fn process_identity(pid: i32) -> Option<String> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let rest = stat.rsplit_once(") ")?.1;
    rest.split_whitespace().nth(19).map(str::to_string)
}

#[cfg(windows)]
pub(crate) fn process_identity(pid: i32) -> Option<String> {
    let output = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            &format!("(Get-CimInstance Win32_Process -Filter 'ProcessId = {pid}').CreationDate"),
        ])
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .filter(|value| !value.is_empty())
}

#[cfg(all(unix, not(target_os = "linux")))]
pub(crate) fn process_identity(pid: i32) -> Option<String> {
    Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "lstart="])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| {
            let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
            (!value.is_empty()).then(|| value.split_whitespace().collect::<Vec<_>>().join("_"))
        })
}

#[cfg(not(any(unix, windows)))]
pub(crate) fn process_identity(_pid: i32) -> Option<String> {
    None
}

pub(super) fn process_record(pid: i32) -> String {
    process_identity(pid).map_or_else(
        || format!("{pid}\n"),
        |identity| format!("{pid} {identity}\n"),
    )
}

pub(super) fn read_process_record(path: &Path) -> Option<(i32, Option<String>)> {
    let contents = fs::read_to_string(path).ok()?;
    let mut fields = contents.split_whitespace();
    let pid = fields.next()?.parse().ok()?;
    Some((pid, fields.next().map(str::to_string)))
}

pub(super) fn helper_process_matches(
    pid: i32,
    name: &str,
    expected_identity: Option<&str>,
) -> bool {
    pid_matches(pid, name)
        && expected_identity
            .is_none_or(|expected| process_identity(pid).is_some_and(|actual| actual == expected))
}
