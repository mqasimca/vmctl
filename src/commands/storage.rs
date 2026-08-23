use super::*;

#[allow(clippy::too_many_arguments)] // Mirrors the one-to-one `vmctl set` CLI options.
pub(super) fn set_vm(
    dirs: &Dirs,
    name: &str,
    ram: Option<&str>,
    cpu_cores: Option<u32>,
    disk_size: Option<&str>,
    cpu_model: Option<&str>,
    cpu_pinning: Option<&str>,
    macaddr: Option<&str>,
    bridge: Option<&str>,
    port_forwards: &[String],
    boot_menu: Option<&str>,
    boot_once: Option<&str>,
    disk_cache: Option<&str>,
    disk_aio: Option<&str>,
    discard: Option<&str>,
    output: OutputFormat,
) -> Result<()> {
    if ram.is_none()
        && cpu_cores.is_none()
        && disk_size.is_none()
        && cpu_model.is_none()
        && cpu_pinning.is_none()
        && macaddr.is_none()
        && bridge.is_none()
        && port_forwards.is_empty()
        && boot_menu.is_none()
        && boot_once.is_none()
        && disk_cache.is_none()
        && disk_aio.is_none()
        && discard.is_none()
    {
        return Err(Error::invalid_argument(
            "VM settings",
            "provide at least one setting option",
        ));
    }
    if let Some(ram) = ram {
        crate::config::validate_ram_size(ram)?;
    }
    if let Some(size) = disk_size {
        crate::qemu::validate_disk_size(size)?;
        if size.starts_with('+') {
            return Err(Error::message(
                "--disk-size must be an absolute size such as 64G; use `vmctl disk VM resize +4G` to grow an existing disk",
            ));
        }
    }

    if let Some(pinning) = cpu_pinning {
        validate_cpu_pinning(pinning)?;
    }
    let vm = find(&dirs.vm_dir, &dirs.state_root, name)?;
    let _operation_lock = acquire_vm_lock(&vm.paths)?;
    let mut updated = vm.config.clone();
    if let Some(ram) = ram {
        updated.ram = Some(ram.to_string());
    }
    if let Some(cpu_cores) = cpu_cores {
        updated.cpu_cores = Some(cpu_cores);
    }
    if let Some(disk_size) = disk_size {
        updated.disk_size = disk_size.to_string();
    }
    if let Some(cpu_model) = cpu_model {
        updated.cpu_model = Some(cpu_model.to_string());
    }
    if let Some(cpu_pinning) = cpu_pinning {
        updated.cpu_pinning = Some(cpu_pinning.to_string());
    }
    if let Some(macaddr) = macaddr {
        updated.macaddr = Some(macaddr.to_string());
    }
    if let Some(bridge) = bridge {
        updated.bridge = (!bridge.eq_ignore_ascii_case("none")).then(|| bridge.to_string());
    }
    if !port_forwards.is_empty() {
        updated.port_forwards = parse_port_forwards(port_forwards)?;
    }
    if let Some(boot_menu) = boot_menu {
        updated.boot_menu = parse_on_off(boot_menu, "boot menu")?;
    }
    if let Some(boot_once) = boot_once {
        updated.boot_once =
            (!boot_once.eq_ignore_ascii_case("none")).then(|| boot_once.to_ascii_lowercase());
    }
    if let Some(disk_cache) = disk_cache {
        updated.disk_cache = disk_cache.to_ascii_lowercase();
    }
    if let Some(disk_aio) = disk_aio {
        updated.disk_aio = disk_aio.to_ascii_lowercase();
    }
    if let Some(discard) = discard {
        updated.discard = discard.to_ascii_lowercase();
    }
    updated.validate()?;
    if (cpu_cores.is_some() || cpu_pinning.is_some())
        && let Some(pinning) = &updated.cpu_pinning
    {
        let cores = updated.cpu_cores.unwrap_or_else(qemu::default_cpu_cores);
        validate_cpu_pinning_for_host(pinning, env::consts::OS, cores)?;
    }
    let mut updates = Vec::new();
    if let Some(ram) = ram {
        updates.push(("ram", ram.to_string()));
    }
    if let Some(cpu_cores) = cpu_cores {
        updates.push(("cpu_cores", cpu_cores.to_string()));
    }
    if let Some(disk_size) = disk_size {
        updates.push(("disk_size", disk_size.to_string()));
    }
    if let Some(cpu_model) = cpu_model {
        updates.push(("cpu_model", cpu_model.to_string()));
    }
    if let Some(cpu_pinning) = cpu_pinning {
        updates.push(("cpu_pinning", cpu_pinning.to_string()));
    }
    if let Some(macaddr) = macaddr {
        updates.push(("macaddr", macaddr.to_string()));
    }
    if let Some(bridge) = bridge {
        updates.push((
            "bridge",
            if !bridge.eq_ignore_ascii_case("none") {
                bridge.to_string()
            } else {
                String::new()
            },
        ));
    }
    if !port_forwards.is_empty() {
        updates.push((
            "port_forwards",
            format!(
                "({})",
                updated
                    .port_forwards
                    .iter()
                    .map(|(host, guest)| format!("{host}:{guest}"))
                    .collect::<Vec<_>>()
                    .join(" ")
            ),
        ));
    }
    if let Some(boot_menu) = boot_menu {
        updates.push(("boot_menu", boot_menu.to_ascii_lowercase()));
    }
    if let Some(boot_once) = boot_once {
        updates.push((
            "boot_once",
            if !boot_once.eq_ignore_ascii_case("none") {
                boot_once.to_ascii_lowercase()
            } else {
                String::new()
            },
        ));
    }
    if let Some(disk_cache) = disk_cache {
        updates.push(("disk_cache", disk_cache.to_ascii_lowercase()));
    }
    if let Some(disk_aio) = disk_aio {
        updates.push(("disk_aio", disk_aio.to_ascii_lowercase()));
    }
    if let Some(discard) = discard {
        updates.push(("discard", discard.to_ascii_lowercase()));
    }
    let (mut config_file, settings) = prepare_config_settings(&vm.config.config_path, &updates)?;
    if let Some(size) = disk_size {
        require_stopped_disk(&vm, "resize")?;
        disk_resize(&vm.config.disk_img, size, false)?;
    }
    config_file
        .write_all(&settings)
        .map_err(|error| Error::io(vm.config.config_path.display(), error))?;

    let restart_required = updates.iter().any(|(key, _)| *key != "disk_size");
    if output == OutputFormat::Json {
        print_json_success(json!({
            "name": vm.config.name,
            "ram": ram,
            "cpu_cores": cpu_cores,
            "disk_size": disk_size,
            "cpu_model": cpu_model,
            "cpu_pinning": cpu_pinning,
            "macaddr": macaddr,
            "bridge": bridge,
            "port_forwards": port_forwards,
            "boot_menu": boot_menu,
            "boot_once": boot_once,
            "disk_cache": disk_cache,
            "disk_aio": disk_aio,
            "discard": discard,
            "restart_required": restart_required,
        }));
    } else {
        println!("Updated settings for {}", vm.config.name);
        if restart_required {
            println!("Restart the VM to apply setting changes");
        }
    }
    Ok(())
}

