# vmctl

`vmctl` is a Rust CLI for running and managing QEMU/KVM VMs from existing
VM `.conf` files. Configurations are parsed as data; they are never sourced or
executed.

```text
vmctl list                         # list local VMs
vmctl status VM                    # inspect one VM and its runtime state
vmctl plan VM --output json        # inspect the exact QEMU invocation
vmctl plan VM --output json --redact # omit sensitive inline values
vmctl start VM                     # create missing disk/EFI state and start
vmctl start VM --wait ssh          # start and wait for the guest SSH banner
vmctl start VM --ssh-access remote # explicitly expose SSH beyond localhost
vmctl start VM --clipboard         # enable GTK host-guest clipboard sharing
vmctl start VM --viewer-extra-args --foo value
vmctl ssh VM --user USER            # connect through the active SSH forward
vmctl view VM                        # open a SPICE console for a running VM
vmctl stop VM [--force]            # graceful QMP shutdown, then optional kill
vmctl kill VM                      # immediately terminate a running VM
vmctl restart VM [--force]         # stop, then start again
vmctl logs VM --lines 100           # inspect a redacted QEMU log tail
vmctl guest VM ping                # use the QEMU Guest Agent
vmctl guest VM shutdown --timeout 30
vmctl guest VM exec --timeout 30 PROGRAM ARG...
vmctl snapshot VM create TAG       # stopped-disk snapshot
vmctl disk VM info                 # qemu-img disk metadata
vmctl disk VM resize 32G           # grow a stopped disk
vmctl disk VM check                 # stopped-disk integrity check
vmctl disk VM convert VM.raw --format raw
vmctl disk VM compact --yes         # rewrite and reclaim sparse space
vmctl delete-disk VM --yes          # delete a VM disk and its UEFI variables
vmctl delete-vm VM --yes            # delete the configuration, disk, and runtime state
vmctl monitor VM info block        # send an HMP command
vmctl report                       # host capability report
vmctl doctor                       # read-only host readiness checks
vmctl doctor VM --output json      # machine-readable VM diagnostics
vmctl host ignore-msrs-always      # persist KVM MSR handling (Linux)
vmctl shortcut VM                  # create a desktop launcher
vmctl get --list                    # list supported OS images
vmctl get freebsd                   # list current FreeBSD releases and media options
vmctl get ubuntu 24.04              # download an image and create a VM config
vmctl get --create-config NAME IMAGE_OR_URL
```

## Requirements

The host needs a QEMU system binary matching the VM architecture, `qemu-img`,
and firmware files for EFI guests. Linux hosts use `/dev/kvm` when available
and automatically fall back to software emulation when it is not.

Install the complete host toolchain below before building or running `vmctl`.

### Ubuntu 24.04

This installs the full feature set, including x86_64 and ARM guests, GTK/SDL
display, SPICE, TPM, virtiofs, SMB sharing, image downloads, and Windows
unattended-media creation:

```bash
sudo apt update
sudo apt install -y \
  build-essential \
  ca-certificates curl coreutils openssh-client procps util-linux \
  qemu-system-x86 qemu-system-arm qemu-utils qemu-system-gui \
  qemu-system-modules-spice qemu-system-modules-opengl \
  ovmf qemu-efi-aarch64 \
  virt-viewer spice-client-gtk \
  swtpm virtiofsd samba usbutils xdg-user-dirs passt \
  gzip bzip2 unzip 7zip xorriso
```

The project uses Rust 1.85 or newer because it targets edition 2024. Install
the stable toolchain with `rustup` if it is not already available:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"
rustup default stable
```

### Debian

Debian bundles some QEMU modules and `virtiofsd` in its QEMU packages, so use
the Debian package set rather than the Ubuntu-only module package names:

```bash
sudo apt update
sudo apt install -y \
  build-essential \
  ca-certificates curl coreutils openssh-client procps util-linux \
  qemu-system-x86 qemu-system-arm qemu-utils qemu-system-gui \
  ovmf qemu-efi-aarch64 \
  virt-viewer spice-client-gtk \
  swtpm samba usbutils xdg-user-dirs passt \
  gzip bzip2 unzip 7zip xorriso
