use std::env;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::cli::Cli;
use crate::error::{Error, Result};

#[derive(Debug, Clone)]
pub struct Dirs {
    pub vm_dir: PathBuf,
    pub state_root: PathBuf,
}

impl Dirs {
    pub fn from_cli(cli: &Cli) -> Result<Self> {
        Ok(Self {
            vm_dir: cli.vm_dir.clone().map_or_else(default_vm_dir, Ok)?,
            state_root: cli.state_dir.clone().map_or_else(default_state_root, Ok)?,
        })
    }
}

#[derive(Debug, Clone)]
pub struct VmPaths {
    pub state_dir: PathBuf,
}

impl VmPaths {
    pub fn new(state_root: &Path, name: &str) -> Self {
        Self {
            state_dir: state_root.join("vms").join(name),
        }
    }

    pub fn pid_file(&self) -> PathBuf {
        self.state_dir.join("vm.pid")
    }

    pub fn qmp_socket(&self) -> PathBuf {
        self.state_dir.join("qmp.sock")
    }

    pub fn agent_socket(&self) -> PathBuf {
        self.state_dir.join("agent.sock")
    }

    pub fn ipc_state(&self) -> PathBuf {
        self.state_dir.join("ipc.json")
    }

    pub fn serial_socket(&self) -> PathBuf {
        self.state_dir.join("serial.sock")
    }

    pub fn spice_socket(&self) -> PathBuf {
        self.state_dir.join("spice.sock")
    }

    pub fn monitor_socket(&self) -> PathBuf {
        self.state_dir.join("monitor.sock")
    }

    pub fn tpm_socket(&self) -> PathBuf {
        self.state_dir.join("swtpm.sock")
    }

    pub fn tpm_pid_file(&self) -> PathBuf {
        self.state_dir.join("swtpm.pid")
    }

    pub fn virtiofs_socket(&self) -> PathBuf {
        self.state_dir.join("virtiofs.sock")
    }

    pub fn virtiofs_pid_file(&self) -> PathBuf {
        self.state_dir.join("virtiofsd.pid")
    }

    pub fn virtiofs_socket_pid_file(&self) -> PathBuf {
        self.state_dir.join("virtiofs.sock.pid")
    }
}

pub fn default_public_dir() -> Result<Option<PathBuf>> {
    if env::consts::OS == "windows" {
        if let Some(path) = env::var_os("PUBLIC").map(PathBuf::from)
            && path.is_absolute()
            && path.is_dir()
        {
            return Ok(Some(path));
        }
        let home = home_dir()?;
        return Ok(windows_public_path(&home).filter(|path| path.is_dir()));
    } else if let Ok(output) = Command::new("xdg-user-dir").arg("PUBLICSHARE").output()
        && output.status.success()
    {
        let path = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
        if path.is_absolute() && path.is_dir() {
            return Ok(Some(path));
        }
    }

    let path = home_dir()?.join("Public");
    Ok(path.is_dir().then_some(path))
}

pub fn home_dir() -> Result<PathBuf> {
    let path = if env::consts::OS == "windows" {
        absolute_env_path("USERPROFILE")
            .or_else(|| windows_home_from_parts(env::var_os("HOMEDRIVE"), env::var_os("HOMEPATH")))
            .or_else(|| absolute_env_path("HOME"))
    } else {
        absolute_env_path("HOME")
    };
    path.ok_or(Error::HomeDirectoryUnavailable)
}

pub(crate) fn default_vm_dir() -> Result<PathBuf> {
    Ok(platform_roots()?.0.join("vmctl").join("vms"))
}

fn default_state_root() -> Result<PathBuf> {
    Ok(platform_roots()?.1.join("vmctl"))
}

fn platform_roots() -> Result<(PathBuf, PathBuf)> {
    let home = home_dir()?;
    Ok(platform_roots_for(
        env::consts::OS,
        &home,
        absolute_env_path("XDG_CONFIG_HOME").as_deref(),
        absolute_env_path("XDG_STATE_HOME").as_deref(),
        absolute_env_path("APPDATA").as_deref(),
        absolute_env_path("LOCALAPPDATA").as_deref(),
    ))
}

fn absolute_env_path(name: &str) -> Option<PathBuf> {
    let path = env::var_os(name).map(PathBuf::from)?;
    absolute_path(path)
}

fn absolute_path(path: PathBuf) -> Option<PathBuf> {
    (!path.as_os_str().is_empty() && path.is_absolute()).then_some(path)
}

fn windows_home_from_parts(drive: Option<OsString>, home: Option<OsString>) -> Option<PathBuf> {
    let mut path = drive?;
    path.push(home?);
    absolute_path(PathBuf::from(path))
}

fn windows_public_path(home: &Path) -> Option<PathBuf> {
    home.parent().map(|parent| parent.join("Public"))
}

fn platform_roots_for(
    os: &str,
    home: &Path,
    xdg_config: Option<&Path>,
    xdg_state: Option<&Path>,
    app_data: Option<&Path>,
    local_app_data: Option<&Path>,
) -> (PathBuf, PathBuf) {
    match os {
        "windows" => (
            app_data
                .map(Path::to_path_buf)
                .unwrap_or_else(|| home.join("AppData/Roaming")),
            local_app_data
                .or(app_data)
                .map(Path::to_path_buf)
                .unwrap_or_else(|| home.join("AppData/Local")),
        ),
        "macos" => {
            let fallback = home.join("Library/Application Support");
            (fallback.clone(), fallback)
        }
        _ => (
            xdg_config
                .map(Path::to_path_buf)
                .unwrap_or_else(|| home.join(".config")),
            xdg_state
                .map(Path::to_path_buf)
                .unwrap_or_else(|| home.join(".local/state")),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::platform_roots_for;
    use std::path::Path;

    #[test]
    fn platform_roots_use_native_defaults() {
        let home = Path::new("/home/test");
        assert_eq!(
            platform_roots_for("linux", home, None, None, None, None),
            (home.join(".config"), home.join(".local/state"))
        );
        assert_eq!(
            platform_roots_for("macos", home, None, None, None, None),
            (
                home.join("Library/Application Support"),
                home.join("Library/Application Support")
            )
        );
        assert_eq!(
            platform_roots_for("windows", home, None, None, None, None),
            (home.join("AppData/Roaming"), home.join("AppData/Local"))
        );
    }

    #[test]
    fn platform_roots_honor_environment_overrides() {
        let home = Path::new("/home/test");
        assert_eq!(
            platform_roots_for(
                "windows",
                home,
                None,
                None,
                Some(Path::new("/roaming")),
                Some(Path::new("/local")),
            ),
            (
                Path::new("/roaming").to_path_buf(),
                Path::new("/local").to_path_buf()
            )
        );
        assert_eq!(
            platform_roots_for(
                "linux",
                home,
                Some(Path::new("/config")),
                Some(Path::new("/state")),
                None,
                None,
            ),
            (
                Path::new("/config").to_path_buf(),
                Path::new("/state").to_path_buf()
            )
        );
    }
}
