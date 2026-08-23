use super::*;

pub(super) fn load_effective_vm(dirs: &Dirs, name: &str, options: &LaunchOptions) -> Result<Vm> {
    let mut vm = find(&dirs.vm_dir, &dirs.state_root, name)?;
    apply_launch_options(&mut vm, options)?;
    Ok(vm)
}

pub(super) fn apply_launch_options(vm: &mut Vm, options: &LaunchOptions) -> Result<()> {
    reject_consumed_global_flags(&options.viewer_extra_args)?;
    reject_consumed_global_flags(&options.extra_args)?;
    let config = &mut vm.config;
    if let Some(value) = &options.ram {
        config.ram = Some(value.clone());
    }
    if let Some(value) = options.cpu_cores {
        config.cpu_cores = Some(value);
    }
    if let Some(value) = &options.display {
        config.display = value.to_ascii_lowercase();
    }
    if let Some(value) = &options.viewer {
        config.viewer = value.to_ascii_lowercase();
    }
    if let Some(value) = &options.access {
        config.access = value.to_ascii_lowercase();
    }
    config.allow_insecure_remote |= options.allow_insecure_remote;
    if let Some(value) = &options.ssh_access {
        config.ssh_access = value.to_ascii_lowercase();
    }
    if options.braille {
        config.braille = true;
        config.display = "sdl".to_string();
        config.usb_controller = "xhci".to_string();
    }
    config.fullscreen |= options.fullscreen;
    config.clipboard |= options.clipboard;
    config.offline |= options.offline;
    config.status_quo |= options.status_quo;
    config.ignore_tsc_warning |= options.ignore_tsc_warning;
    if let Some(value) = &options.cpu_pinning {
        validate_cpu_pinning(value)?;
        config.cpu_pinning = Some(value.clone());
    }
    if options.width.is_some() || options.height.is_some() {
        config.width = options.width.or(config.width);
        config.height = options.height.or(config.height);
    }
    if let Some(value) = options.ssh_port {
        config.ssh_port = Some(value);
    }
    if let Some(value) = options.spice_port {
        config.spice_port = Some(value);
    }
    config
        .viewer_extra_args
        .extend(options.viewer_extra_args.clone());
    if let Some(value) = &options.public_dir {
        config.public_dir = if value == Path::new("none") {
            None
        } else {
            Some(cli_path(value)?)
        };
    }
    if let Some(value) = &options.monitor {
        config.monitor = value.to_ascii_lowercase();
    }
    if let Some(value) = &options.monitor_cmd {
        config.monitor_cmd = Some(value.clone());
    }
    if let Some(value) = &options.monitor_telnet_host {
        config.monitor_telnet_host = value.clone();
    }
    if let Some(value) = options.monitor_telnet_port {
        config.monitor_telnet_port = value;
    }
    if let Some(value) = &options.serial {
        config.serial = value.to_ascii_lowercase();
    }
    if let Some(value) = &options.serial_telnet_host {
        config.serial_telnet_host = value.clone();
    }
    if let Some(value) = options.serial_telnet_port {
        config.serial_telnet_port = value;
    }
    if let Some(value) = &options.keyboard {
        config.keyboard = value.to_ascii_lowercase();
    }
    if let Some(value) = &options.keyboard_layout {
        config.keyboard_layout = value.clone();
    }
    if let Some(value) = &options.mouse {
        config.mouse = value.to_ascii_lowercase();
    }
    if let Some(value) = &options.usb_controller {
        config.usb_controller = value.to_ascii_lowercase();
    }
    if let Some(value) = &options.sound_card {
        config.sound_card = value.to_ascii_lowercase();
    }
    if let Some(value) = &options.sound_duplex {
        config.sound_duplex = value.to_ascii_lowercase();
    }
    if config.sound_card == "usb-audio" {
        config.usb_controller = "xhci".to_string();
    }
    config.extra_args.extend(options.extra_args.clone());
    config.validate()
}

fn reject_consumed_global_flags(arguments: &[String]) -> Result<()> {
    if arguments.iter().any(|argument| {
        let flag = argument
            .split_once('=')
            .map_or(argument.as_str(), |(flag, _)| flag);
        matches!(
            flag,
            "--dir" | "--vm-dir" | "--state-dir" | "--output" | "--verbose" | "-d"
        ) || flag
            .strip_prefix('-')
            .is_some_and(|flag| !flag.is_empty() && flag.chars().all(|character| character == 'v'))
    }) {
        return Err(Error::message(
            "place vmctl global options before --extra-args or --viewer-extra-args",
        ));
    }
    Ok(())
}
