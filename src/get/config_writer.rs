use super::*;

pub(super) fn write_vm_config(
    root: &Path,
    name: &str,
    os: &str,
    release: &str,
    edition: Option<&str>,
    architecture: &str,
    image: &Path,
) -> Result<PathBuf> {
    let name = validate_vm_name(name)?;
    let config_path = root.join(format!("{name}.conf"));
    let image = image
        .strip_prefix(root)
        .unwrap_or(image)
        .to_string_lossy()
        .replace('\\', "/");
    let image_type = image_kind(&image);
    let guest_os = guest_os(os, release);
    let disk_size = disk_size(os, edition);
    let mut lines = vec![
        format!("guest_os=\"{}\"", config_value(guest_os)),
        format!("arch=\"{}\"", config_value(qemu_architecture(architecture))),
        format!("disk_img=\"{name}/disk.qcow2\""),
    ];
    match image_type {
        ImageKind::Disk => lines[2] = format!("disk_img=\"{}\"", config_value(&image)),
        ImageKind::Img => lines.push(format!("img=\"{}\"", config_value(&image))),
        ImageKind::Iso | ImageKind::Archive => {
            lines.push(format!("iso=\"{}\"", config_value(&image)));
        }
    }
    if let Some(disk_size) = disk_size {
        lines.push(format!("disk_size=\"{disk_size}\""));
    }
    for (key, value) in config_tweaks(os, release) {
        lines.push(format!("{key}=\"{}\"", config_value(value)));
    }
    if guest_os == "macos" {
        lines.push(format!("macos_release=\"{}\"", config_value(release)));
    }
    if matches!(
        os,
        "dragonflybsd"
            | "haiku"
            | "openbsd"
            | "netbsd"
            | "openindiana"
            | "slackware"
            | "slax"
            | "tinycore"
            | "freedos"
            | "kolibrios"
            | "reactos"
    ) {
        lines.push("boot=\"legacy\"".to_string());
    }
    if matches!(os, "windows" | "windows-server") && matches!(release, "11" | "2022") {
        lines.push("tpm=\"on\"".to_string());
        lines.push("secureboot=\"off\"".to_string());
    }
    let contents = format!("{}\n", lines.join("\n"));
    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent).map_err(|error| Error::io(parent.display(), error))?;
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&config_path)
        .map_err(|error| Error::io(config_path.display(), error))?;
    file.write_all(contents.as_bytes())
        .map_err(|error| Error::io(config_path.display(), error))?;
    Ok(config_path)
}

pub(super) struct CloudVmConfig<'a> {
    pub(super) os: &'a str,
    pub(super) release: &'a str,
    pub(super) architecture: &'a str,
    pub(super) base: Option<&'a Path>,
    pub(super) disk: &'a Path,
    pub(super) seed: &'a Path,
    pub(super) ssh_user: &'a str,
}

pub(super) fn write_cloud_vm_config(
    root: &Path,
    name: &str,
    cloud: CloudVmConfig<'_>,
) -> Result<PathBuf> {
    let name = validate_vm_name(name)?;
    let config_path = root.join(format!("{name}.conf"));
    let relative = |path: &Path| {
        path.strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/")
    };
    let mut lines = vec![
        format!(
            "guest_os=\"{}\"",
            config_value(guest_os(cloud.os, cloud.release))
        ),
        format!(
            "arch=\"{}\"",
            config_value(qemu_architecture(cloud.architecture))
        ),
        format!("disk_img=\"{}\"", config_value(&relative(cloud.disk))),
    ];
    if let Some(base) = cloud.base {
        lines.push(format!(
            "cloud_base_img=\"{}\"",
            config_value(&relative(base))
        ));
    }
    lines.extend([
        format!("cloud_init_iso=\"{}\"", config_value(&relative(cloud.seed))),
        format!("ssh_user=\"{}\"", config_value(cloud.ssh_user)),
    ]);
    let contents = format!("{}\n", lines.join("\n"));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&config_path)
        .map_err(|error| Error::io(config_path.display(), error))?;
    file.write_all(contents.as_bytes())
        .map_err(|error| Error::io(config_path.display(), error))?;
    Ok(config_path)
}

pub(super) fn image_kind(path: &str) -> ImageKind {
    match Path::new(path)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "qcow2" | "raw" => ImageKind::Disk,
        "img" | "dmg" => ImageKind::Img,
        "zip" | "7z" | "gz" | "bz2" => ImageKind::Archive,
        _ => ImageKind::Iso,
    }
}

pub(super) fn image_kind_name(kind: ImageKind) -> &'static str {
    match kind {
        ImageKind::Iso => "iso",
        ImageKind::Img => "img",
        ImageKind::Disk => "disk",
        ImageKind::Archive => "archive",
    }
}

pub(super) fn infer_guest_os(path: &Path) -> &'static str {
    let value = path.to_string_lossy().to_ascii_lowercase();
    if value.contains("freebsd") || value.contains("ghostbsd") {
        "freebsd"
    } else if value.contains("reactos") {
        "reactos"
    } else if value.contains("kolibrios") {
        "kolibrios"
    } else if value.contains("windows-server")
        || value.contains("eval_oemret")
        || value.contains("eval_x")
    {
        "windows-server"
    } else if value.contains("windows") || value.contains("win10") || value.contains("win11") {
        "windows"
    } else {
        "linux"
    }
}

