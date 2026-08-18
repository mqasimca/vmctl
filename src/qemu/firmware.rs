use super::*;

pub(super) fn arm_monolithic_firmware(config: &VmConfig) -> Option<PathBuf> {
    (config.arch == "aarch64" && config.guest_os == "windows" && !config.secureboot).then(|| {
        first_existing(&[
            "/usr/share/edk2/aarch64/QEMU_EFI.fd",
            "/usr/share/qemu-efi-aarch64/QEMU_EFI.fd",
        ])
        .or_else(|| {
            firmware_data_dirs()
                .into_iter()
                .map(|dir| dir.join("qemu-efi-aarch64").join("QEMU_EFI.fd"))
                .find(|path| path.is_file())
        })
    })?
}

pub(super) fn firmware_data_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(root) = env::var_os("QEMU_HOME") {
        let root = PathBuf::from(root);
        dirs.push(root.join("share"));
        dirs.push(root);
    }
    #[cfg(target_os = "macos")]
    dirs.extend([
        PathBuf::from("/opt/homebrew/share/qemu"),
        PathBuf::from("/usr/local/share/qemu"),
    ]);
    #[cfg(target_os = "windows")]
    {
        for variable in ["ProgramFiles", "ProgramFiles(x86)"] {
            if let Some(root) = env::var_os(variable) {
                dirs.push(PathBuf::from(root).join("qemu").join("share"));
            }
        }
    }
    for binary in ["qemu-system-x86_64", "qemu-system-aarch64"] {
        if let Some(path) = find_executable(binary)
            && let Some(parent) = Path::new(&path).parent()
        {
            dirs.extend([parent.join("../share"), parent.join("../share/qemu")]);
        }
    }
    dirs
}

pub(super) fn firmware_pair_candidates(pairs: &[(&str, &str)]) -> Vec<(PathBuf, PathBuf)> {
    let mut candidates = pairs
        .iter()
        .map(|(code, vars)| (PathBuf::from(code), PathBuf::from(vars)))
        .collect::<Vec<_>>();
    for dir in firmware_data_dirs() {
        candidates.extend([
            (
                dir.join("edk2-x86_64-code.fd"),
                dir.join("edk2-i386-vars.fd"),
            ),
            (
                dir.join("edk2-x86_64-secure-code.fd"),
                dir.join("edk2-i386-vars.fd"),
            ),
            (
                dir.join("edk2-aarch64-code.fd"),
                dir.join("edk2-arm-vars.fd"),
            ),
            (
                dir.join("edk2").join("x64").join("OVMF_CODE.4m.fd"),
                dir.join("edk2").join("x64").join("OVMF_VARS.4m.fd"),
            ),
        ]);
    }
    candidates
}

