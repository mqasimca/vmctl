use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::Write;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{self, Command};

const VERSION: &str = "0.1.0";

#[derive(Debug)]
struct Vm {
    name: String,
    config_path: PathBuf,
    state_dir: PathBuf,
    disk_img: Option<String>,
    guest_os: Option<String>,
    display: Option<String>,
    ssh_port: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
enum VmState {
    Running(i32),
    Stopped,
}

enum Action {
    List,
    Status(Option<String>),
    Start(String),
    Stop(String),
    Help,
    Version,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("vmctl: {error}");
        process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let (root, action) = parse_args()?;

    match action {
        Action::Help => {
            print_usage();
            Ok(())
        }
        Action::Version => {
            println!("vmctl {VERSION}");
            Ok(())
        }
        Action::List => list_vms(&root),
        Action::Status(name) => status_vms(&root, name.as_deref()),
        Action::Start(name) => start_vm(&root, &name),
        Action::Stop(name) => stop_vm(&root, &name),
    }
}

fn parse_args() -> Result<(PathBuf, Action), String> {
    let mut args = env::args().skip(1).peekable();
    let mut root = PathBuf::from("../vms");

    while let Some(argument) = args.peek().cloned() {
        match argument.as_str() {
            "-d" | "--dir" => {
                args.next();
                let path = args
                    .next()
                    .ok_or_else(|| "--dir requires a path".to_string())?;
                root = PathBuf::from(path);
            }
            "-h" | "--help" => {
                return Ok((root, Action::Help));
            }
            "--version" => {
                return Ok((root, Action::Version));
            }
            _ => break,
        }
    }

    let command = args.next().unwrap_or_else(|| "list".to_string());
    let action = match command.as_str() {
        "list" => {
            reject_extra_args(&mut args, "list")?;
            Action::List
        }
        "status" => Action::Status(optional_name(&mut args, "status")?),
        "start" => Action::Start(required_name(&mut args, "start")?),
        "stop" => Action::Stop(required_name(&mut args, "stop")?),
        "help" => Action::Help,
        _ => return Err(format!("unknown command '{command}'\n\n{}", usage_text())),
    };

    Ok((root, action))
}

fn required_name(args: &mut impl Iterator<Item = String>, command: &str) -> Result<String, String> {
    let name = args
        .next()
        .ok_or_else(|| format!("{command} requires a VM name"))?;
    reject_extra_args(args, command)?;
    Ok(name)
}

fn optional_name(
    args: &mut impl Iterator<Item = String>,
    command: &str,
) -> Result<Option<String>, String> {
    let name = args.next();
    reject_extra_args(args, command)?;
    Ok(name)
}

fn reject_extra_args(args: &mut impl Iterator<Item = String>, command: &str) -> Result<(), String> {
    if let Some(extra) = args.next() {
        return Err(format!("unexpected argument '{extra}' for {command}"));
    }
    Ok(())
}

fn list_vms(root: &Path) -> Result<(), String> {
    let vms = discover_vms(root)?;

    if vms.is_empty() {
        println!("No VM configurations found in {}", root.display());
        return Ok(());
    }

    println!("{:<32} {:<10} {:<8} CONFIG", "NAME", "STATE", "SSH");
    for vm in vms {
        let ssh = vm.ssh_port.as_deref().unwrap_or("-");
        println!(
            "{:<32} {:<10} {:<8} {}",
            vm.name,
            state_label(&vm),
            ssh,
            vm.config_path.display()
        );
    }

    Ok(())
}

fn status_vms(root: &Path, name: Option<&str>) -> Result<(), String> {
    if let Some(name) = name {
        let vm = find_vm(root, name)?;
        print_vm_status(&vm);
        return Ok(());
    }

    list_vms(root)
}

fn start_vm(root: &Path, name: &str) -> Result<(), String> {
    let vm = find_vm(root, name)?;

    if let VmState::Running(pid) = vm_state(&vm) {
        println!("{} is already running (pid {pid})", vm.name);
        return Ok(());
    }

    println!("Starting {}...", vm.name);
    let status = Command::new("quickemu")
        .current_dir(root)
        .arg("--vm")
        .arg(&vm.config_path)
        .status()
        .map_err(|error| format!("could not start quickemu: {error}"))?;

    if !status.success() {
        return Err(format!("quickemu exited with {status}"));
    }

    Ok(())
}

fn stop_vm(root: &Path, name: &str) -> Result<(), String> {
    let vm = find_vm(root, name)?;

    let VmState::Running(pid) = vm_state(&vm) else {
        println!("{} is already stopped", vm.name);
        return Ok(());
    };

    let socket = vm.monitor_socket();
    let mut stream = UnixStream::connect(&socket).map_err(|error| {
        format!(
            "{} is running as pid {pid}, but its monitor socket {} is unavailable: {error}",
            vm.name,
            socket.display()
        )
    })?;

    stream
        .write_all(b"system_powerdown\n")
        .map_err(|error| format!("could not request shutdown for {}: {error}", vm.name))?;

    println!("Shutdown requested for {} (pid {pid})", vm.name);
    Ok(())
}

fn discover_vms(root: &Path) -> Result<Vec<Vm>, String> {
    let entries = fs::read_dir(root)
        .map_err(|error| format!("cannot read VM directory {}: {error}", root.display()))?;
    let mut vms = Vec::new();

    for entry in entries {
        let entry = entry.map_err(|error| format!("cannot read VM directory entry: {error}"))?;
        let path = entry.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("conf") {
            continue;
        }
        vms.push(parse_vm(root, path)?);
    }

    vms.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(vms)
}

fn find_vm(root: &Path, name: &str) -> Result<Vm, String> {
    let wanted = name.strip_suffix(".conf").unwrap_or(name);
    discover_vms(root)?
        .into_iter()
        .find(|vm| vm.name == wanted)
        .ok_or_else(|| format!("VM '{name}' was not found in {}", root.display()))
}

fn parse_vm(root: &Path, config_path: PathBuf) -> Result<Vm, String> {
    let name = config_path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .ok_or_else(|| format!("invalid VM configuration path {}", config_path.display()))?
        .to_string();
    let contents = fs::read_to_string(&config_path)
        .map_err(|error| format!("cannot read {}: {error}", config_path.display()))?;
    let values = parse_config(&contents);
    let disk_img = values.get("disk_img").cloned();
    let state_dir = disk_img
        .as_deref()
        .map(Path::new)
        .and_then(Path::parent)
        .map(|path| {
            if path.as_os_str().is_empty() {
                root
            } else {
                path
            }
        })
        .map(|path| {
            if path.is_absolute() {
                path.to_path_buf()
            } else {
                root.join(path)
            }
        })
        .unwrap_or_else(|| root.join(&name));

    Ok(Vm {
        name,
        config_path,
        state_dir,
        disk_img,
        guest_os: values.get("guest_os").cloned(),
        display: values.get("display").cloned(),
        ssh_port: values.get("ssh_port").cloned(),
    })
}

fn parse_config(contents: &str) -> HashMap<String, String> {
    contents
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                return None;
            }
            let (key, value) = line.split_once('=')?;
            let key = key.trim();
            if key.is_empty() {
                return None;
            }
            Some((key.to_string(), unquote(value.trim()).to_string()))
        })
        .collect()
}

