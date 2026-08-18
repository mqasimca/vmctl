use super::*;

pub(super) fn download_cached_image(
    args: &GetArgs,
    dirs: &Dirs,
    output: OutputFormat,
) -> Result<()> {
    let os = find_os(required_arg(args.os.as_deref(), "OS")?).map(|info| info.id)?;
    let release = required_arg(args.release_or_input.as_deref(), "RELEASE")?;
    let architecture = requested_architectures(args, os)?
        .into_iter()
        .next()
        .ok_or_else(|| Error::message("an architecture is required"))?;
    if os == "macos" {
        return download_image(args, dirs, false, output);
    }
    let image = if matches!(os, "windows" | "windows-server") {
        let edition = required_edition(find_os(os)?, args.edition_or_language.as_deref())?;
        let (url, kind, checksum) = windows_asset(os, release, edition.as_deref())?;
        ResolvedImage {
            os: os.to_string(),
            release: release.to_string(),
            edition,
            architecture: architecture.to_string(),
            file_name: file_name_from_url(&url).unwrap_or_else(|| format!("{os}-{release}.iso")),
            url,
            kind,
            checksum,
        }
    } else {
        resolve_remote_image(
            os,
            release,
            args.edition_or_language.as_deref(),
            &architecture,
        )?
    };
    fs::create_dir_all(&dirs.vm_dir).map_err(|error| Error::io(dirs.vm_dir.display(), error))?;
    let cached = cache_image(
        &dirs.vm_dir,
        &image,
        args.insecure,
        args.refresh_cache,
        false,
        None,
    )?;
    let result = json!({
        "os": image.os,
        "release": image.release,
        "edition": image.edition,
        "architecture": image.architecture,
        "url": image.url,
        "kind": image_kind_name(image.kind),
        "image": cached.path,
        "cache": { "status": cached.status.as_str(), "object": cached.path, "sha256": cached.sha256 },
    });
    if output == OutputFormat::Json {
        crate::print_json_success(result);
    } else {
        println!(
            "{} {}",
            if cached.status == CacheStatus::Hit {
                "Using cached"
            } else {
                "Downloaded"
            },
            cached.path.display()
        );
        println!(
            "Create a VM with: vmctl create NAME --from {}",
            cached
                .path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
        );
    }
    Ok(())
}

pub(super) fn create_cached_vm(args: &CreateArgs, dirs: &Dirs, output: OutputFormat) -> Result<()> {
    let source = cached_source(&dirs.vm_dir, &args.image)?;
    if source.cloud {
        return create_cached_cloud_vm(args, dirs, source, output);
    }
    if args.disk_mode.is_some()
        || !args.ssh_keys.is_empty()
        || args.hostname.is_some()
        || args.network_config.is_some()
    {
        return Err(Error::invalid_argument(
            "cloud options",
            "--disk-mode, --ssh-key, --hostname, and --network-config require a cached cloud image",
        ));
    }
    let name = validate_vm_name(&args.name)?;
    let root = &dirs.vm_dir;
    let config_path = root.join(format!("{name}.conf"));
    let target_dir = root.join(name);
    if config_path.exists() {
        return Err(Error::message(format!(
            "configuration already exists: {}",
            config_path.display()
        )));
    }
    fs::create_dir_all(root).map_err(|error| Error::io(root.display(), error))?;
    fs::create_dir(&target_dir).map_err(|error| Error::io(target_dir.display(), error))?;
    let image = if source.kind == ImageKind::Iso {
        source.path.clone()
    } else {
        let file_name = source
            .path
            .file_name()
            .ok_or_else(|| Error::message("cached image has no file name"))?;
        let destination = target_dir.join(file_name);
        fs::copy(&source.path, &destination)
            .map_err(|error| Error::io(destination.display(), error))?;
        destination
    };
    let config = write_vm_config(
        root,
        name,
        &source.os,
        &source.release,
        source.edition.as_deref(),
        &source.architecture,
        &image,
    )?;
    let result = json!({
        "name": name,
        "image": image,
        "config": config,
        "source": { "os": source.os, "release": source.release, "kind": image_kind_name(source.kind) },
    });
    if output == OutputFormat::Json {
        crate::print_json_success(result);
    } else {
        println!("Created {}", config.display());
    }
    Ok(())
}

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
    let name = args
        .name
        .clone()
        .unwrap_or_else(|| suggested_name(os, release, image.edition.as_deref(), &architecture));
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
    let (target, cache) = if create_config {
        let cache = cache_image(
            &root,
            &image,
            args.insecure,
            args.refresh_cache,
            false,
            None,
        )?;
        let target = if image.kind == ImageKind::Iso {
            cache.path.clone()
        } else {
            let target = target_dir.join(&image.file_name);
            fs::copy(&cache.path, &target).map_err(|error| Error::io(target.display(), error))?;
            prepare_resolved_image(os, &target)?
        };
        (target, Some(cache))
    } else {
        let target = target_dir.join(&image.file_name);
        download_file(&image.url, &target, args.insecure)?;
        if let Err(error) = verify_checksum(&target, image.checksum.as_deref()) {
            let _ = fs::remove_file(&target);
            return Err(error);
        }
        (target, None)
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
        "cache": cache.as_ref().map(|cache| json!({
            "status": cache.status.as_str(),
            "object": cache.path,
            "sha256": cache.sha256,
        })),
    });
    if output == OutputFormat::Json {
        crate::print_json_success(result);
    } else if let Some(config_path) = config_path {
        if let Some(cache) = &cache {
            println!(
                "{} {}",
                if cache.status == CacheStatus::Hit {
                    "Using cached"
                } else {
                    "Downloaded"
                },
                cache.path.display()
            );
        } else {
            println!("Downloaded {}", target.display());
        }
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
    let cache = if input.starts_with("http://") || input.starts_with("https://") {
        Some(cache_url(
            root,
            input,
            &source_name,
            image_kind(&source_name),
            None,
            args.insecure,
            args.refresh_cache,
        )?)
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
        None
    };
    let image = if let Some(cache) = &cache {
        if image_kind(&source_name) == ImageKind::Iso {
            cache.path.clone()
        } else {
            fs::copy(&cache.path, &destination)
                .map_err(|error| Error::io(destination.display(), error))?;
            prepare_image(&destination)?
        }
    } else {
        prepare_image(&destination)?
    };
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
        "cache": cache.as_ref().map(|cache| json!({
            "status": cache.status.as_str(),
            "object": cache.path,
            "sha256": cache.sha256,
        })),
    });
    if output == OutputFormat::Json {
        crate::print_json_success(result);
    } else {
        println!("Created {}", config_path.display());
    }
    Ok(())
}
