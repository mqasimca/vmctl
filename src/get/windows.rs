use super::*;

pub(super) fn windows_asset(
    os: &str,
    release: &str,
    language: Option<&str>,
) -> Result<(String, ImageKind, Option<String>)> {
    let language = language.unwrap_or("English International");
    let url = if os == "windows-server" {
        windows_server_url(release)?
    } else {
        windows_workstation_url(release, language)?
    };
    Ok((url, ImageKind::Iso, None))
}

pub(super) fn windows_server_url(release: &str) -> Result<String> {
    let page = fetch_text(&format!(
        "https://www.microsoft.com/en-us/evalcenter/download-windows-server-{release}"
    ))?;
    let link = first_token(&page, |value| {
        value.starts_with("https://go.microsoft.com/fwlink/p/?")
            && value.contains("culture=en-us")
            && value.contains("country=US")
    })
    .ok_or_else(|| dynamic_url_error("windows-server"))?;
    fetch_redirect(&link)
}

pub(super) fn windows_workstation_url(release: &str, language: &str) -> Result<String> {
    let page_url = if release == "10" {
        "https://www.microsoft.com/en-us/software-download/windows10ISO".to_string()
    } else {
        format!("https://www.microsoft.com/en-us/software-download/windows{release}")
    };
    let user_agent = "Mozilla/5.0 (X11; Linux x86_64; rv:100.0) Gecko/20100101 Firefox/100.0";
    let user_agent_header = format!("User-Agent: {user_agent}");
    let page = curl_request(&page_url, &["Accept:", &user_agent_header], None, None)?;
    let product_id = page
        .split("<option value=\"")
        .skip(1)
        .find_map(|part| {
            let (value, rest) = part.split_once('"')?;
            (rest.starts_with(">Windows")
                && value.chars().all(|character| character.is_ascii_digit()))
            .then_some(value)
        })
        .ok_or_else(|| dynamic_url_error("windows"))?;
    let session = format!(
        "{}-{}-{}-{}-{}",
        random_hex(8),
        random_hex(4),
        random_hex(4),
        random_hex(4),
        random_hex(12)
    );
    curl_request(
        &format!("https://vlscppe.microsoft.com/tags?org_id=y6jn8c31&session_id={session}"),
        &["Accept:", &user_agent_header],
        None,
        None,
    )?;
    windows_ov_df_handshake(&session, &user_agent_header)?;
    let sku_data = curl_request(
        &format!(
            "https://www.microsoft.com/software-download-connector/api/getskuinformationbyproductedition?profile=606624d44113&ProductEditionId={product_id}&SKU=undefined&friendlyFileName=undefined&Locale=en-US&sessionID={session}"
        ),
        &["Accept:", &user_agent_header],
        None,
        None,
    )?;
    let sku_values: Value = serde_json::from_str(&sku_data)
        .map_err(|error| Error::message(format!("invalid Microsoft SKU data: {error}")))?;
    let sku = sku_values
        .get("Skus")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find(|entry| {
            entry
                .get("LocalizedLanguage")
                .and_then(Value::as_str)
                .is_some_and(|value| value == language)
                || entry
                    .get("Language")
                    .and_then(Value::as_str)
                    .is_some_and(|value| value == language)
        })
        .and_then(|entry| entry.get("Id"))
        .and_then(Value::as_str)
        .ok_or_else(|| Error::message(format!("Microsoft does not offer Windows in {language}")))?;
    let links_data = curl_request(
        &format!(
            "https://www.microsoft.com/software-download-connector/api/GetProductDownloadLinksBySku?profile=606624d44113&productEditionId=undefined&SKU={sku}&friendlyFileName=undefined&Locale=en-US&sessionID={session}"
        ),
        &[
            "Accept:",
            &user_agent_header,
            &format!("Referer: {page_url}"),
        ],
        None,
        None,
    )?;
    if links_data.contains("Sentinel marked this request as rejected") {
        return Err(Error::message(
            "Microsoft rejected the automated Windows download request; download the ISO in a browser and retry later",
        ));
    }
    let links: Value = serde_json::from_str(&links_data)
        .map_err(|error| Error::message(format!("invalid Microsoft download data: {error}")))?;
    links
        .get("ProductDownloadOptions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.get("Uri").and_then(Value::as_str))
        .find(|uri| uri.to_ascii_lowercase().contains("x64"))
        .map(str::to_string)
        .ok_or_else(|| dynamic_url_error("windows"))
}

pub(super) fn windows_ov_df_handshake(session: &str, user_agent_header: &str) -> Result<()> {
    let instance_id = "560dc9f3-1aa5-4a2f-b63c-9e18f8d0e175";
    let headers = ["Accept:", user_agent_header];
    let response = curl_request(
        &format!(
            "https://ov-df.microsoft.com/mdt.js?instanceId={instance_id}&PageId=si&session_id={session}"
        ),
        &headers,
        None,
        None,
    )?;
    let width = windows_ov_df_value(&response, "w", |character| character.is_ascii_hexdigit())
        .ok_or_else(|| Error::message("Microsoft Windows download response did not include w"))?;
    let rticks = windows_ov_df_value(&response, "rticks", |character| character.is_ascii_digit())
        .ok_or_else(|| {
        Error::message("Microsoft Windows download response did not include rticks")
    })?;
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| Error::message(format!("system clock is before the Unix epoch: {error}")))?
        .as_millis();
    curl_request(
        &format!(
            "https://ov-df.microsoft.com/?session_id={session}&CustomerId={instance_id}&PageId=si&w={width}&mdt={timestamp}&rticks={rticks}"
        ),
        &headers,
        None,
        None,
    )?;
    Ok(())
}

