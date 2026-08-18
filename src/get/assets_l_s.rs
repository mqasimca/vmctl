use super::*;

pub(super) fn easyos_asset(release: &str) -> Result<(String, ImageKind, Option<String>)> {
    let base = "https://distro.ibiblio.org/easyos/amd64/releases/kirkstone";
    for year in (2020..=2035).rev() {
        let directory = format!("{base}/{year}/{release}");
        let Ok(page) = fetch_text(&format!("{directory}/")) else {
            continue;
        };
        let file = first_token(&page, |value| {
            value.starts_with("easy-") && value.ends_with("-amd64.img")
        })
        .unwrap_or_else(|| format!("easy-{release}-amd64.img"));
        let checksum = checksum_at(&format!("{directory}/md5.sum.txt"), &file, "sha256");
        return Ok((format!("{directory}/{file}"), ImageKind::Img, checksum));
    }
    Err(dynamic_url_error("easyos"))
}

pub(super) fn endeavouros_asset(release: &str) -> Result<(String, ImageKind, Option<String>)> {
    let base = "https://mirror.alpix.eu/endeavouros/iso";
    let page = fetch_text(&format!("{base}/"))?;
    let file = first_token(&page, |value| {
        value.ends_with(".iso")
            && !value.contains("x86_64")
            && (release == "latest"
                || value
                    .to_ascii_lowercase()
                    .contains(&release.to_ascii_lowercase()))
    })
    .ok_or_else(|| dynamic_url_error("endeavouros"))?;
    let checksum = checksum_at(&format!("{base}/{file}.sha512sum"), &file, "sha512");
    Ok((format!("{base}/{file}"), ImageKind::Iso, checksum))
}

pub(super) fn endless_asset(
    release: &str,
    edition: Option<&str>,
) -> Result<(String, ImageKind, Option<String>)> {
    let edition = edition.ok_or_else(|| Error::message("Endless OS requires a language"))?;
    let timestamp = match (release, edition) {
        ("6.0.4", "base") => "241023-183516",
        ("6.0.4", "en") => "241023-200926",
        ("6.0.4", "es") => "241023-184649",
        ("6.0.4", "fr") => "241023-191212",
        ("6.0.4", "pt_BR") => "241023-191427",
        _ => return Err(dynamic_url_error("endless")),
    };
    let short = release.chars().take(3).collect::<String>();
    let file = format!("eos-eos{short}-amd64-amd64.{timestamp}.{edition}.iso");
    let base =
        format!("https://images-dl.endlessm.com/release/{release}/eos-amd64-amd64/{edition}");
    Ok((format!("{base}/{file}"), ImageKind::Iso, None))
}

pub(super) fn garuda_asset(edition: Option<&str>) -> Result<(String, ImageKind, Option<String>)> {
    let edition = edition.ok_or_else(|| Error::message("Garuda Linux requires an edition"))?;
    let base = "https://iso.builds.garudalinux.org/iso/latest/garuda";
    let file = format!("{edition}/latest.iso");
    let checksum = checksum_at(&format!("{base}/{file}.sha256"), &file, "sha256");
    Ok((format!("{base}/{file}"), ImageKind::Iso, checksum))
}

pub(super) fn gentoo_asset(
    release: &str,
    edition: Option<&str>,
) -> Result<(String, ImageKind, Option<String>)> {
    let edition = edition.ok_or_else(|| Error::message("Gentoo requires an edition"))?;
    let base = "https://mirrors.kernel.org/gentoo/releases/amd64/autobuilds";
    let listing = fetch_text(&format!("{base}/{release}-iso.txt"))?;
    let marker = if edition == "livegui" {
        "livegui"
    } else {
        "install"
    };
    let file = first_token(&listing, |value| {
        value.ends_with(".iso") && value.contains(marker)
    })
    .ok_or_else(|| dynamic_url_error("gentoo"))?;
    let sums = fetch_text(&format!("{base}/{file}.DIGESTS"))?;
    let checksum = checksum_from_text(&sums, &file, "sha512");
    Ok((format!("{base}/{file}"), ImageKind::Iso, checksum))
}

