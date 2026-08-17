# vmctl

Host-side CLI for managing QEMU/KVM virtual machines.

Initial scope:

- Quickemu VMs first
- QMP for VM lifecycle and status
- QEMU Guest Agent for commands and guest file operations
- Optional libvirt backend later

This project is intentionally a management layer, not a new hypervisor.

## Development

```bash
cargo run -- list
cargo run -- status ubuntu-26.04
cargo run -- start ubuntu-26.04
cargo run -- stop ubuntu-26.04
```

By default, `vmctl` looks for Quickemu configurations in `../vms`. Override
that location with `--dir PATH`.
