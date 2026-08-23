use super::*;

pub(super) fn print_plan(plan: &qemu::QemuPlan, output: OutputFormat, redact: bool) {
    let args = redact_plan_args(&plan.args, redact);
    if output == OutputFormat::Json {
        print_json_success(json!({
            "binary": plan.binary,
            "args": args,
            "command": shell_join(&plan.binary, &args),
            "ssh_port": plan.ssh_port,
            "ssh_host": plan.ssh_host,
            "spice_port": plan.spice_port,
            "spice_host": plan.spice_host,
            "monitor_telnet": plan.monitor_telnet.as_ref().map(|(host, port)| json!({"host": host, "port": port})),
            "serial_telnet": plan.serial_telnet.as_ref().map(|(host, port)| json!({"host": host, "port": port})),
            "redacted": redact,
        }));
        return;
    }
    println!("{}", shell_join(&plan.binary, &args));
    if let Some(port) = plan.ssh_port {
        println!("ssh_port={port}");
    }
    if let Some(port) = plan.spice_port {
        println!("spice_port={port}");
    }
    if let Some((host, port)) = &plan.monitor_telnet {
        println!("monitor_telnet={host}:{port}");
    }
    if let Some((host, port)) = &plan.serial_telnet {
        println!("serial_telnet={host}:{port}");
    }
}

pub(super) fn redact_plan_args(args: &[String], redact: bool) -> Vec<String> {
    if !redact {
        return args.to_vec();
    }
    let mut redact_next = false;
    args.iter()
        .map(|arg| {
            if redact_next {
                redact_next = false;
                return "<redacted>".to_string();
            }
            if matches!(arg.as_str(), "--password" | "--secret" | "--token") {
                redact_next = true;
                return arg.clone();
            }
            redact_inline_value(arg)
        })
        .collect()
}

pub(super) fn redact_inline_value(value: &str) -> String {
    for key in ["osk=", "password=", "secret=", "token="] {
        if let Some(start) = value.find(key) {
            let end = value[start + key.len()..]
                .find(',')
                .map_or(value.len(), |offset| start + key.len() + offset);
            return format!("{}<redacted>{}", &value[..start + key.len()], &value[end..]);
        }
    }
    value.to_string()
}

pub(super) fn vm_summary(vm: &Vm) -> Result<Value> {
    let (state, pid) = match vm.state()? {
        VmState::Running(pid) => ("running", Some(pid)),
        VmState::Stopped => ("stopped", None),
    };
    let ssh_host = if state == "running" {
        runtime_ssh_host(vm)?
    } else {
        None
    };
    Ok(json!({
        "name": vm.config.name,
        "state": state,
        "pid": pid,
        "config": vm.config.config_path,
        "ssh_port": effective_ssh_port(vm)?,
        "ssh_host": ssh_host,
        "guest_os": vm.config.guest_os,
        "arch": vm.config.arch,
        "ssh_access": vm.config.ssh_access,
        "ssh_user": vm.config.ssh_user,
    }))
}

pub(super) fn vm_status(vm: &Vm, live: bool) -> Result<Value> {
    let summary = vm_summary(vm)?;
    let ipc = ipc_report(&vm.paths, vm.config.guest_agent)?;
    let qmp_status = if summary["state"] == "running" {
        match qmp_status(&vm.paths) {
            Ok(status) => json!({"reachable": true, "status": status}),
            Err(error) => json!({
                "reachable": false,
                "status": null,
                "error": error.to_string(),
            }),
        }
    } else {
        json!({"reachable": false, "status": "stopped"})
    };
    let live_resources = if live && summary["state"] == "running" {
        match qmp_live_resources(&vm.paths) {
            Ok(resources) => json!({"reachable": true, "resources": resources}),
            Err(error) => json!({"reachable": false, "error": error.to_string()}),
        }
    } else {
        Value::Null
    };
    Ok(json!({
        "name": vm.config.name,
        "state": summary["state"].clone(),
        "pid": summary["pid"].clone(),
        "config": vm.config.config_path,
        "state_dir": vm.paths.state_dir,
        "guest_os": vm.config.guest_os,
        "arch": vm.config.arch,
        "display": vm.config.display,
        "disk": vm.config.disk_img,
        "disk_size": vm.config.disk_size,
        "configured_ram": vm.config.ram,
        "configured_cpu_cores": vm.config.cpu_cores,
        "boot": vm.config.boot,
        "ssh_port": summary["ssh_port"].clone(),
        "ssh_host": summary["ssh_host"].clone(),
        "ssh_access": vm.config.ssh_access,
        "ssh_user": vm.config.ssh_user,
        "ipc": ipc,
        "qmp_status": qmp_status,
        "live_resources": live_resources,
        "monitor": vm.paths.monitor_socket(),
        "serial": vm.paths.serial_socket(),
    }))
}