fn parse_on_off(value: &str, setting: &str) -> Result<bool> {
    match value.to_ascii_lowercase().as_str() {
        "on" => Ok(true),
        "off" => Ok(false),
        _ => Err(Error::message(format!("{setting} must be on or off"))),
    }
}

fn parse_port_forwards(values: &[String]) -> Result<Vec<(u16, u16)>> {
    if values.len() == 1 && values[0].eq_ignore_ascii_case("none") {
        return Ok(Vec::new());
    }
    values
        .iter()
        .map(|value| {
            if value.eq_ignore_ascii_case("none") {
                return Err(Error::message(
                    "--port-forward none cannot be combined with port pairs",
                ));
            }
            let (host, guest) = value
                .split_once(':')
                .ok_or_else(|| Error::message(format!("invalid port forward '{value}'")))?;
            let host = host
                .parse::<u16>()
                .ok()
                .filter(|port| *port != 0)
                .ok_or_else(|| Error::message(format!("invalid host port in '{value}'")))?;
            let guest = guest
                .parse::<u16>()
                .ok()
                .filter(|port| *port != 0)
                .ok_or_else(|| Error::message(format!("invalid guest port in '{value}'")))?;
            Ok((host, guest))
        })
        .collect()
}

fn prepare_config_settings(path: &Path, updates: &[(&str, String)]) -> Result<(File, Vec<u8>)> {
    let mut settings = String::new();
    for (key, value) in updates {
        if value.contains(['\n', '\r']) {
            return Err(Error::invalid_argument(
                "VM setting",
                "values cannot contain newlines",
            ));
        }
        let value = value.replace('\\', "\\\\").replace('"', "\\\"");
        settings.push_str(&format!("{key}=\"{value}\"\n"));
    }
    let mut file = crate::config::open_config_for_append(path)?;
    let mut contents = Vec::new();
    file.read_to_end(&mut contents)
        .map_err(|error| Error::io(path.display(), error))?;
    let mut settings = settings.into_bytes();
    if !contents.is_empty() && !contents.ends_with(b"\n") {
        settings.insert(0, b'\n');
    }
    Ok((file, settings))
}

