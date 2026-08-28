use super::*;

pub(super) const NULL_DEVICE: &str = if cfg!(windows) { "NUL" } else { "/dev/null" };

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
    download_file_with_headers(url, destination, &[], insecure)
}

pub(super) fn download_file_with_headers(
    url: &str,
    destination: &Path,
    headers: &[String],
    insecure: bool,
) -> Result<()> {
    let parent = destination
        .parent()
        .ok_or_else(|| Error::message("download destination has no parent directory"))?;
    fs::create_dir_all(parent).map_err(|error| Error::io(parent.display(), error))?;
    let (temporary, file) = stage_new_file(destination)?;
    let output = match file.try_clone() {
        Ok(file) => file,
        Err(error) => {
            let _ = fs::remove_file(&temporary);
            return Err(Error::io(temporary.display(), error));
        }
    };
    let mut command = Command::new("curl");
    command.args(["--disable", "--fail", "--location"]);
    command.args(curl_security_args(insecure));
    for header in headers {
        command.args(["--header", header]);
    }
    let status = command
        .arg("--")
        .arg(url)
        .stdout(Stdio::from(output))
        .status()
        .map_err(|error| Error::command_unavailable("curl", error));
    let downloaded = match status {
        Ok(status) if status.success() => file
            .sync_all()
            .map_err(|error| Error::io(temporary.display(), error)),
        Ok(status) => Err(Error::command_failed_status("curl", status)),
        Err(error) => Err(error),
    };
    drop(file);
    let result = downloaded.and_then(|()| commit_new_file(&temporary, destination));
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

pub(super) fn ensure_new_file(destination: &Path) -> Result<()> {
    match fs::symlink_metadata(destination) {
        Ok(_) => Err(Error::message(format!(
            "download destination already exists: {}",
            destination.display()
        ))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(Error::io(destination.display(), error)),
    }
}

pub(super) fn stage_new_file(destination: &Path) -> Result<(PathBuf, fs::File)> {
    ensure_new_file(destination)?;
    let file_name = destination
        .file_name()
        .ok_or_else(|| Error::message("download destination has no file name"))?
        .to_string_lossy();
    for attempt in 0..100u32 {
        let temporary = destination.with_file_name(format!(
            ".{file_name}.vmctl-{}-{attempt}.tmp",
            std::process::id()
        ));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
        {
            Ok(file) => return Ok((temporary, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(Error::io(temporary.display(), error)),
        }
    }
    Err(Error::message(format!(
        "could not create a private download file beside {}",
        destination.display()
    )))
}

pub(super) fn commit_new_file(temporary: &Path, destination: &Path) -> Result<()> {
    if let Err(error) = fs::hard_link(temporary, destination) {
        let _ = fs::remove_file(temporary);
        return Err(Error::io(destination.display(), error));
    }
    let _ = fs::remove_file(temporary);
    Ok(())
}

pub(super) fn verify_checksum(path: &Path, expected: Option<&str>) -> Result<()> {
    let Some(expected) = expected else {
        return Ok(());
    };
    let (algorithm, expected) = expected.split_once(':').unwrap_or(("sha256", expected));
    let expected = expected.to_ascii_lowercase();
    let actual = checksum_digest(path, algorithm)?;
    if actual == expected {
        Ok(())
    } else {
        Err(Error::message(format!(
            "checksum mismatch for {} (expected {expected}, got {actual})",
            path.display()
        )))
    }
}

pub(super) fn checksum_digest(path: &Path, algorithm: &str) -> Result<String> {
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
    Ok(String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase())
}

pub(super) use crate::util::command_available as command_exists;

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
    crate::util::ensure_not_symlink(path, "process")?;
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if !matches!(extension.as_str(), "zip" | "7z" | "gz" | "bz2" | "xz") {
        return Ok(path.to_path_buf());
    }
    let parent = path
        .parent()
        .ok_or_else(|| Error::message("archive has no parent directory"))?;
    if matches!(extension.as_str(), "gz" | "bz2" | "xz") {
        let output = path.with_extension("");
        crate::util::ensure_not_symlink(&output, "decompress through")?;
        let command = match extension.as_str() {
            "gz" => "gzip",
            "bz2" => "bzip2",
            "xz" => "xz",
            _ => unreachable!(),
        };
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
    crate::util::ensure_not_symlink(&destination, "replace")?;
    if let Ok(metadata) = fs::symlink_metadata(&destination) {
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
