use super::*;

#[derive(Debug, Clone)]
pub(super) struct CloudImage {
    pub(super) image: ResolvedImage,
    pub(super) ssh_user: &'static str,
}

pub(super) fn resolve_requested_image(
    cloud: bool,
    os: &str,
    release: &str,
    edition: Option<&str>,
    architecture: &str,
) -> Result<ResolvedImage> {
    if cloud {
        resolve_cloud_image(os, release, architecture).map(|image| image.image)
    } else {
        resolve_remote_image(os, release, edition, architecture)
    }
}

pub(super) fn resolve_cloud_image(
    os: &str,
    release: &str,
    architecture: &str,
) -> Result<CloudImage> {
    let os = find_os(os)?.id;
    let architecture = normalize_architecture(architecture)?;
    let (url, file_name, checksum, ssh_user) = match os {
        "ubuntu" => ubuntu_cloud(release, architecture)?,
        "debian" => debian_cloud(release, architecture)?,
        "fedora" => fedora_cloud(release, architecture)?,
        "freebsd" => freebsd_cloud(release, architecture)?,
        _ => {
            return Err(Error::message(format!(
                "cloud images are currently available for Ubuntu, Debian, Fedora, and FreeBSD (not {os})"
            )));
        }
    };
    Ok(CloudImage {
        image: ResolvedImage {
            os: os.to_string(),
            release: release.to_string(),
            edition: None,
            architecture: architecture.to_string(),
            url,
            file_name,
            kind: ImageKind::Disk,
            checksum: Some(checksum),
        },
        ssh_user,
    })
}

fn freebsd_cloud(
    release: &str,
    architecture: &str,
) -> Result<(String, String, String, &'static str)> {
    if architecture != "amd64" {
        return Err(Error::message(
            "FreeBSD cloud images are currently available for amd64 only",
        ));
    }
    let directory =
        format!("https://download.freebsd.org/releases/VM-IMAGES/{release}-RELEASE/amd64/Latest");
    let manifest = fetch_text(&format!("{directory}/CHECKSUM.SHA256"))?;
    freebsd_cloud_from_manifest(release, &directory, &manifest)
}

fn freebsd_cloud_from_manifest(
    release: &str,
    directory: &str,
    manifest: &str,
) -> Result<(String, String, String, &'static str)> {
    let file_name = format!("FreeBSD-{release}-RELEASE-amd64-BASIC-CLOUDINIT-zfs.qcow2.xz");
    let checksum = checksum_for(manifest, &file_name, "sha256")?;
    Ok((
        format!("{directory}/{file_name}"),
        file_name,
        checksum,
        "freebsd",
    ))
}

fn ubuntu_cloud(
    release: &str,
    architecture: &str,
) -> Result<(String, String, String, &'static str)> {
    let arch = if architecture == "amd64" {
        "amd64"
    } else {
        "arm64"
    };
    let directory = format!("https://cloud-images.ubuntu.com/releases/{release}/release");
    let file_name = format!("ubuntu-{release}-server-cloudimg-{arch}.img");
    let checksum = checksum_for(
        &fetch_text(&format!("{directory}/SHA256SUMS"))?,
        &file_name,
        "sha256",
    )?;
    Ok((
        format!("{directory}/{file_name}"),
        file_name,
        checksum,
        "ubuntu",
    ))
}

fn debian_cloud(
    release: &str,
    architecture: &str,
) -> Result<(String, String, String, &'static str)> {
    let release = debian_cloud_release(release);
    let directory = format!("https://cloud.debian.org/images/cloud/{release}/latest");
    let arch = if architecture == "amd64" {
        "amd64"
    } else {
        "arm64"
    };
    let manifest = fetch_text(&format!("{directory}/SHA512SUMS"))?;
    let file_name = manifest_asset(&manifest, "debian-", &format!("-generic-{arch}.qcow2"))?;
    let checksum = checksum_for(&manifest, &file_name, "sha512")?;
    Ok((
        format!("{directory}/{file_name}"),
        file_name,
        checksum,
        "debian",
    ))
}

fn debian_cloud_release(release: &str) -> &str {
    match release {
        "13" => "trixie",
        "12" => "bookworm",
        "11" => "bullseye",
        "10" => "buster",
        _ => release,
    }
}

