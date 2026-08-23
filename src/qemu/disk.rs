use super::*;

pub fn ensure_disk(vm: &Vm) -> Result<()> {
    if fs::symlink_metadata(&vm.config.disk_img)
        .is_ok_and(|metadata| metadata.file_type().is_symlink())
    {
        return Err(Error::message(format!(
            "refusing to use disk symlink {}",
            vm.config.disk_img.display()
        )));
    }
    if vm.config.disk_img.exists() {
        let status = Command::new("qemu-img")
            .args(["info", vm.config.disk_img.to_string_lossy().as_ref()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(|error| Error::command_unavailable("qemu-img", error))?;
        if !status.success() {
            return Err(Error::message(format!(
                "qemu-img could not read {}",
                vm.config.disk_img.display()
            )));
        }
        return Ok(());
    }

    if vm.config.cloud_base_img.is_some() || vm.config.cloud_init_iso.is_some() {
        return Err(Error::message(format!(
            "cloud disk {} is missing; recreate the VM with `vmctl get --cloud`",
            vm.config.disk_img.display()
        )));
    }
    if vm.config.iso.is_none()
        && vm.config.fixed_iso.is_none()
        && vm.config.img.is_none()
        && vm.config.guest_os != "macos"
    {
        return Err(Error::message(format!(
            "disk {} does not exist and no ISO was configured",
            vm.config.disk_img.display()
        )));
    }
    validate_disk_size(&vm.config.disk_size)?;
    if vm.config.disk_size.starts_with('+') {
        return Err(Error::message(
            "disk_size must be an absolute size such as 20G when creating a disk",
        ));
    }
    if let Some(parent) = vm.config.disk_img.parent() {
        fs::create_dir_all(parent).map_err(|error| Error::io(parent.display(), error))?;
    }
    let mut command = Command::new("qemu-img");
    command.args(["create", "-f", &vm.config.disk_format]);
    let options = match vm.config.disk_format.as_str() {
        "qcow2" => format!(
            "lazy_refcounts=on,preallocation={},nocow=on",
            vm.config.preallocation
        ),
        "raw" => format!("preallocation={}", vm.config.preallocation),
        _ => String::new(),
    };
    if !options.is_empty() {
        command.args(["-o", options.as_str()]);
    }
    let output = command
        .args([
            vm.config.disk_img.to_string_lossy().as_ref(),
            &vm.config.disk_size,
        ])
        .output()
        .map_err(|error| Error::command_unavailable("qemu-img", error))?;
    if !output.status.success() {
        return Err(qemu_img_failure("create", output));
    }
    Ok(())
}

pub(crate) fn create_cloud_overlay(base: &Path, overlay: &Path, backing: &str) -> Result<()> {
    require_disk_file(base)?;
    if fs::symlink_metadata(overlay).is_ok() {
        return Err(Error::message(format!(
            "cloud disk already exists: {}",
            overlay.display()
        )));
    }
    if backing.is_empty()
        || Path::new(backing).is_absolute()
        || backing.chars().any(|character| character.is_control())
    {
        return Err(Error::message(
            "cloud backing reference must be a safe relative path",
        ));
    }
    let temporary = overlay.with_extension("qcow2.tmp");
    if fs::symlink_metadata(&temporary).is_ok() {
        return Err(Error::message(format!(
            "temporary cloud disk already exists: {}",
            temporary.display()
        )));
    }
    let output = Command::new("qemu-img")
        .args(["create", "-q", "-f", "qcow2", "-F", "qcow2", "-b", backing])
        .arg(&temporary)
        .output()
        .map_err(|error| Error::command_unavailable("qemu-img", error))?;
    if !output.status.success() {
        let _ = fs::remove_file(&temporary);
        return Err(qemu_img_failure("create", output));
    }
    if let Err(error) = fs::rename(&temporary, overlay) {
        let _ = fs::remove_file(&temporary);
        return Err(Error::io(overlay.display(), error));
    }
    Ok(())
}

pub(crate) fn create_cloud_copy(base: &Path, disk: &Path) -> Result<()> {
    require_disk_file(base)?;
    if fs::symlink_metadata(disk).is_ok() {
        return Err(Error::message(format!(
            "cloud disk already exists: {}",
            disk.display()
        )));
    }
    let temporary = disk.with_extension("qcow2.tmp");
    if fs::symlink_metadata(&temporary).is_ok() {
        return Err(Error::message(format!(
            "temporary cloud disk already exists: {}",
            temporary.display()
        )));
    }
    let output = Command::new("qemu-img")
        .args(["convert", "-q", "-f", "qcow2", "-O", "qcow2"])
        .arg(base)
        .arg(&temporary)
        .output()
        .map_err(|error| Error::command_unavailable("qemu-img", error))?;
    if !output.status.success() {
        let _ = fs::remove_file(&temporary);
        return Err(qemu_img_failure("convert", output));
    }
    if let Err(error) = fs::rename(&temporary, disk) {
        let _ = fs::remove_file(&temporary);
        return Err(Error::io(disk.display(), error));
    }
    Ok(())
}

#[derive(Debug, Clone)]
pub(crate) struct DiskCheckResult {
    pub report: Value,
    pub healthy: bool,
}

pub(crate) fn disk_info(path: &Path) -> Result<Value> {
    require_disk_file(path)?;
    let args = vec![
        "info".to_string(),
        "-U".to_string(),
        "--output=json".to_string(),
        path.to_string_lossy().into_owned(),
    ];
    let output = run_qemu_img(&args)?;
    if !output.status.success() {
        return Err(qemu_img_failure("info", output));
    }
    serde_json::from_slice(&output.stdout)
        .map_err(|error| Error::message(format!("qemu-img info returned invalid JSON: {error}")))
}

pub(crate) fn disk_resize(path: &Path, size: &str, shrink: bool) -> Result<Value> {
    require_disk_file(path)?;
    validate_disk_size(size)?;
    let mut args = vec!["resize".to_string()];
    if shrink {
        args.push("--shrink".to_string());
    }
    args.extend([path.to_string_lossy().into_owned(), size.to_string()]);
    let output = run_qemu_img(&args)?;
    if !output.status.success() {
        return Err(qemu_img_failure("resize", output));
    }
    disk_info(path)
}

pub(crate) fn disk_check(path: &Path, repair: bool) -> Result<DiskCheckResult> {
    require_disk_file(path)?;
    let mut args = vec!["check".to_string(), "--output=json".to_string()];
    if repair {
        args.push("--repair=all".to_string());
    }
    args.push(path.to_string_lossy().into_owned());
    let output = run_qemu_img(&args)?;
    let report: Value = serde_json::from_slice(&output.stdout).map_err(|error| {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if detail.is_empty() {
            Error::message(format!("qemu-img check returned invalid JSON: {error}"))
        } else {
            Error::message(format!("qemu-img check failed: {detail}"))
        }
    })?;
    let healthy = output.status.success()
        && ["check-errors", "corruptions", "leaks"]
            .iter()
            .all(|key| report.get(*key).and_then(Value::as_u64).unwrap_or(0) == 0);
    Ok(DiskCheckResult { report, healthy })
}

pub(crate) fn disk_convert(
    source: &Path,
    destination: &Path,
    format: &str,
    compress: bool,
    force: bool,
) -> Result<Value> {
    require_disk_file(source)?;
    validate_disk_format(format)?;
    if same_path(source, destination) {
        return Err(Error::message(
            "disk conversion output must be different from the source disk",
        ));
    }
    prepare_conversion_destination(destination, force)?;
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).map_err(|error| Error::io(parent.display(), error))?;
    }
    let mut args = vec![
        "convert".to_string(),
        "-q".to_string(),
        "-O".to_string(),
        format.to_string(),
    ];
    if compress {
        if !matches!(format, "qcow" | "qcow2") {
            return Err(Error::message(
                "--compress is only supported for qcow and qcow2 output",
            ));
        }
        args.push("-c".to_string());
    }
    args.extend([
        source.to_string_lossy().into_owned(),
        destination.to_string_lossy().into_owned(),
    ]);
    let output = run_qemu_img(&args)?;
    if !output.status.success() {
        return Err(qemu_img_failure("convert", output));
    }
    disk_info(destination)
}