```

Install Rust stable with the `rustup` commands shown above. On older Debian
releases, `p7zip-full` can be used instead of `7zip`.

### Arch Linux / CachyOS

```bash
sudo pacman -Syu --needed \
  base-devel rustup \
  ca-certificates curl coreutils openssh procps-ng util-linux \
  qemu-full edk2-ovmf edk2-aarch64 \
  virt-viewer spice-gtk \
  swtpm virtiofsd samba usbutils xdg-user-dirs passt \
  gzip bzip2 unzip 7zip cdrtools libisoburn
rustup default stable
```

### Fedora 43+

```bash
sudo dnf install -y \
  @development-tools rustup \
  ca-certificates curl coreutils openssh-clients procps-ng util-linux \
  qemu-system-x86 qemu-system-aarch64 qemu-img \
  qemu-ui-gtk qemu-ui-sdl qemu-ui-opengl qemu-ui-spice-core qemu-ui-spice-app \
  edk2-ovmf edk2-aarch64 \
  virt-viewer spice-gtk \
  swtpm virtiofsd samba usbutils xdg-user-dirs passt \
  gzip bzip2 unzip 7zip xorriso
rustup default stable
```

### openSUSE Tumbleweed

```bash
sudo zypper install -y \
  gcc make pkg-config rustup \
  ca-certificates curl coreutils openssh procps util-linux \
  qemu qemu-arm qemu-utils qemu-ovmf-x86_64 qemu-uefi-aarch64 \
  virt-viewer spice-gtk \
  swtpm virtiofsd samba usbutils xdg-user-dirs passt \
  gzip bzip2 unzip 7zip xorriso
rustup default stable
```

`network=passt` needs both QEMU 10.1 or newer and the `passt` executable.
Fedora 43+ and Tumbleweed package both. On a distribution or QEMU build that
does not meet those requirements, keep the default `network=user`; `vmctl
doctor` reports the exact missing prerequisite.

On Linux, add your user to the `kvm` group when `/dev/kvm` exists, then log
out and back in:

```bash
sudo usermod -aG kvm "$USER"
```

For `guest` commands, GTK clipboard sharing, and SPICE clipboard/resize integration, install these inside
an Ubuntu or Debian Linux guest (they are not host dependencies):

```bash
sudo apt update
sudo apt install -y qemu-guest-agent spice-vdagent spice-webdavd
sudo systemctl enable --now qemu-guest-agent
```

`guest exec` returns the guest PID, exit status, decoded `stdout`/`stderr`,
and the original base64 fields in JSON. UTF-8 output is available in the
decoded field; binary output uses a `null` decoded field plus
`stdout_base64`/`stderr_base64` and `*_encoding: "base64"`. A non-zero exit status or signal is a
`guest_command_failed` error; an unavailable or unresponsive agent is reported
as `guest_agent_unavailable` with an installation hint. Use `--` before
arguments that look like vmctl options, for example
`vmctl guest VM exec /bin/sh -- -c 'echo hello'`.
If the guest process starts but does not finish before the timeout, vmctl
returns `guest_command_timeout` with the guest PID and timeout; connection or
protocol failures before a guest PID exists use their corresponding agent
error.
`guest shutdown --timeout SECONDS` returns `guest_shutdown_timeout` when QEMU
does not exit or enter its `shutdown` state before the deadline.

`vmctl ssh VM` opens OpenSSH on the VM's active forwarded port; use
`--user USER` (or `-l USER`) when the guest login differs from the host user.
It requires a running VM with `network=user` or `network=passt` and an SSH
service inside the guest. Use `vmctl start VM --wait ssh` to wait up to 120
seconds for the guest SSH banner; override that with `--wait-timeout SECONDS`.
It does not read or write known-host files, so rebuilt VMs do not
cause stale-key conflicts. This means the VM host key is not authenticated;
use plain `ssh` with its explicit port when host-key verification is required.

`network=passt` is an opt-in, unprivileged Linux alternative to the default
user-mode network. It gives the guest current host networking, including IPv6,
while vmctl forwards only its SSH port and configured `port_forwards`. QEMU
starts `passt`; do not start a separate daemon. A `passt` VM cannot use the
QEMU SMB share for Windows guests, so use `network=user` when that share is
needed.

`qemu-bridge-helper` is supplied by the QEMU packages; bridged networking may
also require a permitted host bridge. `vmctl doctor VM` verifies that a configured
bridge exists on Linux and that the helper is installed without changing networking.
It cannot verify the helper's bridge policy without attempting a VM start.
`xorriso` is used for Windows
unattended media and can be replaced by `mkisofs` or `genisoimage`.

These optional programs enable additional features:

- `swtpm` for virtual TPM 2.0 devices
- `spicy` or `remote-viewer` for SPICE display sessions
- `virtiofsd` and a QEMU build with `vhost-user-fs-pci` for Linux virtiofs
  sharing
- `usbutils` for clear USB pass-through preflight checks
- `qemu-bridge-helper` for bridged networking

`remote-viewer` is the default SPICE client. Set `viewer="spicy"` to use the
alternative GTK client explicitly.

Build from source with a current stable Rust toolchain:

```bash
cargo build --release
./target/release/vmctl --help
./target/release/vmctl report

