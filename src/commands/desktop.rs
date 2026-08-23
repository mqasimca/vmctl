use super::*;

pub(super) fn shortcut_vm(
    dirs: &Dirs,
    name: &str,
    path: Option<PathBuf>,
    output: OutputFormat,
) -> Result<()> {
    let vm = find(&dirs.vm_dir, &dirs.state_root, name)?;
    let path = match path {
        Some(path) => path,
        None => paths::home_dir()?
            .join(".local/share/applications")
            .join(format!("{}.desktop", vm.config.name)),
    };
    let executable = env::current_exe().unwrap_or_else(|_| PathBuf::from("vmctl"));
    let config_root = vm
        .config
        .config_path
        .parent()
        .unwrap_or_else(|| Path::new("."));
    let content = format!(
        "[Desktop Entry]\nType=Application\nName={}\nComment=Start {} with vmctl\nTerminal=false\nExec={} --dir {} start {}\nPath={}\nCategories=System;Virtualization;\n",
        vm.config.name,
        vm.config.name,
        desktop_exec_quote(&executable),
        desktop_exec_quote(config_root),
        desktop_exec_quote(Path::new(&vm.config.name)),
        desktop_quote(config_root),
    );
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| Error::io(parent.display(), error))?;
    }
    let mut file =
        qemu::create_truncated_file(&path).map_err(|error| Error::io(path.display(), error))?;
    file.write_all(content.as_bytes())
        .map_err(|error| Error::io(path.display(), error))?;
    if output == OutputFormat::Json {
        print_json_success(json!({"name": vm.config.name, "shortcut": path}));
    } else {
        println!("Created {}", path.display());
    }
    Ok(())
}
