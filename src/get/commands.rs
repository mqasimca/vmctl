use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Operation {
    List,
    ListCsv,
    ListJson,
    Version,
    Show,
    Homepage,
    Url,
    Check { all_architectures: bool },
    Download,
    CreateConfig,
    CreateVm,
    CreateCloudVm,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ImageKind {
    Iso,
    Img,
    Disk,
    Archive,
}

#[derive(Debug, Clone)]
pub(super) struct ResolvedImage {
    pub(super) os: String,
    pub(super) release: String,
    pub(super) edition: Option<String>,
    pub(super) architecture: String,
    pub(super) url: String,
    pub(super) file_name: String,
    pub(super) kind: ImageKind,
    pub(super) checksum: Option<String>,
}

pub fn run(args: &GetArgs, dirs: &Dirs, output: OutputFormat) -> Result<()> {
    let mut args = args.clone();
    let insecure_flag = args.insecure;
    args.insecure |= env::var("VMCTL_INSECURE").is_ok_and(|value| value == "1");
    let operation = select_operation(&args)?;
    validate_operation_arguments(&args, operation, insecure_flag)?;
    if args.insecure
        && output != OutputFormat::Json
        && matches!(
            operation,
            Operation::Check { .. }
                | Operation::Download
                | Operation::CreateConfig
                | Operation::CreateVm
                | Operation::CreateCloudVm
        )
    {
        eprintln!(
            "vmctl: warning: --insecure disables TLS certificate verification for this get operation"
        );
    }
    match operation {
        Operation::List => list_human(&args, output),
        Operation::ListCsv => list_csv(output),
        Operation::ListJson => list_json(),
        Operation::Version => print_version(output),
        Operation::Show => show(&args, output),
        Operation::Homepage => open_homepage(&args, output),
        Operation::Url => print_images(&args, output),
        Operation::Check { all_architectures } => check_images(&args, all_architectures, output),
        Operation::Download => download_image(&args, dirs, false, output),
        Operation::CreateConfig => create_custom_config(&args, dirs, output),
        Operation::CreateVm => download_cached_image(&args, dirs, output),
        Operation::CreateCloudVm => download_cached_cloud_image(&args, dirs, output),
    }
}

pub(super) fn validate_operation_arguments(
    args: &GetArgs,
    operation: Operation,
    insecure_flag: bool,
) -> Result<()> {
    if args.arch.is_some()
        && !matches!(
            operation,
            Operation::Url
                | Operation::Check { .. }
                | Operation::Download
                | Operation::CreateVm
                | Operation::CreateCloudVm
        )
    {
        return Err(Error::invalid_argument(
            "--arch",
            "only URL, check, download, and VM creation operations accept it",
        ));
    }
    if insecure_flag
        && !matches!(
            operation,
            Operation::Check { .. }
                | Operation::Download
                | Operation::CreateConfig
                | Operation::CreateVm
                | Operation::CreateCloudVm
        )
    {
        return Err(Error::invalid_argument(
            "--insecure",
            "only network checks and image/config creation operations accept it",
        ));
    }
    if args.disable_unattended
        && !matches!(operation, Operation::CreateConfig | Operation::CreateVm)
    {
        return Err(Error::invalid_argument(
            "--disable-unattended",
            "only VM/config creation operations accept it",
        ));
    }
    if args.refresh_cache
        && !matches!(
            operation,
            Operation::CreateConfig | Operation::CreateVm | Operation::CreateCloudVm
        )
    {
        return Err(Error::invalid_argument(
            "--refresh-cache",
            "only VM creation operations accept it",
        ));
    }
    if args.manifest_keyring.is_some() && !matches!(operation, Operation::CreateCloudVm) {
        return Err(Error::invalid_argument(
            "--manifest-keyring",
            "only cloud-image downloads accept manifest signature verification",
        ));
    }
    if args.refresh_cache
        && args
            .os
            .as_deref()
            .is_some_and(|os| os.eq_ignore_ascii_case("macos"))
    {
        return Err(Error::invalid_argument(
            "--refresh-cache",
            "macOS provisioning manages its own Apple download workflow",
        ));
    }
    if args.cloud
        && !matches!(
            operation,
            Operation::Url | Operation::Check { .. } | Operation::CreateCloudVm
        )
    {
        return Err(Error::invalid_argument(
            "--cloud",
            "only cloud URL, check, and VM creation operations accept it",
        ));
    }
    if args.cloud && args.edition_or_language.is_some() {
        return Err(Error::invalid_argument(
            "EDITION_OR_LANGUAGE",
            "cloud images do not accept editions; use OS and RELEASE only",
        ));
    }
    if matches!(
        operation,
        Operation::ListCsv | Operation::ListJson | Operation::Version
    ) && (args.os.is_some()
        || args.release_or_input.is_some()
        || args.edition_or_language.is_some())
    {
        return Err(Error::message(format!(
            "get {} does not take positional arguments",
            match operation {
                Operation::ListCsv => "--list-csv",
                Operation::ListJson => "--list-json",
                Operation::Version => "--version",
                _ => unreachable!(),
            }
        )));
    }
    Ok(())
}

pub(crate) fn create(args: &CreateArgs, dirs: &Dirs, output: OutputFormat) -> Result<()> {
    create_cached_vm(args, dirs, output)
}

pub(super) fn select_operation(args: &GetArgs) -> Result<Operation> {
    let flags = [
        (args.list, Operation::List),
        (args.list_csv, Operation::ListCsv),
        (args.list_json, Operation::ListJson),
        (args.version, Operation::Version),
        (args.show, Operation::Show),
        (args.open_homepage, Operation::Homepage),
        (args.url, Operation::Url),
        (
            args.check || args.check_all_arch,
            Operation::Check {
                all_architectures: args.check_all_arch,
            },
        ),
        (args.download, Operation::Download),
        (args.create_config, Operation::CreateConfig),
    ];
    let mut selected = flags.iter().filter(|(set, _)| *set).map(|(_, op)| *op);
    let Some(operation) = selected.next() else {
        if args.release_or_input.is_none() && args.edition_or_language.is_none() {
            match args.os.as_deref() {
                Some("list") => return Ok(Operation::List),
                Some("list_csv") => return Ok(Operation::ListCsv),
                Some("list_json") => return Ok(Operation::ListJson),
                _ => {}
            }
        }
        return if args.os.is_some() {
            if args.release_or_input.is_none() && args.edition_or_language.is_none() {
                Ok(Operation::Show)
            } else {
                Ok(if args.cloud {
                    Operation::CreateCloudVm
                } else {
                    Operation::CreateVm
                })
            }
        } else {
            Ok(Operation::List)
        };
    };
    if selected.next().is_some() {
        return Err(Error::message("get accepts one operation flag at a time"));
    }
    Ok(operation)
}

pub(super) fn list_human(args: &GetArgs, output: OutputFormat) -> Result<()> {
    if args.os.is_some() || args.release_or_input.is_some() || args.edition_or_language.is_some() {
        return Err(Error::message("--list does not take positional arguments"));
    }
    if output == OutputFormat::Json {
        return list_json();
    }
    for info in OS_CATALOG {
        println!("{}", info.id);
    }
    Ok(())
}

pub(super) fn list_csv(output: OutputFormat) -> Result<()> {
    if output == OutputFormat::Json {
        return list_json();
    }
    println!("Display Name,OS,Release,Option,Homepage,Architecture");
    for info in OS_CATALOG {
        println!(
            "{},{},{},{},{},{}",
            csv_field(info.name),
            info.id,
            csv_field(info.releases),
            csv_field(info.editions),
            info.homepage,
            info.architectures.replace(' ', "|")
        );
    }
    Ok(())
}

pub(super) fn list_json() -> Result<()> {
    let values: Vec<Value> = OS_CATALOG.iter().map(info_json).collect();
    crate::print_json_success(json!(values));
    Ok(())
}

pub(super) fn print_version(output: OutputFormat) -> Result<()> {
    if output == OutputFormat::Json {
        crate::print_json_success(json!({"version": env!("CARGO_PKG_VERSION")}));
    } else {
        println!("{}", env!("CARGO_PKG_VERSION"));
    }
    Ok(())
}

pub(super) fn show(args: &GetArgs, output: OutputFormat) -> Result<()> {
    if args.release_or_input.is_some() || args.edition_or_language.is_some() {
        return Err(Error::message("--show accepts only an optional OS"));
    }
    let Some(os) = args.os.as_deref() else {
        return if output == OutputFormat::Json {
            list_json()
        } else {
            for info in OS_CATALOG {
                print_info(info, None);
            }
            Ok(())
        };
    };
    let info = find_os(os)?;
    let releases = (info.id == "freebsd").then(freebsd_releases).transpose()?;
    if output == OutputFormat::Json {
        let mut value = info_json(&info);
        if let Some(releases) = &releases {
            value["releases"] = json!(releases);
        }
        crate::print_json_success(value);
    } else {
        print_info(&info, releases.as_deref());
        if info.id == "freebsd" {
            println!("  use:           vmctl get freebsd <RELEASE> <disc1|dvd1>");
        }
    }
    Ok(())
}

pub(super) fn print_info(info: &OsInfo, releases: Option<&[String]>) {
    println!("{} ({})", info.name, info.id);
    println!("  homepage:      {}", info.homepage);
    println!("  guest OS:      {}", info.guest_os);
    println!("  architectures: {}", info.architectures.replace(' ', ", "));
    println!(
        "  releases:      {}",
        releases.map_or_else(|| info.releases.to_string(), |releases| releases.join(", "))
    );
    if !info.editions.is_empty() {
        println!("  editions:      {}", info.editions);
    }
}

pub(super) fn freebsd_releases() -> Result<Vec<String>> {
    let listing = fetch_text("https://download.freebsd.org/releases/amd64/amd64/ISO-IMAGES/")
        .map_err(|error| {
            Error::message(format!(
                "could not list current FreeBSD releases: {error}; retry later or specify a release, for example: vmctl get freebsd 15.1"
            ))
        })?;
    let mut releases = Vec::new();
    for release in freebsd_releases_from_listing(&listing) {
        let listing = fetch_text(&format!("{FREEBSD_ISO_IMAGES}{release}/")).map_err(|error| {
            Error::message(format!(
                "could not inspect FreeBSD {release} media: {error}; retry later or specify a release, for example: vmctl get freebsd 15.1 disc1"
            ))
        })?;
        if freebsd_release_is_available(&release, &listing) {
            releases.push(release);
        }
    }
    if releases.is_empty() {
        return Err(Error::message(
            "FreeBSD release listing contained no current RELEASE images; retry later or specify a release, for example: vmctl get freebsd 15.1",
        ));
    }
    Ok(releases)
}

pub(super) fn freebsd_releases_from_listing(listing: &str) -> Vec<String> {
    let listing = listing.to_ascii_lowercase();
    let mut releases = Vec::new();
    let mut offset = 0;
    while let Some(index) = listing[offset..].find("href") {
        let after = offset + index + "href".len();
        offset = after;
        let value = listing[after..].trim_start();
        let Some(value) = value.strip_prefix('=').map(str::trim_start) else {
            continue;
        };
        let value = match value.chars().next() {
            Some(quote @ ('\'' | '"')) => value[1..].split(quote).next().unwrap_or_default(),
            _ => value
                .split(|character: char| character.is_whitespace() || character == '>')
                .next()
                .unwrap_or_default(),
        };
        let Some(release) = value.strip_suffix('/') else {
            continue;
        };
        if release.split('.').all(|part| {
            !part.is_empty() && part.chars().all(|character| character.is_ascii_digit())
        }) && !releases.iter().any(|value| value == release)
        {
            releases.push(release.to_string());
        }
    }
    releases
}

pub(super) fn freebsd_release_is_available(release: &str, listing: &str) -> bool {
    ["disc1", "dvd1"]
        .iter()
        .all(|edition| listing.contains(&format!("FreeBSD-{release}-RELEASE-amd64-{edition}.iso")))
}

pub(super) fn open_homepage(args: &GetArgs, output: OutputFormat) -> Result<()> {
    let Some(os) = args.os.as_deref() else {
        return Err(Error::message("--open-homepage requires an OS"));
    };
    if args.release_or_input.is_some() || args.edition_or_language.is_some() {
        return Err(Error::message("--open-homepage accepts only an OS"));
    }
    let info = find_os(os)?;
    let (command, arguments) = homepage_opener();
    Command::new(command)
        .args(arguments)
        .arg(info.homepage)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| Error::command_unavailable(command, error))?;
    if output == OutputFormat::Json {
        crate::print_json_success(
            json!({"os": info.id, "homepage": info.homepage, "opened": true}),
        );
    } else {
        println!("Opened {}", info.homepage);
    }
    Ok(())
}