pub(super) fn snapshot_vm(
    dirs: &Dirs,
    name: &str,
    action: SnapshotAction,
    output: OutputFormat,
) -> Result<()> {
    let vm = find(&dirs.vm_dir, &dirs.state_root, name)?;
    let _operation_lock = acquire_vm_lock(&vm.paths)?;
    if matches!(vm.state()?, VmState::Running(_)) {
        return Err(Error::message(
            "disk snapshots require a stopped VM; use the QEMU monitor for live snapshots",
        ));
    }
    let (operation, tag) = match action {
        SnapshotAction::Create { tag } => ("-c", Some(tag)),
        SnapshotAction::Apply { tag } => ("-a", Some(tag)),
        SnapshotAction::Delete { tag } => ("-d", Some(tag)),
        SnapshotAction::Info => ("-l", None),
    };
    let result = disk_snapshot(&vm, operation, tag.as_deref())?;
    if output == OutputFormat::Json {
        print_json_success(
            json!({"name": vm.config.name, "action": operation, "tag": tag, "result": result}),
        );
    } else if result.is_empty() {
        println!("Snapshot operation completed for {}", vm.config.name);
    } else {
        println!("{result}");
    }
    Ok(())
}

pub(super) fn disk_vm(
    dirs: &Dirs,
    name: &str,
    action: DiskAction,
    output: OutputFormat,
) -> Result<()> {
    let vm = find(&dirs.vm_dir, &dirs.state_root, name)?;
    let _operation_lock = acquire_vm_lock(&vm.paths)?;
    match action {
        DiskAction::Info => {
            let disk = disk_info(&vm.config.disk_img)?;
            let result = json!({
                "name": vm.config.name,
                "action": "info",
                "disk": disk,
            });
            if output == OutputFormat::Json {
                print_json(&result);
            } else {
                println!("Disk: {}", vm.config.disk_img.display());
                print_disk_info_human(&result["disk"]);
            }
        }
        DiskAction::Resize { size, shrink, yes } => {
            require_stopped_disk(&vm, "resize")?;
            if shrink && !yes {
                return Err(Error::message(
                    "shrinking a disk requires --yes because it can destroy data",
                ));
            }
            let disk = disk_resize(&vm.config.disk_img, &size, shrink)?;
            let result = json!({
                "name": vm.config.name,
                "action": "resize",
                "size": size,
                "shrink": shrink,
                "disk": disk,
            });
            if output == OutputFormat::Json {
                print_json(&result);
            } else {
                println!("Resized {} to {size}", vm.config.name);
                print_disk_info_human(&result["disk"]);
            }
        }
        DiskAction::Check { repair, yes } => {
            require_stopped_disk(&vm, "check")?;
            if repair && !yes {
                return Err(Error::message(
                    "disk repair requires --yes because it changes the image",
                ));
            }
            let check = disk_check(&vm.config.disk_img, repair)?;
            if !check.healthy {
                if output != OutputFormat::Json {
                    print_disk_check_human(&check.report);
                }
                return Err(Error::disk_check_failed(&vm.config.disk_img, check.report));
            }
            let result = json!({
                "name": vm.config.name,
                "action": "check",
                "repair": repair,
                "healthy": true,
                "report": check.report,
            });
            if output == OutputFormat::Json {
                print_json(&result);
            } else {
                println!("Disk check passed for {}", vm.config.name);
                print_disk_check_human(&result["report"]);
            }
        }
        DiskAction::Convert {
            destination,
            format,
            compress,
            force,
        } => {
            require_stopped_disk(&vm, "convert")?;
            let format = format.unwrap_or_else(|| vm.config.disk_format.clone());
            let disk = disk_convert(&vm.config.disk_img, &destination, &format, compress, force)?;
            let result = json!({
                "name": vm.config.name,
                "action": "convert",
                "output": destination,
                "format": format,
                "compressed": compress,
                "disk": disk,
            });
            if output == OutputFormat::Json {
                print_json(&result);
            } else {
                println!("Converted {} to {}", vm.config.name, destination.display());
                print_disk_info_human(&result["disk"]);
            }
        }
        DiskAction::Compact { yes } => {
            require_stopped_disk(&vm, "compact")?;
            if !yes {
                return Err(Error::message(
                    "compacting replaces the disk image and discards internal snapshots; rerun with --yes",
                ));
            }
            let disk = disk_compact(&vm.config.disk_img)?;
            let result = json!({
                "name": vm.config.name,
                "action": "compact",
                "disk": disk,
            });
            if output == OutputFormat::Json {
                print_json(&result);
            } else {
                println!("Compacted disk for {}", vm.config.name);
                print_disk_info_human(&result["disk"]);
            }
        }
    }
    Ok(())
}