pub(crate) fn disk_compact(path: &Path) -> Result<Value> {
    require_disk_file(path)?;
    let permissions = fs::metadata(path)
        .map_err(|error| Error::io(path.display(), error))?
        .permissions();
    let info = disk_info(path)?;
    let format = info
        .get("format")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::message("qemu-img info did not report a disk format"))?;
    validate_disk_format(format)?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| Error::message("disk path has no valid file name"))?;
    let temporary = path.with_file_name(format!(
        ".{file_name}.vmctl-compact-{}.tmp",
        std::process::id()
    ));
    if fs::symlink_metadata(&temporary).is_ok() {
        return Err(Error::message(format!(
            "temporary compacted disk already exists: {}",
            temporary.display()
        )));
    }
    let mut args = vec![
        "convert".to_string(),
        "-q".to_string(),
        "-O".to_string(),
        format.to_string(),
    ];
    if matches!(format, "qcow" | "qcow2") {
        args.push("-c".to_string());
    }
    args.extend([
        path.to_string_lossy().into_owned(),
        temporary.to_string_lossy().into_owned(),
    ]);
    let output = run_qemu_img(&args)?;
    if !output.status.success() {
        let _ = fs::remove_file(&temporary);
        return Err(qemu_img_failure("compact", output));
    }
    if let Err(error) = fs::set_permissions(&temporary, permissions) {
        let _ = fs::remove_file(&temporary);
        return Err(Error::io(temporary.display(), error));
    }
    if let Err(error) = replace_file(&temporary, path) {
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }
    disk_info(path)
}

