use super::*;

pub(super) fn alpine_asset(
    release: &str,
    architecture: &str,
) -> Result<(String, ImageKind, Option<String>)> {
    let qemu_arch = qemu_architecture(architecture);
    let base = format!("https://dl-cdn.alpinelinux.org/alpine/{release}/releases/{qemu_arch}");
    let yaml = fetch_text(&format!("{base}/latest-releases.yaml"))?;
    let mut virtual_section = false;
    let mut version = None;
    let mut checksum = None;
    for line in yaml.lines() {
        if line.contains("\"Virtual\"") {
            virtual_section = true;
        } else if virtual_section && line.contains("\"Xen\"") {
            break;
        } else if virtual_section && line.trim_start().starts_with("version:") {
            version = line
                .split_once(':')
                .map(|(_, value)| value.trim().to_string());
        } else if virtual_section && line.trim_start().starts_with("sha256:") {
            checksum = line
                .split_once(':')
                .map(|(_, value)| format!("sha256:{}", value.trim().trim_matches('"')));
        }
    }
    let version = version.ok_or_else(|| dynamic_url_error("alpine"))?;
    let file = format!("alpine-virt-{version}-{qemu_arch}.iso");
    Ok((format!("{base}/{file}"), ImageKind::Iso, checksum))
}

pub(super) fn antix_asset(
    release: &str,
    edition: Option<&str>,
) -> Result<(String, ImageKind, Option<String>)> {
    let edition = edition.ok_or_else(|| Error::message("antiX requires an edition"))?;
    let mut name = format!("antiX-{release}");
    let mut base = format!("Final/antiX-{release}");
    if edition.contains("runit") {
        name.push_str("-runit");
        if release == "21" {
            base.push_str("/runit-bullseye");
        } else {
            base.push_str(&format!("/runit-antiX-{release}"));
        }
    }
    let suffix = if edition.starts_with("base-") {
        "_x64-base.iso"
    } else if edition.starts_with("core-") {
        "_x64-core.iso"
    } else if edition.starts_with("full-") {
        "_x64-full.iso"
    } else {
        "-net_x64-net.iso"
    };
    name.push_str(suffix);
    let url = sourceforge_asset("antix-linux", &format!("{base}/{name}"))?;
    Ok((url, ImageKind::Iso, None))
}

pub(super) fn archcraft_asset(release: &str) -> Result<(String, ImageKind, Option<String>)> {
    let url = sourceforge_asset("archcraft", release)?;
    Ok((url, ImageKind::Iso, None))
}

pub(super) fn artixlinux_asset(
    release: &str,
    edition: Option<&str>,
) -> Result<(String, ImageKind, Option<String>)> {
    let edition = edition.ok_or_else(|| Error::message("Artix Linux requires an edition"))?;
    let base = "https://iso.artixlinux.org/iso";
    let page = fetch_text(&format!("{base}/"))?;
    let prefix = format!("artix-{edition}-");
    let file = first_token(&page, |value| {
        value.starts_with(&prefix)
            && value.ends_with("-x86_64.iso")
            && (release == "latest" || value.contains(release))
    })
    .ok_or_else(|| dynamic_url_error("artixlinux"))?;
    let checksum = checksum_at(&format!("{base}/sha256sums"), &file, "sha256");
    Ok((format!("{base}/{file}"), ImageKind::Iso, checksum))
}

pub(super) fn azurelinux_asset(
    release: &str,
    architecture: &str,
) -> Result<(String, ImageKind, Option<String>)> {
    let arch = qemu_architecture(architecture);
    let url = fetch_redirect(&format!("https://aka.ms/azurelinux-{release}-{arch}.iso"))?;
    Ok((url, ImageKind::Iso, None))
}

pub(super) fn batocera_asset(release: &str) -> Result<(String, ImageKind, Option<String>)> {
    let base = format!("https://mirrors.o2switch.fr/batocera/x86_64/stable/{release}");
    let page = fetch_text(&format!("{base}/"))?;
    let file = first_token(&page, |value| {
        value.starts_with("batocera") && value.ends_with("img.gz")
    })
    .ok_or_else(|| dynamic_url_error("batocera"))?;
    Ok((format!("{base}/{file}"), ImageKind::Archive, None))
}