# Build and install to ~/.local/bin/vmctl
make install
```

## Shell completion

Generate and install the script for your shell, then open a new terminal:

```bash
# Bash
mkdir -p ~/.local/share/bash-completion/completions
vmctl completion bash > ~/.local/share/bash-completion/completions/vmctl

# Zsh
mkdir -p ~/.zfunc
vmctl completion zsh > ~/.zfunc/_vmctl
# Add these once to ~/.zshrc:
fpath=(~/.zfunc $fpath)
autoload -Uz compinit && compinit

# Fish
mkdir -p ~/.config/fish/completions
vmctl completion fish > ~/.config/fish/completions/vmctl.fish
```

`vmctl completion --help` lists every supported shell, including PowerShell
and Elvish. The command only writes the completion script to standard output.
The generated script queries vmctl at tab time, so VM-name suggestions stay
current and honor `--dir PATH`. Regenerate the script after upgrading vmctl.

Use `--dir PATH` for the directory containing `.conf` files and `--state-dir
PATH` for runtime state. `--output json` is available on command paths that
return structured data. `plan` does not start the managed VM; it may run short
QEMU capability probes. `start` creates missing disk/EFI state; disk, delete,
stop, and get operations modify only the explicitly requested targets.
Keep a custom state directory private to the account running vmctl; do not use
a shared or untrusted directory for runtime state.

## Configuration files

`vmctl` reads `.conf` files as data. It does not source them, execute shell
code, or rewrite them. Relative paths are resolved from the directory
containing the configuration file.

A small Linux configuration can look like this:

```ini
guest_os="linux"
arch="x86_64"
disk_img="ubuntu/disk.qcow2"
iso="ubuntu/ubuntu.iso"
ram="8G"
cpu_cores="4"
display="gtk"
public_dir="none"
```

Existing compatible configurations can be used unchanged. `vmctl` applies
safe defaults for omitted values and reports invalid modes, ports, addresses,
and other unsafe settings before starting QEMU.

VM runtime state is kept separate from VM data by default:

```text
~/.local/state/vmctl/vms/<name>/
  qemu.command  qemu.log  ports  vm.pid
  qmp.sock      monitor.sock  agent.sock  serial.sock  ipc.json
  swtpm.sock    swtpm.pid     virtiofs.sock  virtiofs.sock.pid  virtiofsd.pid
