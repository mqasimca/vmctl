use super::*;
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CacheStatus {
    Hit,
    Miss,
    Refreshed,
}

impl CacheStatus {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Hit => "hit",
            Self::Miss => "miss",
            Self::Refreshed => "refreshed",
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct CachedImage {
    pub(super) path: PathBuf,
    pub(super) sha256: String,
    pub(super) status: CacheStatus,
}

#[derive(Debug, Clone)]
pub(super) struct CachedSource {
    pub(super) path: PathBuf,
    pub(super) os: String,
    pub(super) release: String,
    pub(super) edition: Option<String>,
    pub(super) architecture: String,
    pub(super) kind: ImageKind,
    pub(super) cloud: bool,
    pub(super) ssh_user: Option<String>,
}

struct CacheRequest<'a> {
    url: &'a str,
    file_name: &'a str,
    kind: ImageKind,
    checksum: Option<&'a str>,
    source: Option<&'a Value>,
}

pub(super) fn cache_image(
    root: &Path,
    image: &ResolvedImage,
    insecure: bool,
    refresh: bool,
    cloud: bool,
    ssh_user: Option<&str>,
) -> Result<CachedImage> {
    let source = json!({
        "os": image.os,
        "release": image.release,
        "edition": image.edition,
        "architecture": image.architecture,
        "kind": image_kind_name(image.kind),
        "cloud": cloud,
        "ssh_user": ssh_user,
    });
    cache_request(
        root,
        CacheRequest {
            url: &image.url,
            file_name: &image.file_name,
            kind: image.kind,
            checksum: image.checksum.as_deref(),
            source: Some(&source),
        },
        insecure,
        refresh,
    )
}

pub(super) fn cache_url(
    root: &Path,
    url: &str,
    file_name: &str,
    kind: ImageKind,
    checksum: Option<&str>,
    insecure: bool,
    refresh: bool,
) -> Result<CachedImage> {
    cache_request(
        root,
        CacheRequest {
            url,
            file_name,
            kind,
            checksum,
            source: None,
        },
        insecure,
        refresh,
    )
}

/// Files in the cache object store that no VM references and may be removed.
pub(crate) fn cache_prune_candidates(
    root: &Path,
    referenced: &BTreeSet<String>,
) -> Result<Vec<PathBuf>> {
    let cache = root.join(".cache");
    let objects = cache.join("objects");
    match fs::symlink_metadata(&cache) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(Error::message(format!(
                "refusing to use cache directory symlink {}",
                cache.display()
            )));
        }
        Ok(metadata) if !metadata.is_dir() => {
            return Err(Error::message(format!(
                "cache path is not a directory: {}",
                cache.display()
            )));
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(Error::io(cache.display(), error)),
    }
    match fs::symlink_metadata(&objects) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(Error::message(format!(
                "refusing to use cache directory symlink {}",
                objects.display()
            )));
        }
        Ok(metadata) if !metadata.is_dir() => {
            return Err(Error::message(format!(
                "cache path is not a directory: {}",
                objects.display()
            )));
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(Error::io(objects.display(), error)),
    }
    let index = read_index(&cache.join("index.json"))?;
    let mut indexed = BTreeSet::new();
    let mut candidates = BTreeSet::new();

    for entry in index.values() {
        if let Some((path, sha256)) = cached_entry(&objects, entry)? {
            verify_checksum(&path, Some(&format!("sha256:{sha256}")))?;
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .expect("cache object names are validated UTF-8");
            indexed.insert(name.to_string());
            if !referenced.contains(name) {
                candidates.insert(path);
            }
        }
    }

    let entries = match fs::read_dir(&objects) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(Error::io(objects.display(), error)),
    };
    for entry in entries {
        let entry = entry.map_err(|error| Error::io(objects.display(), error))?;
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let metadata =
            fs::symlink_metadata(&path).map_err(|error| Error::io(path.display(), error))?;
        if name.starts_with(".vmctl-download-")
            || name.ends_with(".lock")
            || metadata.file_type().is_symlink()
            || !metadata.is_file()
        {
            continue;
        }
        if !indexed.contains(name.as_ref()) && !referenced.contains(name.as_ref()) {
            candidates.insert(path);
        }
    }
    Ok(candidates.into_iter().collect())
}