pub(super) fn bazzite_asset(edition: Option<&str>) -> Result<(String, ImageKind, Option<String>)> {
    let edition = edition.ok_or_else(|| Error::message("Bazzite requires an edition"))?;
    let file = match edition {
        "gnome" => "bazzite-gnome-stable-amd64.iso",
        "plasma" => "bazzite-stable-amd64.iso",
        "deck-gnome" => "bazzite-deck-gnome-stable-amd64.iso",
        "deck-plasma" => "bazzite-deck-stable-amd64.iso",
        _ => return Err(dynamic_url_error("bazzite")),
    };
    let base = "https://download.bazzite.gg";
    let checksum = checksum_at(&format!("{base}/{file}-CHECKSUM"), file, "sha256");
    Ok((format!("{base}/{file}"), ImageKind::Iso, checksum))
}

pub(super) fn biglinux_asset(
    release: &str,
    edition: Option<&str>,
) -> Result<(String, ImageKind, Option<String>)> {
    let edition = edition.ok_or_else(|| Error::message("BigLinux requires an edition"))?;
    let file = format!("biglinux_{release}_{edition}.iso");
    Ok((
        format!("https://iso.biglinux.com.br/{file}"),
        ImageKind::Iso,
        None,
    ))
}

pub(super) fn blendos_asset() -> Result<(String, ImageKind, Option<String>)> {
    let base = "https://git.blendos.co/api/v4/projects/32/jobs/artifacts/main/raw";
    let file = "blendOS.iso";
    let checksum = fetch_text(&format!("{base}/checksum?job=build-job"))
        .ok()
        .and_then(|value| {
            value
                .split_whitespace()
                .next()
                .map(|hash| format!("sha256:{hash}"))
        });
    Ok((
        format!("{base}/{file}?job=build-job"),
        ImageKind::Iso,
        checksum,
    ))
}

pub(super) fn bodhi_asset(
    release: &str,
    edition: Option<&str>,
) -> Result<(String, ImageKind, Option<String>)> {
    let edition = edition.unwrap_or("standard");
    let file = if edition == "standard" {
        format!("bodhi-{release}-64.iso")
    } else {
        format!("bodhi-{release}-64-{edition}.iso")
    };
    let base = format!("https://sourceforge.net/projects/bodhilinux/files/{release}");
    let url = sourceforge_asset("bodhilinux", &format!("{release}/{file}"))?;
    let checksum = checksum_at(&format!("{base}/{file}.sha256"), &file, "sha256");
    Ok((url, ImageKind::Iso, checksum))
}

pub(super) fn bunsenlabs_asset() -> Result<(String, ImageKind, Option<String>)> {
    let base = "https://ddl.bunsenlabs.org/ddl";
    let sums = fetch_text(&format!("{base}/release.sha256.txt"))?;
    let file = first_token(&sums, |value| value.ends_with(".iso"))
        .ok_or_else(|| dynamic_url_error("bunsenlabs"))?;
    let checksum = checksum_from_text(&sums, &file, "sha256");
    Ok((format!("{base}/{file}"), ImageKind::Iso, checksum))
}

pub(super) fn cachyos_asset(edition: Option<&str>) -> Result<(String, ImageKind, Option<String>)> {
    let edition = edition.ok_or_else(|| Error::message("CachyOS requires an edition"))?;
    let page = fetch_text("https://cachyos.org/download/")?;
    let file = first_token(&page, |value| {
        value.ends_with(".iso") && value.contains(edition) && !value.ends_with(".sha256")
    })
    .ok_or_else(|| dynamic_url_error("cachyos"))?;
    let url = if file.starts_with("http") {
        file.clone()
    } else {
        format!("https://iso.cachyos.org/{file}")
    };
    let checksum = checksum_at(&format!("{url}.sha256"), &file, "sha256");
    Ok((url, ImageKind::Iso, checksum))
}

pub(super) fn chimeralinux_asset(
    release: &str,
    edition: Option<&str>,
) -> Result<(String, ImageKind, Option<String>)> {
    let edition = edition.ok_or_else(|| Error::message("Chimera Linux requires an edition"))?;
    let base = format!("https://repo.chimera-linux.org/live/{release}");
    let sums = fetch_text(&format!("{base}/sha256sums.txt"))?;
    let file = first_token(&sums, |value| {
        value.ends_with(".iso") && value.contains("x86_64-LIVE") && value.contains(edition)
    })
    .ok_or_else(|| dynamic_url_error("chimeralinux"))?;
    let checksum = checksum_from_text(&sums, &file, "sha256");
    Ok((format!("{base}/{file}"), ImageKind::Iso, checksum))
}

