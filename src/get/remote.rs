use super::*;

pub(super) fn required_edition(info: OsInfo, edition: Option<&str>) -> Result<Option<String>> {
    if info.editions.is_empty() {
        return Ok(None);
    }
    if matches!(info.id, "windows" | "windows-server") {
        if let Some(edition) = edition
            && edition != info.editions
        {
            return Err(Error::message(format!(
                "{edition} is not a supported {} language (supported: {})",
                info.name, info.editions
            )));
        }
        return Ok(Some(info.editions.to_string()));
    }
    if let Some(edition) = edition {
        if info.editions != "dynamic"
            && !info
                .editions
                .split_whitespace()
                .any(|value| value == edition)
        {
            return Err(Error::message(format!(
                "{edition} is not a supported {} edition (supported: {})",
                info.name, info.editions
            )));
        }
        return Ok(Some(edition.to_string()));
    }
    if info.editions == "dynamic" {
        return Err(Error::message(format!(
            "{} editions are discovered from the upstream provider; specify one explicitly",
            info.name
        )));
    }
    if info.editions.split_whitespace().count() == 1 {
        return Ok(info.editions.split_whitespace().next().map(str::to_string));
    }
    Err(Error::message(format!(
        "{} requires an edition; supported: {}",
        info.name, info.editions
    )))
}

pub(super) fn resolve_remote_image(
    os: &str,
    release: &str,
    edition: Option<&str>,
    architecture: &str,
) -> Result<ResolvedImage> {
    let os = find_os(os)?.id;
    if is_dynamic_provider(os) {
        return resolve_dynamic_image(os, release, edition, architecture);
    }
    let mut image = resolve_image(os, release, edition, architecture)?;
    if is_ubuntu_family(os) {
        if let Some((url, file_name, checksum)) = ubuntu_asset(os, release, architecture) {
            image.url = url;
            image.file_name = file_name;
            image.checksum = checksum;
        }
    } else if os == "debian"
        && let Some((url, file_name, checksum)) = debian_asset(release, edition, architecture)
    {
        image.url = url;
        image.file_name = file_name;
        image.checksum = checksum;
    }
    Ok(image)
}

pub(super) fn is_dynamic_provider(os: &str) -> bool {
    matches!(
        os,
        "alpine"
            | "android"
            | "antix"
            | "archcraft"
            | "artixlinux"
            | "azurelinux"
            | "batocera"
            | "bazzite"
            | "biglinux"
            | "blendos"
            | "bodhi"
            | "bunsenlabs"
            | "cachyos"
            | "chimeralinux"
            | "crunchbang++"
            | "easyos"
            | "elementary"
            | "endeavouros"
            | "endless"
            | "fedora"
            | "garuda"
            | "gentoo"
            | "ghostbsd"
            | "gnomeos"
            | "kdeneon"
            | "kdelinux"
            | "kali"
            | "kolibrios"
            | "mageia"
            | "manjaro"
            | "mxlinux"
            | "nitrux"
            | "nwg-shell"
            | "pclinuxos"
            | "peppermint"
            | "primtux"
            | "pureos"
            | "popos"
            | "rebornos"
            | "siduction"
            | "slax"
            | "solus"
            | "sparkylinux"
            | "spirallinux"
            | "tails"
            | "tuxedo-os"
            | "vanillaos"
            | "windows"
            | "windows-server"
            | "macos"
            | "zorin"
    )
}

