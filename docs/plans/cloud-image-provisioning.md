# Cloud-image provisioning plan

Status: implemented. The current workflow is `vmctl get --cloud OS RELEASE`,
then `vmctl create VM --from IMAGE --ssh-key PATH`; examples below are
historical unless they use that split.

## Historical proposal

Add an opt-in `vmctl get --cloud` workflow for Ubuntu, Debian, Fedora, and
FreeBSD. `get` caches the image; `create` provisions a bootable,
SSH-key-provisioned VM without an installer UI:

```bash
vmctl get --cloud ubuntu 24.04
vmctl create ubuntu-01 --from <cached-image> --ssh-key ~/.ssh/id_ed25519.pub
vmctl start ubuntu-01 --wait ssh
vmctl ssh ubuntu-01
```

Each VM receives a verified immutable base image, a writable QCOW2 overlay,
and a NoCloud seed ISO. It remains a regular vmctl configuration and uses the
existing `start`, `plan`, `doctor`, `ssh`, `disk`, and JSON interfaces.

## Goals

- Remove the installer, account-creation, and manual SSH-key setup steps for
  common Linux development and agent VMs.
- Keep the downloaded base immutable; guest writes go only to the VM overlay.
- Provision access without passwords, private keys, or credentials in config,
  logs, plan output, or JSON output.
- Use official HTTPS sources and verify every downloaded base against an
  upstream cryptographic checksum.
- Make every generated artifact and next command explicit in human and JSON
  output.

## Non-goals for the first release

- No support for installer ISOs, FreeBSD, Windows, macOS, or arbitrary cloud
  providers in this path.
- No password generation, password injection, or private-key handling.
- No automatic cloud-init completion wait: it would require guest access and
  cloud images do not reliably include `qemu-guest-agent`.
- No shared global image cache, image garbage collector, clone command,
  live backup, or networking redesign.
- No YAML merge engine. Arbitrary custom `user-data` comes after the safe
  key-only path is proven.

## Public CLI

Extend `get` with these options:

```text
--cloud
    Create a cloud-image VM. Valid only for Ubuntu, Debian, or Fedora VM
    creation; it cannot be combined with --download or --create-config.

--ssh-key PATH
    Inject one OpenSSH public key into the provider's default cloud user.
    Repeatable. At least one key is required when --cloud creates a VM.

--hostname NAME
    Optional guest hostname. Defaults to the generated VM name.

--network-config PATH
    Optional cloud-init network-config document copied to the NoCloud seed.
```

Examples:

```bash
# Create an SSH-ready Ubuntu VM using the default generated VM name.
vmctl get --cloud --ssh-key ~/.ssh/id_ed25519.pub ubuntu 24.04

# Add two operators and provide a static network configuration.
vmctl get --cloud \
  --ssh-key ~/.ssh/id_ed25519.pub \
  --ssh-key ~/.ssh/work.pub \
  --hostname build-agent \
  --network-config ./network-config.yaml \
  debian 13

# Inspect the exact official source without creating files.
vmctl get --cloud --url fedora 44
```

`--cloud --url` and `--cloud --check` are read-only. `--cloud` VM creation is
state-changing and retains the existing explicit `--insecure` warning and
behavior.

### Authentication and SSH behavior

The generated cloud-config contains only `ssh_authorized_keys`; it does not
enable password SSH or write a password. The provider default user is recorded
in the VM config as `ssh_user`, so `vmctl ssh NAME` works without `--user`:

| Provider | Default user |
| --- | --- |
| Ubuntu | `ubuntu` |
| Debian generic | `debian` |
| Fedora Cloud | `fedora` |

Existing configurations keep their current SSH behavior: when `ssh_user` is
absent, `vmctl ssh` leaves user selection to OpenSSH.

`start --wait ssh` means the SSH service accepted a connection. It is not a
claim that all cloud-init modules or user scripts have completed.

## Provider contract

Initial providers are intentionally limited to sources with QEMU-ready cloud
images and published checksums:

| Provider | Artifact | Verification |
| --- | --- | --- |
| Ubuntu | server cloud image (`.img`, QCOW2 content) for amd64/arm64 | `SHA256SUMS` from `cloud-images.ubuntu.com` |
| Debian | `generic` QCOW2 image for amd64/arm64 | `SHA512SUMS` from `cloud.debian.org` |
| Fedora | Cloud Base QEMU QCOW2 image for amd64/arm64 | published SHA-256 checksum |

