use super::*;

pub(crate) fn guest_command(vm: &Vm, command: &str, arguments: Option<Value>) -> Result<Value> {
    guest_command_with_timeout(vm, command, arguments, Duration::from_secs(2), true)
}

pub(crate) fn guest_shutdown(vm: &Vm, deadline: Instant) -> Result<Value> {
    guest_command_until(vm, "guest-shutdown", None, deadline, false)
}

pub(crate) fn guest_fsfreeze_status(vm: &Vm) -> Result<String> {
    guest_command(vm, "guest-fsfreeze-status", None)?
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| {
            Error::guest_agent_protocol("guest-fsfreeze-status", "response is not a string")
        })
}

pub(crate) fn guest_fsfreeze_freeze(vm: &Vm) -> Result<u64> {
    guest_fsfreeze_count(vm, "guest-fsfreeze-freeze")
}

pub(crate) fn guest_fsfreeze_thaw(vm: &Vm) -> Result<u64> {
    guest_fsfreeze_count(vm, "guest-fsfreeze-thaw")
}

pub(crate) fn guest_fstrim(vm: &Vm) -> Result<Value> {
    guest_command(vm, "guest-fstrim", None)
}

fn guest_fsfreeze_count(vm: &Vm, command: &str) -> Result<u64> {
    guest_command(vm, command, None)?.as_u64().ok_or_else(|| {
        Error::guest_agent_protocol(command, "response is not a non-negative integer")
    })
}

pub(super) fn guest_command_with_timeout(
    vm: &Vm,
    command: &str,
    arguments: Option<Value>,
    read_timeout: Duration,
    expect_response: bool,
) -> Result<Value> {
    let deadline = Instant::now()
        .checked_add(read_timeout)
        .ok_or_else(|| Error::message("guest-agent timeout is too large"))?;
    guest_command_until(vm, command, arguments, deadline, expect_response)
}

pub(super) fn guest_command_until(
    vm: &Vm,
    command: &str,
    arguments: Option<Value>,
    deadline: Instant,
    expect_response: bool,
) -> Result<Value> {
    let endpoint = agent_endpoint_for_paths(&vm.paths)?;
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Err(Error::guest_agent_unavailable(
            command,
            "timeout expired before connecting",
        ));
    }
    let stream = connect_endpoint_retry_with_timeout(&endpoint, "guest-agent", remaining)
        .map_err(|error| Error::guest_agent_unavailable(command, error.to_string()))?;
    stream
        .set_read_timeout(Some(remaining))
        .map_err(|error| Error::io(endpoint.display(), error))?;
    stream
        .set_write_timeout(Some(remaining))
        .map_err(|error| Error::io(endpoint.display(), error))?;
    let mut reader = BufReader::new(stream);
    let sync_id = next_guest_sync_id();
    sync_guest_agent(&mut reader, sync_id, deadline).map_err(|error| match error.kind() {
        io::ErrorKind::TimedOut
        | io::ErrorKind::WouldBlock
        | io::ErrorKind::UnexpectedEof
        | io::ErrorKind::ConnectionReset
        | io::ErrorKind::BrokenPipe
        | io::ErrorKind::NotConnected
        | io::ErrorKind::ConnectionAborted => {
            Error::guest_agent_unavailable(command, "did not respond during synchronization")
        }
        io::ErrorKind::InvalidData => Error::guest_agent_protocol(command, error.to_string()),
        _ => Error::io(endpoint.display(), error),
    })?;
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Err(Error::guest_agent_unavailable(
            command,
            "timeout expired during synchronization",
        ));
    }
    reader
        .get_mut()
        .set_read_timeout(Some(remaining))
        .map_err(|error| Error::io(endpoint.display(), error))?;
    reader
        .get_mut()
        .set_write_timeout(Some(remaining))
        .map_err(|error| Error::io(endpoint.display(), error))?;

    let request = match arguments {
        Some(arguments) => json!({"execute": command, "arguments": arguments}),
        None => json!({"execute": command}),
    };
    write_all_until(
        reader.get_mut(),
        format!("{request}\n").as_bytes(),
        deadline,
    )
    .map_err(|error| Error::io(endpoint.display(), error))?;
    if !expect_response {
        let shutdown_deadline = Instant::now()
            .checked_add(Duration::from_millis(250))
            .map_or(deadline, |candidate| candidate.min(deadline));
        reader
            .get_mut()
            .set_read_timeout(Some(
                shutdown_deadline.saturating_duration_since(Instant::now()),
            ))
            .map_err(|error| Error::io(endpoint.display(), error))?;
        let line =
            match read_bounded_line_until(&mut reader, MAX_GUEST_AGENT_RESPONSE, shutdown_deadline)
            {
                Ok(line) if line.is_empty() => return Ok(Value::Null),
                Ok(line) if line.trim().is_empty() => {
                    return Err(Error::guest_agent_protocol(
                        command,
                        "response was an empty JSON line",
                    ));
                }
                Ok(line) => line,
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
                    ) =>
                {
                    return Ok(Value::Null);
                }
                Err(error) => return Err(Error::io(endpoint.display(), error)),
            };
        let response: Value = serde_json::from_str(line.trim()).map_err(|error| {
            Error::guest_agent_protocol(command, format!("invalid JSON: {error}"))
        })?;
        let object = response
            .as_object()
            .ok_or_else(|| Error::guest_agent_protocol(command, "response is not a JSON object"))?;
        if let Some(error) = object.get("error") {
            return Err(Error::guest_agent_protocol(
                command,
                format!("command rejected: {error}"),
            ));
        }
        return object.get("return").cloned().ok_or_else(|| {
            Error::guest_agent_protocol(command, "response has neither return nor error")
        });
    }

    let line = read_bounded_line_until(&mut reader, MAX_GUEST_AGENT_RESPONSE, deadline).map_err(
        |error| {
            if matches!(
                error.kind(),
                io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
            ) {
                Error::guest_agent_unavailable(
                    command,
                    "did not respond within the configured timeout",
                )
            } else if error.kind() == io::ErrorKind::InvalidData {
                Error::guest_agent_protocol(command, error.to_string())
            } else {
                Error::io(endpoint.display(), error)
            }
        },
    )?;
    if line.trim().is_empty() {
        return Err(Error::guest_agent_unavailable(
            command,
            "closed the connection without responding",
        ));
    }
    let response: Value = serde_json::from_str(line.trim())
        .map_err(|error| Error::guest_agent_protocol(command, format!("invalid JSON: {error}")))?;
    let object = response
        .as_object()
        .ok_or_else(|| Error::guest_agent_protocol(command, "response is not a JSON object"))?;
    if let Some(error) = object.get("error") {
        return Err(Error::guest_agent_protocol(
            command,
            format!("command rejected: {error}"),
        ));
    }
    object.get("return").cloned().ok_or_else(|| {
        Error::guest_agent_protocol(command, "response has neither return nor error")
    })
}

