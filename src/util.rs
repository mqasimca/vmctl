use std::env;
use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::error::{Error, Result};

pub(crate) fn ensure_not_symlink(path: &Path, action: &str) -> Result<()> {
    if fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        return Err(Error::message(format!(
            "refusing to {action} symlink {}",
            path.display()
        )));
    }
    Ok(())
}

pub(crate) fn command_available(command: &str) -> bool {
    Command::new(command)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

pub(crate) fn find_executable(command: &str) -> Option<String> {
    let path = env::var_os("PATH")?;
    let names = executable_names(command);
    env::split_paths(&path).find_map(|directory| {
        names
            .iter()
            .map(|name| directory.join(name))
            .find(|candidate| is_executable_file(candidate))
            .map(|candidate| candidate.to_string_lossy().into_owned())
    })
}

pub(crate) fn is_executable_file(path: &Path) -> bool {
    let Ok(metadata) = fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

pub(crate) fn executable_names(command: &str) -> Vec<String> {
    #[cfg(windows)]
    {
        let mut names = vec![command.to_string()];
        if Path::new(command).extension().is_none() {
            let extensions =
                env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_string());
            names.extend(
                extensions
                    .split(';')
                    .filter(|extension| !extension.is_empty())
                    .map(|extension| format!("{command}{extension}")),
            );
        }
        names
    }
    #[cfg(not(windows))]
    {
        vec![command.to_string()]
    }
}