use super::*;

pub(super) fn macos_asset(
    release: &str,
    architecture: &str,
) -> Result<(String, ImageKind, Option<String>)> {
    if architecture != "amd64" {
        return Err(Error::message("macOS recovery is only available for amd64"));
    }
    let recovery = fetch_macos_recovery(release)?;
    Ok((recovery.url, ImageKind::Img, None))
}

pub(super) fn fetch_macos_recovery(release: &str) -> Result<MacosRecovery> {
    let (board_id, mlb, os_type) = match release {
        "mojave" => ("Mac-7BA5B2DFE22DDD8C", "00000000000KXPG00", "default"),
        "catalina" => ("Mac-00BE6ED71E35EB86", "00000000000000000", "default"),
        "big-sur" => ("Mac-2BD1B31983FE1663", "00000000000000000", "default"),
        "monterey" => ("Mac-B809C3757DA9BB8D", "00000000000000000", "latest"),
        "ventura" => ("Mac-4B682C642B45593E", "00000000000000000", "latest"),
        "sonoma" => ("Mac-827FAC58A8FDFA22", "00000000000000000", "default"),
        "sequoia" => ("Mac-7BA5B2D9E42DDD94", "00000000000000000", "default"),
        "tahoe" => ("Mac-CFF7D910A743CAAF", "00000000000000000", "latest"),
        _ => {
            return Err(Error::message(format!(
                "unsupported macOS release '{release}'"
            )));
        }
    };
    let session = apple_session()?;
    let body = format!(
        "cid={}\nsn={mlb}\nbid={board_id}\nk={}\nfg={}\nos={os_type}",
        random_hex(16),
        random_hex(64),
        random_hex(64)
    );
    let info = curl_request(
        "https://osrecovery.apple.com/InstallationPayload/RecoveryImage",
        &[
            "Host: osrecovery.apple.com",
            "Connection: close",
            "User-Agent: InternetRecovery/1.0",
            "Content-Type: text/plain",
        ],
        Some(&format!("session=\"{session}\"")),
        Some(&body),
    )?;
    let url = first_token(&info, |value| {
        value.contains("oscdn") && value.contains(".dmg")
    })
    .ok_or_else(|| Error::message("Apple did not return a macOS recovery image"))?;
    let chunklist_url = first_token(&info, |value| {
        value.contains("oscdn") && value.contains("chunklist")
    })
    .ok_or_else(|| Error::message("Apple did not return a recovery chunk list"))?;
    let asset_token = apple_asset_token(&info, "dmg")?;
    let chunklist_token = apple_asset_token(&info, "chunklist")?;
    Ok(MacosRecovery {
        url,
        asset_token,
        chunklist_url,
        chunklist_token,
    })
}

pub(super) fn apple_session() -> Result<String> {
    let output = Command::new("curl")
        .args([
            "--disable",
            "--silent",
            "--show-error",
            "--dump-header",
            "-",
            "--output",
            "/dev/null",
            "-H",
            "Host: osrecovery.apple.com",
            "-H",
            "Connection: close",
            "-A",
            "InternetRecovery/1.0",
            "--",
            "https://osrecovery.apple.com/",
        ])
        .output()
        .map_err(|error| Error::command_unavailable("curl", error))?;
    if !output.status.success() {
        return Err(Error::command_failed_status("curl", output.status));
    }
    let headers = String::from_utf8_lossy(&output.stdout);
    headers
        .split([';', '\n', '\r'])
        .find_map(|part| part.split_once("session=").map(|(_, value)| value))
        .map(|value| value.trim_matches('"').trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| Error::message("Apple did not return a recovery session"))
}

pub(super) fn apple_asset_token(info: &str, kind: &str) -> Result<String> {
    let token = info.lines().find_map(|line| {
        if !line.contains(kind) || !line.contains("expires=") {
            return None;
        }
        line.split_once("expires=").and_then(|(_, value)| {
            value
                .split_whitespace()
                .next()
                .map(|value| value.trim_matches(['"', '\'', ';']).to_string())
        })
    });
    token
        .filter(|value| !value.is_empty())
        .ok_or_else(|| Error::message(format!("Apple did not return a {kind} asset token")))
}

pub(super) fn random_hex(length: usize) -> String {
    let seed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_nanos() as u64)
        .unwrap_or_default()
        ^ u64::from(std::process::id());
    let mut state = seed | 1;
    let mut result = String::with_capacity(length);
    while result.len() < length {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        result.push_str(&format!("{state:016x}"));
    }
    result.truncate(length);
    result
}

pub(super) fn curl_request(
    url: &str,
    headers: &[&str],
    cookie: Option<&str>,
    body: Option<&str>,
) -> Result<String> {
    let mut command = Command::new("curl");
    command.args([
        "--disable",
        "--fail",
        "--location",
        "--silent",
        "--show-error",
    ]);
    for header in headers {
        command.args(["-H", header]);
    }
    if let Some(cookie) = cookie {
        command.args(["--cookie", cookie]);
    }
    if let Some(body) = body {
        command.args(["--request", "POST", "--data-raw", body]);
    }
    let output = command
        .arg("--")
        .arg(url)
        .output()
        .map_err(|error| Error::command_unavailable("curl", error))?;
    if !output.status.success() {
        return Err(Error::command_failed_status("curl", output.status));
    }
    String::from_utf8(output.stdout)
        .map_err(|error| Error::message(format!("invalid UTF-8 from {url}: {error}")))
}

