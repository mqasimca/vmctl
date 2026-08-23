use super::*;

pub(super) fn list_vms(dirs: &Dirs, output: OutputFormat) -> Result<()> {
    let vms = discover(&dirs.vm_dir, &dirs.state_root)?;
    if output == OutputFormat::Json {
        let values: Vec<Value> = vms.iter().map(vm_summary).collect::<Result<_>>()?;
        print_json_success(json!(values));
        return Ok(());
    }

    if vms.is_empty() {
        println!("No VM configurations found in {}", dirs.vm_dir.display());
        return Ok(());
    }

    println!("{:<28} {:<16} {:<8} CONFIG", "NAME", "STATE", "SSH");
    for vm in vms {
        let ssh =
            effective_ssh_port(&vm)?.map_or_else(|| "auto".to_string(), |port| port.to_string());
        println!(
            "{:<28} {:<16} {:<8} {}",
            vm.config.name,
            state_label(&vm)?,
            ssh,
            vm.config.config_path.display()
        );
    }
    Ok(())
}

pub(super) fn status_vms(
    dirs: &Dirs,
    name: Option<&str>,
    live: bool,
    output: OutputFormat,
) -> Result<()> {
    if let Some(name) = name {
        let vm = find(&dirs.vm_dir, &dirs.state_root, name)?;
        if output == OutputFormat::Json {
            print_json_success(vm_status(&vm, live)?);
        } else {
            print_vm_status(&vm, live)?;
        }
        Ok(())
    } else {
        list_vms(dirs, output)
    }
}

pub(super) fn plan_vm(
    dirs: &Dirs,
    name: &str,
    options: &LaunchOptions,
    output: OutputFormat,
    redact: bool,
) -> Result<()> {
    let vm = load_effective_vm(dirs, name, options)?;
    let host = HostCapabilities::detect(&vm.config)?;
    if let Some(pinning) = &vm.config.cpu_pinning {
        validate_cpu_pinning_for_host(
            pinning,
            &host.host_os,
            vm.config.cpu_cores.unwrap_or(host.cpu_cores),
        )?;
    }
    let plan = build_plan(&vm, &host, false)?;
    print_plan(&plan, output, redact);
    Ok(())
}