pub(crate) fn cache_lock(root: &Path) -> Result<Option<crate::qemu::FileLock>> {
    let cache = root.join(".cache");
    match fs::symlink_metadata(&cache) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(Error::message(format!(
            "refusing to use cache directory symlink {}",
            cache.display()
        ))),
        Ok(metadata) if !metadata.is_dir() => Err(Error::message(format!(
            "cache path is not a directory: {}",
            cache.display()
        ))),
        Ok(_) => acquire_cache_lock(&cache.join("download.lock")).map(Some),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(Error::io(cache.display(), error)),
    }
}

pub(crate) fn remove_cache_candidates(
    root: &Path,
    candidates: &[PathBuf],
    _lock: &crate::qemu::FileLock,
) -> Result<()> {
    if candidates.is_empty() {
        return Ok(());
    }
    let cache = root.join(".cache");
    let objects = cache.join("objects");
    let mut names = BTreeSet::new();
    for path in candidates {
        if path.parent() != Some(objects.as_path()) {
            return Err(Error::message(format!(
                "refusing to prune a file outside the cache: {}",
                path.display()
            )));
        }
        let metadata =
            fs::symlink_metadata(path).map_err(|error| Error::io(path.display(), error))?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(Error::message(format!(
                "refusing to prune a non-regular cache object: {}",
                path.display()
            )));
        }
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| {
                Error::message(format!(
                    "cache object has no valid name: {}",
                    path.display()
                ))
            })?;
        names.insert(name.to_string());
    }
    for path in candidates {
        fs::remove_file(path).map_err(|error| Error::io(path.display(), error))?;
    }
    let mut index = read_index(&cache.join("index.json"))?;
    index.retain(|_, entry| {
        !entry
            .get("object")
            .and_then(Value::as_str)
            .is_some_and(|object| names.contains(object))
    });
    write_index(&cache.join("index.json"), &index)
}

fn cache_request(
    root: &Path,
    request: CacheRequest<'_>,
    insecure: bool,
    refresh: bool,
) -> Result<CachedImage> {
    let CacheRequest {
        url,
        file_name,
        kind,
        checksum,
        source,
    } = request;
    let cache = root.join(".cache");
    let objects = cache.join("objects");
    ensure_cache_directory(&cache)?;
    ensure_cache_directory(&objects)?;
    let lock_path = cache.join("download.lock");
    let _lock = acquire_cache_lock(&lock_path)?;
    let index_path = cache.join("index.json");
    let mut index = read_index(&index_path)?;
    if !refresh
        && let Some(entry) = index.get(url)
        && let Some((path, sha256)) = cached_entry(&objects, entry)?
    {
        verify_checksum(&path, Some(&format!("sha256:{sha256}")))?;
        return Ok(CachedImage {
            path,
            sha256,
            status: CacheStatus::Hit,
        });
    }

    let download = objects.join(format!(
        ".vmctl-download-{}{}",
        std::process::id(),
        if file_name.ends_with(".xz") {
            ".xz"
        } else {
            ""
        }
    ));
    if fs::symlink_metadata(&download).is_ok() {
        return Err(Error::message(format!(
            "temporary cache download already exists: {}",
            download.display()
        )));
    }
    let result = (|| {
        download_file(url, &download, insecure)?;
        verify_checksum(&download, checksum)?;
        let compressed_disk = kind == ImageKind::Disk && file_name.ends_with(".xz");
        let temporary = if compressed_disk {
            prepare_image(&download)?
        } else {
            download.clone()
        };
        let sha256 = checksum_digest(&temporary, "sha256")?;
        let object_name = cache_object_name(
            if compressed_disk {
                file_name.strip_suffix(".xz").unwrap_or(file_name)
            } else {
                file_name
            },
            kind,
            &sha256,
        )?;
        let object = objects.join(&object_name);
        if fs::symlink_metadata(&object).is_ok() {
            verify_checksum(&object, Some(&format!("sha256:{sha256}")))?;
            fs::remove_file(&temporary).map_err(|error| Error::io(temporary.display(), error))?;
        } else {
            fs::rename(&temporary, &object).map_err(|error| Error::io(object.display(), error))?;
        }
        index.insert(
            url.to_string(),
            json!({
                "object": object_name,
                "sha256": sha256,
                "checksum": checksum,
                "size": fs::metadata(&object).map_err(|error| Error::io(object.display(), error))?.len(),
                "source": source,
            }),
        );
        write_index(&index_path, &index)?;
        Ok(CachedImage {
            path: object,
            sha256,
            status: if refresh {
                CacheStatus::Refreshed
            } else {
                CacheStatus::Miss
            },
        })
    })();
    if result.is_err() {
        let _ = fs::remove_file(&download);
        let _ = fs::remove_file(download.with_extension(""));
    }
    result
}

