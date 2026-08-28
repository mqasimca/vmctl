pub mod cli;

mod agent;
mod config;
mod error;
mod get;
mod paths;
mod qemu;

use std::collections::BTreeSet;
use std::env;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::process::{Command as ProcessCommand, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use clap::CommandFactory;
use clap_complete::{CompleteEnv, Shell};

#[cfg(unix)]
use std::os::unix::{fs::PermissionsExt, process::CommandExt};

use serde_json::{Value, json};

pub(crate) use agent::print_json_success;

use cli::{
    CacheAction, Cli, CloneArgs, Command as VmCommand, DiskAction, GuestAction, HostAction,
    LaunchOptions, OutputFormat, SnapshotAction, StartWait,
};
use config::{discover, find};
use paths::Dirs;
use qemu::{
    HostCapabilities, acquire_vm_lock, build_plan, configured_bridge, disk_check, disk_compact,
    disk_convert, disk_info, disk_resize, disk_snapshot, ensure_disk,
    ensure_ipc_endpoints_available, guest_command, guest_exec, guest_fsfreeze_freeze,
    guest_fsfreeze_status, guest_fsfreeze_thaw, guest_fstrim, guest_shutdown, ipc_report,
    kill_process, qemu_capability_report, qmp_live_resources, qmp_ping, qmp_status,
    remove_runtime_sockets, render_node, send_monitor_command, shell_join, shutdown_via_qmp,
    spice_address, start_tpm, start_virtiofsd, stop_tpm, stop_virtiofsd, virtiofs_requested,
    virtiofsd_available, wait_for_exit, write_runtime_files,
};

pub const AGENT_SCHEMA_VERSION: u32 = 1;

pub use config::{Vm, VmConfig, VmState, parse_config, parse_tokens};
pub use error::{Error, Error as VmctlError, Result};
pub use paths::VmPaths;
pub use qemu::{QemuPlan, QemuPlanContext};

pub fn run(cli: Cli) -> Result<()> {
    if matches!(cli.command.as_ref(), Some(VmCommand::Schema)) {
        print_json_success(agent::schema());
        return Ok(());
    }
    if let Some(VmCommand::Completion { shell }) = cli.command.as_ref() {
        return generate_completions(*shell);
    }

    let dirs = Dirs::from_cli(&cli)?;
    let output = cli.output;

    if cli.verbose > 0 && output != OutputFormat::Json {
        eprintln!(
            "vmctl: vm-dir={} state-dir={}",
            dirs.vm_dir.display(),
            dirs.state_root.display()
        );
    }

    match cli.command.unwrap_or(VmCommand::List) {
        VmCommand::List => list_vms(&dirs, output),
        VmCommand::Schema => unreachable!("schema handled before path setup"),
        VmCommand::Completion { .. } => unreachable!("completion handled before path setup"),
        VmCommand::Status { vm, live } => status_vms(&dirs, vm.as_deref(), live, output),
        VmCommand::Set {
            vm,
            ram,
            cpu_cores,
            disk_size,
            cpu_model,
            cpu_pinning,
            macaddr,
            bridge,
            port_forwards,
            boot_menu,
            boot_once,
            disk_cache,
            disk_aio,
            discard,
        } => set_vm(
            &dirs,
            &vm,
            ram.as_deref(),
            cpu_cores,
            disk_size.as_deref(),
            cpu_model.as_deref(),
            cpu_pinning.as_deref(),
            macaddr.as_deref(),
            bridge.as_deref(),
            &port_forwards,
            boot_menu.as_deref(),
            boot_once.as_deref(),
            disk_cache.as_deref(),
            disk_aio.as_deref(),
            discard.as_deref(),
            output,
        ),
        VmCommand::Plan {
            vm,
            redact,
            options,
        } => plan_vm(&dirs, &vm, &options, output, redact),
        VmCommand::Start {
            vm,
            options,
            wait,
            wait_timeout,
        } => start_vm(&dirs, &vm, &options, wait, wait_timeout, output),
        VmCommand::Ssh { vm, user } => ssh_vm(&dirs, &vm, user.as_deref()),
        VmCommand::View { vm, viewer } => view_vm(&dirs, &vm, viewer.as_deref(), output),
        VmCommand::Stop { vm, timeout, force } => stop_vm(&dirs, &vm, timeout, force, output),
        VmCommand::Kill { vm } => kill_vm(&dirs, &vm, output),
        VmCommand::Logs { vm, lines } => logs_vm(&dirs, &vm, lines as usize, output),
        VmCommand::Restart {
            vm,
            timeout,
            force,
            options,
        } => {
            let mut vm = find(&dirs.vm_dir, &dirs.state_root, &vm)?;
            let _operation_lock = acquire_vm_lock(&vm.paths)?;
            apply_launch_options(&mut vm, &options)?;
            preflight_vm(&vm, output == OutputFormat::Json)?;
            stop_vm_loaded(&vm, timeout, force, output, false)?;
            start_vm_loaded(&vm, output, None)
        }
        VmCommand::Snapshot { vm, action } => snapshot_vm(&dirs, &vm, action, output),
        VmCommand::Cache { action } => cache_vm(&dirs, action, output),
        VmCommand::Backup { vm, destination } => backup_vm(&dirs, &vm, &destination, output),
        VmCommand::Reset { vm, yes } => reset_cloud_vm(&dirs, &vm, yes, output),
        VmCommand::Disk { vm, action } => disk_vm(&dirs, &vm, action, output),
        VmCommand::DeleteDisk { vm, yes } => delete_disk(&dirs, &vm, yes, output),
        VmCommand::DeleteVm { vm, yes } => delete_vm(&dirs, &vm, yes, output),
        VmCommand::Monitor { vm, command } => monitor_vm(&dirs, &vm, &command, output),
        VmCommand::Guest { vm, action } => guest_vm(&dirs, &vm, action, output),
        VmCommand::Shortcut { vm, path } => shortcut_vm(&dirs, &vm, path, output),
        VmCommand::Report => report_host(output),
        VmCommand::Doctor { vm } => doctor(&dirs, vm.as_deref(), output),
        VmCommand::Host { action } => host_action(action, output),
        VmCommand::Get(args) => get::run(&args, &dirs, output),
        VmCommand::Create(args) => get::create(&args, &dirs, output),
        VmCommand::Clone(args) => clone_vm(&dirs, &args, output),
    }
}

fn generate_completions(shell: Shell) -> Result<()> {
    let shell = shell.to_string();
    // Safe: this process uses the variable only to generate the requested script.
    unsafe { env::set_var("VMCTL_COMPLETE", shell) };
    let args = [env::args_os().next().unwrap_or_else(|| "vmctl".into())];
    let current_dir = env::current_dir().ok();
    CompleteEnv::with_factory(Cli::command)
        .var("VMCTL_COMPLETE")
        .try_complete(args, current_dir.as_deref())
        .map_err(|error| Error::message(error.to_string()))?;
    Ok(())
}

#[path = "commands/inventory.rs"]
mod inventory;
use inventory::*;
#[path = "commands/lifecycle.rs"]
mod lifecycle;
use lifecycle::*;
#[path = "commands/storage.rs"]
mod storage;
use storage::*;
#[path = "commands/cache.rs"]
mod cache;
use cache::*;
#[path = "commands/guest.rs"]
mod guest;
use guest::*;
#[path = "commands/desktop.rs"]
mod desktop;
use desktop::*;
#[path = "commands/report.rs"]
mod report;
use report::*;
#[path = "commands/doctor.rs"]
mod doctor;
use doctor::*;
#[path = "commands/diagnostics.rs"]
mod diagnostics;
use diagnostics::*;
#[path = "commands/host.rs"]
mod host;
use host::*;
#[path = "commands/launch.rs"]
mod launch;
use launch::*;
#[path = "commands/presentation.rs"]
mod presentation;
use presentation::*;
#[path = "commands/runtime.rs"]
mod runtime;
use runtime::*;
#[path = "commands/clone.rs"]
mod clone;
use clone::*;

#[cfg(test)]
#[path = "commands/tests.rs"]
mod tests;