pub(super) fn next_guest_sync_id() -> i64 {
    let sequence = NEXT_GUEST_SYNC_ID.fetch_add(1, Ordering::Relaxed) as u64;
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos() as u64);
    let process = u64::from(std::process::id());
    let id = nanos ^ process.rotate_left(17) ^ sequence.rotate_left(31);
    (id & i64::MAX as u64).max(1) as i64
}

pub(super) fn sync_guest_agent(
    reader: &mut BufReader<IpcStream>,
    id: i64,
    deadline: Instant,
) -> io::Result<()> {
    let mut consumed = 0_usize;
    let mut sync_request = Vec::new();
    let request = json!({
        "execute": "guest-sync-delimited",
        "arguments": {"id": id},
    });
    sync_request.push(0xff);
    sync_request.extend_from_slice(format!("{request}\n").as_bytes());
    write_all_until(reader.get_mut(), &sync_request, deadline)?;

    let mut byte = [0_u8; 1];
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "guest agent synchronization timed out",
            ));
        }
        reader.get_mut().set_read_timeout(Some(remaining))?;
        if reader.read(&mut byte)? == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "guest agent closed during synchronization",
            ));
        }
        consumed += 1;
        if consumed > MAX_GUEST_AGENT_RESPONSE {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "guest agent synchronization exceeded the safety limit",
            ));
        }
        if byte[0] == 0xff {
            let remaining_limit = MAX_GUEST_AGENT_RESPONSE.saturating_sub(consumed);
            let line = read_bounded_line_until(reader, remaining_limit, deadline)?;
            consumed = consumed.saturating_add(line.len());
            if line.is_empty() {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "guest agent closed after synchronization sentinel",
                ));
            }
            let response: Value = serde_json::from_str(line.trim()).map_err(|error| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("invalid guest-sync-delimited response: {error}"),
                )
            })?;
            if response.get("return").and_then(Value::as_i64) == Some(id) {
                return Ok(());
            }
            continue;
        }
    }
}

