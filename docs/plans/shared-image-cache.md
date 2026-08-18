# Shared Image Cache and Cloud Clones

Status: implemented. The current workflow is `vmctl get` to cache media, then
`vmctl create VM --from IMAGE` to create a VM. `IMAGE` completes from the
verified shared cache.

## Outcome

`vmctl get` downloads a given image only once per VM directory. `vmctl create`
uses the verified cached object and gives each VM its own disk. Cloud VMs
default to a linked QCOW2 clone; users can explicitly request a full
independent copy.

```bash
# Download once.
vmctl get --cloud ubuntu 24.04

# Create linked overlays without downloading again.
vmctl create web-01 --from <cached-image> --ssh-key ~/.ssh/id_ed25519.pub
vmctl create worker-01 --from <cached-image> --ssh-key ~/.ssh/id_ed25519.pub

# Independent, movable disk.
vmctl create test-01 --from <cached-image> --disk-mode copy \
  --ssh-key ~/.ssh/id_ed25519.pub
```

Use Tab to complete `<cached-image>`.

## Historical design notes

### One cache per `--dir`

Store the cache under the VM root, not in a second global location:

```text
<vm-dir>/
  .cache/
    objects/
      ubuntu-26.04-live-server-arm64--sha256-8ca3d718f25e.iso
      ubuntu-26.04-desktop-arm64--sha256-5fa0decb7912.iso
      ubuntu-26.04-server-cloudimg-arm64--sha256-4d3b8a716c20.qcow2
    index.json
    locks/
  web-01.conf
  web-01/
    disk.qcow2
    seed.iso
```

This preserves the meaning of `--dir`: moving or backing up one VM root moves
its image cache too. The VM inventory already ignores directories without
`.conf` files, so `.cache` is not listed as a VM.

Do not use symlinks. Config paths remain regular relative paths and all cache
and VM destinations reject symlinks.

### Human-readable, verified object names

Every cached download gets a locally calculated SHA-256 digest. Its cache name
uses the resolver's descriptive source filename plus the first 12 digest
characters before the extension:

```text
<safe-source-stem>--sha256-<first-12-hex>.<extension>
```

For example:

```text
ubuntu-26.04-live-server-arm64--sha256-8ca3d718f25e.iso
ubuntu-26.04-desktop-arm64--sha256-5fa0decb7912.iso
ubuntu-26.04-server-cloudimg-arm64--sha256-4d3b8a716c20.qcow2
Fedora-Cloud-Base-Generic-44-1.7.x86_64--sha256-28680fe5b371.qcow2
```

The name tells a person what the image is; the digest suffix makes collisions
impossible in normal use. `index.json` remains authoritative and stores the
full SHA-256, canonical source URL, optional upstream checksum, byte size, and
the exact cache filename. A 12-character suffix is presentation only; vmctl
always compares the complete digest.

The generic resolver already supplies `file_name`; use that as the label after
rejecting path separators, controls, and unsafe characters. Preserve source
case where it is meaningful. Cloud sources known to contain QCOW2 data are
labelled `.qcow2` even when an upstream URL ends in `.img`; all other images
retain their source extension. This is a naming decision, not a format probe.

On a cache hit, vmctl verifies the object against the recorded local SHA-256
before using it. When upstream publishes a checksum, vmctl verifies that too
on the initial download. Thus a cache hit requires no network request but
still detects local corruption.

`--refresh-cache` bypasses a matching index entry, redownloads the source,
verifies it, and atomically replaces that URL's index entry. `--check` always
checks upstream and never changes the cache.

This covers VM-creating `get` downloads, including normal installer ISOs,
cloud images, and managed Windows support media. Plain `get --download` keeps
its current contract of writing directly to the current directory.

### Cloud clone modes

Add this `get` option, valid only with cloud VM creation:

```text
--disk-mode <linked|copy>    [default: linked]
```

| Mode | Disk | Best for |
| --- | --- | --- |
| `linked` | A new QCOW2 overlay backed by the shared cache object | Default: fast creation and minimal storage |
| `copy` | A full QCOW2 copy converted from the shared cache object | A self-contained VM disk or independent portability |

`linked` is an external QCOW2 snapshot, not a QEMU internal snapshot. It is
the correct default: QEMU guarantees the backing file is not written by normal
overlay use, and relative backing paths are resolved relative to the overlay.
The overlay records a relative reference such as
`../.cache/objects/ubuntu-26.04-server-cloudimg-arm64--sha256-4d3b8a716c20.qcow2`;
it remains portable as a complete VM root.