pub(super) fn cached_source(root: &Path, object: &str) -> Result<CachedSource> {
    if object.contains('/') || object.contains('\\') || object.starts_with('.') {
        return Err(Error::invalid_argument(
            "--from",
            "use a cached image file name, not a path",
        ));
    }
    let objects = root.join(".cache/objects");
    let index = read_index(&root.join(".cache/index.json"))?;
    let entry = index
        .values()
        .find(|entry| entry.get("object").and_then(Value::as_str) == Some(object))
        .ok_or_else(|| {
            Error::message(format!(
                "cached image not found: {object}; run `vmctl get` first"
            ))
        })?;
    let (path, sha256) = cached_entry(&objects, entry)?.ok_or_else(|| {
        Error::message(format!(
            "cached image is incomplete: {object}; run `vmctl get --refresh-cache`"
        ))
    })?;
    verify_checksum(&path, Some(&format!("sha256:{sha256}")))?;
    let source = entry
        .get("source")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            Error::message(format!(
                "cached image lacks source metadata: {object}; run `vmctl get --refresh-cache`"
            ))
        })?;
    let required = |field| {
        source
            .get(field)
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| {
                Error::message(format!(
                    "cached image has invalid {field} metadata: {object}"
                ))
            })
    };
    let kind = match required("kind")?.as_str() {
        "iso" => ImageKind::Iso,
        "img" => ImageKind::Img,
        "disk" => ImageKind::Disk,
        "archive" => ImageKind::Archive,
        _ => {
            return Err(Error::message(format!(
                "cached image has invalid kind metadata: {object}"
            )));
        }
    };
    Ok(CachedSource {
        path,
        os: required("os")?,
        release: required("release")?,
        edition: source
            .get("edition")
            .and_then(Value::as_str)
            .map(str::to_string),
        architecture: required("architecture")?,
        kind,
        cloud: source
            .get("cloud")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        ssh_user: source
            .get("ssh_user")
            .and_then(Value::as_str)
            .map(str::to_string),
    })
}

fn read_index(path: &Path) -> Result<serde_json::Map<String, Value>> {
    if fs::symlink_metadata(path)
        .ok()
        .is_some_and(|metadata| metadata.file_type().is_symlink())
    {
        return Err(Error::message(format!(
            "refusing to read cache index symlink {}",
            path.display()
        )));
    }
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Default::default()),
        Err(error) => return Err(Error::io(path.display(), error)),
    };
    let value: Value = serde_json::from_str(&contents).map_err(|error| {
        Error::message(format!("invalid cache index {}: {error}", path.display()))
    })?;
    value
        .get("entries")
        .and_then(Value::as_object)
        .cloned()
        .ok_or_else(|| {
            Error::message(format!(
                "invalid cache index {}: entries is missing",
                path.display()
            ))
        })
}

fn write_index(path: &Path, entries: &serde_json::Map<String, Value>) -> Result<()> {
    if fs::symlink_metadata(path)
        .ok()
        .is_some_and(|metadata| metadata.file_type().is_symlink())
    {
        return Err(Error::message(format!(
            "refusing to replace cache index symlink {}",
            path.display()
        )));
    }
    let contents = serde_json::to_vec_pretty(&json!({"version": 1, "entries": entries}))
        .expect("JSON values are serializable");
    crate::qemu::write_atomic_file(path, &contents)
}

fn ensure_cache_directory(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(Error::message(format!(
            "refusing to use cache directory symlink {}",
            path.display()
        ))),
        Ok(metadata) if metadata.is_dir() => Ok(()),
        Ok(_) => Err(Error::message(format!(
            "cache path is not a directory: {}",
            path.display()
        ))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir(path).map_err(|error| Error::io(path.display(), error))
        }
        Err(error) => Err(Error::io(path.display(), error)),
    }
}