pub(super) fn ghostbsd_asset(
    release: &str,
    edition: Option<&str>,
) -> Result<(String, ImageKind, Option<String>)> {
    let edition = edition.ok_or_else(|| Error::message("GhostBSD requires an edition"))?;
    let file = match edition {
        "mate" => format!("GhostBSD-{release}.iso"),
        "xfce" => format!("GhostBSD-{release}-XFCE.iso"),
        _ => return Err(dynamic_url_error("ghostbsd")),
    };
    let base = format!("https://download.ghostbsd.org/releases/amd64/{release}");
    let checksum = checksum_at(&format!("{base}/{file}.sha256"), &file, "sha256");
    Ok((format!("{base}/{file}"), ImageKind::Iso, checksum))
}

pub(super) fn gnomeos_asset(release: &str) -> Result<(String, ImageKind, Option<String>)> {
    let (base, file) = if release == "nightly" {
        ("https://os.gnome.org/download/latest".to_string(), None)
    } else {
        (
            format!("https://download.gnome.org/gnomeos/{release}"),
            Some(format!("gnome_os_installer_{release}.iso")),
        )
    };
    let url = if let Some(file) = file {
        fetch_redirect(&format!("{base}/{file}"))?
    } else {
        fetch_redirect(&base)?
    };
    Ok((url, ImageKind::Iso, None))
}

pub(super) fn kdeneon_asset(
    release: &str,
    edition: Option<&str>,
) -> Result<(String, ImageKind, Option<String>)> {
    let edition = edition.ok_or_else(|| Error::message("KDE neon requires an edition"))?;
    let base = format!("https://files.kde.org/neon/images/{edition}/{release}/current");
    let sums = fetch_text(&format!(
        "{base}/neon-{release}-{edition}-current.sha256sum"
    ))?;
    let file = first_token(&sums, |value| value.ends_with(".iso"))
        .ok_or_else(|| dynamic_url_error("kdeneon"))?;
    let checksum = checksum_from_text(&sums, &file, "sha256");
    Ok((format!("{base}/{file}"), ImageKind::Iso, checksum))
}

pub(super) fn kdelinux_asset(release: &str) -> Result<(String, ImageKind, Option<String>)> {
    if release != "latest" {
        return Err(Error::message("KDE Linux currently supports only latest"));
    }
    let base = "https://files.kde.org/kde-linux";
    let listing = fetch_text(&format!("{base}/?C=M;O=D"))?;
    let file =
        first_token(&listing, is_kde_linux_iso).ok_or_else(|| dynamic_url_error("kdelinux"))?;
    let sums = fetch_text(&format!("{base}/SHA256SUMS"))?;
    let checksum = checksum_from_text(&sums, &file, "sha256");
    Ok((format!("{base}/{file}"), ImageKind::Iso, checksum))
}

pub(super) fn is_kde_linux_iso(value: &str) -> bool {
    value
        .strip_prefix("kde-linux_")
        .and_then(|value| value.strip_suffix(".iso"))
        .is_some_and(|value| value.len() == 12 && value.bytes().all(|byte| byte.is_ascii_digit()))
}

pub(super) fn kolibrios_asset(
    edition: Option<&str>,
) -> Result<(String, ImageKind, Option<String>)> {
    let edition = edition.ok_or_else(|| Error::message("KolibriOS requires a language"))?;
    let base = format!("http://builds.kolibrios.org/{edition}");
    let file = "latest-iso.7z";
    let checksum = checksum_at(&format!("{base}/sha256sums.txt"), file, "sha256");
    Ok((format!("{base}/{file}"), ImageKind::Archive, checksum))
}