pub(super) fn crunchbang_asset(release: &str) -> Result<(String, ImageKind, Option<String>)> {
    let data = fetch_text("https://api.github.com/repos/CBPP/cbpp/releases")?;
    let values: Value = serde_json::from_str(&data)
        .map_err(|error| Error::message(format!("invalid CrunchBang++ release data: {error}")))?;
    let url = values
        .as_array()
        .into_iter()
        .flatten()
        .filter(|entry| {
            release == "latest" || entry.get("tag_name").and_then(Value::as_str) == Some(release)
        })
        .flat_map(|entry| entry.get("assets").and_then(Value::as_array))
        .flatten()
        .find_map(|asset| {
            let name = asset.get("name").and_then(Value::as_str)?;
            (name.contains("amd64") && name.ends_with(".iso"))
                .then(|| asset.get("browser_download_url").and_then(Value::as_str))
                .flatten()
        })
        .ok_or_else(|| dynamic_url_error("crunchbang++"))?;
    Ok((url.to_string(), ImageKind::Iso, None))
}

pub(super) fn android_asset(
    release: &str,
    edition: Option<&str>,
) -> Result<(String, ImageKind, Option<String>)> {
    let edition = edition.ok_or_else(|| Error::message("Android x86 requires x86 or x86_64"))?;
    let page = fetch_text("https://www.fosshub.com/Android-x86-old.html")?;
    let settings = page
        .split_once("var settings =")
        .and_then(|(_, value)| value.split_once(';').map(|(json, _)| json.trim()))
        .ok_or_else(|| dynamic_url_error("android"))?;
    let values: Value = serde_json::from_str(settings)
        .map_err(|error| Error::message(format!("invalid Android release data: {error}")))?;
    let prefix = format!("android-{edition}-{release}");
    let image = values
        .pointer("/pool/f")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find(|entry| {
            entry
                .get("n")
                .and_then(Value::as_str)
                .is_some_and(|name| name.starts_with(&prefix) && name.ends_with(".iso"))
        })
        .ok_or_else(|| dynamic_url_error("android"))?;
    let file = image
        .get("n")
        .and_then(Value::as_str)
        .ok_or_else(|| dynamic_url_error("android"))?;
    let checksum = image
        .pointer("/hash/sha256")
        .and_then(Value::as_str)
        .map(|hash| format!("sha256:{hash}"));
    let mirror = "https://mirrors.gigenet.com/OSDN/android-x86";
    let directories = fetch_text(mirror)?;
    for directory in directories
        .split(|character: char| !character.is_ascii_digit())
        .filter(|value| value.len() == 5)
    {
        let base = format!("{mirror}/{directory}");
        if fetch_text(&format!("{base}/")).is_ok_and(|listing| listing.contains(file)) {
            return Ok((format!("{base}/{file}"), ImageKind::Iso, checksum));
        }
    }
    Err(dynamic_url_error("android"))
}

pub(super) fn elementary_asset(release: &str) -> Result<(String, ImageKind, Option<String>)> {
    let stamp = match release {
        "7.0" => ".20230129rc",
        "7.1" => ".20230926rc",
        "8.0" => ".20241122rc",
        "8.1" => "-amd64.20251211",
        _ => return Err(dynamic_url_error("elementary")),
    };
    let file = format!("elementaryos-{release}-stable{stamp}.iso");
    let token = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_secs().to_string())
        .unwrap_or_else(|_| "download".to_string());
    let checksum = (release == "8.1").then(|| {
        "sha256:eee6cad081664717681bec767fbfe1aa1fd920938fedad6c83b41fd341e8f306".to_string()
    });
    Ok((
        format!("https://ams3.dl.elementary.io/download/{token}/{file}"),
        ImageKind::Iso,
        checksum,
    ))
}

pub(super) fn siduction_asset(
    edition: Option<&str>,
) -> Result<(String, ImageKind, Option<String>)> {
    let edition = edition.ok_or_else(|| Error::message("siduction requires an edition"))?;
    let root = fetch_text("https://mirror.math.princeton.edu/pub/siduction/iso/")?;
    let release = first_token(&root, |value| {
        value.starts_with(|character: char| character.is_ascii_digit()) && value.ends_with('/')
    })
    .ok_or_else(|| dynamic_url_error("siduction"))?
    .trim_end_matches('/')
    .to_string();
    let base = format!("https://mirrors.dotsrc.org/siduction/iso/{release}/{edition}");
    let page = fetch_text(&base)?;
    let file = first_token(&page, |value| value.ends_with(".iso"))
        .ok_or_else(|| dynamic_url_error("siduction"))?;
    let checksum = checksum_at(&format!("{base}/{file}.md5"), &file, "sha256");
    Ok((format!("{base}/{file}"), ImageKind::Iso, checksum))
}

#[derive(Debug, Clone)]
pub(super) struct MacosRecovery {
    pub(super) url: String,
    pub(super) asset_token: String,
    pub(super) chunklist_url: String,
    pub(super) chunklist_token: String,
}
