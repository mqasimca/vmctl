use super::*;

use std::collections::BTreeSet;

pub(super) fn cache_vm(dirs: &Dirs, action: CacheAction, output: OutputFormat) -> Result<()> {
    match action {
        CacheAction::Prune { yes } => prune_cache(dirs, yes, output),
    }
}

fn prune_cache(dirs: &Dirs, yes: bool, output: OutputFormat) -> Result<()> {
    let objects = dirs.vm_dir.join(".cache/objects");
    let cache_lock = get::cache_lock(&dirs.vm_dir)?;
    let mut referenced = BTreeSet::new();
    for vm in discover(&dirs.vm_dir, &dirs.state_root)? {
        for path in vm_paths(&vm.config) {
            if let Ok(relative) = path.strip_prefix(&objects)
                && relative.components().count() == 1
                && let Some(name) = relative.to_str()
            {
                referenced.insert(name.to_string());
            }
        }
    }
    let candidates = get::cache_prune_candidates(&dirs.vm_dir, &referenced)?;
    if yes && let Some(lock) = cache_lock.as_ref() {
        get::remove_cache_candidates(&dirs.vm_dir, &candidates, lock)?;
    }
    let files = candidates
        .iter()
        .map(|path| path.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    if output == OutputFormat::Json {
        print_json_success(json!({
            "action": "prune",
            "deleted": yes,
            "objects": files,
        }));
    } else if candidates.is_empty() {
        println!("No unreferenced cache objects found");
    } else if yes {
        for path in &candidates {
            println!("Removed {}", path.display());
        }
    } else {
        for path in &candidates {
            println!("Would remove {}", path.display());
        }
        println!("Rerun with --yes to remove these cache objects");
    }
    Ok(())
}

fn vm_paths(config: &VmConfig) -> impl Iterator<Item = &Path> {
    [
        Some(&config.disk_img),
        config.iso.as_ref(),
        config.fixed_iso.as_ref(),
        config.unattended_iso.as_ref(),
        config.cloud_base_img.as_ref(),
        config.cloud_init_iso.as_ref(),
        config.floppy.as_ref(),
        config.img.as_ref(),
    ]
    .into_iter()
    .flatten()
    .map(PathBuf::as_path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn cache_paths_include_all_attached_media() {
        let root = tempdir().unwrap();
        let config_path = root.path().join("vm.conf");
        std::fs::write(
            &config_path,
            "disk_img=\"vm/disk.qcow2\"\niso=\".cache/objects/installer.iso\"\ncloud_base_img=\".cache/objects/base.qcow2\"\n",
        )
        .unwrap();
        let vm = crate::config::load_vm(root.path(), root.path(), config_path).unwrap();
        let paths = vm_paths(&vm.config).collect::<Vec<_>>();
        assert!(paths.iter().any(|path| path.ends_with("installer.iso")));
        assert!(paths.iter().any(|path| path.ends_with("base.qcow2")));
    }
}