pub(super) fn read_bounded_line_until(
    reader: &mut BufReader<IpcStream>,
    limit: usize,
    deadline: Instant,
) -> io::Result<String> {
    let mut bytes = Vec::new();
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "bounded read timed out",
            ));
        }
        reader.get_mut().set_read_timeout(Some(remaining))?;
        let chunk = reader.fill_buf()?;
        if chunk.is_empty() {
            break;
        }
        let newline = chunk.iter().position(|byte| *byte == b'\n');
        let count = newline.map_or(chunk.len(), |index| index + 1);
        if bytes.len() + count > limit {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("IPC response exceeded the {limit}-byte safety limit"),
            ));
        }
        bytes.extend_from_slice(&chunk[..count]);
        reader.consume(count);
        if newline.is_some() {
            break;
        }
    }
    String::from_utf8(bytes)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))
}

#[cfg(test)]
pub(super) fn read_bounded_line(reader: &mut impl BufRead, limit: usize) -> io::Result<String> {
    let mut bytes = Vec::new();
    loop {
        let chunk = reader.fill_buf()?;
        if chunk.is_empty() {
            break;
        }
        let newline = chunk.iter().position(|byte| *byte == b'\n');
        let count = newline.map_or(chunk.len(), |index| index + 1);
        if bytes.len() + count > limit {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("guest-agent response exceeded the {limit}-byte safety limit"),
            ));
        }
        bytes.extend_from_slice(&chunk[..count]);
        reader.consume(count);
        if newline.is_some() {
            break;
        }
    }
    String::from_utf8(bytes)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))
}

pub(crate) fn guest_exec(
    vm: &Vm,
    program: &str,
    args: &[String],
    timeout_secs: u64,
) -> Result<Value> {
    if timeout_secs == 0 {
        return Err(Error::message(
            "guest command timeout must be greater than zero",
        ));
    }
    let deadline = Instant::now()
        .checked_add(Duration::from_secs(timeout_secs))
        .ok_or_else(|| Error::message("guest command timeout is too large"))?;
    let pid = guest_command_until(
        vm,
        "guest-exec",
        Some(json!({
            "path": program,
            "arg": args,
            "capture-output": true,
        })),
        deadline,
        true,
    )?
    .get("pid")
    .and_then(Value::as_u64)
    .filter(|pid| *pid > 0)
    .ok_or_else(|| {
        Error::guest_agent_protocol("guest-exec", "response has no positive integer process id")
    })?;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(Error::guest_command_timeout(program, pid, timeout_secs));
        }
        let result = match guest_command_until(
            vm,
            "guest-exec-status",
            Some(json!({"pid": pid})),
            deadline,
            true,
        ) {
            Ok(result) => result,
            Err(_error) if Instant::now() >= deadline => {
                return Err(Error::guest_command_timeout(program, pid, timeout_secs));
            }
            Err(error) => return Err(error),
        };
        let exited = result
            .get("exited")
            .and_then(Value::as_bool)
            .ok_or_else(|| {
                Error::guest_agent_protocol(
                    "guest-exec-status",
                    "response has no boolean exited field",
                )
            })?;
        let exit_code = guest_status_integer(&result, "exitcode")?;
        let signal = guest_status_integer(&result, "signal")?;
        if exited {
            if exit_code.is_some() == signal.is_some() {
                return Err(Error::guest_agent_protocol(
                    "guest-exec-status",
                    "exited response must contain exactly one non-negative exitcode or positive signal",
                ));
            }
            return normalize_guest_exec_result(result);
        }
        if exit_code.is_some() || signal.is_some() {
            return Err(Error::guest_agent_protocol(
                "guest-exec-status",
                "running response must not contain exitcode or signal",
            ));
        }
        let sleep_for = deadline
            .saturating_duration_since(Instant::now())
            .min(Duration::from_millis(100));
        if sleep_for.is_zero() {
            return Err(Error::guest_command_timeout(program, pid, timeout_secs));
        }
        thread::sleep(sleep_for);
    }
}

const MAX_GUEST_AGENT_RESPONSE: usize = 8 * 1024 * 1024;

pub(super) fn guest_status_integer(result: &Value, key: &str) -> Result<Option<i64>> {
    let Some(value) = result.get(key) else {
        return Ok(None);
    };
    let number = value.as_i64().ok_or_else(|| {
        Error::guest_agent_protocol("guest-exec-status", format!("{key} must be an integer"))
    })?;
    let valid = if key == "signal" {
        number > 0
    } else {
        number >= 0
    };
    if !valid {
        return Err(Error::guest_agent_protocol(
            "guest-exec-status",
            format!("{key} has an invalid value"),
        ));
    }
    Ok(Some(number))
}