fn unquote(value: &str) -> &str {
    if value.len() >= 2 {
        let quoted = (value.starts_with('"') && value.ends_with('"'))
            || (value.starts_with('\'') && value.ends_with('\''));
        if quoted {
            return &value[1..value.len() - 1];
        }
    }
    value
}

fn state_label(vm: &Vm) -> String {
    match vm_state(vm) {
        VmState::Running(pid) => format!("running({pid})"),
        VmState::Stopped => "stopped".to_string(),
    }
}

fn vm_state(vm: &Vm) -> VmState {
    let pid_path = vm.state_dir.join(format!("{}.pid", vm.name));
    let Ok(contents) = fs::read_to_string(pid_path) else {
        return VmState::Stopped;
    };
    let Ok(pid) = contents.trim().parse::<i32>() else {
        return VmState::Stopped;
    };
    if process_exists(pid) {
        VmState::Running(pid)
    } else {
        VmState::Stopped
    }
}

fn process_exists(pid: i32) -> bool {
    Command::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn print_vm_status(vm: &Vm) {
    println!("name:      {}", vm.name);
    println!("state:     {}", state_label(vm));
    println!("config:    {}", vm.config_path.display());
    println!("state dir: {}", vm.state_dir.display());
    if let Some(guest_os) = &vm.guest_os {
        println!("guest os:  {guest_os}");
    }
    if let Some(display) = &vm.display {
        println!("display:   {display}");
    }
    if let Some(disk_img) = &vm.disk_img {
        println!("disk:      {disk_img}");
    }
    if let Some(ssh_port) = &vm.ssh_port {
        println!("ssh port:  {ssh_port}");
    }
    println!("monitor:   {}", vm.monitor_socket().display());
    println!("agent:     {}", vm.agent_socket().display());
}

impl Vm {
    fn monitor_socket(&self) -> PathBuf {
        self.state_dir.join(format!("{}-monitor.socket", self.name))
    }

    fn agent_socket(&self) -> PathBuf {
        self.state_dir.join(format!("{}-agent.sock", self.name))
    }
}

fn usage_text() -> &'static str {
    "Usage: vmctl [--dir PATH] <command> [VM]\n\nCommands:\n  list              List VM configurations (default)\n  status [VM]       Show VM state and details\n  start VM          Start a VM with Quickemu\n  stop VM           Request a graceful guest shutdown\n  help              Show this help\n\nOptions:\n  -d, --dir PATH    VM configuration directory (default: ../vms)\n      --version     Show version"
}

fn print_usage() {
    println!("{}", usage_text());
}

#[cfg(test)]
mod tests {
    use super::parse_config;

    #[test]
    fn parses_simple_quickemu_config() {
        let values = parse_config(
            r#"#!/usr/bin/quickemu --vm
guest_os="linux"
disk_img="ubuntu/disk.qcow2"
ssh_port="22220"
"#,
        );

        assert_eq!(values.get("guest_os"), Some(&"linux".to_string()));
        assert_eq!(
            values.get("disk_img"),
            Some(&"ubuntu/disk.qcow2".to_string())
        );
        assert_eq!(values.get("ssh_port"), Some(&"22220".to_string()));
    }
}