pub(super) fn homepage_opener() -> (&'static str, &'static [&'static str]) {
    if cfg!(target_os = "macos") {
        ("open", &[])
    } else if cfg!(target_os = "windows") {
        ("cmd", &["/C", "start", ""])
    } else {
        ("xdg-open", &[])
    }
}

pub(super) fn print_images(args: &GetArgs, output: OutputFormat) -> Result<()> {
    let os = required_arg(args.os.as_deref(), "OS")?;
    let release = required_arg(args.release_or_input.as_deref(), "RELEASE")?;
    for architecture in requested_architectures(args, os)? {
        let image = resolve_requested_image(
            args.cloud,
            os,
            release,
            args.edition_or_language.as_deref(),
            &architecture,
        )?;
        print_image(&image, output, None);
    }
    Ok(())
}

pub(super) fn check_images(
    args: &GetArgs,
    all_architectures: bool,
    output: OutputFormat,
) -> Result<()> {
    let os = required_arg(args.os.as_deref(), "OS")?;
    let release = required_arg(args.release_or_input.as_deref(), "RELEASE")?;
    let architectures = if all_architectures {
        vec!["amd64".to_string(), "arm64".to_string()]
    } else {
        requested_architectures(args, os)?
    };
    let mut json_results = Vec::new();
    let mut first_failure: Option<(String, String)> = None;
    for architecture in architectures {
        let image = match resolve_requested_image(
            args.cloud,
            os,
            release,
            args.edition_or_language.as_deref(),
            &architecture,
        ) {
            Ok(image) => image,
            Err(error) if all_architectures => {
                if first_failure.is_none() {
                    first_failure = Some((architecture.clone(), error.to_string()));
                }
                if output == OutputFormat::Json {
                    json_results.push(check_result_json(
                        os,
                        release,
                        args.edition_or_language.as_deref(),
                        &architecture,
                        false,
                        Some(error.to_string()),
                    ));
                } else {
                    print_check_result(
                        os,
                        release,
                        args.edition_or_language.as_deref(),
                        &architecture,
                        false,
                        &error,
                    );
                }
                continue;
            }
            Err(error) => return Err(error),
        };
        let available = if find_os(os)?.id == "macos" {
            let recovery = fetch_macos_recovery(release)?;
            let headers = vec![
                "Host: oscdn.apple.com".to_string(),
                "Connection: close".to_string(),
                "User-Agent: InternetRecovery/1.0".to_string(),
                format!("Cookie: AssetToken={}", recovery.asset_token),
            ];
            url_available_with_headers(&image.url, &headers, args.insecure)?
        } else {
            url_available(&image.url, args.insecure)?
        };
        if !available && first_failure.is_none() {
            first_failure = Some((architecture.clone(), "image URL is unavailable".to_string()));
        }
        if output == OutputFormat::Json {
            json_results.push(check_result_json(
                os,
                release,
                image.edition.as_deref(),
                &architecture,
                available,
                (!available).then(|| "image URL is unavailable".to_string()),
            ));
        } else {
            print_check_result(
                os,
                release,
                image.edition.as_deref(),
                &architecture,
                available,
                &Error::message("image URL is unavailable"),
            );
        }
    }
    if let Some((architecture, cause)) = first_failure {
        return Err(Error::image_unavailable(os, release, &architecture, cause));
    }
    if output == OutputFormat::Json {
        crate::print_json_success(json!(json_results));
    }
    Ok(())
}