Provider-specific resolvers must return an explicit image format (`qcow2`),
URL, filename, architecture, provider default SSH user, and checksum. The
overlay command must always pass `-F qcow2`; QEMU must never probe a backing
file format.

v1 downloads the manifest and image over official HTTPS, then verifies the
downloaded digest. Signature verification of provider manifests is a future
hardening task because portable host keyring management is not currently part
of vmctl.

## On-disk layout and lifecycle

All cloud artifacts live in the existing per-VM data directory. The base is
kept per VM in v1, avoiding a new cache, locking protocol, and garbage
collector.

```text
<vm-dir>/ubuntu-24.04.conf
<vm-dir>/ubuntu-24.04/
  base.qcow2       # verified and never written by vmctl or QEMU
  disk.qcow2       # writable overlay; QEMU system disk
  seed.iso         # NoCloud ISO, volume label CIDATA
```

The overlay has a relative backing reference to `base.qcow2`, making the VM
directory movable as a unit. `delete-vm --yes` removes this directory as it
does today. `delete-disk --yes` removes the overlay and UEFI variables only;
the base and seed remain so a later reset feature can recreate the overlay.

Generated configuration:

```ini
guest_os="linux"
arch="x86_64"
disk_img="ubuntu-24.04/disk.qcow2"
disk_format="qcow2"
cloud_base_img="ubuntu-24.04/base.qcow2"
cloud_init_iso="ubuntu-24.04/seed.iso"
ssh_user="ubuntu"
```

New optional config fields are `cloud_base_img`, `cloud_init_iso`, and
`ssh_user`. `cloud_base_img` and `cloud_init_iso` use the existing relative,
data-only path parsing. A cloud-init ISO is not installer media and must not
disable the existing virtiofs path for Linux guests.

## NoCloud seed contents

The ISO builder stages private temporary files and writes an ISO with volume
label `CIDATA`:

```text
user-data
meta-data
network-config   # only when --network-config was supplied
```

`user-data` is generated, not shell-evaluated:

```yaml
#cloud-config
ssh_authorized_keys:
  - <validated public key>
```

`meta-data` contains a unique, non-secret instance ID and the requested
hostname. A new VM always gets a new instance ID. The ISO remains attached on
later boots; cloud-init's once-per-instance behavior and fixed instance ID
prevent first-boot actions from being rerun. vmctl will not detach media based
on a guest-side guess.

`network-config` is copied as a separate NoCloud file. v1 validates that it is
a regular, bounded file but does not reinterpret its YAML.

The ISO builder is a small shared helper extracted from the existing Windows
unattended-media implementation. It prefers `xorriso`, then `mkisofs`, then
`genisoimage`, uses a caller-specified volume label, refuses symlink targets,
and atomically publishes the finished ISO. `doctor` reports the availability
of one of these tools and explains how to install it.

## Safety and failure handling

- Require at least one readable, regular OpenSSH public-key file. Reject empty
  inputs, private-key markers, control characters, and files above a small
  public-key limit.
- Require every destination to be absent before work starts: config, VM data
  directory, base, overlay, and seed. Never overwrite or follow a symlink.
- Download to a private temporary file, verify its checksum, then rename it to
  `base.qcow2`. Remove temporary and newly created cloud artifacts if a later
  creation step fails.
- Create `disk.qcow2` with explicit QCOW2 backing format and verify its backing
  chain before writing the config last.
- Require the seed ISO and base image to be regular files before start. A
  missing base reports the specific path and recovery command rather than a
  generic QEMU disk error.
- Preserve all current `--insecure` semantics: it is opt-in, warned in human
  output, and never disables checksum verification.
- Do not put SSH key text, custom network configuration, or any secret-like
  contents in errors, logs, plan output, or JSON result objects.

## JSON and agent contract

All successful cloud operations continue to use the current envelope:

```json
{
  "schema_version": 1,
  "ok": true,
  "result": {
    "name": "ubuntu-24.04",
    "source": {"provider": "ubuntu", "url": "…", "checksum": "sha256:…"},
    "base_image": "…/base.qcow2",
    "overlay": "…/disk.qcow2",
    "seed_iso": "…/seed.iso",
    "ssh_user": "ubuntu",
    "ssh_key_count": 1,
    "config": "…/ubuntu-24.04.conf",
    "next": ["vmctl start ubuntu-24.04 --wait ssh", "vmctl ssh ubuntu-24.04"]
  }
}
```

