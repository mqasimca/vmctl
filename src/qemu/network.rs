use super::*;

pub fn spice_address(config: &VmConfig) -> &str {
    match config.access.as_str() {
        "local" | "" => "127.0.0.1",
        "remote" => "0.0.0.0",
        address => address,
    }
}

pub(super) fn qemu_host(host: &str) -> String {
    if host.parse::<std::net::Ipv6Addr>().is_ok() {
        format!("[{host}]")
    } else {
        host.to_string()
    }
}

pub(super) fn is_loopback_host(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

pub(super) fn ssh_address(config: &VmConfig) -> &str {
    match config.ssh_access.as_str() {
        "local" | "" => "127.0.0.1",
        "remote" => "0.0.0.0",
        address => address,
    }
}

pub(super) fn add_network_args(
    args: &mut Vec<String>,
    vm: &Vm,
    ssh_port: Option<u16>,
    smbd: bool,
    bridge_helper: Option<&str>,
) -> Result<()> {
    if vm.config.offline {
        args.extend(["-nic".to_string(), "none".to_string()]);
        return Ok(());
    }
    if !vm.config.port_forwards.is_empty() && !uses_port_forwarding_network(&vm.config) {
        return Err(Error::message(
            "port forwards require network=user, network=restrict, or network=passt",
        ));
    }
    if ssh_port.is_some()
        && uses_user_network(&vm.config)
        && ssh_address(&vm.config)
            .parse::<std::net::Ipv4Addr>()
            .is_err()
    {
        return Err(Error::message(
            "QEMU user networking requires ssh_access to resolve to a numeric IPv4 address",
        ));
    }

    let net_device = match vm.config.guest_os.as_str() {
        "freedos" => "pcnet",
        "haiku" | "kolibrios" | "solaris" => "rtl8139",
        "reactos" | "windows-server" => "e1000",
        "macos"
            if matches!(
                vm.config.macos_release.as_deref(),
                Some("big-sur" | "monterey" | "ventura" | "sonoma" | "sequoia" | "tahoe")
            ) =>
        {
            "virtio-net-pci"
        }
        "macos" => "vmxnet3",
        _ => "virtio-net-pci",
    };

    if vm.config.network.eq_ignore_ascii_case("none") {
        args.extend(["-nic".to_string(), "none".to_string()]);
        return Ok(());
    }
    let bridge = configured_bridge(&vm.config);
    if let Some(bridge) = bridge {
        let helper = bridge_helper.ok_or_else(|| {
            Error::message(
                "bridged networking requires qemu-bridge-helper; install it or use network=user",
            )
        })?;
        let mac = vm
            .config
            .macaddr
            .as_deref()
            .map_or_else(String::new, |mac| format!(",mac={mac}"));
        args.extend([
            "-nic".to_string(),
            format!(
                "bridge,br={bridge},helper={},model={net_device}{mac}",
                qemu_path(Path::new(helper))
            ),
        ]);
        return Ok(());
    }

    if uses_passt_network(&vm.config) {
        if smbd
            && matches!(vm.config.guest_os.as_str(), "windows" | "windows-server")
            && vm
                .config
                .public_dir
                .as_ref()
                .is_some_and(|path| path.is_dir())
        {
            return Err(Error::message(
                "Windows SMB sharing requires network=user; passt does not provide QEMU's SMB server",
            ));
        }
        let tcp_ports = vm
            .config
            .port_forwards
            .iter()
            .map(|(host, guest)| format!("{host}:{guest}"))
            .collect::<Vec<_>>();
        let mac = vm
            .config
            .macaddr
            .as_deref()
            .map_or_else(String::new, |mac| format!(",mac={mac}"));
        let mut net = "passt,id=nic,tcp-ports=none,udp-ports=none".to_string();
        if let Some(port) = ssh_port {
            net.push_str(&format!(
                ",param=--tcp-ports={}/{port}:22",
                ssh_address(&vm.config)
            ));
        }
        if !tcp_ports.is_empty() {
            let ports = tcp_ports.join(",,");
            net.push_str(&format!(",param=--tcp-ports=127.0.0.1/{ports}"));
            net.push_str(&format!(",param=--udp-ports=127.0.0.1/{ports}"));
        }
        args.extend([
            "-device".to_string(),
            format!("{net_device},netdev=nic{mac}"),
            "-netdev".to_string(),
            net,
        ]);
        return Ok(());
    }

    let mut net = format!("user,id=nic,hostname={}", vm.config.name);
    if vm.config.network.eq_ignore_ascii_case("restrict") {
        net.push_str(",restrict=on");
    }
    if let Some(port) = ssh_port {
        net.push_str(&format!(
            ",hostfwd=tcp:{}:{port}-:22",
            ssh_address(&vm.config)
        ));
    }
    for (host, guest) in &vm.config.port_forwards {
        net.push_str(&format!(",hostfwd=tcp:127.0.0.1:{host}-:{guest}"));
        net.push_str(&format!(",hostfwd=udp:127.0.0.1:{host}-:{guest}"));
    }
    if smbd
        && matches!(vm.config.guest_os.as_str(), "windows" | "windows-server")
        && let Some(public_dir) = &vm.config.public_dir
        && public_dir.is_dir()
    {
        net.push_str(&format!(",smb={}", qemu_path(public_dir)));
    }
    let mac = vm
        .config
        .macaddr
        .as_deref()
        .map_or_else(String::new, |mac| format!(",mac={mac}"));
    args.extend([
        "-device".to_string(),
        format!("{net_device},netdev=nic{mac}"),
        "-netdev".to_string(),
        net,
    ]);
    Ok(())
}