pub(super) fn windows_ov_df_value(
    response: &str,
    key: &str,
    valid: fn(char) -> bool,
) -> Option<String> {
    let marker = format!("{key}=");
    response.match_indices(&marker).find_map(|(start, _)| {
        let value = response[start + marker.len()..].trim_start_matches('+');
        let value: String = value
            .chars()
            .take_while(|character| valid(*character))
            .collect();
        (!value.is_empty()).then_some(value)
    })
}

pub(super) fn download_windows(
    args: &GetArgs,
    dirs: &Dirs,
    os: &str,
    release: &str,
    architecture: &str,
    create_config: bool,
    output: OutputFormat,
) -> Result<()> {
    if architecture != "amd64" {
        return Err(Error::message(
            "Windows downloads are only available for amd64",
        ));
    }
    let edition = required_edition(find_os(os)?, args.edition_or_language.as_deref())?;
    let image = windows_asset(os, release, edition.as_deref())?;
    let name = args
        .name
        .clone()
        .unwrap_or_else(|| suggested_name(os, release, edition.as_deref(), architecture));
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
    let config_file = root.join(format!("{name}.conf"));
    if create_config && config_file.exists() {
        return Err(Error::message(format!(
            "configuration already exists: {}",
            config_file.display()
        )));
    }
    let target_dir_existed = target_dir.exists();
    let mut written_config = None;
    let provision = (|| {
        fs::create_dir_all(&target_dir).map_err(|error| Error::io(target_dir.display(), error))?;
        let file_name =
            file_name_from_url(&image.0).unwrap_or_else(|| format!("{os}-{release}.iso"));
        let cached = if create_config {
            Some(cache_url(
                &root,
                &image.0,
                &file_name,
                ImageKind::Iso,
                None,
                args.insecure,
                args.refresh_cache,
            )?)
        } else {
            None
        };
        let iso = cached
            .as_ref()
            .map(|cache| cache.path.clone())
            .unwrap_or_else(|| target_dir.join(file_name));
        if !create_config {
            download_file(&image.0, &iso, args.insecure)?;
        }
        let (fixed_iso, unattended_iso) = if create_config {
            let fixed_iso = cache_url(
                &root,
                "https://fedorapeople.org/groups/virt/virtio-win/direct-downloads/stable-virtio/virtio-win.iso",
                "virtio-win.iso",
                ImageKind::Iso,
                None,
                args.insecure,
                args.refresh_cache,
            )?
            .path;
            let unattended_iso = if args.disable_unattended {
                None
            } else {
                Some(create_unattended_iso(&target_dir, args.insecure)?)
            };
            (Some(fixed_iso), unattended_iso)
        } else {
            (None, None)
        };
        let config = if create_config {
            let config = write_vm_config(
                &root,
                &name,
                os,
                release,
                edition.as_deref(),
                architecture,
                &iso,
                VmResources::default(),
            )?;
            written_config = Some(config.clone());
            if let Some(fixed_iso) = fixed_iso.as_deref() {
                append_iso(&root, &config, "fixed_iso", fixed_iso)?;
            }
            if let Some(unattended_iso) = unattended_iso.as_deref() {
                append_iso(&root, &config, "unattended_iso", unattended_iso)?;
            }
            Some(config)
        } else {
            None
        };
        Ok((iso, fixed_iso, unattended_iso, config, cached))
    })();
    let (iso, fixed_iso, unattended_iso, config, cached) = match provision {
        Ok(created) => created,
        Err(error) => {
            if create_config && !target_dir_existed {
                let _ = fs::remove_dir_all(&target_dir);
            }
            if let Some(config) = written_config {
                let _ = fs::remove_file(config);
            }
            return Err(error);
        }
    };
    let result = json!({
        "os": os,
        "release": release,
        "edition": edition,
        "architecture": architecture,
        "url": image.0,
        "image": iso,
        "fixed_iso": fixed_iso,
        "unattended_iso": unattended_iso,
        "config": config,
        "unattended": unattended_iso.is_some(),
        "cache": cached.as_ref().map(|cache| json!({
            "status": cache.status.as_str(),
            "object": cache.path,
            "sha256": cache.sha256,
        })),
    });
    if output == OutputFormat::Json {
        crate::print_json_success(result);
    } else if let Some(config) = config {
        if let Some(cache) = &cached {
            println!(
                "{} {}",
                if cache.status == CacheStatus::Hit {
                    "Using cached"
                } else {
                    "Downloaded"
                },
                cache.path.display()
            );
        } else {
            println!("Downloaded {}", iso.display());
        }
        println!("Created {}", config.display());
    } else {
        println!("Downloaded {}", iso.display());
    }
    Ok(())
}