```

Persistent VM data stays beside the configured disk. Existing files are not
moved or rewritten.

`stop` requests a guest shutdown through QMP and waits for the configured
timeout. Add `--force` to terminate QEMU if graceful shutdown is unavailable.
`restart` accepts the same `--timeout` and `--force` controls before starting
the VM again.
`kill` terminates it immediately. Runtime sockets and helper-process PIDs are
removed after a successful stop.

Disk management is explicit and safe by default. `disk info` is read-only;
`resize`, `check`, `convert`, and `compact` require a stopped VM. Shrinking
requires `--shrink --yes`, repairs require `--repair --yes`, conversion refuses
to overwrite an existing file unless `--force` is supplied, and compaction
requires `--yes` because it replaces the image and does not preserve internal
snapshots.

On Linux, a Linux VM with an existing disk and a configured `public_dir` uses
virtiofs when both `virtiofsd` and the QEMU device are available. Installer
media uses the compatible 9p share instead. If virtiofsd cannot start, vmctl
prints a warning and retries the VM with 9p. macOS guests use the compatible
9p share directly; inside the guest, run `sudo mount_9p Public-<host-user>`
using the sanitized host username shown in `vmctl plan VM --output json`.
SPICE WebDAV requires `spice-webdavd` inside Linux or Windows guests.
Windows and Windows Server guests use QEMU's SMB share when `network=user`, an
existing `public_dir` is configured, and Samba is available; filesystem share
devices are not attached to Windows.

`spice-app` starts the configured SPICE viewer through vmctl and uses non-GL
virtio graphics by default for reliable boot and display output. Set `gl="on"`
in the VM configuration only when the host GPU and guest virgl support have
been verified.

SPICE, monitor Telnet, and serial Telnet listeners bind to localhost by
default. vmctl rejects unauthenticated non-loopback listeners; use an explicit
local bind address, or pass `--allow-insecure-remote` only when a trusted
network or external authentication layer protects the listener. Safety-critical
QEMU options such as `-qmp`, `-pidfile`, `-drive`, `-device`, and `-netdev` are
rejected in `extra_args`; use the corresponding configuration fields instead.

`get` supports catalog listing, OS metadata, stable image URL generation, live
provider resolution, resumable downloads, published SHA-256/SHA-512
verification, URL checks, archive extraction, and safe config creation.
macOS recovery media and Windows Server downloads are resolved directly;
consumer Windows downloads can be rejected by Microsoft's anti-automation
service, in which case the CLI gives the browser/manual-import path.

Windows VM creation also downloads VirtIO drivers and builds unattended
installation media by default. Pass `--disable-unattended` to skip that media.
Use `--create-config NAME IMAGE_OR_URL` for a manually downloaded image or a
provider that requires browser authentication.

`get --insecure` (or `VMCTL_INSECURE=1`) disables TLS certificate
verification for URL checks and media downloads. This is unsafe and should be
used only on a trusted network.

`vmctl get OS` shows image options without downloading. For FreeBSD it queries
the official release directory for current releases; use
`vmctl get freebsd RELEASE EDITION` (where EDITION is `disc1` or `dvd1`) to create a VM. GTK clipboard sharing
is disabled by default: set `clipboard="on"` in a GTK VM configuration or pass
`vmctl start VM --clipboard`. It requires QEMU 11.1 or newer and the guest
`spice-vdagent` service.

## Diagnostics

Use these commands when a VM does not start:

```bash
vmctl report
vmctl doctor
vmctl doctor VM
vmctl --output json status VM
vmctl plan VM --output json
```

The doctor command never starts, stops, or modifies a VM. It checks host
dependencies, KVM access, display clients, media paths, the QEMU plan, runtime
IPC reachability, and the bounded tail of `qemu.log`. Its JSON result has a stable
`schema_version`, check IDs, `ok|warn|error|skip` statuses, and remediation
hints.

`report --output json` includes separate x86_64 and aarch64 QEMU capability
matrices: compiled and runtime accelerators, display and network backends,
optional device support candidates, CPU models, and probe completeness. `doctor`
reports the `host.network.passt` and `vm.network.passt` prerequisites. `plan` and `start` validate the
selected CPU model/features and display backend before launching QEMU; when
hardware acceleration is unavailable, the plan reports the TCG fallback.

For a running VM, `status --output json` also queries QMP and returns
`qmp_status.status` (for example `running` or `paused`) or a bounded error when
the process exists but its QMP endpoint is unavailable.

Use `--output json` for automation. Successful diagnostics are written to
stdout; failures are written as one JSON error object to stderr and return a
non-zero exit status. Error objects include a stable error code, message,
context, and (for `doctor`) the complete diagnostic report. Human output is
intended for terminals and is not a machine-readable interface.

Each VM state directory contains `qemu.log` and `qemu.command`. Startup
failures identify the QEMU log; silent failures also identify the saved
command. `vmctl logs VM --output json` returns a bounded, redacted tail for
automation; diagnostic log snippets are capped and redact common credential
fields.

## Development

```bash
make verify                 # format, compile, test, lint, docs, release build
cargo doc --no-deps --all-features
cargo run -- report
```