pub(super) fn normalize_guest_exec_result(mut result: Value) -> Result<Value> {
    let object = result.as_object_mut().ok_or_else(|| {
        Error::guest_agent_protocol("guest-exec-status", "response is not a JSON object")
    })?;
    for (encoded_key, text_key) in [("out-data", "stdout"), ("err-data", "stderr")] {
        let Some(value) = object.get(encoded_key) else {
            continue;
        };
        let encoded = value.as_str().map(str::to_owned).ok_or_else(|| {
            Error::guest_agent_protocol(
                "guest-exec-status",
                format!("{encoded_key} is not a base64 string"),
            )
        })?;
        let bytes = decode_base64(&encoded).map_err(|error| {
            Error::guest_agent_protocol(
                "guest-exec-status",
                format!("invalid {encoded_key}: {error}"),
            )
        })?;
        let utf8 = String::from_utf8(bytes).ok();
        object.insert(
            text_key.to_string(),
            utf8.map_or(Value::Null, Value::String),
        );
        object.insert(format!("{text_key}_base64"), Value::String(encoded));
        object.insert(
            format!("{text_key}_encoding"),
            Value::String(
                if object[text_key].is_null() {
                    "base64"
                } else {
                    "utf-8"
                }
                .to_string(),
            ),
        );
    }
    Ok(result)
}

pub(super) fn decode_base64(value: &str) -> Result<Vec<u8>> {
    let bytes = value
        .bytes()
        .filter(|byte| !byte.is_ascii_whitespace())
        .collect::<Vec<_>>();
    if bytes.is_empty() {
        return Ok(Vec::new());
    }
    if bytes.len() % 4 == 1 {
        return Err(Error::message("base64 data has an invalid length"));
    }
    let mut output = Vec::with_capacity(bytes.len() * 3 / 4);
    for (chunk_index, chunk) in bytes.chunks(4).enumerate() {
        let last = chunk_index == bytes.len().div_ceil(4) - 1;
        let mut digits = [0_u8; 4];
        for (index, byte) in chunk.iter().copied().enumerate() {
            if byte == b'=' {
                if !last || index < 2 {
                    return Err(Error::message("base64 padding is in an invalid position"));
                }
                continue;
            }
            digits[index] = base64_digit(byte)
                .ok_or_else(|| Error::message("base64 data contains an invalid character"))?;
        }
        if chunk.len() < 4 && chunk.contains(&b'=') {
            return Err(Error::message("base64 padding is incomplete"));
        }
        if chunk.len() == 2 {
            if digits[1] & 0x0f != 0 {
                return Err(Error::message("base64 data has non-zero trailing bits"));
            }
            output.push((digits[0] << 2) | (digits[1] >> 4));
        } else if chunk.len() == 3 {
            if digits[2] & 0x03 != 0 {
                return Err(Error::message("base64 data has non-zero trailing bits"));
            }
            output.extend([
                (digits[0] << 2) | (digits[1] >> 4),
                (digits[1] << 4) | (digits[2] >> 2),
            ]);
        } else if chunk[2] == b'=' {
            if chunk[3] != b'=' || digits[1] & 0x0f != 0 {
                return Err(Error::message(
                    "base64 padding or trailing bits are invalid",
                ));
            }
            output.push((digits[0] << 2) | (digits[1] >> 4));
        } else if chunk[3] == b'=' {
            if digits[2] & 0x03 != 0 {
                return Err(Error::message("base64 data has non-zero trailing bits"));
            }
            output.extend([
                (digits[0] << 2) | (digits[1] >> 4),
                (digits[1] << 4) | (digits[2] >> 2),
            ]);
        } else if chunk.len() == 4 {
            output.extend([
                (digits[0] << 2) | (digits[1] >> 4),
                (digits[1] << 4) | (digits[2] >> 2),
                (digits[2] << 6) | digits[3],
            ]);
        }
    }
    Ok(output)
}

pub(super) fn base64_digit(byte: u8) -> Option<u8> {
    match byte {
        b'A'..=b'Z' => Some(byte - b'A'),
        b'a'..=b'z' => Some(byte - b'a' + 26),
        b'0'..=b'9' => Some(byte - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}

pub(crate) fn disk_snapshot(vm: &Vm, action: &str, tag: Option<&str>) -> Result<String> {
    require_disk_file(&vm.config.disk_img)?;
    let mut args = vec!["snapshot".to_string()];
    if let Some(tag) = tag {
        args.extend([action.to_string(), tag.to_string()]);
    } else {
        args.push(action.to_string());
    }
    args.push(vm.config.disk_img.to_string_lossy().into_owned());
    let output = run_qemu_img(&args)?;
    if !output.status.success() {
        return Err(qemu_img_failure("snapshot", output));
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok(if text.is_empty() {
        String::from_utf8_lossy(&output.stderr).trim().to_string()
    } else {
        text
    })
}
