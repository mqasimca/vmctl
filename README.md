# vmctl

Host-side CLI for managing QEMU/KVM virtual machines.

Initial scope:

- Quickemu VMs first
- QMP for VM lifecycle and status
- QEMU Guest Agent for commands and guest file operations
- Optional libvirt backend later

This project is intentionally a management layer, not a new hypervisor.