pub(super) fn download_virtio_iso(target_dir: &Path, insecure: bool) -> Result<PathBuf> {
    let path = target_dir.join("virtio-win.iso");
    download_file(
        "https://fedorapeople.org/groups/virt/virtio-win/direct-downloads/stable-virtio/virtio-win.iso",
        &path,
        insecure,
    )?;
    Ok(path)
}

pub(super) const WINDOWS_UNATTENDED_XML: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<unattend xmlns="urn:schemas-microsoft-com:unattend"
  xmlns:wcm="http://schemas.microsoft.com/WMIConfig/2002/State"
  xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
  <settings pass="windowsPE">
    <component name="Microsoft-Windows-Setup" processorArchitecture="amd64" publicKeyToken="31bf3856ad364e35" language="neutral" versionScope="nonSxS">
      <DiskConfiguration>
        <Disk wcm:action="add">
          <DiskID>0</DiskID>
          <WillWipeDisk>true</WillWipeDisk>
          <CreatePartitions>
            <CreatePartition wcm:action="add"><Order>1</Order><Type>EFI</Type><Size>260</Size></CreatePartition>
            <CreatePartition wcm:action="add"><Order>2</Order><Type>MSR</Type><Size>128</Size></CreatePartition>
            <CreatePartition wcm:action="add"><Order>3</Order><Type>Primary</Type><Extend>true</Extend></CreatePartition>
          </CreatePartitions>
          <ModifyPartitions>
            <ModifyPartition wcm:action="add"><Order>1</Order><PartitionID>1</PartitionID><Format>FAT32</Format><Label>System</Label></ModifyPartition>
            <ModifyPartition wcm:action="add"><Order>2</Order><PartitionID>2</PartitionID></ModifyPartition>
            <ModifyPartition wcm:action="add"><Order>3</Order><PartitionID>3</PartitionID><Format>NTFS</Format><Label>Windows</Label><Letter>C</Letter></ModifyPartition>
          </ModifyPartitions>
        </Disk>
      </DiskConfiguration>
      <ImageInstall><OSImage><InstallTo><DiskID>0</DiskID><PartitionID>3</PartitionID></InstallTo></OSImage></ImageInstall>
      <RunSynchronous>
        <RunSynchronousCommand wcm:action="add"><Order>1</Order><Path>reg add HKLM\System\Setup\LabConfig /v BypassCPUCheck /t REG_DWORD /d 1 /f</Path></RunSynchronousCommand>
        <RunSynchronousCommand wcm:action="add"><Order>2</Order><Path>reg add HKLM\System\Setup\LabConfig /v BypassRAMCheck /t REG_DWORD /d 1 /f</Path></RunSynchronousCommand>
        <RunSynchronousCommand wcm:action="add"><Order>3</Order><Path>reg add HKLM\System\Setup\LabConfig /v BypassSecureBootCheck /t REG_DWORD /d 1 /f</Path></RunSynchronousCommand>
        <RunSynchronousCommand wcm:action="add"><Order>4</Order><Path>reg add HKLM\System\Setup\LabConfig /v BypassTPMCheck /t REG_DWORD /d 1 /f</Path></RunSynchronousCommand>
      </RunSynchronous>
      <UserData>
        <AcceptEula>true</AcceptEula>
        <FullName>vmctl</FullName>
        <Organization>vmctl</Organization>
        <ProductKey><Key>W269N-WFGWX-YVC9B-4J6C9-T83GX</Key><WillShowUI>Never</WillShowUI></ProductKey>
      </UserData>
    </component>
    <component name="Microsoft-Windows-PnpCustomizationsWinPE" processorArchitecture="amd64" publicKeyToken="31bf3856ad364e35" language="neutral" versionScope="nonSxS">
      <DriverPaths>
        <PathAndCredentials wcm:action="add" wcm:keyValue="1"><Path>E:\qemufwcfg\w10\amd64</Path></PathAndCredentials>
        <PathAndCredentials wcm:action="add" wcm:keyValue="2"><Path>E:\vioscsi\w10\amd64</Path></PathAndCredentials>
        <PathAndCredentials wcm:action="add" wcm:keyValue="3"><Path>E:\viostor\w10\amd64</Path></PathAndCredentials>
        <PathAndCredentials wcm:action="add" wcm:keyValue="4"><Path>E:\NetKVM\w10\amd64</Path></PathAndCredentials>
      </DriverPaths>
    </component>
  </settings>
  <settings pass="oobeSystem">
    <component name="Microsoft-Windows-Shell-Setup" processorArchitecture="amd64" publicKeyToken="31bf3856ad364e35" language="neutral" versionScope="nonSxS">
      <AutoLogon><Password><Value>vmctl</Value><PlainText>true</PlainText></Password><Enabled>true</Enabled><Username>vmctl</Username></AutoLogon>
      <OOBE><HideEULAPage>true</HideEULAPage><HideOnlineAccountScreens>true</HideOnlineAccountScreens><HideWirelessSetupInOOBE>true</HideWirelessSetupInOOBE><NetworkLocation>Home</NetworkLocation><ProtectYourPC>3</ProtectYourPC><SkipMachineOOBE>true</SkipMachineOOBE><SkipUserOOBE>true</SkipUserOOBE></OOBE>
      <UserAccounts><LocalAccounts><LocalAccount wcm:action="add"><Password><Value>vmctl</Value><PlainText>true</PlainText></Password><Description>vmctl</Description><DisplayName>vmctl</DisplayName><Group>Administrators</Group><Name>vmctl</Name></LocalAccount></LocalAccounts></UserAccounts>
      <FirstLogonCommands>
        <SynchronousCommand wcm:action="add"><Order>1</Order><CommandLine>msiexec /i E:\guest-agent\qemu-ga-x86_64.msi /quiet /qn</CommandLine><Description>Install QEMU Guest Agent</Description></SynchronousCommand>
        <SynchronousCommand wcm:action="add"><Order>2</Order><CommandLine>msiexec /i F:\spice-webdavd-x64-latest.msi /quiet /qn</CommandLine><Description>Install SPICE WebDAV</Description></SynchronousCommand>
        <SynchronousCommand wcm:action="add"><Order>3</Order><CommandLine>msiexec /i F:\spice-vdagent-x64-0.10.0.msi /quiet /qn</CommandLine><Description>Install SPICE agent</Description></SynchronousCommand>
      </FirstLogonCommands>
    </component>
  </settings>
