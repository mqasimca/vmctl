use super::*;

pub(super) fn add_storage_args(args: &mut Vec<String>, vm: &Vm) -> Result<()> {
    let config = &vm.config;
    let optimisations = "discard=unmap,detect-zeroes=unmap,cache=writeback,aio=threads";

    if config.guest_os == "macos" {
        let parent = config.disk_img.parent().unwrap_or_else(|| Path::new("."));
        let bootloader = [parent.join("OpenCore.qcow2"), parent.join("ESP.qcow2")]
            .into_iter()
            .find(|path| path.is_file())
            .ok_or_else(|| {
                Error::message(format!(
                    "macOS bootloader not found beside {} (expected OpenCore.qcow2 or ESP.qcow2)",
                    config.disk_img.display()
                ))
            })?;
        args.extend([
            "-device".to_string(),
            "ahci,id=ahci".to_string(),
            "-drive".to_string(),
            format!(
                "id=BootLoader,if=none,format=qcow2,file={}",
                qemu_path(&bootloader)
            ),
            "-device".to_string(),
            "ide-hd,bus=ahci.0,drive=BootLoader,bootindex=0".to_string(),
        ]);
        if let Some(image) = &config.img {
            add_optional_drive_with_id(args, image, "RecoveryImage", "raw", "")?;
            args.extend([
                "-device".to_string(),
                "ide-hd,bus=ahci.1,drive=RecoveryImage".to_string(),
            ]);
        }
        let device = match config.macos_release.as_deref() {
            Some(
                "catalina" | "big-sur" | "monterey" | "ventura" | "sonoma" | "sequoia" | "tahoe",
            ) => "virtio-blk-pci",
            _ => "ide-hd,bus=ahci.2",
        };
        add_system_disk(args, config, device, optimisations)?;
        return Ok(());
    }

    let has_iso =
        config.iso.is_some() || config.fixed_iso.is_some() || config.unattended_iso.is_some();
    if config.arch == "aarch64" && has_iso {
        args.extend([
            "-device".to_string(),
            "virtio-scsi-pci,id=scsi0".to_string(),
        ]);
        if let Some(iso) = &config.iso {
            add_optional_drive_with_id(args, iso, "cd0", "raw", "media=cdrom,readonly=on")?;
            args.extend([
                "-device".to_string(),
                "scsi-cd,drive=cd0,bus=scsi0.0,bootindex=1".to_string(),
            ]);
        }
        if let Some(iso) = &config.fixed_iso {
            add_optional_drive_with_id(args, iso, "cd1", "raw", "media=cdrom,readonly=on")?;
            args.extend([
                "-device".to_string(),
                "scsi-cd,drive=cd1,bus=scsi0.0,bootindex=3".to_string(),
            ]);
        }
        if let Some(iso) = &config.unattended_iso {
            add_optional_drive_with_id(args, iso, "cd2", "raw", "media=cdrom,readonly=on")?;
            args.extend([
                "-device".to_string(),
                "scsi-cd,drive=cd2,bus=scsi0.0,bootindex=4".to_string(),
            ]);
        }
    } else {
        if let Some(iso) = &config.iso {
            let options = if config.guest_os == "reactos" {
                "if=ide,index=2,media=cdrom"
            } else {
                "media=cdrom,index=0,readonly=on"
            };
            add_optional_drive(args, &Some(iso.clone()), options)?;
        }
        if let Some(iso) = &config.fixed_iso {
            add_optional_drive(args, &Some(iso.clone()), "media=cdrom,index=1,readonly=on")?;
        }
        if let Some(iso) = &config.unattended_iso {
            add_optional_drive(args, &Some(iso.clone()), "media=cdrom,index=2,readonly=on")?;
        }
    }
    add_optional_drive(args, &config.floppy, "if=floppy,format=raw")?;

    if config.guest_os == "batocera" {
        let image = config
            .img
            .as_ref()
            .ok_or_else(|| Error::message("batocera requires img"))?;
        add_optional_drive_with_id(args, image, "BootDisk", "raw", "")?;
        args.extend([
            "-device".to_string(),
            "virtio-blk-pci,drive=BootDisk".to_string(),
        ]);
    }
    if config.guest_os == "freedos" && config.iso.is_some() {
        args.extend(["-boot".to_string(), "order=dc".to_string()]);
    }
    if config.guest_os == "kolibrios" && config.iso.is_some() {
        args.extend(["-boot".to_string(), "order=d".to_string()]);
    }

    if config.guest_os == "reactos" {
        add(
            args,
            "-drive",
            format!(
                "if=ide,index=0,media=disk,format={},file={}",
                config.disk_format,
                qemu_path(&config.disk_img)
            ),
        );
        return Ok(());
    }
    if config.guest_os == "kolibrios" {
        args.extend(["-device".to_string(), "ahci,id=ahci".to_string()]);
    }
    let device = match config.guest_os.as_str() {
        "windows-server" => "ide-hd",
        "kolibrios" => "ide-hd,bus=ahci.0",
        "macos" => match config.macos_release.as_deref() {
            Some(
                "catalina" | "big-sur" | "monterey" | "ventura" | "sonoma" | "sequoia" | "tahoe",
            ) => "virtio-blk-pci",
            _ => "ide-hd,bus=ahci.2",
        },
        _ if config.arch == "aarch64" => "virtio-blk-pci,bootindex=2",
        _ => "virtio-blk-pci",
    };
    add_system_disk(args, config, device, optimisations)
}

pub(super) fn add_system_disk(
    args: &mut Vec<String>,
    config: &VmConfig,
    device: &str,
    optimisations: &str,
) -> Result<()> {
    add(
        args,
        "-drive",
        format!(
            "id=SystemDisk,if=none,format={},file={},{}",
            config.disk_format,
            qemu_path(&config.disk_img),
            optimisations
        ),
    );
    args.extend(["-device".to_string(), format!("{device},drive=SystemDisk")]);
    Ok(())
}

pub(super) fn add_optional_drive_with_id(
    args: &mut Vec<String>,
    path: &Path,
    id: &str,
    format: &str,
    options: &str,
) -> Result<()> {
    if !path.is_file() {
        return Err(Error::message(format!(
            "configured media file {} does not exist",
            path.display()
        )));
    }
    let options = (!options.is_empty()).then_some(format!(",{options}"));
    add(
        args,
        "-drive",
        format!(
            "id={id},if=none,format={format}{},file={}",
            options.as_deref().unwrap_or_default(),
            qemu_path(path)
        ),
    );
    Ok(())
}