pub(super) fn require_stopped_disk(vm: &Vm, operation: &str) -> Result<()> {
    if let VmState::Running(pid) = vm.state()? {
        return Err(Error::message(format!(
            "disk {operation} requires a stopped VM; {} is running with pid {pid}",
            vm.config.name
        )));
    }
    Ok(())
}

pub(super) fn print_json(value: &Value) {
    print_json_success(value.clone());
}

pub(super) fn print_disk_info_human(info: &Value) {
    for (label, key) in [
        ("format", "format"),
        ("virtual size", "virtual-size"),
        ("actual size", "actual-size"),
        ("cluster size", "cluster-size"),
        ("backing file", "backing-filename"),
    ] {
        if let Some(value) = info.get(key) {
            println!("{label}: {}", display_json_value(value));
        }
    }
    if let Some(snapshots) = info.get("snapshots").and_then(Value::as_array) {
        println!("snapshots: {}", snapshots.len());
    }
}

pub(super) fn print_disk_check_human(report: &Value) {
    println!(
        "{}",
        serde_json::to_string_pretty(report).unwrap_or_default()
    );
}

pub(super) fn display_json_value(value: &Value) -> String {
    value
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| value.to_string())
}

pub(super) fn delete_disk(dirs: &Dirs, name: &str, yes: bool, output: OutputFormat) -> Result<()> {
    let vm = find(&dirs.vm_dir, &dirs.state_root, name)?;
    let _operation_lock = acquire_vm_lock(&vm.paths)?;
    ensure_delete_allowed(&vm, yes)?;
    remove_if_present(&vm.config.disk_img)?;
    for path in persistent_efi_vars(&vm) {
        remove_if_present(&path)?;
    }
    if output == OutputFormat::Json {
        print_json_success(json!({"name": vm.config.name, "deleted": "disk"}));
    } else {
        println!("Deleted disk data for {}", vm.config.name);
    }
    Ok(())
}

pub(super) fn delete_vm(dirs: &Dirs, name: &str, yes: bool, output: OutputFormat) -> Result<()> {
    let vm = find(&dirs.vm_dir, &dirs.state_root, name)?;
    let operation_lock = acquire_vm_lock(&vm.paths)?;
    ensure_delete_allowed(&vm, yes)?;
    let data_dir = vm
        .config
        .config_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(&vm.config.name);
    if fs::symlink_metadata(&data_dir).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        return Err(Error::message(format!(
            "refusing to remove VM data symlink {}",
            data_dir.display()
        )));
    }
    remove_if_present(&vm.config.disk_img)?;
    for path in persistent_efi_vars(&vm) {
        remove_if_present(&path)?;
    }
    if data_dir.is_dir() {
        fs::remove_dir_all(&data_dir).map_err(|error| Error::io(data_dir.display(), error))?;
    }
    remove_if_present(&vm.config.config_path)?;
    drop(operation_lock);
    if vm.paths.state_dir.is_dir() {
        fs::remove_dir_all(&vm.paths.state_dir)
            .map_err(|error| Error::io(vm.paths.state_dir.display(), error))?;
    }
    if output == OutputFormat::Json {
        print_json_success(json!({"name": vm.config.name, "deleted": "vm"}));
    } else {
        println!("Deleted VM {}", vm.config.name);
    }
    Ok(())
}