</unattend>
"#;

pub(super) fn create_unattended_iso(target_dir: &Path, insecure: bool) -> Result<PathBuf> {
    let source_dir = target_dir.join("unattended");
    fs::create_dir_all(&source_dir).map_err(|error| Error::io(source_dir.display(), error))?;
    let xml = source_dir.join("autounattend.xml");
    if fs::symlink_metadata(&xml)
        .ok()
        .is_some_and(|metadata| metadata.file_type().is_symlink())
    {
        return Err(Error::message(format!(
            "refusing to write through symlink {}",
            xml.display()
        )));
    }
    fs::write(&xml, WINDOWS_UNATTENDED_XML).map_err(|error| Error::io(xml.display(), error))?;
    for (url, file) in [
        (
            "https://www.spice-space.org/download/windows/spice-webdavd/spice-webdavd-x64-latest.msi",
            "spice-webdavd-x64-latest.msi",
        ),
        (
            "https://www.spice-space.org/download/windows/vdagent/vdagent-win-0.10.0/spice-vdagent-x64-0.10.0.msi",
            "spice-vdagent-x64-0.10.0.msi",
        ),
    ] {
        download_file(url, &source_dir.join(file), insecure)?;
    }
    let destination = target_dir.join("unattended.iso");
    if fs::symlink_metadata(&destination)
        .ok()
        .is_some_and(|metadata| metadata.file_type().is_symlink())
    {
        return Err(Error::message(format!(
            "refusing to write through symlink {}",
            destination.display()
        )));
    }
    let result = create_iso(&source_dir, &destination, None);
    let _ = fs::remove_dir_all(&source_dir);
    result?;
    Ok(destination)
}