fn cached_entry(objects: &Path, entry: &Value) -> Result<Option<(PathBuf, String)>> {
    let object = entry.get("object").and_then(Value::as_str);
    let sha256 = entry.get("sha256").and_then(Value::as_str);
    let Some((object, sha256)) = object.zip(sha256) else {
        return Ok(None);
    };
    if object.contains('/') || object.contains('\\') || object.starts_with('.') {
        return Err(Error::message("cache index contains an unsafe object path"));
    }
    let path = objects.join(object);
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(Error::io(path.display(), error)),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(Error::message(format!(
            "cached image is not a regular file: {}",
            path.display()
        )));
    }
    Ok(Some((path, sha256.to_string())))
}

fn cache_object_name(file_name: &str, kind: ImageKind, sha256: &str) -> Result<String> {
    if sha256.len() < 12
        || !sha256
            .chars()
            .all(|character| character.is_ascii_hexdigit())
    {
        return Err(Error::message(
            "cache digest is not a SHA-256 hexadecimal value",
        ));
    }
    let path = Path::new(file_name);
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("image");
    let extension = if kind == ImageKind::Disk {
        "qcow2".to_string()
    } else {
        let outer = path
            .extension()
            .and_then(|value| value.to_str())
            .filter(|value| !value.is_empty())
            .unwrap_or("image");
        if kind == ImageKind::Archive
            && ["gz", "bz2", "xz"]
                .iter()
                .any(|extension| outer.eq_ignore_ascii_case(extension))
        {
            path.with_extension("")
                .extension()
                .and_then(|value| value.to_str())
                .map(|inner| format!("{inner}.{outer}"))
                .unwrap_or_else(|| outer.to_string())
        } else {
            outer.to_string()
        }
    };
    let suffix = format!(".{extension}");
    let stem = file_name
        .strip_suffix(&suffix)
        .or_else(|| path.file_stem().and_then(|value| value.to_str()))
        .unwrap_or("image");
    let mut stem: String = stem
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-') {
                character
            } else {
                '-'
            }
        })
        .collect();
    while stem.contains("--") {
        stem = stem.replace("--", "-");
    }
    let stem = stem.trim_matches(['.', '-']).to_string();
    if stem.is_empty() {
        return Err(Error::message("image file name has no safe cache label"));
    }
    Ok(format!("{stem}--sha256-{}.{}", &sha256[..12], extension))
}