pub(super) fn mageia_asset(
    release: &str,
    edition: Option<&str>,
) -> Result<(String, ImageKind, Option<String>)> {
    let edition = edition.ok_or_else(|| Error::message("Mageia requires an edition"))?;
    let query = format!("Mageia-{release}-Live-{edition}-x86_64.iso");
    let page = fetch_text(&format!(
        "https://www.mageia.org/en/downloads/get/?q={query}"
    ))?;
    let url = first_token(&page, |value| {
        value.starts_with("http") && value.ends_with(".iso")
    })
    .ok_or_else(|| dynamic_url_error("mageia"))?;
    let checksum = checksum_at(&format!("{url}.sha512"), &url, "sha512");
    Ok((url, ImageKind::Iso, checksum))
}

pub(super) fn manjaro_asset(
    release: &str,
    edition: Option<&str>,
) -> Result<(String, ImageKind, Option<String>)> {
    let data = if release == "sway" {
        fetch_text("https://mirror.manjaro-sway.download/manjaro-sway/release.json")?
    } else {
        fetch_text("https://gitlab.manjaro.org/web/iso-info/-/raw/master/file-info.json")?
    };
    let values: Value = serde_json::from_str(&data)
        .map_err(|error| Error::message(format!("invalid Manjaro release data: {error}")))?;
    let url = if release == "sway" {
        values.as_array().into_iter().flatten().find_map(|entry| {
            let name = entry.get("name").and_then(Value::as_str)?;
            (name.starts_with("manjaro-sway-") && name.ends_with(".iso"))
                .then(|| entry.get("url").and_then(Value::as_str))
                .flatten()
        })
    } else {
        let group = if matches!(release, "cinnamon" | "i3") {
            "community"
        } else {
            "official"
        };
        let suffix = if edition == Some("minimal") {
            ".minimal"
        } else {
            ""
        };
        let key = format!("{release}{suffix}");
        values
            .get(group)
            .and_then(|group| group.get(&key))
            .and_then(|entry| entry.get("image"))
            .and_then(Value::as_str)
    }
    .ok_or_else(|| dynamic_url_error("manjaro"))?;
    let checksum = checksum_at(&format!("{url}.sha512"), url, "sha512");
    Ok((url.to_string(), ImageKind::Iso, checksum))
}

pub(super) fn mxlinux_asset(
    release: &str,
    edition: Option<&str>,
) -> Result<(String, ImageKind, Option<String>)> {
    let edition = edition.ok_or_else(|| Error::message("MX Linux requires an edition"))?;
    let suffix = match edition {
        "Xfce" => "Xfce",
        "KDE" => "KDE",
        "Fluxbox" => "fluxbox",
        _ => return Err(dynamic_url_error("mxlinux")),
    };
    let file = format!("MX-{release}_{suffix}_x64.iso");
    let url = sourceforge_asset("mx-linux", &format!("Final/{edition}/{file}"))?;
    let checksum = checksum_at(
        &format!("https://sourceforge.net/projects/mx-linux/files/Final/{edition}/{file}.sha256"),
        &file,
        "sha256",
    );
    Ok((url, ImageKind::Iso, checksum))
}

pub(super) fn nitrux_asset() -> Result<(String, ImageKind, Option<String>)> {
    let page = fetch_text("https://sourceforge.net/projects/nitruxos/rss?path=/Release/ISO")?;
    let file = first_token(&page, |value| value.ends_with(".iso"))
        .ok_or_else(|| dynamic_url_error("nitrux"))?;
    let url = sourceforge_asset("nitruxos", &format!("Release/ISO/{file}"))?;
    Ok((url, ImageKind::Iso, None))
}

