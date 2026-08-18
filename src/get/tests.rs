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
    )
    .unwrap();
    let contents = fs::read_to_string(&config).unwrap();
    assert!(contents.contains("iso=\"ubuntu-24.04/ubuntu.iso\""));
    assert!(contents.contains("disk_img=\"ubuntu-24.04/disk.qcow2\""));
    assert!(
        write_vm_config(
            root.path(),
            "ubuntu-24.04",
            "ubuntu",
            "24.04",
            None,
            "amd64",
            &image,
        )
        .is_err()
    );
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