pub(super) fn state_label(vm: &Vm) -> Result<String> {
    Ok(match vm.state()? {
        VmState::Running(pid) => format!("running({pid})"),
        VmState::Stopped => "stopped".to_string(),
    })
}

pub(super) fn print_vm_status(vm: &Vm, live: bool) -> Result<()> {
    let ipc = ipc_report(&vm.paths, vm.config.guest_agent)?;
    let guest_agent = if ipc["guest_agent"].is_null() {
        "disabled".to_string()
    } else {
        ipc_endpoint_label(&ipc["guest_agent"])
    };
    println!("name:        {}", vm.config.name);
    println!("state:       {}", state_label(vm)?);
    println!("config:      {}", vm.config.config_path.display());
    println!("state dir:   {}", vm.paths.state_dir.display());
    println!("guest os:    {}", vm.config.guest_os);
    println!("arch:        {}", vm.config.arch);
    println!("display:     {}", vm.config.display);
    println!("disk:        {}", vm.config.disk_img.display());
    println!("disk size:   {}", vm.config.disk_size);
    println!(
        "configured ram: {}",
        vm.config.ram.as_deref().unwrap_or("host default")
    );
    println!(
        "configured cpu cores: {}",
        vm.config
            .cpu_cores
            .map_or_else(|| "host default".to_string(), |cores| cores.to_string())
    );
    println!("boot:        {}", vm.config.boot);
    println!(
        "ssh port:    {}",
        effective_ssh_port(vm)?.map_or_else(|| "auto".to_string(), |port| port.to_string())
    );
    if matches!(vm.state()?, VmState::Running(_))
        && let Some(host) = runtime_ssh_host(vm)?
    {
        println!("ssh host:    {host}");
    }
    if let Some(user) = &vm.config.ssh_user {
        println!("ssh user:    {user}");
    }
    println!("qmp:         {}", ipc_endpoint_label(&ipc["qmp"]));
    let qmp_state = match vm.state()? {
        VmState::Stopped => "stopped".to_string(),
        VmState::Running(_) => qmp_status(&vm.paths).unwrap_or_else(|_| "unavailable".to_string()),
    };
    println!("qmp state:   {qmp_state}");
    println!("monitor:     {}", vm.paths.monitor_socket().display());
    println!("guest agent: {guest_agent}");
    println!("serial:      {}", vm.paths.serial_socket().display());
    println!("runtime:     {}", vm.paths.state_dir.display());
    if live && matches!(vm.state()?, VmState::Running(_)) {
        match qmp_live_resources(&vm.paths) {
            Ok(resources) => {
                println!(
                    "live vcpus: {}",
                    resources["cpus"].as_array().map_or(0, Vec::len)
                );
                println!("live memory: {}", resources["memory"]);
            }
            Err(error) => println!("live resources: unavailable ({error})"),
        }
    }
    Ok(())
}

pub(super) fn ipc_endpoint_label(value: &Value) -> String {
    match value.get("transport").and_then(Value::as_str) {
        Some("tcp") => format!(
            "tcp://{}:{}",
            value
                .get("host")
                .and_then(Value::as_str)
                .unwrap_or("127.0.0.1"),
            value
                .get("port")
                .and_then(Value::as_u64)
                .unwrap_or_default()
        ),
        Some("unix") => value
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or("unavailable")
            .to_string(),
        _ => "unavailable".to_string(),
    }
}