`--url` returns source metadata and the provider's default user without local
paths. `--check` returns source reachability plus the expected checksum. The
raw SSH keys and network configuration are never returned. Existing error
codes remain stable; add focused codes only when an agent needs to distinguish
unsupported provider, invalid key, missing ISO builder, checksum mismatch, or
unsafe destination.

## Implementation sequence

1. Add CLI flags and validation in `src/cli.rs` and `src/get/commands.rs`.
   Keep normal installer `get` behavior untouched.
2. Add a small cloud-provider resolver module under `src/get/` for the three
   official sources and checksum manifests. It returns only typed metadata;
   no provider HTML parsing or generic mirror fallback.
3. Extract generic ISO creation from `src/get/windows.rs` into a shared
   `src/get/iso.rs`. Add the NoCloud staging and public-key validation helpers.
4. Add safe overlay creation to `src/qemu/disk.rs`: explicit base format,
   regular-file checks, temporary output, and backing-chain verification.
5. Add the three config fields and validation in `src/config.rs`; write cloud
   configurations in `src/get/config_writer.rs`.
6. Attach `cloud_init_iso` as a non-bootable read-only CD-ROM in
   `src/qemu/storage.rs` for x86_64 and aarch64. Update the generated QEMU plan
   tests.
7. Make `ssh_vm` and human start output use optional `ssh_user`; expose it in
   status JSON and human status.
8. Extend `doctor` with host ISO-builder availability and VM base/seed checks.
   Add the cloud result fields to README and `vmctl schema` documentation.
9. Add tests, run `make verify`, cross-compile for Windows, then create a real
   Ubuntu cloud VM and exercise the full SSH lifecycle.

## Test plan

### Unit tests

- Parse all new CLI combinations and reject invalid flag combinations.
- Resolve each provider using static manifest fixtures; cover amd64, arm64,
  unsupported releases, missing checksums, and non-HTTPS URLs.
- Validate public-key files and ensure private-key-like input is rejected.
- Verify generated `user-data`, `meta-data`, and optional `network-config`
  filenames and content without printing their data.
- Verify generated cloud config uses relative paths and `ssh_user`.
- Verify overlay commands set `-f qcow2`, `-F qcow2`, a relative backing file,
  and never overwrite an existing target.
- Verify x86_64 and aarch64 QEMU plans attach the seed ISO as read-only media
  without changing disk boot order or virtiofs eligibility.
- Verify doctor reports missing ISO builder, base, and seed with actionable
  hints.
- Verify JSON results are wrapped by the current agent envelope and omit key
  material.

### End-to-end verification on Linux

```bash
make verify
cargo check --target x86_64-pc-windows-gnu --all-targets --all-features
vmctl get --cloud --ssh-key ~/.ssh/id_ed25519.pub ubuntu 24.04
vmctl doctor ubuntu-24.04
vmctl plan ubuntu-24.04 --output json
vmctl start ubuntu-24.04 --wait ssh
vmctl ssh ubuntu-24.04
vmctl stop ubuntu-24.04
qemu-img info --backing-chain <vm-dir>/ubuntu-24.04/disk.qcow2
```

Repeat the smoke test for Debian and Fedora. Confirm that a missing ISO builder,
bad checksum, invalid key, unavailable provider, and colliding VM name each
leave no partial config or runnable VM.

## Follow-up work, only after v1 proves useful

- Optional raw `--user-data` support with an explicit mutually exclusive mode
  instead of unsafe YAML merging.
- A cloud-init completion wait that uses an explicitly installed guest agent or
  an SSH command, reporting unsupported images clearly.
- A shared image cache with locks, reference accounting, and an explicit prune
  command.
- Overlay reset/clone commands and signed-manifest verification with portable
  provider keyrings.

## References

- [cloud-init NoCloud datasource](https://cloudinit.readthedocs.io/en/19.3/topics/datasources/nocloud.html)
- [cloud-init SSH-key configuration](https://cloudinit.readthedocs.io/en/stable/reference/yaml_examples/ssh.html)
- [Ubuntu public cloud image artifacts](https://documentation.ubuntu.com/public-images/public-images-reference/artifacts/)
- [Ubuntu image checksum verification](https://documentation.ubuntu.com/public-images/public-images-how-to/verify-image-checksum/)
- [Debian cloud-image guidance](https://wiki.debian.org/Cloud)
- [Fedora Cloud downloads and verification](https://fedoraproject.org/cloud/download/)
- [QEMU backing-file safety](https://qemu.readthedocs.io/en/v9.2.4/tools/qemu-img.html)