fn acquire_cache_lock(path: &Path) -> Result<crate::qemu::FileLock> {
    crate::qemu::acquire_file_lock(path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::WouldBlock {
            Error::message("another vmctl operation is using this cache; retry when it finishes")
        } else {
            Error::io(path.display(), error)
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn cache_names_are_readable_and_unique() {
        assert_eq!(
            cache_object_name(
                "ubuntu-26.04-live-server-arm64.iso",
                ImageKind::Iso,
                &"a".repeat(64)
            )
            .unwrap(),
            "ubuntu-26.04-live-server-arm64--sha256-aaaaaaaaaaaa.iso"
        );
        assert_eq!(
            cache_object_name(
                "ubuntu-26.04-server-cloudimg-arm64.img",
                ImageKind::Disk,
                &"b".repeat(64)
            )
            .unwrap(),
            "ubuntu-26.04-server-cloudimg-arm64--sha256-bbbbbbbbbbbb.qcow2"
        );
        assert_eq!(
            cache_object_name(
                "FreeBSD-15.1-RELEASE-amd64-BASIC-CLOUDINIT-ufs.qcow2",
                ImageKind::Disk,
                &"c".repeat(64)
            )
            .unwrap(),
            "FreeBSD-15.1-RELEASE-amd64-BASIC-CLOUDINIT-ufs--sha256-cccccccccccc.qcow2"
        );
        let archive =
            cache_object_name("batocera.img.gz", ImageKind::Archive, &"d".repeat(64)).unwrap();
        assert_eq!(archive, "batocera--sha256-dddddddddddd.img.gz");
        assert_eq!(
            image_kind(&Path::new(&archive).with_extension("").to_string_lossy()),
            ImageKind::Img
        );
    }

    #[test]
    fn cached_entry_is_reused_without_downloading() {
        let root = tempdir().unwrap();
        let objects = root.path().join(".cache/objects");
        fs::create_dir_all(&objects).unwrap();
        let object = objects.join("ubuntu-26.04-desktop-arm64--sha256-000000000000.iso");
        fs::write(&object, "cached image").unwrap();
        let sha256 = checksum_digest(&object, "sha256").unwrap();
        let file_name =
            cache_object_name("ubuntu-26.04-desktop-arm64.iso", ImageKind::Iso, &sha256).unwrap();
        fs::rename(&object, objects.join(&file_name)).unwrap();
        fs::write(
            root.path().join(".cache/index.json"),
            serde_json::to_vec(&json!({"version": 1, "entries": {
                "https://example.invalid/ubuntu.iso": {"object": file_name, "sha256": sha256}
            }}))
            .unwrap(),
        )
        .unwrap();
        let cached = cache_url(
            root.path(),
            "https://example.invalid/ubuntu.iso",
            "ubuntu-26.04-desktop-arm64.iso",
            ImageKind::Iso,
            None,
            false,
            false,
        )
        .unwrap();
        assert_eq!(cached.status, CacheStatus::Hit);
        assert!(cached.path.is_file());
    }

    #[test]
    fn cached_source_reads_verified_metadata() {
        let root = tempdir().unwrap();
        let objects = root.path().join(".cache/objects");
        fs::create_dir_all(&objects).unwrap();
        let object = objects.join("ubuntu-24.04-desktop-amd64--sha256-000000000000.iso");
        fs::write(&object, "cached image").unwrap();
        let sha256 = checksum_digest(&object, "sha256").unwrap();
        let object =
            cache_object_name("ubuntu-24.04-desktop-amd64.iso", ImageKind::Iso, &sha256).unwrap();
        fs::rename(
            objects.join("ubuntu-24.04-desktop-amd64--sha256-000000000000.iso"),
            objects.join(&object),
        )
        .unwrap();
        fs::write(
            root.path().join(".cache/index.json"),
            serde_json::to_vec(&json!({"version": 1, "entries": {
                "https://example.invalid/ubuntu.iso": {
                    "object": object,
                    "sha256": sha256,
                    "source": {"os": "ubuntu", "release": "24.04", "edition": "desktop", "architecture": "amd64", "kind": "iso", "cloud": false, "ssh_user": null}
                }
            }}))
            .unwrap(),
        )
        .unwrap();
        let source = cached_source(root.path(), &object).unwrap();
        assert_eq!(source.os, "ubuntu");
        assert_eq!(source.kind, ImageKind::Iso);
        assert!(!source.cloud);
    }

    #[test]
    fn missing_cached_object_is_reported_as_incomplete() {
        let root = tempdir().unwrap();
        fs::create_dir_all(root.path().join(".cache")).unwrap();
        fs::write(
            root.path().join(".cache/index.json"),
            serde_json::to_vec(&json!({"version": 1, "entries": {
                "https://example.invalid/ubuntu.iso": {
                    "object": "missing.iso",
                    "sha256": "00".repeat(32)
                }
            }}))
            .unwrap(),
        )
        .unwrap();
        let error = cached_source(root.path(), "missing.iso").unwrap_err();
        assert!(error.to_string().contains("cached image is incomplete"));
    }

    #[test]
    fn cache_lock_recovers_after_exit_without_ignoring_a_live_holder() {
        let root = tempdir().unwrap();
        let path = root.path().join("download.lock");
        fs::write(&path, "stale marker").unwrap();
        let lock = acquire_cache_lock(&path).unwrap();
        assert!(acquire_cache_lock(&path).is_err());
        drop(lock);
        assert!(acquire_cache_lock(&path).is_ok());
    }

    #[test]
    fn cache_index_replaces_an_existing_file_and_cleans_failed_temporary_files() {
        let root = tempdir().unwrap();
        let path = root.path().join("index.json");
        write_index(
            &path,
            &serde_json::Map::from_iter([("old".into(), json!(1))]),
        )
        .unwrap();
        write_index(
            &path,
            &serde_json::Map::from_iter([("new".into(), json!(2))]),
        )
        .unwrap();
        assert_eq!(read_index(&path).unwrap().get("new"), Some(&json!(2)));

        let blocked = root.path().join("blocked.json");
        fs::create_dir(&blocked).unwrap();
        assert!(write_index(&blocked, &serde_json::Map::new()).is_err());
        assert!(
            !blocked
                .with_extension(format!("json.{}.tmp", std::process::id()))
                .exists()
        );
    }

    #[cfg(unix)]
    #[test]
    fn cache_index_ignores_the_old_predictable_temporary_path() {
        use std::os::unix::fs::symlink;

        let root = tempdir().unwrap();
        let path = root.path().join("index.json");
        let victim = root.path().join("victim");
        let old_temporary = path.with_extension(format!("json.{}.tmp", std::process::id()));
        fs::write(&victim, b"original").unwrap();
        symlink(&victim, old_temporary).unwrap();

        write_index(
            &path,
            &serde_json::Map::from_iter([("safe".into(), json!(true))]),
        )
        .unwrap();
        assert_eq!(fs::read(victim).unwrap(), b"original");
        assert_eq!(read_index(&path).unwrap().get("safe"), Some(&json!(true)));
    }

    #[test]
    fn cache_prune_lists_unreferenced_indexed_and_orphan_objects_only() {
        let root = tempdir().unwrap();
        let objects = root.path().join(".cache/objects");
        fs::create_dir_all(&objects).unwrap();
        let kept = "kept.iso";
        let stale = "stale.iso";
        let orphan = "orphan.iso";
        let referenced_orphan = "referenced-orphan.iso";
        fs::write(objects.join(kept), "kept").unwrap();
        fs::write(objects.join(stale), "stale").unwrap();
        fs::write(objects.join(orphan), "orphan").unwrap();
        fs::write(objects.join(referenced_orphan), "referenced orphan").unwrap();
        fs::write(objects.join(".vmctl-download-123"), "temporary").unwrap();
        fs::write(objects.join("download.lock"), "lock").unwrap();
        let entries = [(kept, "kept"), (stale, "stale")]
            .into_iter()
            .map(|(name, contents)| {
                (
                    format!("https://example.invalid/{name}"),
                    json!({
                        "object": name,
                        "sha256": checksum_digest(&objects.join(name), "sha256").unwrap(),
                        "size": contents.len(),
                    }),
                )
            })
            .collect();
        write_index(&root.path().join(".cache/index.json"), &entries).unwrap();

        let candidates = cache_prune_candidates(
            root.path(),
            &BTreeSet::from([kept.to_string(), referenced_orphan.to_string()]),
        )
        .unwrap();
        assert_eq!(candidates, vec![objects.join(orphan), objects.join(stale)]);
        assert!(objects.join(stale).is_file());
        assert!(objects.join(referenced_orphan).is_file());
    }

    #[test]
    fn cache_prune_rejects_invalid_index() {
        let root = tempdir().unwrap();
        let objects = root.path().join(".cache/objects");
        fs::create_dir_all(&objects).unwrap();
        fs::write(root.path().join(".cache/index.json"), "not JSON").unwrap();

        assert!(cache_prune_candidates(root.path(), &BTreeSet::new()).is_err());
    }

    #[test]
    fn removing_pruned_cache_objects_removes_their_index_entries() {
        let root = tempdir().unwrap();
        let objects = root.path().join(".cache/objects");
        fs::create_dir_all(&objects).unwrap();
        let object = objects.join("stale.iso");
        fs::write(&object, "stale").unwrap();
        write_index(
            &root.path().join(".cache/index.json"),
            &serde_json::Map::from_iter([(
                "https://example.invalid/stale.iso".to_string(),
                json!({
                    "object": "stale.iso",
                    "sha256": checksum_digest(&object, "sha256").unwrap(),
                }),
            )]),
        )
        .unwrap();

        let lock = cache_lock(root.path()).unwrap().unwrap();
        remove_cache_candidates(root.path(), std::slice::from_ref(&object), &lock).unwrap();
        assert!(!object.exists());
        assert!(
            read_index(&root.path().join(".cache/index.json"))
                .unwrap()
                .is_empty()
        );
    }

    #[cfg(unix)]
    #[test]
    fn cache_prune_rejects_object_directory_symlinks() {
        use std::os::unix::fs::symlink;

        let root = tempdir().unwrap();
        let outside = tempdir().unwrap();
        fs::create_dir(root.path().join(".cache")).unwrap();
        symlink(outside.path(), root.path().join(".cache/objects")).unwrap();

        assert!(
            cache_prune_candidates(root.path(), &BTreeSet::new())
                .unwrap_err()
                .to_string()
                .contains("cache directory symlink")
        );
    }
}
