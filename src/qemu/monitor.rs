use super::*;

#[cfg(unix)]
pub(super) fn is_unix_socket(path: &Path) -> bool {
    fs::metadata(path).is_ok_and(|metadata| metadata.file_type().is_socket())
}

#[cfg(windows)]
pub(super) fn is_unix_socket(path: &Path) -> bool {
    path.is_file()
}

pub(crate) fn send_monitor_command(vm: &Vm, command: &str) -> Result<String> {
    if vm.config.monitor == "none" {
        return Err(Error::message("the QEMU monitor is disabled"));
    }
    let response = match vm.config.monitor.as_str() {
        "socket" => {
            return send_qmp_human_monitor_command(vm, command);
        }
        "telnet" => {
            let address = format!(
                "{}:{}",
                qemu_host(monitor_connect_host(&vm.config.monitor_telnet_host)),
                vm.config.monitor_telnet_port
            );
            let deadline = qmp_deadline()?;
            let mut stream = connect_monitor(&address, deadline)?;
            stream
                .set_write_timeout(Some(QMP_TIMEOUT))
                .map_err(|error| {
                    Error::message(format!("cannot configure monitor {address}: {error}"))
                })?;
            stream
                .write_all(format!("{command}\n").as_bytes())
                .map_err(|error| Error::message(format!("cannot send monitor command: {error}")))?;
            read_monitor_response(&mut stream, &address, deadline)?
        }
        mode => {
            return Err(Error::message(format!(
                "monitor mode '{mode}' is not supported"
            )));
        }
    };
    Ok(clean_monitor_output(&response))
}

pub(super) fn monitor_connect_host(host: &str) -> &str {
    match host {
        "0.0.0.0" => "127.0.0.1",
        "::" => "::1",
        host => host,
    }
}

pub(super) fn connect_monitor(address: &str, deadline: Instant) -> Result<TcpStream> {
    let addresses = resolve_monitor_addresses(address, deadline)?;
    let mut last_error = None;
    for socket_address in addresses {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        match TcpStream::connect_timeout(&socket_address, remaining) {
            Ok(stream) => return Ok(stream),
            Err(error) => last_error = Some(error),
        }
    }
    let error = last_error
        .unwrap_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "connection timed out"));
    Err(Error::message(format!(
        "cannot connect to monitor {address}: {error}"
    )))
}

pub(super) fn resolve_monitor_addresses(
    address: &str,
    deadline: Instant,
) -> Result<Vec<SocketAddr>> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Err(Error::message(format!(
            "monitor address resolution timed out for {address}"
        )));
    }
    let (sender, receiver) = mpsc::sync_channel(1);
    let address = address.to_string();
    let resolve_address = address.clone();
    thread::Builder::new()
        .spawn(move || {
            let resolved = resolve_address
                .to_socket_addrs()
                .map(|addresses| addresses.collect::<Vec<_>>());
            let _ = sender.send(resolved);
        })
        .map_err(|error| Error::message(format!("cannot resolve monitor {address}: {error}")))?;
    match receiver.recv_timeout(remaining) {
        Ok(Ok(addresses)) if !addresses.is_empty() => Ok(addresses),
        Ok(Ok(_)) => Err(Error::message(format!(
            "monitor {address} did not resolve to an IP address"
        ))),
        Ok(Err(error)) => Err(Error::message(format!(
            "cannot resolve monitor {address}: {error}"
        ))),
        Err(mpsc::RecvTimeoutError::Timeout) => Err(Error::message(format!(
            "monitor address resolution timed out for {address}"
        ))),
        Err(mpsc::RecvTimeoutError::Disconnected) => Err(Error::message(format!(
            "monitor address resolution failed for {address}"
        ))),
    }
}

pub(super) const MAX_MONITOR_RESPONSE: usize = 1024 * 1024;

pub(super) fn read_monitor_response(
    stream: &mut TcpStream,
    address: &str,
    deadline: Instant,
) -> Result<Vec<u8>> {
    let mut response = Vec::new();
    let mut buffer = [0_u8; 8192];
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(Error::message(format!(
                "monitor {address} did not become idle within {} seconds",
                QMP_TIMEOUT.as_secs()
            )));
        }
        stream
            .set_read_timeout(Some(remaining.min(Duration::from_millis(500))))
            .map_err(|error| {
                Error::message(format!("cannot configure monitor {address}: {error}"))
            })?;
        match stream.read(&mut buffer) {
            Ok(0) => return Ok(response),
            Ok(count) => {
                if response.len() + count > MAX_MONITOR_RESPONSE {
                    return Err(Error::message(format!(
                        "monitor {address} response exceeds the {} byte safety limit",
                        MAX_MONITOR_RESPONSE
                    )));
                }
                response.extend_from_slice(&buffer[..count]);
            }
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
                ) =>
            {
                if response.is_empty() {
                    continue;
                }
                return Ok(response);
            }
            Err(error) => {
                return Err(Error::message(format!(
                    "cannot read monitor {address}: {error}"
                )));
            }
        }
    }
}

pub(super) fn send_qmp_human_monitor_command(vm: &Vm, command: &str) -> Result<String> {
    let mut connection = QmpConnection::connect(&vm.paths)?;
    connection.execute("qmp_capabilities", "vmctl-monitor-capabilities", None)?;
    let response = connection.execute(
        "human-monitor-command",
        "vmctl-monitor-command",
        Some(json!({"command-line": command})),
    )?;
    response
        .as_str()
        .map(str::trim)
        .map(str::to_string)
        .ok_or_else(|| Error::Qmp("human-monitor-command returned no text".to_string()))
}

pub(super) fn clean_monitor_output(response: &[u8]) -> String {
    let mut output = String::new();
    let mut escape = false;
    let mut csi = false;
    for byte in response {
        if csi {
            if (0x40..=0x7e).contains(byte) {
                csi = false;
            }
            continue;
        }
        if escape {
            escape = false;
            csi = *byte == b'[';
            continue;
        }
        if *byte == 0x1b {
            escape = true;
        } else if *byte == b'\n' || *byte == b'\t' || !byte.is_ascii_control() {
            output.push(*byte as char);
        }
    }
    output.trim().to_string()
}
