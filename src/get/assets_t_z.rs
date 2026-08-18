use super::*;

pub(super) fn tuxedo_asset() -> Result<(String, ImageKind, Option<String>)> {
    let page = fetch_text("https://os.tuxedocomputers.com/")?;
    let url = first_token(&page, |value| {
        value.starts_with("http") && value.ends_with("current.iso")
    })
    .or_else(|| {
        first_token(&page, |value| {
            value.starts_with('/') && value.ends_with("current.iso")
        })
        .map(|value| format!("https://os.tuxedocomputers.com{value}"))
    })
    .ok_or_else(|| dynamic_url_error("tuxedo-os"))?;
    let file = file_name_from_url(&url).unwrap_or_else(|| "current.iso".to_string());
    let checksum = checksum_at(
        &format!("https://os.tuxedocomputers.com/checksums/{file}.sha256"),
        &file,
        "sha256",
    );
    Ok((url, ImageKind::Iso, checksum))
}

pub(super) fn vanillaos_asset(release: &str) -> Result<(String, ImageKind, Option<String>)> {
    let data = fetch_text("https://api.github.com/repos/Vanilla-OS/live-iso/releases")?;
    let values: Value = serde_json::from_str(&data)
        .map_err(|error| Error::message(format!("invalid Vanilla OS release data: {error}")))?;
    let release_data = values
        .as_array()
        .into_iter()
        .flatten()
        .find(|entry| {
            release == "latest" || entry.get("tag_name").and_then(Value::as_str) == Some(release)
        })
        .ok_or_else(|| dynamic_url_error("vanillaos"))?;
    let url = release_data
        .get("assets")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find_map(|asset| {
            let name = asset.get("name").and_then(Value::as_str)?;
            name.ends_with(".iso")
                .then(|| asset.get("browser_download_url").and_then(Value::as_str))
                .flatten()
        })
        .ok_or_else(|| dynamic_url_error("vanillaos"))?;
    let checksum = fetch_text(&format!("{url}.sha256.txt"))
        .ok()
        .and_then(|value| {
            value
                .split_whitespace()
                .next()
                .map(|hash| format!("sha256:{hash}"))
        });
    Ok((url.to_string(), ImageKind::Iso, checksum))
}

pub(super) fn zorin_asset(
    release: &str,
    edition: Option<&str>,
) -> Result<(String, ImageKind, Option<String>)> {
    let edition = edition.ok_or_else(|| Error::message("Zorin OS requires an edition"))?;
    let edition_name = match edition {
        "core64" => "Core-64-bit",
        "lite64" => "Lite-64-bit",
        "education64" => "Education-64-bit",
        _ => return Err(dynamic_url_error("zorin")),
    };
    let base = format!("https://plug-mirror.rcac.purdue.edu/zorin-iso/{release}");
    let page = fetch_text(&format!("{base}/"))?;
    let prefix = format!("Zorin-OS-{release}-{edition_name}");
    let file = first_token(&page, |value| {
        value.starts_with(&prefix) && value.ends_with(".iso") && !value.contains("Beta")
    })
    .ok_or_else(|| dynamic_url_error("zorin"))?;
    let checksum = checksum_at(&format!("{base}/SHA256SUMS.txt"), &file, "sha256");
    Ok((format!("{base}/{file}"), ImageKind::Iso, checksum))
}

pub(super) fn fedora_asset(
    release: &str,
    edition: Option<&str>,
    architecture: &str,
) -> Result<(String, ImageKind, Option<String>)> {
    let edition = edition.ok_or_else(|| Error::message("Fedora requires an edition"))?;
    let data = fetch_text("https://getfedora.org/releases.json")?;
    let values: Value = serde_json::from_str(&data)
        .map_err(|error| Error::message(format!("invalid Fedora release data: {error}")))?;
    let wanted_release = release.replace('_', " ");
    let qemu_arch = if architecture == "arm64" {
        "aarch64"
    } else {
        "x86_64"
    };
    let entry = values
        .as_array()
        .into_iter()
        .flatten()
        .find(|entry| {
            entry.get("version").and_then(Value::as_str) == Some(wanted_release.as_str())
                && entry.get("arch").and_then(Value::as_str) == Some(qemu_arch)
                && entry.get("subvariant").and_then(Value::as_str) == Some(edition)
                && entry
                    .get("link")
                    .and_then(Value::as_str)
                    .is_some_and(|value| value.ends_with(".iso"))
        })
        .ok_or_else(|| dynamic_url_error("fedora"))?;
    let link = entry
        .get("link")
        .and_then(Value::as_str)
        .ok_or_else(|| dynamic_url_error("fedora"))?;
    let checksum = entry
        .get("sha256")
        .and_then(Value::as_str)
        .map(str::to_string);
    Ok((link.to_string(), ImageKind::Iso, checksum))
}

pub(super) fn kali_asset(
    release: &str,
    architecture: &str,
) -> Result<(String, ImageKind, Option<String>)> {
    let arch = if architecture == "arm64" {
        "arm64"
    } else {
        "amd64"
    };
    let base = format!("https://cdimage.kali.org/{release}");
    let page = fetch_text(&format!("{base}/?C=M;O=D"))?;
    let name = page
        .split("kali-linux-")
        .skip(1)
        .filter_map(|value| value.split(['\"', '<', '>']).next())
        .map(|value| format!("kali-linux-{value}"))
        .find(|value| value.ends_with(".iso") && value.contains(arch))
        .ok_or_else(|| dynamic_url_error("kali"))?;
    let checksum = fetch_text(&format!("{base}/SHA256SUMS"))
        .ok()
        .and_then(|sums| {
            sums.lines().find_map(|line| {
                let mut fields = line.split_whitespace();
                let hash = fields.next()?;
                let file = fields.next()?.trim_start_matches('*');
                (file == name).then(|| hash.to_string())
            })
        });
    Ok((format!("{base}/{name}"), ImageKind::Iso, checksum))
}

