use super::*;
use tempfile::tempdir;

#[test]
fn normalizes_image_architectures() {
    assert_eq!(normalize_architecture("x86_64").unwrap(), "amd64");
    assert_eq!(normalize_architecture("aarch64").unwrap(), "arm64");
    assert!(normalize_architecture("ppc64le").is_err());
}

#[test]
fn config_values_are_shell_safe() {
    assert_eq!(config_value(r#"a\b"c"#), r#"a\\b\"c"#);
}

#[test]
fn catalog_contains_supported_systems() {
    assert_eq!(find_os("ubuntu").unwrap().name, "Ubuntu");
    assert_eq!(
        find_os("windows-server").unwrap().guest_os,
        "windows-server"
    );
    assert_eq!(find_os("kdelinux").unwrap().releases, "latest");
    assert!(required_edition(find_os("kdeneon").unwrap(), None).is_err());
    assert!(find_os("not-an-os").is_err());
}

#[test]
fn get_without_release_shows_os_options() {
    let args = GetArgs {
        os: Some("freebsd".to_string()),
        ..GetArgs::default()
    };
    assert_eq!(select_operation(&args).unwrap(), Operation::Show);

    let args = GetArgs {
        os: Some("freebsd".to_string()),
        release_or_input: Some("15.1".to_string()),
        ..GetArgs::default()
    };
    assert_eq!(select_operation(&args).unwrap(), Operation::CreateVm);
}

#[test]
fn cloud_get_selects_a_download_without_requiring_a_key() {
    let args = GetArgs {
        cloud: true,
        os: Some("ubuntu".to_string()),
        release_or_input: Some("24.04".to_string()),
        ..GetArgs::default()
    };
    assert_eq!(select_operation(&args).unwrap(), Operation::CreateCloudVm);
    assert!(validate_operation_arguments(&args, Operation::CreateCloudVm, false).is_ok());
}

#[test]
fn manifest_keyring_is_limited_to_cloud_downloads() {
    let args = GetArgs {
        cloud: true,
        os: Some("ubuntu".to_string()),
        release_or_input: Some("24.04".to_string()),
        manifest_keyring: Some(PathBuf::from("ubuntu.gpg")),
        ..GetArgs::default()
    };
    assert!(validate_operation_arguments(&args, Operation::CreateCloudVm, false).is_ok());
    assert!(validate_operation_arguments(&args, Operation::CreateVm, false).is_err());
}

#[test]
fn create_requires_a_cached_image_name() {
    assert!(cached_source(tempdir().unwrap().path(), "../image.iso").is_err());
}

#[test]
fn freebsd_cloud_get_selects_a_cloud_vm() {
    let args = GetArgs {
        cloud: true,
        os: Some("freebsd".to_string()),
        release_or_input: Some("15.1".to_string()),
        ..GetArgs::default()
    };
    assert_eq!(select_operation(&args).unwrap(), Operation::CreateCloudVm);
    assert!(validate_operation_arguments(&args, Operation::CreateCloudVm, false).is_ok());
}

#[test]
fn parses_freebsd_release_directories() {
    let listing = r#"
        <a href="../">Parent directory</a>
        <a HREF = '14.4/'>14.4/</a>
        <a href=15.1/>15.1/</a>
        <a href="15.1/">15.1/</a>
        <a href="README.TXT">README.TXT</a>
    "#;
    assert_eq!(
        freebsd_releases_from_listing(listing),
        vec!["14.4".to_string(), "15.1".to_string()]
    );
    assert!(freebsd_release_is_available(
        "15.1",
        "FreeBSD-15.1-RELEASE-amd64-disc1.iso FreeBSD-15.1-RELEASE-amd64-dvd1.iso"
    ));
    assert!(!freebsd_release_is_available(
        "14.5",
        "FreeBSD-14.5-BETA2-amd64-disc1.iso FreeBSD-14.5-BETA2-amd64-dvd1.iso"
    ));
}

#[test]
fn recognizes_kde_linux_release_images() {
    assert!(is_kde_linux_iso("kde-linux_202608171234.iso"));
    assert!(!is_kde_linux_iso("kde-linux_latest.iso"));
    assert!(!is_kde_linux_iso("kde-linux_20260817123.iso"));
}

#[test]
fn validates_windows_download_handshake_tokens() {
    let response = r#"window.location='?w=ABC123&rticks=+456';"#;
    assert_eq!(
        windows_ov_df_value(response, "w", |character| character.is_ascii_hexdigit()),
        Some("ABC123".to_string())
    );
    assert_eq!(
        windows_ov_df_value(response, "rticks", |character| character.is_ascii_digit()),
        Some("456".to_string())
    );
    assert!(
        windows_ov_df_value("rticks=not-a-number", "rticks", |character| {
            character.is_ascii_digit()
        })
        .is_none()
    );
}

#[test]
fn insecure_curl_mode_is_explicit() {
    assert_eq!(curl_security_args(false), &[] as &[&str]);
    assert_eq!(curl_security_args(true), &["--insecure"]);
    assert_eq!(NULL_DEVICE, if cfg!(windows) { "NUL" } else { "/dev/null" });
}

#[test]
fn downloads_never_touch_existing_destinations() {
    let root = tempdir().unwrap();
    let destination = root.path().join("image.iso");
    fs::write(&destination, b"keep me").unwrap();

    let error = download_file("invalid://url", &destination, false).unwrap_err();
    assert!(error.to_string().contains("already exists"));
    let error = download_file_with_headers(
        "invalid://url",
        &destination,
        &["Authorization: secret".to_string()],
        false,
    )
    .unwrap_err();
    assert!(error.to_string().contains("already exists"));
    assert_eq!(fs::read(&destination).unwrap(), b"keep me");
}

#[test]
fn staged_download_cannot_replace_a_raced_destination() {
    let root = tempdir().unwrap();
    let destination = root.path().join("image.iso");
    let (temporary, mut file) = stage_new_file(&destination).unwrap();
    file.write_all(b"new").unwrap();
    drop(file);
    fs::write(&destination, b"existing").unwrap();

    assert!(commit_new_file(&temporary, &destination).is_err());
    assert_eq!(fs::read(destination).unwrap(), b"existing");
    assert!(!temporary.exists());
}

#[test]
fn macos_conversion_refuses_an_existing_recovery_image_before_network_access() {
    let root = tempdir().unwrap();
    let vm_dir = root.path().join("vms");
    let target = vm_dir.join("macos-test");
    fs::create_dir_all(&target).unwrap();
    let image = target.join("RecoveryImage.img");
    fs::write(&image, b"existing").unwrap();
    let args = GetArgs {
        name: Some("macos-test".to_string()),
        ..GetArgs::default()
    };
    let dirs = Dirs {
        vm_dir,
        state_root: root.path().join("state"),
    };

    let error =
        download_macos(&args, &dirs, "sequoia", "amd64", true, OutputFormat::Human).unwrap_err();
    assert!(error.to_string().contains("already exists"));
    assert_eq!(fs::read(image).unwrap(), b"existing");
}

#[test]
fn macos_recovery_conversion_publishes_a_new_file_without_replacing_one() {
    if !command_exists("qemu-img") {
        return;
    }
    let root = tempdir().unwrap();
    let source = root.path().join("RecoveryImage.raw");
    let destination = root.path().join("RecoveryImage.img");
    fs::write(&source, vec![0; 1024]).unwrap();

    convert_recovery_image(&source, &destination).unwrap();
    assert_eq!(fs::metadata(&destination).unwrap().len(), 1024);
    let error = convert_recovery_image(&source, &destination).unwrap_err();
    assert!(error.to_string().contains("already exists"));
    assert_eq!(fs::metadata(destination).unwrap().len(), 1024);
}

#[cfg(unix)]
#[test]
fn windows_iso_append_refuses_a_symlinked_config() {
    use std::os::unix::fs::symlink;

    let root = tempdir().unwrap();
    let victim = root.path().join("victim.conf");
    let config = root.path().join("vm.conf");
    fs::write(&victim, b"original\n").unwrap();
    symlink(&victim, &config).unwrap();

    assert!(append_iso(root.path(), &config, "fixed_iso", Path::new("fixed.iso")).is_err());
    assert_eq!(fs::read(victim).unwrap(), b"original\n");
}

#[test]
fn homepage_launcher_uses_a_real_windows_executable() {
    let (command, arguments) = homepage_opener();
    assert!(!command.is_empty());
    if cfg!(windows) {
        assert_eq!(arguments, ["/C", "start", ""]);
    }
}

#[test]
fn rejects_get_flags_that_do_not_apply_to_the_selected_operation() {
    let mut args = GetArgs {
        arch: Some("amd64".to_string()),
        ..GetArgs::default()
    };
    let error = validate_operation_arguments(&args, Operation::Show, false).unwrap_err();
    assert_eq!(error.code(), "invalid_argument");

    args.arch = None;
    assert!(validate_operation_arguments(&args, Operation::Show, true).is_err());
    args.insecure = true;
    assert!(
        validate_operation_arguments(
            &args,
            Operation::Check {
                all_architectures: false
            },
            true
        )
        .is_ok()
    );
}

#[test]
fn generated_configs_get_current_arch_and_debian_defaults() {
    assert_eq!(
        config_tweaks("archlinux", "latest"),
        vec![("secureboot", "on"), ("tpm", "on"), ("disk_size", "32G")]
    );
    assert_eq!(
        config_tweaks("debian", "12"),
        vec![("secureboot", "on"), ("tpm", "on")]
    );
    assert_eq!(config_tweaks("debian", "11"), vec![("secureboot", "on")]);
}

#[test]
fn create_resources_reject_relative_sizes_and_zero_cpus() {
    let mut args = CreateArgs {
        name: "test".to_string(),
        image: "test.iso".to_string(),
        ram: None,
        cpu_cores: Some(1),
        disk_size: Some("+4G".to_string()),
        disk_mode: None,
        ssh_keys: Vec::new(),
        hostname: None,
        network_config: None,
        user_data: None,
    };
    assert!(VmResources::from_create(&args).is_err());
    args.disk_size = None;
    args.cpu_cores = Some(0);
    assert!(VmResources::from_create(&args).is_err());
}

#[test]
fn windows_and_legacy_guest_defaults_are_explicit() {
    assert_eq!(
        required_edition(find_os("windows").unwrap(), None).unwrap(),
        Some("English International".to_string())
    );
    assert_eq!(guest_os("ubuntu", "14.04"), "linux_old");
    assert_eq!(guest_os("ubuntu", "24.04"), "linux");
    assert!(WINDOWS_UNATTENDED_XML.contains("<unattend"));
}

#[test]
fn stable_url_templates_are_pure() {
    let image = resolve_image("ubuntu", "24.04", None, "amd64").unwrap();
    assert_eq!(image.file_name, "ubuntu-24.04-desktop-amd64.iso");
    assert!(image.url.starts_with("https://"));
    let freebsd = resolve_image("freebsd", "15.1", Some("disc1"), "amd64").unwrap();
    assert!(freebsd.url.starts_with(FREEBSD_ISO_IMAGES));
    assert_eq!(
        resolve_image("Ubuntu", "24.04", None, "amd64").unwrap().os,
        "ubuntu"
    );
    assert!(!ubuntu_arm64_release("24.04"));
    assert!(ubuntu_arm64_release("25.10"));
    assert!(resolve_image("debian", "12", Some("standard"), "arm64").is_err());
}

#[test]
fn parses_provider_checksums_without_shell_parsing() {
    let hash = "a".repeat(128);
    let sums = format!("{hash}  image.iso\n");
    assert_eq!(
        checksum_from_text(&sums, "image.iso", "sha512"),
        Some(format!("sha512:{hash}"))
    );
    assert_eq!(
        first_token("href=\"https://example.test/image.iso\"", |value| {
            value.ends_with(".iso")
        }),
        Some("https://example.test/image.iso".to_string())
    );
}

#[test]
fn unsafe_custom_names_are_rejected() {
    assert!(validate_vm_name("../vm").is_err());
    assert!(validate_vm_name("unusable vm").is_err());
    assert!(validate_vm_name("good-vm").is_ok());
}

#[test]
fn generated_config_is_relative_and_not_overwritten() {
    let root = tempdir().unwrap();
    let image_dir = root.path().join("ubuntu-24.04");
    fs::create_dir_all(&image_dir).unwrap();
    let image = image_dir.join("ubuntu.iso");
    fs::write(&image, b"test").unwrap();

    let config = write_vm_config(
        root.path(),
        "ubuntu-24.04",
        "ubuntu",
        "24.04",
        None,
        "amd64",
        &image,
        VmResources::default(),
    )
    .unwrap();
    let contents = fs::read_to_string(&config).unwrap();
    assert!(contents.contains("iso=\"ubuntu-24.04/ubuntu.iso\""));
    assert!(contents.contains("disk_img=\"ubuntu-24.04/disk.qcow2\""));
    let resources = VmResources {
        ram: Some("4G"),
        cpu_cores: Some(2),
        disk_size: Some("32G"),
    };
    let config = write_vm_config(
        root.path(),
        "resources",
        "ubuntu",
        "24.04",
        None,
        "amd64",
        &image,
        resources,
    )
    .unwrap();
    let contents = fs::read_to_string(config).unwrap();
    assert!(contents.contains("disk_size=\"32G\""));
    assert!(contents.contains("ram=\"4G\""));
    assert!(contents.contains("cpu_cores=\"2\""));
    assert!(
        write_vm_config(
            root.path(),
            "ubuntu-24.04",
            "ubuntu",
            "24.04",
            None,
            "amd64",
            &image,
            VmResources::default(),
        )
        .is_err()
    );
}

#[test]
fn explicit_resources_override_os_tweaks() {
    let root = tempdir().unwrap();
    let image = root.path().join("archlinux.iso");
    fs::write(&image, b"test").unwrap();
    let config = write_vm_config(
        root.path(),
        "archlinux",
        "archlinux",
        "latest",
        None,
        "amd64",
        &image,
        VmResources {
            ram: Some("8G"),
            cpu_cores: Some(4),
            disk_size: Some("64G"),
        },
    )
    .unwrap();
    let vm = crate::config::load_vm(root.path(), root.path(), config).unwrap();
    assert_eq!(vm.config.ram.as_deref(), Some("8G"));
    assert_eq!(vm.config.cpu_cores, Some(4));
    assert_eq!(vm.config.disk_size, "64G");
}

#[test]
fn custom_config_copies_a_local_image_without_sourcing_it() {
    let root = tempdir().unwrap();
    let source = root.path().join("installer.iso");
    fs::write(&source, b"not a shell script").unwrap();
    let vm_dir = root.path().join("vms");
    let dirs = Dirs {
        vm_dir: vm_dir.clone(),
        state_root: root.path().join("state"),
    };
    let args = GetArgs {
        os: Some("demo".to_string()),
        release_or_input: Some(source.display().to_string()),
        ..GetArgs::default()
    };

    create_custom_config(&args, &dirs, OutputFormat::Json).unwrap();
    let config = vm_dir.join("demo.conf");
    assert!(config.is_file());
    assert!(vm_dir.join("demo/installer.iso").is_file());
    assert!(
        fs::read_to_string(config)
            .unwrap()
            .contains("guest_os=\"linux\"")
    );
}

#[test]
fn failed_custom_config_can_be_retried() {
    let root = tempdir().unwrap();
    let vm_dir = root.path().join("vms");
    let dirs = Dirs {
        vm_dir: vm_dir.clone(),
        state_root: root.path().join("state"),
    };
    let mut args = GetArgs {
        os: Some("demo".to_string()),
        release_or_input: Some(root.path().join("missing.iso").display().to_string()),
        ..GetArgs::default()
    };
    assert!(create_custom_config(&args, &dirs, OutputFormat::Human).is_err());
    assert!(!vm_dir.join("demo").exists());

    let source = root.path().join("installer.iso");
    fs::write(&source, b"installer").unwrap();
    args.release_or_input = Some(source.display().to_string());
    create_custom_config(&args, &dirs, OutputFormat::Human).unwrap();
    assert!(vm_dir.join("demo.conf").is_file());
}

#[test]
fn cached_xz_archive_is_unpacked_when_the_vm_is_created() {
    if !command_exists("xz") {
        return;
    }
    let root = tempdir().unwrap();
    let source = root.path().join("installer.iso");
    fs::write(&source, b"installer").unwrap();
    assert!(
        Command::new("xz")
            .args(["-k", "-f"])
            .arg(&source)
            .status()
            .unwrap()
            .success()
    );
    let archive = source.with_extension("iso.xz");
    let vm_dir = root.path().join("vms");
    fs::create_dir(&vm_dir).unwrap();
    let cached = cache_image(
        &vm_dir,
        &ResolvedImage {
            os: "ubuntu".to_string(),
            release: "24.04".to_string(),
            edition: None,
            architecture: "amd64".to_string(),
            url: format!("file://{}", archive.display()),
            file_name: "installer.iso.xz".to_string(),
            kind: ImageKind::Archive,
            checksum: None,
        },
        false,
        false,
        false,
        None,
    )
    .unwrap();
    assert_eq!(
        cached.path.extension().and_then(|value| value.to_str()),
        Some("xz")
    );
    let dirs = Dirs {
        vm_dir: vm_dir.clone(),
        state_root: root.path().join("state"),
    };
    create_cached_vm(
        &CreateArgs {
            name: "archive".to_string(),
            image: cached
                .path
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
            ram: None,
            cpu_cores: None,
            disk_size: None,
            disk_mode: None,
            ssh_keys: Vec::new(),
            hostname: None,
            network_config: None,
            user_data: None,
        },
        &dirs,
        OutputFormat::Human,
    )
    .unwrap();
    let vm = crate::config::load_vm(
        &vm_dir,
        root.path().join("state").as_path(),
        vm_dir.join("archive.conf"),
    )
    .unwrap();
    assert_eq!(fs::read(vm.config.iso.unwrap()).unwrap(), b"installer");
}

#[test]
fn checksum_verification_rejects_tampering() {
    let root = tempdir().unwrap();
    let image = root.path().join("image.iso");
    fs::write(&image, b"test").unwrap();
    assert!(
        verify_checksum(
            &image,
            Some("9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08")
        )
        .is_ok()
    );
    assert!(verify_checksum(&image, Some("deadbeef")).is_err());
}