The per-VM NoCloud seed remains unique. It is never shared because it contains
the VM hostname, unique instance ID, and the caller's public-key selection.

## Lifecycle

Generated cloud configs point `cloud_base_img` at the shared cache object:

```ini
disk_img="web-01/disk.qcow2"
cloud_base_img=".cache/objects/ubuntu-26.04-server-cloudimg-arm64--sha256-4d3b8a716c20.qcow2"
cloud_init_iso="web-01/seed.iso"
```

- `delete-disk --yes` removes only the VM's writable disk and UEFI variables.
- `delete-vm --yes` removes only that VM's config, private data, and runtime
  state; it never removes a shared object.
- A later `vmctl cache prune --yes` scans parsed configs, removes only
  unreferenced cache objects, then atomically rewrites `index.json`.
- Cache pruning is a separate follow-up command, never an implicit side effect
  of deleting a VM.

## Implementation sequence

1. Add `--name`, `--disk-mode`, and `--refresh-cache` to `GetArgs`, with
   strict operation validation. `--name` changes the generated config and data
   directory name; `--disk-mode` defaults to `linked` and is cloud-only.
2. Add a small `src/get/cache.rs` module. It owns cache layout, safe display
   filename construction from the existing resolver `file_name`, URL index
   parsing/writing with existing `serde_json`, SHA-256 calculation through the
   existing host checksum helper, and private temporary downloads. No new crate
   or background cache daemon is needed.
3. Publish an object only after downloading to a same-directory temporary file,
   calculating SHA-256, and verifying the upstream checksum when present.
   Publish the object and `index.json` with same-filesystem renames. A
   create-new per-object lock prevents concurrent duplicate downloads; a
   second process waits briefly, then reports the active cache lock clearly.
4. Route VM-creating normal-image, cloud-image, and managed Windows media
   downloads through the cache. Preserve `--download` direct output and do not
   cache user-provided local paths.
5. Update cloud overlay creation to accept a checked relative backing path
   rather than requiring base and overlay to share a directory. It continues to
   pass `-F qcow2`, rejects symlinks, creates atomically, and verifies the
   recorded backing chain.
6. Implement `linked` with the shared backing object and `copy` with
   `qemu-img convert -O qcow2` into the VM directory. Both paths create the
   VM config last.
7. Include `cache: { status: hit|miss|refreshed, object, sha256 }` and disk
   mode in cloud JSON output; human output says either `Using cached
   <human-name> …` or `Downloaded <human-name> …`.
8. Add tests for cache hits, cache corruption, missing/invalid indexes,
   concurrent-lock errors, source-name sanitization, digest suffix collisions,
   URL refresh, linked backing paths, full copies, and delete isolation. Run
   one real cloud VM twice with distinct names and confirm only the first
   invocation makes a download.
9. Add `vmctl cache prune --yes` only after the cache creation path has proved
   stable. It must scan parsed configurations, reject symlinks, report every
   candidate in JSON/human output, and never delete an object referenced by a
   config.

## Explicit non-goals for the first cache release

- No cross-`--dir` global cache, remote cache, cache daemon, hard links, or
  reflinks. They add platform-specific ownership and deletion behaviour without
  improving the normal single-root workflow.
- No automatic pruning or cache-size eviction. Deleting a VM must stay
  predictable.
- No internal QEMU snapshots. Linked QCOW2 overlays solve the cloning problem
  with clearer lifecycle and existing disk tooling.

## Verification

- Unit tests use a temporary cache and local objects; no network dependency.
- Integration test creates `web-01` and `worker-01` from one cloud image,
  asserts one cache object, distinct overlays/seeds, and the same backing
  digest.
- Boot both VMs, verify their injected SSH key, stop them, and use `qemu-img
  info --backing-chain` to verify the linked clone.
- Create a `--disk-mode copy` VM, assert it has no backing file, then boot it.
- Run `make verify`, `git diff --check`, and release-binary help/JSON smoke
  tests.

## Rationale

QEMU documents that `qemu-img create -b` records only differences and does not
modify the backing file; it also resolves relative backing paths relative to
the overlay. That makes a shared immutable cache object plus per-VM overlays
the smallest safe design. [QEMU qemu-img documentation](https://qemu.readthedocs.io/en/v9.2.4/tools/qemu-img.html)
