use super::*;

pub(super) fn fetch_text(url: &str) -> Result<String> {
    let output = Command::new("curl")
        .args([
            "--disable",
            "--fail",
            "--location",
            "--silent",
            "--show-error",
            "--max-filesize",
            "8M",
            "--connect-timeout",
            "30",
            "--max-time",
            "60",
            "--user-agent",
            "Mozilla/5.0 (X11; Linux x86_64; rv:100.0) Gecko/20100101 Firefox/100.0",
            "--header",
            "Accept:",
            "--",
        ])
        .arg(url)
        .output()
        .map_err(|error| Error::command_unavailable("curl", error))?;
    if !output.status.success() {
        return Err(Error::command_failed_status("curl", output.status));
    }
    String::from_utf8(output.stdout)
        .map_err(|error| Error::message(format!("invalid UTF-8 from {url}: {error}")))
}

pub(super) fn requested_architectures(args: &GetArgs, os: &str) -> Result<Vec<String>> {
    if let Some(arch) = args.arch.as_deref() {
        return Ok(vec![normalize_architecture(arch)?.to_string()]);
    }
    let info = find_os(os)?;
    let host = normalize_architecture(host_architecture())?;
    if info
        .architectures
        .split_whitespace()
        .any(|arch| arch == host)
    {
        Ok(vec![host.to_string()])
    } else {
        Err(Error::message(format!(
            "{} is not available on this host architecture",
            info.name
        )))
    }
}

pub(super) fn download_file(url: &str, destination: &Path, insecure: bool) -> Result<()> {
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
    let status = command
        .arg(destination)
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

pub(super) fn verify_checksum(path: &Path, expected: Option<&str>) -> Result<()> {
    let Some(expected) = expected else {
        return Ok(());
    };
    let (algorithm, expected) = expected.split_once(':').unwrap_or(("sha256", expected));
    let expected = expected.to_ascii_lowercase();
    let (command, arguments): (&str, &[&str]) = match algorithm.to_ascii_lowercase().as_str() {
        "sha256" => {
            if command_exists("sha256sum") {
                ("sha256sum", &[])
            } else if command_exists("shasum") {
                ("shasum", &["-a", "256"])
            } else {
                return Err(Error::message(
                    "cannot verify the downloaded image: sha256sum or shasum is required",
                ));
            }
        }
        "sha512" => {
            if command_exists("sha512sum") {
                ("sha512sum", &[])
            } else if command_exists("shasum") {
                ("shasum", &["-a", "512"])
            } else {
                return Err(Error::message(
                    "cannot verify the downloaded image: sha512sum or shasum is required",
                ));
            }
        }
        other => {
            return Err(Error::message(format!(
                "unsupported checksum algorithm '{other}'"
            )));
        }
    };
    let output = Command::new(command)
        .args(arguments)
        .arg(path)
        .output()
        .map_err(|error| Error::command_unavailable(command, error))?;
    if !output.status.success() {
        return Err(Error::command_failed_status(command, output.status));
    }
    let actual = String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    if actual == expected {
        Ok(())
    } else {
        Err(Error::message(format!(
            "checksum mismatch for {} (expected {expected}, got {actual})",
            path.display()
        )))
    }
}

pub(super) fn command_exists(command: &str) -> bool {
    Command::new(command)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

pub(super) fn curl_security_args(insecure: bool) -> &'static [&'static str] {
    if insecure { &["--insecure"] } else { &[] }
}

pub(super) fn url_available(url: &str, insecure: bool) -> Result<bool> {
    url_available_with_headers(url, &[], insecure)
}

pub(super) fn url_available_with_headers(
    url: &str,
    headers: &[String],
    insecure: bool,
) -> Result<bool> {
    let mut command = Command::new("curl");
    command.args([
        "--disable",
        "--silent",
        "--show-error",
        "--head",
        "--fail",
        "--location",
        "--connect-timeout",
        "30",
        "--max-time",
        "30",
    ]);
    command.args(curl_security_args(insecure));
    for header in headers {
        command.args(["--header", header]);
    }
    let status = command
        .args(["--", url])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|error| Error::command_unavailable("curl", error))?;
    Ok(status.success())
}