pub(super) fn firmware_paths(vm: &Vm, prepare: bool) -> Result<(PathBuf, PathBuf)> {
    let parent = vm
        .config
        .disk_img
        .parent()
        .unwrap_or_else(|| Path::new("."));

    if vm.config.guest_os == "macos" {
        let code = [parent.join("OVMF_CODE.fd")]
            .into_iter()
            .find(|path| path.is_file())
            .or_else(|| {
                first_existing(&[
                    "/usr/share/edk2/x64/OVMF_CODE.4m.fd",
                    "/usr/share/OVMF/OVMF_CODE_4M.fd",
                    "/usr/share/OVMF/OVMF_CODE.fd",
                    "/usr/share/OVMF/x64/OVMF_CODE.fd",
                ])
            })
            .or_else(|| {
                firmware_data_dirs().into_iter().find_map(|dir| {
                    [
                        dir.join("edk2/x64/OVMF_CODE.4m.fd"),
                        dir.join("OVMF_CODE_4M.fd"),
                        dir.join("OVMF_CODE.fd"),
                    ]
                    .into_iter()
                    .find(|path| path.is_file())
                })
            })
            .ok_or_else(|| Error::message("macOS OVMF_CODE.fd was not found"))?;
        if let Some(vars) = [
            parent.join("OVMF_VARS-1024x768.fd"),
            parent.join("OVMF_VARS-1920x1080.fd"),
            parent.join("OVMF_VARS.fd"),
        ]
        .into_iter()
        .find(|path| path.is_file())
        {
            return Ok((code, vars));
        }
        let vars = parent.join("OVMF_VARS.fd");
        if prepare {
            let template = first_existing(&[
                "/usr/share/edk2/x64/OVMF_VARS.4m.fd",
                "/usr/share/OVMF/OVMF_VARS_4M.fd",
                "/usr/share/OVMF/OVMF_VARS.fd",
                "/usr/share/OVMF/x64/OVMF_VARS.fd",
            ])
            .or_else(|| {
                firmware_data_dirs().into_iter().find_map(|dir| {
                    [
                        dir.join("edk2/x64/OVMF_VARS.4m.fd"),
                        dir.join("OVMF_VARS_4M.fd"),
                        dir.join("OVMF_VARS.fd"),
                    ]
                    .into_iter()
                    .find(|path| path.is_file())
                })
            })
            .ok_or_else(|| Error::message("macOS OVMF variables template was not found"))?;
            fs::create_dir_all(parent).map_err(|error| Error::io(parent.display(), error))?;
            fs::copy(&template, &vars).map_err(|error| {
                Error::message(format!(
                    "cannot copy macOS UEFI variables {} to {}: {error}",
                    template.display(),
                    vars.display()
                ))
            })?;
        }
        return Ok((code, vars));
    }

    if let Some(code) = arm_monolithic_firmware(&vm.config) {
        return Ok((code, parent.join("OVMF_VARS.fd")));
    }

    let static_pairs = if vm.config.arch == "aarch64" {
        vec![
            (
                "/usr/share/AAVMF/AAVMF_CODE.fd",
                "/usr/share/AAVMF/AAVMF_VARS.fd",
            ),
            (
                "/usr/share/edk2/aarch64/QEMU_CODE.fd",
                "/usr/share/edk2/aarch64/QEMU_VARS.fd",
            ),
            (
                "/usr/share/edk2/aarch64/QEMU_EFI-pflash.raw",
                "/usr/share/edk2/aarch64/vars-template-pflash.raw",
            ),
            (
                "/usr/share/qemu/edk2-aarch64-code.fd",
                "/usr/share/qemu/edk2-arm-vars.fd",
            ),
        ]
    } else if vm.config.secureboot {
        vec![
            (
                "/usr/share/edk2/x64/OVMF_CODE.secboot.4m.fd",
                "/usr/share/edk2/x64/OVMF_VARS.4m.fd",
            ),
            (
                "/usr/share/OVMF/OVMF_CODE_4M.secboot.fd",
                "/usr/share/OVMF/OVMF_VARS_4M.ms.fd",
            ),
            (
                "/usr/share/OVMF/OVMF_CODE.secboot.fd",
                "/usr/share/OVMF/OVMF_VARS.fd",
            ),
            (
                "/usr/share/OVMF/x64/OVMF_CODE.secboot.fd",
                "/usr/share/OVMF/x64/OVMF_VARS.fd",
            ),
            (
                "/usr/share/qemu/edk2-x86_64-secure-code.fd",
                "/usr/share/qemu/edk2-i386-vars.fd",
            ),
        ]
    } else {
        vec![
            (
                "/usr/share/edk2/x64/OVMF_CODE.4m.fd",
                "/usr/share/edk2/x64/OVMF_VARS.4m.fd",
            ),
            (
                "/usr/share/OVMF/OVMF_CODE_4M.fd",
                "/usr/share/OVMF/OVMF_VARS_4M.fd",
            ),
            (
                "/usr/share/OVMF/OVMF_CODE.fd",
                "/usr/share/OVMF/OVMF_VARS.fd",
            ),
            (
                "/usr/share/OVMF/x64/OVMF_CODE.fd",
                "/usr/share/OVMF/x64/OVMF_VARS.fd",
            ),
            (
                "/usr/share/qemu/edk2-x86_64-code.fd",
                "/usr/share/qemu/edk2-i386-vars.fd",
            ),
        ]
    };
    let firmware_pairs = firmware_pair_candidates(&static_pairs);
    let (code, template) = firmware_pairs
        .into_iter()
        .find(|(code, vars)| code.is_file() && vars.is_file())
        .ok_or_else(|| Error::message("UEFI firmware pair was not found; install edk2/OVMF"))?;
    let vars = [
        parent.join("OVMF_VARS.fd"),
        parent.join("OVMF_VARS_4M.fd"),
        parent.join(format!("{}-vars.fd", vm.config.name)),
    ]
    .into_iter()
    .find(|path| path.is_file())
    .unwrap_or_else(|| parent.join("OVMF_VARS.fd"));
    if prepare && !vars.is_file() {
        fs::create_dir_all(parent).map_err(|error| Error::io(parent.display(), error))?;
        fs::copy(&template, &vars).map_err(|error| {
            Error::message(format!(
                "cannot copy UEFI variables {} to {}: {error}",
                template.display(),
                vars.display()
            ))
        })?;
    }
    if prepare && !vars.is_file() {
        return Err(Error::message(format!(
            "UEFI variables file {} does not exist",
            vars.display()
        )));
    }
    Ok((code, vars))
}

pub(super) fn add_optional_drive(
    args: &mut Vec<String>,
    path: &Option<PathBuf>,
    options: &str,
) -> Result<()> {
    let Some(path) = path else {
        return Ok(());
    };
    if !path.exists() {
        return Err(Error::message(format!(
            "configured media file {} does not exist",
            path.display()
        )));
    }
    add(
        args,
        "-drive",
        format!("{options},file={}", qemu_path(path)),
    );
    Ok(())
}