pub(super) fn append_iso(root: &Path, config: &Path, key: &str, image: &Path) -> Result<()> {
    let relative = image
        .strip_prefix(root)
        .unwrap_or(image)
        .to_string_lossy()
        .replace('\\', "/");
    let mut file = crate::config::open_config_for_append(config)?;
    writeln!(file, "{key}=\"{}\"", config_value(&relative))
        .map_err(|error| Error::io(config.display(), error))
}

pub(super) fn prepare_resolved_image(os: &str, path: &Path) -> Result<PathBuf> {
    let image = prepare_image(path)?;
    match os {
        "batocera" => {
            let status = Command::new("qemu-img")
                .args(["resize", "-f", "raw"])
                .arg(&image)
                .arg("128G")
                .status()
                .map_err(|error| Error::command_unavailable("qemu-img", error))?;
            if !status.success() {
                return Err(Error::command_failed_status("qemu-img resize", status));
            }
            Ok(image)
        }
        "easyos" => {
            let parent = image
                .parent()
                .ok_or_else(|| Error::message("EasyOS image has no parent directory"))?;
            let disk = parent.join("disk.qcow2");
            if fs::symlink_metadata(&disk)
                .ok()
                .is_some_and(|metadata| metadata.file_type().is_symlink())
            {
                return Err(Error::message(format!(
                    "refusing to write through symlink {}",
                    disk.display()
                )));
            }
            let status = Command::new("qemu-img")
                .args(["convert", "-f", "raw", "-O", "qcow2"])
                .arg(&image)
                .arg(&disk)
                .status()
                .map_err(|error| Error::command_unavailable("qemu-img", error))?;
            if !status.success() {
                return Err(Error::command_failed_status("qemu-img convert", status));
            }
            Ok(disk)
        }
        _ => Ok(image),
    }
}