pub(super) fn download_file_with_headers(
    url: &str,
    destination: &Path,
    headers: &[String],
    insecure: bool,
) -> Result<()> {
    if fs::symlink_metadata(destination)
        .ok()
        .is_some_and(|metadata| metadata.file_type().is_symlink())
    {
        return Err(Error::message(format!(
            "refusing to download through symlink {}",
            destination.display()
        )));
    }
    let parent = destination
        .parent()
        .ok_or_else(|| Error::message("download destination has no parent directory"))?;
    fs::create_dir_all(parent).map_err(|error| Error::io(parent.display(), error))?;
    let mut command = Command::new("curl");
    command.args([
        "--disable",
        "--fail",
        "--location",
        "--continue-at",
        "-",
        "--output",
    ]);
    command.args(curl_security_args(insecure));
    command.arg(destination);
    for header in headers {
        command.args(["--header", header]);
    }
    let status = command
        .arg("--")
        .arg(url)
        .status()
        .map_err(|error| Error::command_unavailable("curl", error))?;
    if status.success() {
        Ok(())
    } else {
        Err(Error::command_failed_status("curl", status))
    }
}

pub(super) fn download_macos(
    args: &GetArgs,
    dirs: &Dirs,
    release: &str,
    architecture: &str,
    create_config: bool,
    output: OutputFormat,
) -> Result<()> {
    if args.edition_or_language.is_some() {
        return Err(Error::message("macOS does not take an edition"));
    }
    if architecture != "amd64" {
        return Err(Error::message("macOS recovery is only available for amd64"));
    }
    let name = suggested_name("macos", release, None, architecture);
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
    if create_config && root.join(format!("{name}.conf")).exists() {
        return Err(Error::message(format!(
            "configuration already exists: {}",
            root.join(format!("{name}.conf")).display()
        )));
    }
    fs::create_dir_all(&target_dir).map_err(|error| Error::io(target_dir.display(), error))?;
    let recovery = fetch_macos_recovery(release)?;
    let recovery_dmg = target_dir.join("RecoveryImage.dmg");
    let recovery_img = target_dir.join("RecoveryImage.img");
    let dmg_headers = vec![
        "Host: oscdn.apple.com".to_string(),
        "Connection: close".to_string(),
        "User-Agent: InternetRecovery/1.0".to_string(),
        format!("Cookie: AssetToken={}", recovery.asset_token),
    ];
    let chunk_headers = vec![
        "Host: oscdn.apple.com".to_string(),
        "Connection: close".to_string(),
        "User-Agent: InternetRecovery/1.0".to_string(),
        format!("Cookie: AssetToken={}", recovery.chunklist_token),
    ];
    download_file_with_headers(&recovery.url, &recovery_dmg, &dmg_headers, args.insecure)?;
    download_file_with_headers(
        &recovery.chunklist_url,
        &target_dir.join("RecoveryImage.chunklist"),
        &chunk_headers,
        args.insecure,
    )?;
    if command_exists("chunkcheck") {
        let status = Command::new("chunkcheck")
            .arg(&target_dir)
            .status()
            .map_err(|error| Error::command_unavailable("chunkcheck", error))?;
        if !status.success() {
            eprintln!("vmctl: warning: Apple recovery chunk verification failed");
        }
    }
    if !recovery_img.exists() {
        let status = Command::new("qemu-img")
            .args([
                "convert",
                recovery_dmg.to_string_lossy().as_ref(),
                "-O",
                "raw",
                recovery_img.to_string_lossy().as_ref(),
            ])
            .status()
            .map_err(|error| Error::command_unavailable("qemu-img", error))?;
        if !status.success() {
            return Err(Error::command_failed_status("qemu-img convert", status));
        }
    }
    let _ = fs::remove_file(&recovery_dmg);
    let _ = fs::remove_file(target_dir.join("RecoveryImage.chunklist"));
    if create_config {
        let commit = "da4b23b5e92c5b939568700034367e8b7649fe90";
        for (file, url) in [
            (
                "OpenCore.qcow2",
                format!("https://github.com/kholia/OSX-KVM/raw/{commit}/OpenCore/OpenCore.qcow2"),
            ),
            (
                "OVMF_CODE.fd",
                format!("https://github.com/kholia/OSX-KVM/raw/{commit}/OVMF_CODE.fd"),
            ),
            (
                "OVMF_VARS-1920x1080.fd",
                format!("https://github.com/kholia/OSX-KVM/raw/{commit}/OVMF_VARS-1920x1080.fd"),
            ),
        ] {
            download_file(&url, &target_dir.join(file), args.insecure)?;
        }
    }
    let config_path = if create_config {
        Some(write_vm_config(
            &root,
            &name,
            "macos",
            release,
            None,
            architecture,
            &recovery_img,
        )?)
    } else {
        None
    };
    let result = json!({
        "os": "macos",
        "release": release,
        "architecture": architecture,
        "image": recovery_img,
        "config": config_path,
    });
    if output == OutputFormat::Json {
        println!(
            "{}",
            serde_json::to_string_pretty(&result).unwrap_or_default()
        );
    } else if let Some(config_path) = config_path {
        println!("Created {}", config_path.display());
    } else {
        println!("Downloaded {}", recovery_img.display());
    }
    Ok(())
}