fn fedora_cloud(
    release: &str,
    architecture: &str,
) -> Result<(String, String, String, &'static str)> {
    let arch = if architecture == "amd64" {
        "x86_64"
    } else {
        "aarch64"
    };
    let directory = format!(
        "https://download.fedoraproject.org/pub/fedora/linux/releases/{release}/Cloud/{arch}/images"
    );
    let listing = fetch_text(&format!("{directory}/"))?;
    let checksum_file = first_token(&listing, |value| {
        value.starts_with("Fedora-Cloud-") && value.ends_with("-CHECKSUM")
    })
    .ok_or_else(|| {
        Error::message(format!(
            "Fedora {release} cloud image checksum was not published"
        ))
    })?;
    let manifest = fetch_text(&format!("{directory}/{checksum_file}"))?;
    let file_name = manifest_asset(
        &manifest,
        "Fedora-Cloud-Base-Generic-",
        &format!(".{arch}.qcow2"),
    )?;
    let checksum = checksum_for(&manifest, &file_name, "sha256")?;
    Ok((
        format!("{directory}/{file_name}"),
        file_name,
        checksum,
        "fedora",
    ))
}

fn manifest_asset(manifest: &str, prefix: &str, suffix: &str) -> Result<String> {
    manifest
        .lines()
        .filter_map(|line| {
            line.strip_prefix("# ")
                .and_then(|line| line.split(':').next())
                .or_else(|| {
                    line.split_whitespace()
                        .nth(1)
                        .map(|name| name.trim_start_matches('*'))
                })
        })
        .find(|name| name.starts_with(prefix) && name.ends_with(suffix))
        .map(str::to_string)
        .ok_or_else(|| {
            Error::message(format!(
                "upstream checksum manifest has no {prefix}*{suffix} image"
            ))
        })
}

fn checksum_for(manifest: &str, file_name: &str, algorithm: &str) -> Result<String> {
    manifest
        .lines()
        .find_map(|line| {
            let mut fields = line.split_whitespace();
            let checksum = fields.next()?;
            let name = fields.next()?.trim_start_matches('*');
            (name == file_name).then(|| format!("{algorithm}:{checksum}"))
        })
        .or_else(|| {
            manifest.lines().find_map(|line| {
                let name = line.strip_prefix("SHA256 (")?.split_once(") =")?.0;
                let checksum = line.split_once("=")?.1.trim();
                (name == file_name).then(|| format!("{algorithm}:{checksum}"))
            })
        })
        .ok_or_else(|| {
            Error::message(format!(
                "upstream checksum manifest has no checksum for {file_name}"
            ))
        })
}

pub(super) fn download_cached_cloud_image(
    args: &GetArgs,
    dirs: &Dirs,
    output: OutputFormat,
) -> Result<()> {
    let os = find_os(required_arg(args.os.as_deref(), "OS")?).map(|info| info.id)?;
    let release = required_arg(args.release_or_input.as_deref(), "RELEASE")?;
    let architecture = requested_architectures(args, os)?
        .into_iter()
        .next()
        .ok_or_else(|| Error::message("an architecture is required"))?;
    let cloud = resolve_cloud_image(os, release, &architecture)?;
    fs::create_dir_all(&dirs.vm_dir).map_err(|error| Error::io(dirs.vm_dir.display(), error))?;
    let cached = cache_image(
        &dirs.vm_dir,
        &cloud.image,
        args.insecure,
        args.refresh_cache,
        true,
        Some(cloud.ssh_user),
    )?;
    let result = json!({
        "os": cloud.image.os,
        "release": cloud.image.release,
        "architecture": cloud.image.architecture,
        "url": cloud.image.url,
        "kind": "cloud",
        "image": cached.path,
        "cache": { "status": cached.status.as_str(), "object": cached.path, "sha256": cached.sha256 },
    });
    if output == OutputFormat::Json {
        crate::print_json_success(result);
    } else {
        println!(
            "{} {}",
            if cached.status == CacheStatus::Hit {
                "Using cached"
            } else {
                "Downloaded"
            },
            cached.path.display()
        );
        println!(
            "Create a VM with: vmctl create NAME --from {} --ssh-key PATH",
            cached
                .path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
        );
    }
    Ok(())
}