pub(super) fn config_tweaks(os: &str, release: &str) -> Vec<(&'static str, &'static str)> {
    match os {
        "archlinux" => vec![("secureboot", "on"), ("tpm", "on"), ("disk_size", "32G")],
        "debian" => {
            let mut tweaks = vec![("secureboot", "on")];
            if release.starts_with("12") || release.starts_with("13") {
                tweaks.push(("tpm", "on"));
            }
            tweaks
        }
        "deepin" => vec![("ram", "4G")],
        "freedos" => vec![("ram", "256M")],
        "kolibrios" => vec![("ram", "128M")],
        "proxmox-ve" => vec![("ram", "4G")],
        "reactos" => vec![("ram", "2048M")],
        "slitaz" => vec![("ram", "512M")],
        "ubuntu-server" => {
            let mut tweaks = vec![("ram", "4G")];
            if release == "22.04" {
                tweaks.push(("tpm", "on"));
            }
            tweaks
        }
        "elementary" if release == "8.1" => vec![("display", "spice")],
        "macos" if release == "monterey" => vec![("cpu_cores", "2")],
        _ => Vec::new(),
    }
}

pub(super) fn disk_size(os: &str, edition: Option<&str>) -> Option<&'static str> {
    match os {
        "alma" | "centos-stream" | "endless" | "garuda" | "gentoo" | "kali" | "nixos"
        | "oraclelinux" | "popos" | "rockylinux" => Some("32G"),
        "batocera" => Some("8G"),
        "bazzite" => Some("64G"),
        "deepin" => Some("64G"),
        "freedos" => Some("4G"),
        "kolibrios" => Some("2G"),
        "macos" => Some("128G"),
        "openindiana" => Some("32G"),
        "proxmox-ve" => Some("20G"),
        "reactos" => Some("12G"),
        "slint" => Some("50G"),
        "slitaz" => Some("4G"),
        "ubuntu-server" => Some("10G"),
        "vanillaos" => Some("64G"),
        "windows" | "windows-server" => Some("64G"),
        "zorin" if edition == Some("education64") => Some("32G"),
        _ => None,
    }
}

pub(super) fn guest_os(os: &str, release: &str) -> &'static str {
    if is_ubuntu_family(os)
        && os != "ubuntu-server"
        && !release.contains("daily")
        && release
            .replace('.', "")
            .parse::<u32>()
            .is_ok_and(|version| version < 1604)
    {
        "linux_old"
    } else {
        find_os(os).map_or("linux", |info| info.guest_os)
    }
}

pub(super) fn suggested_name(
    os: &str,
    release: &str,
    edition: Option<&str>,
    architecture: &str,
) -> String {
    let mut name = format!("{os}-{release}");
    if let Some(edition) = edition {
        name.push('-');
        name.push_str(&edition.replace([' ', '(', ')'], "-"));
    }
    if architecture != host_architecture() {
        name.push('-');
        name.push_str(architecture);
    }
    name.trim_end_matches('-').to_string()
}

pub(super) fn input_file_name(input: &str) -> Result<String> {
    if input.starts_with("http://") || input.starts_with("https://") {
        let path = input.split(['?', '#']).next().unwrap_or(input);
        let name = path.rsplit('/').next().unwrap_or_default();
        if !name.is_empty() && name != "." && name != ".." {
            return Ok(name.to_string());
        }
        return Err(Error::message("image URL does not contain a file name"));
    }
    let path = Path::new(input);
    path.file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty() && *value != "." && *value != "..")
        .map(str::to_string)
        .ok_or_else(|| Error::message("image path does not contain a file name"))
}

pub(super) fn validate_vm_name(name: &str) -> Result<&str> {
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.chars().any(|character| {
            character == '/'
                || character == '\\'
                || character.is_control()
                || character == '='
                || character == ','
        })
    {
        return Err(Error::message(
            "VM name contains unsafe path or process-name characters",
        ));
    }
    Ok(name)
}

pub(super) fn find_os(os: &str) -> Result<OsInfo> {
    let os = os.to_ascii_lowercase();
    OS_CATALOG
        .iter()
        .find(|info| info.id == os)
        .copied()
        .ok_or_else(|| {
            Error::message(format!(
                "unsupported OS '{os}' (use --list to see supported systems)"
            ))
        })
}

pub(super) fn normalize_architecture(value: &str) -> Result<&'static str> {
    match value.to_ascii_lowercase().as_str() {
        "amd64" | "x86_64" => Ok("amd64"),
        "arm64" | "aarch64" => Ok("arm64"),
        _ => Err(Error::message(
            "architecture must be amd64, x86_64, arm64, or aarch64",
        )),
    }
}

pub(super) fn qemu_architecture(value: &str) -> &str {
    if value == "arm64" {
        "aarch64"
    } else {
        "x86_64"
    }
}

pub(super) fn host_architecture() -> &'static str {
    if cfg!(any(target_arch = "aarch64", target_arch = "arm")) {
        "arm64"
    } else {
        "amd64"
    }
}

pub(super) fn file_name_from_url(url: &str) -> Option<String> {
    let path = url.split(['?', '#']).next()?;
    let name = path.rsplit('/').next()?;
    (name.contains('.') && !name.is_empty()).then(|| name.to_string())
}

pub(super) fn dynamic_url_error(os: &str) -> Error {
    Error::message(format!(
        "{os} uses a dynamic provider URL; download it with `vmctl get`, then create a VM from the cached image"
    ))
}

pub(super) fn required_arg<'a>(value: Option<&'a str>, name: &str) -> Result<&'a str> {
    value.ok_or_else(|| Error::message(format!("{name} is required")))
}

pub(super) fn config_value(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

pub(super) fn csv_field(value: &str) -> String {
    if value.contains([',', '"', '\n']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}
