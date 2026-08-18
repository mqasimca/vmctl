use super::*;

pub(super) fn print_image(image: &ResolvedImage, output: OutputFormat, available: Option<bool>) {
    if output == OutputFormat::Json {
        crate::print_json_success(json!({
            "os": image.os,
            "release": image.release,
            "edition": image.edition,
            "architecture": image.architecture,
            "url": image.url,
            "file_name": image.file_name,
            "kind": image_kind_name(image.kind),
            "checksum": image.checksum,
            "available": available,
        }));
    } else if let Some(available) = available {
        println!(
            "{}: {} {}",
            if available { "PASS" } else { "FAIL" },
            image.os,
            image.url
        );
    } else {
        println!("{}", image.url);
    }
}

pub(super) fn print_check_result(
    os: &str,
    release: &str,
    edition: Option<&str>,
    architecture: &str,
    available: bool,
    error: &Error,
) {
    let suffix = edition.map(|value| format!("-{value}")).unwrap_or_default();
    let detail = if available {
        String::new()
    } else {
        format!(" - image URL unavailable ({error})")
    };
    println!(
        "{}: {}-{}{} ({architecture}){}",
        if available { "PASS" } else { "FAIL" },
        os,
        release,
        suffix,
        detail,
    );
}

pub(super) fn check_result_json(
    os: &str,
    release: &str,
    edition: Option<&str>,
    architecture: &str,
    available: bool,
    error: Option<String>,
) -> Value {
    json!({
        "os": os,
        "release": release,
        "edition": edition,
        "architecture": architecture,
        "available": available,
        "error": error,
    })
}

pub(super) fn info_json(info: &OsInfo) -> Value {
    json!({
        "name": info.name,
        "os": info.id,
        "homepage": info.homepage,
        "guest_os": info.guest_os,
        "architectures": info.architectures.split_whitespace().collect::<Vec<_>>(),
        "releases": info.releases.split_whitespace().collect::<Vec<_>>(),
        "editions": info.editions.split_whitespace().collect::<Vec<_>>(),
    })
}