pub(super) fn create_cached_cloud_vm(
    args: &CreateArgs,
    dirs: &Dirs,
    source: CachedSource,
    output: OutputFormat,
) -> Result<()> {
    let resources = VmResources::from_create(args)?;
    if args.ssh_keys.is_empty() {
        return Err(Error::invalid_argument(
            "--ssh-key",
            "at least one OpenSSH public key is required for a cloud VM",
        ));
    }
    let name = validate_vm_name(&args.name)?;
    let hostname = args.hostname.as_deref().unwrap_or(name);
    validate_hostname(hostname)?;
    let root = &dirs.vm_dir;
    let config_path = root.join(format!("{name}.conf"));
    let target_dir = root.join(name);
    if config_path.exists() {
        return Err(Error::message(format!(
            "configuration already exists: {}",
            config_path.display()
        )));
    }
    let ssh_user = source.ssh_user.as_deref().ok_or_else(|| {
        Error::message("cached cloud image lacks an SSH user; run `vmctl get --refresh-cache`")
    })?;
    fs::create_dir_all(root).map_err(|error| Error::io(root.display(), error))?;
    fs::create_dir(&target_dir).map_err(|error| Error::io(target_dir.display(), error))?;
    let disk = target_dir.join("disk.qcow2");
    let disk_mode = args.disk_mode.unwrap_or(DiskMode::Linked);
    let provision = (|| {
        let seed = create_cloud_seed(
            &target_dir,
            &source.os,
            hostname,
            &args.ssh_keys,
            args.network_config.as_deref(),
        )?;
        if disk_mode == DiskMode::Linked {
            let backing = format!(
                "../.cache/objects/{}",
                source
                    .path
                    .file_name()
                    .and_then(|value| value.to_str())
                    .ok_or_else(|| {
                        Error::message("cached cloud image has no valid file name")
                    })?
            );
            crate::qemu::create_cloud_overlay(&source.path, &disk, &backing)?;
        } else {
            crate::qemu::create_cloud_copy(&source.path, &disk)?;
        }
        crate::qemu::disk_resize(&disk, resources.disk_size.unwrap_or("16G"), false)?;
        write_cloud_vm_config(
            root,
            name,
            CloudVmConfig {
                os: &source.os,
                release: &source.release,
                architecture: &source.architecture,
                base: (disk_mode == DiskMode::Linked).then_some(source.path.as_path()),
                disk: &disk,
                seed: &seed,
                ssh_user,
            },
            resources,
        )
    })();
    let config = match provision {
        Ok(config) => config,
        Err(error) => {
            let _ = fs::remove_dir_all(&target_dir);
            return Err(error);
        }
    };
    let result = json!({
        "name": name,
        "image": source.path,
        "disk": disk,
        "disk_mode": match disk_mode { DiskMode::Linked => "linked", DiskMode::Copy => "copy" },
        "ssh_user": ssh_user,
        "config": config,
    });
    if output == OutputFormat::Json {
        crate::print_json_success(result);
    } else {
        println!("Created {}", config.display());
        println!("Connect with: vmctl ssh {name}");
    }
    Ok(())
}

pub(super) fn create_cloud_seed(
    target_dir: &Path,
    os: &str,
    hostname: &str,
    keys: &[PathBuf],
    network_config: Option<&Path>,
) -> Result<PathBuf> {
    let seed = target_dir.join("seed.iso");
    if fs::symlink_metadata(&seed).is_ok() {
        return Err(Error::message(format!(
            "cloud-init seed already exists: {}",
            seed.display()
        )));
    }
    let staging = extraction_directory(target_dir)?;
    let result = (|| {
        let keys = read_public_keys(keys)?;
        fs::write(staging.join("user-data"), cloud_user_data(os, &keys))
            .map_err(|error| Error::io(staging.display(), error))?;
        let instance_id = format!(
            "vmctl-{}-{}",
            hostname,
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|error| Error::message(error.to_string()))?
                .as_nanos()
        );
        fs::write(
            staging.join("meta-data"),
            format!(
                "instance-id: {}\nlocal-hostname: {}\n",
                instance_id,
                serde_json::to_string(hostname).expect("string JSON")
            ),
        )
        .map_err(|error| Error::io(staging.display(), error))?;
        if let Some(path) = network_config {
            let metadata =
                fs::symlink_metadata(path).map_err(|error| Error::io(path.display(), error))?;
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err(Error::message(format!(
                    "network config is not a regular file: {}",
                    path.display()
                )));
            }
            fs::copy(path, staging.join("network-config"))
                .map_err(|error| Error::io(path.display(), error))?;
        }
        create_iso(&staging, &seed, Some("cidata"))
    })();
    let _ = fs::remove_dir_all(&staging);
    result?;
    Ok(seed)
}

