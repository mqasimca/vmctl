use super::*;

pub(super) fn download_image(
    args: &GetArgs,
    dirs: &Dirs,
    create_config: bool,
    output: OutputFormat,
) -> Result<()> {
    let os = find_os(required_arg(args.os.as_deref(), "OS")?).map(|info| info.id)?;
    let release = required_arg(args.release_or_input.as_deref(), "RELEASE")?;
    let architecture = requested_architectures(args, os)?
        .into_iter()
        .next()
        .ok_or_else(|| Error::message("an architecture is required"))?;
    if os == "macos" {
        return download_macos(args, dirs, release, &architecture, create_config, output);
    }
    if matches!(os, "windows" | "windows-server") {
        return download_windows(
            args,
            dirs,
            os,
            release,
            &architecture,
            create_config,
            output,
        );
    }
    let image = resolve_remote_image(
        os,
        release,
        args.edition_or_language.as_deref(),
        &architecture,
    )?;
    let name = suggested_name(os, release, image.edition.as_deref(), &architecture);
    validate_vm_name(&name)?;
    let root = if create_config {
        dirs.vm_dir.clone()
    } else {
        env::current_dir().map_err(|error| Error::io("current directory", error))?
    };
    let target_dir = if create_config {
        root.join(&name)
    } else {
        root.clone()
    };
    if create_config {
        let config_path = root.join(format!("{name}.conf"));
        if config_path.exists() {
            return Err(Error::message(format!(
                "configuration already exists: {}",
                config_path.display()
            )));
        }
    }
    fs::create_dir_all(&target_dir).map_err(|error| Error::io(target_dir.display(), error))?;
    let target = target_dir.join(&image.file_name);
    download_file(&image.url, &target, args.insecure)?;
    if let Err(error) = verify_checksum(&target, image.checksum.as_deref()) {
        let _ = fs::remove_file(&target);
        return Err(error);
    }
    let target = if create_config {
        prepare_resolved_image(os, &target)?
    } else {
        target
    };
    let config_path = if create_config {
        Some(write_vm_config(
            &root,
            &name,
            os,
            release,
            image.edition.as_deref(),
            &architecture,
            &target,
        )?)
    } else {
        None
    };
    let result = json!({
        "os": os,
        "release": release,
        "edition": image.edition,
        "architecture": architecture,
        "url": image.url,
        "kind": image_kind_name(image.kind),
        "checksum": image.checksum,
        "image": target,
        "config": config_path,
    });
    if output == OutputFormat::Json {
        println!(
            "{}",
            serde_json::to_string_pretty(&result).unwrap_or_default()
        );
    } else if let Some(config_path) = config_path {
        println!("Downloaded {}", target.display());
        println!("Created {}", config_path.display());
    } else {
        println!("Downloaded {}", target.display());
    }
    Ok(())
}

pub(super) fn create_custom_config(
    args: &GetArgs,
    dirs: &Dirs,
    output: OutputFormat,
) -> Result<()> {
    let name = validate_vm_name(required_arg(args.os.as_deref(), "VM name")?)?;
    let input = required_arg(args.release_or_input.as_deref(), "image path or URL")?;
    if args.edition_or_language.is_some() {
        return Err(Error::message(
            "--create-config accepts VM_NAME and IMAGE_PATH_OR_URL",
        ));
    }
    let root = &dirs.vm_dir;
    let config_path = root.join(format!("{name}.conf"));
    if config_path.exists() {
        return Err(Error::message(format!(
            "configuration already exists: {}",
            config_path.display()
        )));
    }
    let vm_dir = root.join(name);
    fs::create_dir_all(root).map_err(|error| Error::io(root.display(), error))?;
    if fs::symlink_metadata(&vm_dir).is_ok() {
        return Err(Error::message(format!(
            "VM data directory already exists: {}",
            vm_dir.display()
        )));
    }
    fs::create_dir(&vm_dir).map_err(|error| Error::io(vm_dir.display(), error))?;
    let source_name = input_file_name(input)?;
    let destination = vm_dir.join(&source_name);
    if input.starts_with("http://") || input.starts_with("https://") {
        download_file(input, &destination, args.insecure)?;
    } else {
        let source = PathBuf::from(input);
        if !source.is_file() {
            return Err(Error::message(format!(
                "image path does not exist: {}",
                source.display()
            )));
        }
        if fs::canonicalize(&source).ok() != fs::canonicalize(&destination).ok() {
            fs::copy(&source, &destination)
                .map_err(|error| Error::io(destination.display(), error))?;
        }
    }
    let image = prepare_image(&destination)?;
    let os = infer_guest_os(&image);
    let (fixed_iso, unattended_iso) =
        if matches!(os, "windows" | "windows-server") && !args.disable_unattended {
            let fixed_iso = download_virtio_iso(&vm_dir, args.insecure)?;
            let unattended_iso = create_unattended_iso(&vm_dir, args.insecure)?;
            (Some(fixed_iso), Some(unattended_iso))
        } else {
            (None, None)
        };
    let config_path = write_vm_config(root, name, os, "custom", None, host_architecture(), &image)?;
    if let Some(fixed_iso) = fixed_iso.as_deref() {
        append_iso(root, &config_path, "fixed_iso", fixed_iso)?;
    }
    if let Some(unattended_iso) = unattended_iso.as_deref() {
        append_iso(root, &config_path, "unattended_iso", unattended_iso)?;
    }
    let result = json!({
        "name": name,
        "guest_os": os,
        "image": image,
        "fixed_iso": fixed_iso,
        "unattended_iso": unattended_iso,
        "config": config_path,
    });
    if output == OutputFormat::Json {
        println!(
            "{}",
            serde_json::to_string_pretty(&result).unwrap_or_default()
        );
    } else {
        println!("Created {}", config_path.display());
    }
    Ok(())
}