pub(super) fn resolve_dynamic_image(
    os: &str,
    release: &str,
    edition: Option<&str>,
    architecture: &str,
) -> Result<ResolvedImage> {
    let info = find_os(os)?;
    let os = info.id;
    let architecture = normalize_architecture(architecture)?;
    if !info
        .architectures
        .split_whitespace()
        .any(|value| value == architecture)
    {
        return Err(Error::message(format!(
            "{} is not available for {}",
            info.name, architecture
        )));
    }
    let edition = required_edition(info, edition)?;
    if os == "fedora" && architecture == "arm64" && edition.as_deref() == Some("Onyx") {
        return Err(Error::message("Fedora Onyx is not available for ARM64"));
    }
    let (url, kind, checksum) = match os {
        "alpine" => alpine_asset(release, architecture)?,
        "antix" => antix_asset(release, edition.as_deref())?,
        "archcraft" => archcraft_asset(release)?,
        "artixlinux" => artixlinux_asset(release, edition.as_deref())?,
        "azurelinux" => azurelinux_asset(release, architecture)?,
        "batocera" => batocera_asset(release)?,
        "bazzite" => bazzite_asset(edition.as_deref())?,
        "biglinux" => biglinux_asset(release, edition.as_deref())?,
        "blendos" => blendos_asset()?,
        "bodhi" => bodhi_asset(release, edition.as_deref())?,
        "bunsenlabs" => bunsenlabs_asset()?,
        "cachyos" => cachyos_asset(edition.as_deref())?,
        "chimeralinux" => chimeralinux_asset(release, edition.as_deref())?,
        "crunchbang++" => crunchbang_asset(release)?,
        "easyos" => easyos_asset(release)?,
        "endeavouros" => endeavouros_asset(release)?,
        "endless" => endless_asset(release, edition.as_deref())?,
        "fedora" => fedora_asset(release, edition.as_deref(), architecture)?,
        "garuda" => garuda_asset(edition.as_deref())?,
        "gentoo" => gentoo_asset(release, edition.as_deref())?,
        "ghostbsd" => ghostbsd_asset(release, edition.as_deref())?,
        "gnomeos" => gnomeos_asset(release)?,
        "kdeneon" => kdeneon_asset(release, edition.as_deref())?,
        "kdelinux" => kdelinux_asset(release)?,
        "kali" => kali_asset(release, architecture)?,
        "kolibrios" => kolibrios_asset(edition.as_deref())?,
        "mageia" => mageia_asset(release, edition.as_deref())?,
        "manjaro" => manjaro_asset(release, edition.as_deref())?,
        "mxlinux" => mxlinux_asset(release, edition.as_deref())?,
        "nitrux" => nitrux_asset()?,
        "nwg-shell" => nwg_shell_asset(release)?,
        "pclinuxos" => pclinuxos_asset(edition.as_deref())?,
        "peppermint" => peppermint_asset(edition.as_deref())?,
        "primtux" => primtux_asset(release, edition.as_deref())?,
        "pureos" => pureos_asset(release, edition.as_deref())?,
        "popos" => popos_asset(release, edition.as_deref())?,
        "rebornos" => rebornos_asset()?,
        "slax" => slax_asset(edition.as_deref())?,
        "solus" => solus_asset(release, edition.as_deref())?,
        "sparkylinux" => sparkylinux_asset(release, edition.as_deref())?,
        "spirallinux" => spirallinux_asset(edition.as_deref())?,
        "tails" => tails_asset(release)?,
        "tuxedo-os" => tuxedo_asset()?,
        "vanillaos" => vanillaos_asset(release)?,
        "zorin" => zorin_asset(release, edition.as_deref())?,
        "android" => android_asset(release, edition.as_deref())?,
        "elementary" => elementary_asset(release)?,
        "macos" => macos_asset(release, architecture)?,
        "windows" | "windows-server" => windows_asset(os, release, edition.as_deref())?,
        "siduction" => siduction_asset(edition.as_deref())?,
        _ => return Err(dynamic_url_error(os)),
    };
    let file_name = file_name_from_url(&url).unwrap_or_else(|| format!("{os}-{release}.iso"));
    Ok(ResolvedImage {
        os: os.to_string(),
        release: release.to_string(),
        edition,
        architecture: architecture.to_string(),
        url,
        file_name,
        kind,
        checksum,
    })
}

pub(super) fn fetch_redirect(url: &str) -> Result<String> {
    let output = Command::new("curl")
        .args([
            "--disable",
            "--fail",
            "--location",
            "--silent",
            "--show-error",
            "--head",
            "--output",
            "/dev/null",
            "--write-out",
            "%{url_effective}",
            "--",
        ])
        .arg(url)
        .output()
        .map_err(|error| Error::command_unavailable("curl", error))?;
    if !output.status.success() {
        return Err(Error::command_failed_status("curl", output.status));
    }
    let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!value.is_empty())
        .then_some(value)
        .ok_or_else(|| Error::message(format!("provider returned no redirect for {url}")))
}

pub(super) fn sourceforge_asset(project: &str, path: &str) -> Result<String> {
    fetch_redirect(&format!(
        "https://sourceforge.net/projects/{project}/files/{path}/download"
    ))
}

pub(super) fn first_token(text: &str, predicate: impl Fn(&str) -> bool) -> Option<String> {
    text.split(|character: char| {
        character.is_whitespace() || matches!(character, '"' | '\'' | '<' | '>')
    })
    .map(str::trim)
    .find(|value| !value.is_empty() && predicate(value))
    .map(str::to_string)
}

pub(super) fn checksum_from_text(text: &str, file: &str, algorithm: &str) -> Option<String> {
    let length = match algorithm {
        "sha256" => 64,
        "sha512" => 128,
        _ => return None,
    };
    text.lines()
        .filter(|line| line.contains(file))
        .flat_map(|line| line.split(|character: char| !character.is_ascii_hexdigit()))
        .find(|value| value.len() == length)
        .map(|value| format!("{algorithm}:{value}"))
}

pub(super) fn checksum_at(url: &str, file: &str, algorithm: &str) -> Option<String> {
    fetch_text(url)
        .ok()
        .and_then(|text| checksum_from_text(&text, file, algorithm))
}
