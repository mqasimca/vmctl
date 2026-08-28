use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;

use super::*;

pub(super) fn clone_vm(dirs: &Dirs, args: &CloneArgs, output: OutputFormat) -> Result<()> {
    if args.user_data.is_some() && !args.ssh_keys.is_empty() {
        return Err(Error::invalid_argument(
            "--user-data",
            "cannot be combined with --ssh-key; provide complete cloud-init user-data instead",
        ));
    }
    let name = crate::get::validate_vm_name(&args.name)?;
    let hostname = args.hostname.as_deref().unwrap_or(name);
    if let Some(macaddr) = &args.macaddr
        && (macaddr.split(':').count() != 6
            || macaddr
                .split(':')
                .any(|part| part.len() != 2 || u8::from_str_radix(part, 16).is_err()))
    {
        return Err(Error::invalid_argument(
            "--macaddr",
            "must be six hexadecimal octets",
        ));
    }

    let source = find(&dirs.vm_dir, &dirs.state_root, &args.source)?;
    if source.config.name == name {
        return Err(Error::invalid_argument(
            "NAME",
            "must differ from the source VM name",
        ));
    }
    let _operation_lock = acquire_vm_lock(&source.paths)?;
    require_stopped_disk(&source, "clone")?;
    crate::qemu::require_disk_file(&source.config.disk_img)?;

    let config_path = dirs.vm_dir.join(format!("{name}.conf"));
    let target_dir = dirs.vm_dir.join(name);
    if config_path.exists() {
        return Err(Error::message(format!(
            "configuration already exists: {}",
            config_path.display()
        )));
    }
    if fs::symlink_metadata(&target_dir).is_ok() {
        return Err(Error::message(format!(
            "clone data directory already exists: {}",
            target_dir.display()
        )));
    }
    fs::create_dir_all(&dirs.vm_dir).map_err(|error| Error::io(dirs.vm_dir.display(), error))?;
    fs::create_dir(&target_dir).map_err(|error| Error::io(target_dir.display(), error))?;

    let disk = target_dir.join("disk.qcow2");
    let is_cloud = source.config.cloud_init_iso.is_some()
        || source.config.cloud_base_img.is_some()
        || source.config.ssh_user.is_some();
    if is_cloud && let Err(error) = crate::get::validate_hostname(hostname) {
        let _ = fs::remove_dir_all(&target_dir);
        return Err(error);
    }

    let result = (|| {
        let seed = if is_cloud {
            Some(crate::get::create_cloud_seed(
                &target_dir,
                &source.config.guest_os,
                hostname,
                &args.ssh_keys,
                args.user_data.as_deref(),
                args.network_config.as_deref(),
            )?)
        } else {
            None
        };
        crate::qemu::disk_convert(&source.config.disk_img, &disk, "qcow2", false, false)?;
        write_clone_config(
            &source.config.config_path,
            &config_path,
            &disk,
            seed.as_deref(),
            args.macaddr.as_deref(),
        )
    })();
    let config = match result {
        Ok(config) => config,
        Err(error) => {
            let _ = fs::remove_dir_all(&target_dir);
            return Err(error);
        }
    };

    let next = if is_cloud {
        vec![
            format!("vmctl start {name} --wait ssh"),
            format!("vmctl ssh {name}"),
        ]
    } else {
        vec![format!("vmctl start {name}")]
    };
    let result = json!({
        "name": name,
        "source": source.config.name,
        "disk": disk,
        "cloud": is_cloud,
        "config": config,
        "next": next,
    });
    if output == OutputFormat::Json {
        print_json_success(result);
    } else {
        println!("Cloned {} to {}", source.config.name, config.display());
        for command in next {
            println!("{}", command);
        }
    }
    Ok(())
}

fn write_clone_config(
    source_config_path: &Path,
    config_path: &Path,
    disk: &Path,
    seed: Option<&Path>,
    macaddr: Option<&str>,
) -> Result<PathBuf> {
    let contents = fs::read_to_string(source_config_path)
        .map_err(|error| Error::io(source_config_path.display(), error))?;
    let mut values = crate::config::parse_config(&contents);

    values.insert(
        "disk_img".to_string(),
        relative_value(config_path.parent().unwrap_or(Path::new(".")), disk),
    );
    for key in [
        "iso",
        "fixed_iso",
        "unattended_iso",
        "floppy",
        "img",
        "cloud_base_img",
        "boot_once",
    ] {
        values.remove(key);
    }
    if let Some(seed) = seed {
        values.insert(
            "cloud_init_iso".to_string(),
            relative_value(config_path.parent().unwrap_or(Path::new("")), seed),
        );
    } else {
        values.remove("cloud_init_iso");
    }
    match macaddr {
        Some(macaddr) => {
            values.insert("macaddr".to_string(), macaddr.to_string());
        }
        None => {
            values.remove("macaddr");
        }
    }

    let mut lines = Vec::new();
    for (key, value) in &values {
        let value = value.replace('\\', "\\\\").replace('"', "\\\"");
        lines.push(format!("{key}=\"{value}\""));
    }
    lines.sort();
    let contents = format!("{}\n", lines.join("\n"));
    write_new_config(config_path, &contents)
}

fn relative_value(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn write_new_config(path: &Path, contents: &str) -> Result<PathBuf> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| Error::io(parent.display(), error))?;
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| Error::io(path.display(), error))?;
    if let Err(error) = file.write_all(contents.as_bytes()) {
        drop(file);
        let _ = fs::remove_file(path);
        return Err(Error::io(path.display(), error));
    }
    Ok(path.to_path_buf())
}