pub(super) fn require_disk_file(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            Error::message(format!(
                "disk {} does not exist or is not a regular file",
                path.display()
            ))
        } else {
            Error::io(path.display(), error)
        }
    })?;
    if metadata.file_type().is_symlink() {
        return Err(Error::message(format!(
            "refusing to use disk symlink {}",
            path.display()
        )));
    }
    if !metadata.file_type().is_file() {
        return Err(Error::message(format!(
            "disk {} does not exist or is not a regular file",
            path.display()
        )));
    }
    Ok(())
}

pub(crate) fn validate_disk_size(size: &str) -> Result<()> {
    let value = size.strip_prefix('+').unwrap_or(size);
    if size.starts_with('-') || crate::config::validate_ram_size(value).is_err() {
        return Err(Error::message(format!(
            "invalid disk size '{size}'; use a value such as 20G or +4G"
        )));
    }
    Ok(())
}

pub(super) fn validate_disk_format(format: &str) -> Result<()> {
    if format.is_empty()
        || format.starts_with('-')
        || format.chars().any(|character| {
            character.is_ascii_control()
                || character.is_ascii_whitespace()
                || !(character.is_ascii_alphanumeric() || ".-_".contains(character))
        })
    {
        return Err(Error::message(format!(
            "invalid disk format '{format}'; use a qemu-img format such as qcow2 or raw"
        )));
    }
    Ok(())
}

pub(super) fn prepare_conversion_destination(path: &Path, force: bool) -> Result<()> {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return Ok(());
    };
    if metadata.file_type().is_symlink() {
        return Err(Error::message(format!(
            "refusing to write through output symlink {}",
            path.display()
        )));
    }
    if !metadata.file_type().is_file() {
        return Err(Error::message(format!(
            "conversion output is not a regular file: {}",
            path.display()
        )));
    }
    if !force {
        return Err(Error::message(format!(
            "conversion output already exists: {}; rerun with --force to replace it",
            path.display()
        )));
    }
    Ok(())
}

pub(super) fn same_path(left: &Path, right: &Path) -> bool {
    if left == right {
        return true;
    }
    match (fs::canonicalize(left), fs::canonicalize(right)) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

pub(super) fn run_qemu_img(args: &[String]) -> Result<Output> {
    Command::new("qemu-img")
        .args(args)
        .output()
        .map_err(|error| Error::command_unavailable("qemu-img", error))
}

pub(super) fn qemu_img_failure(operation: &str, output: Output) -> Error {
    let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if detail.is_empty() {
        Error::command_failed_status(&format!("qemu-img {operation}"), output.status)
    } else {
        Error::message(format!("qemu-img {operation} failed: {detail}"))
    }
}
