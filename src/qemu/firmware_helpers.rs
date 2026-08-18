use super::*;

pub(super) fn first_existing(paths: &[&str]) -> Option<PathBuf> {
    paths.iter().map(PathBuf::from).find(|path| path.is_file())
}

#[cfg(test)]
pub(super) fn first_complete_pair(pairs: &[(&str, &str)]) -> Option<(PathBuf, PathBuf)> {
    pairs.iter().find_map(|(code, vars)| {
        let code = Path::new(code);
        let vars = Path::new(vars);
        (code.is_file() && vars.is_file()).then(|| (code.to_path_buf(), vars.to_path_buf()))
    })
}

pub(super) fn firmware_format(path: &Path) -> &'static str {
    let mut magic = [0; 4];
    if File::open(path)
        .and_then(|mut file| file.read_exact(&mut magic))
        .is_ok()
        && magic == [0x51, 0x46, 0x49, 0xfb]
    {
        "qcow2"
    } else {
        "raw"
    }
}

pub(super) fn add(args: &mut Vec<String>, flag: &str, value: String) {
    args.push(flag.to_string());
    args.push(value);
}

pub(super) fn qemu_path(path: &Path) -> String {
    path.display().to_string().replace(',', ",,")
}

pub(super) fn control_endpoint(path: &Path, host_os: &str) -> String {
    if host_os == "windows" {
        #[cfg(windows)]
        return format!("pipe:{}", control_pipe_name(path));
        #[cfg(not(windows))]
        return format!("tcp:127.0.0.1:{},server=on,wait=off", control_port(path));
    }
    #[cfg(unix)]
    {
        format!("unix:{},server=on,wait=off", qemu_path(path))
    }
    #[cfg(not(unix))]
    {
        format!("tcp:127.0.0.1:{},server=on,wait=off", control_port(path))
    }
}

#[cfg(windows)]
pub(super) fn control_pipe_name(path: &Path) -> String {
    let mut hash = 2_166_136_261u32;
    for byte in path.to_string_lossy().bytes() {
        hash ^= u32::from(byte);
        hash = hash.wrapping_mul(16_777_619);
    }
    format!("vmctl-control-{hash:08x}")
}

pub(super) fn socket_chardev(path: &Path, id: &str, host_os: &str) -> String {
    if host_os == "windows" {
        return format!(
            "socket,id={id},host=127.0.0.1,port={},server=off,wait=off",
            control_port(path)
        );
    }
    #[cfg(unix)]
    {
        format!(
            "socket,id={id},path={},server=off,wait=off",
            qemu_path(path)
        )
    }
    #[cfg(not(unix))]
    {
        format!(
            "socket,id={id},host=127.0.0.1,port={},server=off,wait=off",
            control_port(path)
        )
    }
}

pub(super) fn control_port(path: &Path) -> u16 {
    let mut hash = 2_166_136_261u32;
    for byte in path.to_string_lossy().bytes() {
        hash ^= u32::from(byte);
        hash = hash.wrapping_mul(16_777_619);
    }
    40_000 + (hash % 20_000) as u16
}