pub(super) fn prepare_image(path: &Path) -> Result<PathBuf> {
    if fs::symlink_metadata(path)
        .ok()
        .is_some_and(|metadata| metadata.file_type().is_symlink())
    {
        return Err(Error::message(format!(
            "refusing to process symlink {}",
            path.display()
        )));
    }
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if !matches!(extension.as_str(), "zip" | "7z" | "gz" | "bz2") {
        return Ok(path.to_path_buf());
    }
    let parent = path
        .parent()
        .ok_or_else(|| Error::message("archive has no parent directory"))?;
    if extension == "gz" || extension == "bz2" {
        let output = path.with_extension("");
        if fs::symlink_metadata(&output)
            .ok()
            .is_some_and(|metadata| metadata.file_type().is_symlink())
        {
            return Err(Error::message(format!(
                "refusing to decompress through symlink {}",
                output.display()
            )));
        }
        let command = if extension == "gz" { "gzip" } else { "bzip2" };
        let status = Command::new(command)
            .args(["-d", "-f"])
            .arg(path)
            .status()
            .map_err(|error| Error::command_unavailable(command, error))?;
        if !status.success() {
            return Err(Error::command_failed_status(command, status));
        }
        return Ok(output);
    }
    let extract_dir = extraction_directory(parent)?;
    let result = extract_archive(path, extract_dir.as_path(), &extension);
    let _ = fs::remove_dir_all(&extract_dir);
    result
}

pub(super) fn extraction_directory(parent: &Path) -> Result<PathBuf> {
    for attempt in 0..100u32 {
        let candidate = parent.join(format!(".vmctl-extract-{}-{attempt}", std::process::id()));
        match fs::create_dir(&candidate) {
            Ok(()) => return Ok(candidate),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(Error::io(candidate.display(), error)),
        }
    }
    Err(Error::message(format!(
        "could not create a private extraction directory in {}",
        parent.display()
    )))
}

pub(super) fn extract_archive(path: &Path, extract_dir: &Path, extension: &str) -> Result<PathBuf> {
    let command = if extension == "zip" { "unzip" } else { "7z" };
    let status = if extension == "zip" {
        Command::new(command)
            .args(["-q", "-o", "-j"])
            .arg(path)
            .arg("-d")
            .arg(extract_dir)
            .status()
    } else {
        Command::new(command)
            .args(["e", "-y"])
            .arg(format!("-o{}", extract_dir.display()))
            .arg(path)
            .status()
    }
    .map_err(|error| Error::command_unavailable(command, error))?;
    if !status.success() {
        return Err(Error::command_failed_status(command, status));
    }

    let mut candidates = fs::read_dir(extract_dir)
        .map_err(|error| Error::io(extract_dir.display(), error))?
        .filter_map(std::result::Result::ok)
        .map(|entry| entry.path())
        .filter(|candidate| {
            candidate
                .extension()
                .and_then(|value| value.to_str())
                .is_some_and(|value| matches!(value.to_ascii_lowercase().as_str(), "iso" | "img"))
                && fs::symlink_metadata(candidate)
                    .is_ok_and(|metadata| metadata.file_type().is_file())
        })
        .collect::<Vec<_>>();
    candidates.sort();
    let candidate = candidates.into_iter().next().ok_or_else(|| {
        Error::message(format!(
            "no ISO or IMG found after extracting {}",
            path.display()
        ))
    })?;
    let parent = path
        .parent()
        .ok_or_else(|| Error::message("archive has no parent directory"))?;
    let file_name = candidate
        .file_name()
        .ok_or_else(|| Error::message("archive entry has no file name"))?;
    let destination = parent.join(file_name);
    if let Ok(metadata) = fs::symlink_metadata(&destination) {
        if metadata.file_type().is_symlink() {
            return Err(Error::message(format!(
                "refusing to replace symlink {}",
                destination.display()
            )));
        }
        if metadata.is_file() {
            return Ok(destination);
        }
        return Err(Error::message(format!(
            "archive output is not a regular file: {}",
            destination.display()
        )));
    }
    fs::rename(&candidate, &destination)
        .map_err(|error| Error::io(destination.display(), error))?;
    Ok(destination)
}