fn cloud_user_data(os: &str, keys: &[String]) -> String {
    let keys = |indent| {
        keys.iter()
            .map(|key| {
                format!(
                    "{indent}- {}",
                    serde_json::to_string(key).expect("string JSON")
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    if os == "freebsd" {
        format!(
            "#cloud-config\nssh_pwauth: false\nusers:\n  - name: freebsd\n    groups: wheel\n    shell: /bin/sh\n    doas: permit nopass %u as root\n    ssh_authorized_keys:\n{}\npackages:\n  - doas\nchpasswd:\n  users:\n    - name: root\n      password: RANDOM\n    - name: freebsd\n      password: RANDOM\n",
            keys("      ")
        )
    } else {
        format!(
            "#cloud-config\nssh_pwauth: false\nusers:\n  - default\nssh_authorized_keys:\n{}\n",
            keys("  ")
        )
    }
}

fn read_public_keys(paths: &[PathBuf]) -> Result<Vec<String>> {
    paths
        .iter()
        .map(|path| {
            let metadata =
                fs::symlink_metadata(path).map_err(|error| Error::io(path.display(), error))?;
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err(Error::invalid_argument(
                    "--ssh-key",
                    format!("{} is not a regular public-key file", path.display()),
                ));
            }
            let value =
                fs::read_to_string(path).map_err(|error| Error::io(path.display(), error))?;
            let key = value
                .lines()
                .find(|line| !line.trim().is_empty())
                .unwrap_or_default()
                .trim();
            if !key.starts_with("ssh-") && !key.starts_with("ecdsa-") && !key.starts_with("sk-") {
                return Err(Error::invalid_argument(
                    "--ssh-key",
                    format!("{} does not contain an OpenSSH public key", path.display()),
                ));
            }
            Ok(key.to_string())
        })
        .collect()
}

fn validate_hostname(hostname: &str) -> Result<()> {
    if hostname.is_empty()
        || hostname.len() > 253
        || hostname.split('.').any(|label| {
            label.is_empty()
                || label.len() > 63
                || !label
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_alphanumeric())
                || !label
                    .chars()
                    .last()
                    .is_some_and(|c| c.is_ascii_alphanumeric())
                || label
                    .chars()
                    .any(|c| !(c.is_ascii_alphanumeric() || c == '-'))
        })
    {
        return Err(Error::invalid_argument(
            "--hostname",
            "use DNS labels containing letters, digits, and hyphens",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn reads_checksum_manifest_assets() {
        let manifest = "abc  debian-13-generic-amd64.qcow2\n";
        assert_eq!(
            manifest_asset(manifest, "debian-", "-generic-amd64.qcow2").unwrap(),
            "debian-13-generic-amd64.qcow2"
        );
        assert_eq!(
            checksum_for(manifest, "debian-13-generic-amd64.qcow2", "sha512").unwrap(),
            "sha512:abc"
        );
    }

    #[test]
    fn reads_fedora_checksum_format() {
        let manifest = "# Fedora-Cloud-Base-Generic-44-1.7.x86_64.qcow2: 1 bytes\nSHA256 (Fedora-Cloud-Base-Generic-44-1.7.x86_64.qcow2) = abc\n";
        let name = manifest_asset(manifest, "Fedora-Cloud-Base-Generic-", ".x86_64.qcow2").unwrap();
        assert_eq!(name, "Fedora-Cloud-Base-Generic-44-1.7.x86_64.qcow2");
        assert_eq!(
            checksum_for(manifest, &name, "sha256").unwrap(),
            "sha256:abc"
        );
    }

    #[test]
    fn hostname_validation_rejects_invalid_values() {
        assert!(validate_hostname("cloud-vm.example").is_ok());
        assert!(validate_hostname("bad host").is_err());
    }

    #[test]
    fn freebsd_cloud_user_data_enables_key_only_administration() {
        let user_data = cloud_user_data("freebsd", &["ssh-ed25519 example".to_string()]);
        assert!(user_data.contains("packages:\n  - doas"));
        assert!(user_data.contains("doas: permit nopass %u as root"));
        assert!(user_data.contains("password: RANDOM"));
        assert!(user_data.contains("      - \"ssh-ed25519 example\""));
        assert!(!user_data.contains("  - default"));
    }

    #[test]
    fn debian_versions_resolve_to_upstream_codenames() {
        assert_eq!(debian_cloud_release("13"), "trixie");
        assert_eq!(debian_cloud_release("trixie"), "trixie");
    }

    #[test]
    fn freebsd_cloud_image_uses_the_official_zfs_qcow2_layout() {
        let (url, file_name, checksum, user) = freebsd_cloud_from_manifest(
            "15.1",
            "https://example.invalid",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa  FreeBSD-15.1-RELEASE-amd64-BASIC-CLOUDINIT-zfs.qcow2.xz\n",
        )
        .unwrap();
        assert_eq!(user, "freebsd");
        assert!(url.ends_with(&file_name));
        assert!(file_name.ends_with("BASIC-CLOUDINIT-zfs.qcow2.xz"));
        assert_eq!(checksum, format!("sha256:{}", "a".repeat(64)));
    }

    #[test]
    fn cloud_create_resizes_the_default_disk_and_cleans_up_a_failed_attempt() {
        if !command_exists("qemu-img")
            || !["mkisofs", "genisoimage", "xorriso"]
                .into_iter()
                .any(command_exists)
        {
            return;
        }
        let root = tempdir().unwrap();
        let vm_dir = root.path().join("vms");
        let objects = vm_dir.join(".cache/objects");
        fs::create_dir_all(&objects).unwrap();
        let base = objects.join("base.qcow2");
        assert!(
            Command::new("qemu-img")
                .args(["create", "-q", "-f", "qcow2"])
                .arg(&base)
                .arg("8M")
                .status()
                .unwrap()
                .success()
        );
        let key = root.path().join("id.pub");
        fs::write(&key, "ssh-ed25519 test\n").unwrap();
        let dirs = Dirs {
            vm_dir: vm_dir.clone(),
            state_root: root.path().join("state"),
        };
        let source = CachedSource {
            path: base,
            os: "ubuntu".to_string(),
            release: "24.04".to_string(),
            edition: None,
            architecture: "amd64".to_string(),
            kind: ImageKind::Disk,
            cloud: true,
            ssh_user: Some("ubuntu".to_string()),
        };
        let mut args = CreateArgs {
            name: "cloud".to_string(),
            image: "base.qcow2".to_string(),
            ram: None,
            cpu_cores: None,
            disk_size: Some("1M".to_string()),
            disk_mode: None,
            ssh_keys: vec![key],
            hostname: None,
            network_config: None,
        };
        let error =
            create_cached_cloud_vm(&args, &dirs, source.clone(), OutputFormat::Human).unwrap_err();
        assert!(error.to_string().contains("qemu-img resize"));
        assert!(!vm_dir.join("cloud").exists());
        assert!(!vm_dir.join("cloud.conf").exists());

        args.disk_size = None;
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("missing.conf", vm_dir.join("cloud.conf")).unwrap();
            assert!(
                create_cached_cloud_vm(&args, &dirs, source.clone(), OutputFormat::Human).is_err()
            );
            assert!(!vm_dir.join("cloud").exists());
            assert!(
                fs::symlink_metadata(vm_dir.join("cloud.conf"))
                    .unwrap()
                    .file_type()
                    .is_symlink()
            );
            fs::remove_file(vm_dir.join("cloud.conf")).unwrap();
        }
        create_cached_cloud_vm(&args, &dirs, source, OutputFormat::Human).unwrap();
        let disk = crate::qemu::disk_info(&vm_dir.join("cloud/disk.qcow2")).unwrap();
        assert_eq!(disk["virtual-size"], 16 * 1024_u64.pow(3));
        let vm = crate::config::load_vm(
            &vm_dir,
            root.path().join("state").as_path(),
            vm_dir.join("cloud.conf"),
        )
        .unwrap();
        assert_eq!(vm.config.disk_size, "16G");
    }
}