pub(super) fn popos_asset(
    release: &str,
    edition: Option<&str>,
) -> Result<(String, ImageKind, Option<String>)> {
    let edition = edition.ok_or_else(|| Error::message("Pop!_OS requires intel or nvidia"))?;
    let data = fetch_text(&format!(
        "https://api.pop-os.org/builds/{release}/{edition}"
    ))?;
    let values: Value = serde_json::from_str(&data)
        .map_err(|error| Error::message(format!("invalid Pop!_OS release data: {error}")))?;
    let url = values
        .get("url")
        .and_then(Value::as_str)
        .ok_or_else(|| dynamic_url_error("popos"))?;
    let checksum = values
        .get("sha_sum")
        .and_then(Value::as_str)
        .map(str::to_string);
    Ok((url.to_string(), ImageKind::Iso, checksum))
}

pub(super) fn tails_asset(release: &str) -> Result<(String, ImageKind, Option<String>)> {
    let data = fetch_text(&format!(
        "https://tails.boum.org/install/v2/Tails/amd64/{release}/latest.json"
    ))?;
    let values: Value = serde_json::from_str(&data)
        .map_err(|error| Error::message(format!("invalid Tails release data: {error}")))?;
    let file = values
        .get("installations")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|installation| installation.get("installation-paths"))
        .flat_map(|paths| paths.as_array().into_iter().flatten())
        .find(|path| path.get("type").and_then(Value::as_str) == Some("iso"))
        .and_then(|path| path.get("target-files"))
        .and_then(Value::as_array)
        .and_then(|files| files.first())
        .and_then(|file| file.as_object())
        .ok_or_else(|| dynamic_url_error("tails"))?;
    let url = file
        .get("url")
        .and_then(Value::as_str)
        .ok_or_else(|| dynamic_url_error("tails"))?;
    let checksum = file
        .get("sha256")
        .and_then(Value::as_str)
        .map(str::to_string);
    Ok((url.to_string(), ImageKind::Iso, checksum))
}

pub(super) fn is_ubuntu_family(os: &str) -> bool {
    matches!(
        os,
        "ubuntu"
            | "edubuntu"
            | "kubuntu"
            | "lubuntu"
            | "ubuntu-budgie"
            | "ubuntucinnamon"
            | "ubuntukylin"
            | "ubuntu-mate"
            | "ubuntu-server"
            | "ubuntustudio"
            | "ubuntu-unity"
            | "xubuntu"
    )
}

pub(super) fn is_ubuntu_desktop(os: &str) -> bool {
    is_ubuntu_family(os) && os != "ubuntu-server"
}

pub(super) fn ubuntu_arm64_release(release: &str) -> bool {
    let Some((major, minor)) = release.split_once('.') else {
        return false;
    };
    let Ok(major) = major.parse::<u32>() else {
        return false;
    };
    let Ok(minor) = minor.parse::<u32>() else {
        return false;
    };
    major > 25 || (major == 25 && minor >= 10)
}

pub(super) fn ubuntu_asset(
    os: &str,
    release: &str,
    architecture: &str,
) -> Option<(String, String, Option<String>)> {
    let base = if release.contains("daily") || release == "dvd" {
        format!("https://cdimage.ubuntu.com/{os}/{release}/current")
    } else if matches!(os, "ubuntu" | "ubuntu-server") && architecture == "amd64" {
        format!("https://releases.ubuntu.com/{release}")
    } else if os == "ubuntu" {
        format!("https://cdimage.ubuntu.com/releases/{release}/release")
    } else {
        format!("https://cdimage.ubuntu.com/{os}/releases/{release}/release")
    };
    let sums = fetch_text(&format!("{base}/SHA256SUMS")).ok()?;
    let (file, checksum) = sums
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let hash = fields.next()?;
            let file = fields.next()?.trim_start_matches('*');
            let variant = if os == "ubuntu-server" {
                file.contains("server")
            } else {
                file.contains("desktop") || file.contains("dvd")
            };
            let is_image = file.ends_with(".iso")
                && file.contains(architecture)
                && !file.contains("+mac")
                && variant;
            is_image.then(|| (file.to_string(), hash.to_string()))
        })
        .next_back()?;
    Some((format!("{base}/{file}"), file, Some(checksum)))
}

pub(super) fn debian_asset(
    release: &str,
    edition: Option<&str>,
    architecture: &str,
) -> Option<(String, String, Option<String>)> {
    let edition = edition.unwrap_or("standard");
    let (base, sums_name) = if edition == "netinst" {
        (
            format!("https://cdimage.debian.org/debian-cd/{release}/{architecture}/iso-cd"),
            "SHA512SUMS",
        )
    } else {
        (
            format!(
                "https://cdimage.debian.org/debian-cd/{release}-live/{architecture}/iso-hybrid"
            ),
            "SHA512SUMS",
        )
    };
    let sums = fetch_text(&format!("{base}/{sums_name}")).ok()?;
    let (file, checksum) = sums
        .lines()
        .filter_map(|line| {
            let checksum = line.split_whitespace().next()?;
            let file = line.split_whitespace().nth(1)?.trim_start_matches('*');
            (file.ends_with(".iso") && file.contains(edition) && file.contains(architecture))
                .then(|| (file.to_string(), checksum.to_string()))
        })
        .next_back()?;
    Some((
        format!("{base}/{file}"),
        file,
        Some(format!("sha512:{checksum}")),
    ))
}