pub(super) fn nwg_shell_asset(release: &str) -> Result<(String, ImageKind, Option<String>)> {
    let page = fetch_text("https://sourceforge.net/projects/nwg-iso/rss?path=/")?;
    let file = first_token(&page, |value| {
        value.ends_with(".iso") && value.contains("nwg-live") && value.contains(release)
    })
    .ok_or_else(|| dynamic_url_error("nwg-shell"))?;
    let url = sourceforge_asset("nwg-iso", &file)?;
    Ok((url, ImageKind::Iso, None))
}

pub(super) fn pclinuxos_asset(
    edition: Option<&str>,
) -> Result<(String, ImageKind, Option<String>)> {
    let edition = edition.ok_or_else(|| Error::message("PCLinuxOS requires an edition"))?;
    let base = "https://ftp.fau.de/pclinuxos/pclinuxos/iso";
    let page = fetch_text(&format!("{base}/"))?;
    let prefix = format!("pclinuxos64-{edition}-");
    let file = first_token(&page, |value| {
        value.starts_with(&prefix) && value.ends_with(".iso")
    })
    .ok_or_else(|| dynamic_url_error("pclinuxos"))?;
    let checksum = checksum_at(&format!("{base}/{file}.md5sum"), &file, "sha256");
    Ok((format!("{base}/{file}"), ImageKind::Iso, checksum))
}

pub(super) fn peppermint_asset(
    edition: Option<&str>,
) -> Result<(String, ImageKind, Option<String>)> {
    let edition = edition.ok_or_else(|| Error::message("PeppermintOS requires an edition"))?;
    let (directory, file) = match edition {
        "devuan-xfce" => ("XFCE", "PeppermintOS-devuan_64_xfce.iso"),
        "debian-xfce" => ("XFCE", "PeppermintOS-Debian-64.iso"),
        "devuan-gnome" => ("Gnome_FlashBack", "PeppermintOS-devuan_64_gfb.iso"),
        "debian-gnome" => ("Gnome_FlashBack", "PeppermintOS-Debian_64_gfb.iso"),
        _ => return Err(dynamic_url_error("peppermint")),
    };
    let base = format!("https://sourceforge.net/projects/peppermintos/files/isos/{directory}");
    let url = sourceforge_asset("peppermintos", &format!("isos/{directory}/{file}"))?;
    let checksum = checksum_at(&format!("{base}/{file}-sha512.checksum"), file, "sha512");
    Ok((url, ImageKind::Iso, checksum))
}

pub(super) fn primtux_asset(
    release: &str,
    edition: Option<&str>,
) -> Result<(String, ImageKind, Option<String>)> {
    let edition = edition.ok_or_else(|| Error::message("PrimTux requires an edition"))?;
    let file = format!("PrimTux{release}-amd64-{edition}.iso");
    let url = sourceforge_asset("primtux", &format!("Distribution/{file}"))?;
    Ok((url, ImageKind::Iso, None))
}

pub(super) fn pureos_asset(
    release: &str,
    edition: Option<&str>,
) -> Result<(String, ImageKind, Option<String>)> {
    let edition = edition.ok_or_else(|| Error::message("PureOS requires an edition"))?;
    let page = fetch_text("https://www.pureos.net/download/")?;
    let url = first_token(&page, |value| {
        value.starts_with("https://downloads.puri.sm/") && value.ends_with(".iso")
    })
    .or_else(|| {
        let lower = edition.to_ascii_lowercase();
        first_token(&page, |value| {
            value.starts_with("https://downloads.puri.sm/")
                && value.contains(&lower)
                && value.contains(release)
                && value.ends_with(".iso")
        })
    })
    .ok_or_else(|| dynamic_url_error("pureos"))?;
    let file = file_name_from_url(&url).unwrap_or_else(|| format!("pureos-{release}.iso"));
    let checksum = checksum_at(
        &format!(
            "{}/{}.checksums_sha256.txt",
            url.trim_end_matches(&file),
            file.trim_end_matches(".iso")
        ),
        &file,
        "sha256",
    );
    Ok((url, ImageKind::Iso, checksum))
}

