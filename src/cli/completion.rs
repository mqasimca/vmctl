use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};

use clap_complete::engine::CompletionCandidate;

pub(super) fn complete_vm_names(current: &OsStr) -> Vec<CompletionCandidate> {
    vm_name_candidates(&completion_vm_dir(), current)
}

pub(super) fn complete_cached_images(current: &OsStr) -> Vec<CompletionCandidate> {
    let Some(current) = current.to_str() else {
        return Vec::new();
    };
    cached_image_candidates(&completion_vm_dir().join(".cache/objects"), current)
        .into_iter()
        .map(CompletionCandidate::new)
        .collect()
}

fn cached_image_candidates(dir: &Path, current: &str) -> Vec<String> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut images = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            path.is_file()
                .then(|| {
                    entry
                        .file_name()
                        .to_str()
                        .filter(|name| !name.starts_with('.'))
                        .map(str::to_string)
                })
                .flatten()
        })
        .filter(|name| name.starts_with(current))
        .collect::<Vec<_>>();
    images.sort();
    images
}

fn completion_vm_dir() -> PathBuf {
    let args = std::env::args_os().skip(1).collect::<Vec<_>>();
    completion_vm_dir_from_args(&args)
        .unwrap_or_else(|| crate::paths::default_vm_dir().unwrap_or_default())
}

fn completion_vm_dir_from_args(args: &[OsString]) -> Option<PathBuf> {
    let args = args
        .iter()
        .position(|arg| arg == "--")
        .map_or(args, |index| &args[index + 1..]);
    let mut dir = None;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        if arg == "-d" || arg == "--dir" || arg == "--vm-dir" {
            dir = args.next().cloned().map(PathBuf::from);
        } else if let Some(value) = arg.to_str().and_then(|arg| {
            arg.strip_prefix("--dir=")
                .or_else(|| arg.strip_prefix("--vm-dir="))
        }) {
            dir = Some(PathBuf::from(value));
        } else if let Some(value) = arg
            .to_str()
            .and_then(|arg| arg.strip_prefix("-d").filter(|value| !value.is_empty()))
        {
            dir = Some(PathBuf::from(value));
        }
    }
    dir
}

fn vm_name_candidates(dir: &Path, current: &OsStr) -> Vec<CompletionCandidate> {
    let Some(current) = current.to_str() else {
        return Vec::new();
    };
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut names = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            if path.is_file()
                && path
                    .extension()
                    .is_some_and(|extension| extension == "conf")
            {
                path.file_stem().and_then(OsStr::to_str).map(str::to_string)
            } else {
                None
            }
        })
        .filter(|name| name.starts_with(current))
        .collect::<Vec<_>>();
    names.sort();
    names.into_iter().map(CompletionCandidate::new).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vm_name_completion_lists_matching_config_stems() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("alpha.conf"), []).unwrap();
        fs::write(root.path().join("beta.conf"), []).unwrap();
        fs::write(root.path().join("ignored.txt"), []).unwrap();

        let candidates = vm_name_candidates(root.path(), OsStr::new("a"));
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].get_value(), OsStr::new("alpha"));
    }

    #[test]
    fn vm_name_completion_honors_dir_arguments() {
        let args = [
            OsString::from("--"),
            OsString::from("vmctl"),
            OsString::from("start"),
            OsString::from("--dir"),
            OsString::from("/tmp/vmctl-vms"),
            OsString::new(),
        ];
        assert_eq!(
            completion_vm_dir_from_args(&args),
            Some(PathBuf::from("/tmp/vmctl-vms"))
        );
    }

    #[test]
    fn cached_image_completion_lists_objects() {
        let root = tempfile::tempdir().unwrap();
        let objects = root.path().join(".cache/objects");
        fs::create_dir_all(&objects).unwrap();
        fs::write(objects.join("freebsd.qcow2"), []).unwrap();
        fs::write(objects.join("ubuntu.iso"), []).unwrap();

        let candidates = cached_image_candidates(&objects, "free");
        assert_eq!(candidates, vec!["freebsd.qcow2"]);
    }
}