pub(super) fn rebornos_asset() -> Result<(String, ImageKind, Option<String>)> {
    let data = fetch_text("https://meta.cdn.soulharsh007.dev/RebornOS-ISO?format=json")?;
    let values: Value = serde_json::from_str(&data)
        .map_err(|error| Error::message(format!("invalid RebornOS release data: {error}")))?;
    let url = values
        .get("url")
        .and_then(Value::as_str)
        .ok_or_else(|| dynamic_url_error("rebornos"))?;
    let checksum = values
        .get("sha256")
        .or_else(|| values.get("md5"))
        .and_then(Value::as_str)
        .filter(|value| value.len() == 64)
        .map(|value| format!("sha256:{value}"));
    Ok((url.to_string(), ImageKind::Iso, checksum))
}

pub(super) fn slax_asset(edition: Option<&str>) -> Result<(String, ImageKind, Option<String>)> {
    let edition = edition.ok_or_else(|| Error::message("Slax requires an edition"))?;
    let base = match edition {
        "debian" => "https://ftp.fi.muni.cz/pub/linux/slax/Slax-12.x",
        "slackware" => "https://ftp.fi.muni.cz/pub/linux/slax/Slax-15.x",
        _ => return Err(dynamic_url_error("slax")),
    };
    let sums = fetch_text(&format!("{base}/md5.txt"))?;
    let file = first_token(&sums, |value| {
        value.contains("64bit-") && value.ends_with(".iso")
    })
    .ok_or_else(|| dynamic_url_error("slax"))?;
    Ok((format!("{base}/{file}"), ImageKind::Iso, None))
}

pub(super) fn solus_asset(
    release: &str,
    edition: Option<&str>,
) -> Result<(String, ImageKind, Option<String>)> {
    let edition = edition.ok_or_else(|| Error::message("Solus requires an edition"))?;
    let base = format!("https://downloads.getsol.us/isos/{release}");
    let data = fetch_text(&format!("{base}/"))?;
    let values: Value = serde_json::from_str(&data)
        .map_err(|error| Error::message(format!("invalid Solus release data: {error}")))?;
    let edition_upper = edition.to_ascii_uppercase();
    let file = values
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.get("name").and_then(Value::as_str))
        .find(|name| {
            name.ends_with(".iso")
                && name.contains(&edition_upper)
                && (name.contains("Release") || name.contains("Beta"))
        })
        .ok_or_else(|| dynamic_url_error("solus"))?;
    let checksum = checksum_at(&format!("{base}/{file}.sha256sum"), file, "sha256");
    Ok((format!("{base}/{file}"), ImageKind::Iso, checksum))
}

pub(super) fn sparkylinux_asset(
    release: &str,
    edition: Option<&str>,
) -> Result<(String, ImageKind, Option<String>)> {
    let edition = edition.ok_or_else(|| Error::message("SparkyLinux requires an edition"))?;
    let file = format!("sparkylinux-{release}-x86_64-{edition}.iso");
    let directory = match edition {
        "minimalcli" => "cli",
        "minimalgui" => "base",
        _ => edition,
    };
    let base = format!("https://sourceforge.net/projects/sparkylinux/files/{directory}");
    let url = sourceforge_asset("sparkylinux", &format!("{directory}/{file}"))?;
    let checksum = checksum_at(&format!("{base}/{file}.allsums.txt"), &file, "sha256");
    Ok((url, ImageKind::Iso, checksum))
}

pub(super) fn spirallinux_asset(
    edition: Option<&str>,
) -> Result<(String, ImageKind, Option<String>)> {
    let edition = edition.ok_or_else(|| Error::message("SpiralLinux requires an edition"))?;
    let file = format!("SpiralLinux_{edition}_12.231005_x86-64.iso");
    let url = sourceforge_asset("spirallinux", &format!("12.231005/{file}"))?;
    Ok((url, ImageKind::Iso, None))
}
