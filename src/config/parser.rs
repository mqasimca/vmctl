use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::paths::default_public_dir;

pub(super) fn validate_disk_format_value(path: &Path, format: &str) -> Result<()> {
    if format.is_empty()
        || format.starts_with('-')
        || format.chars().any(|character| {
            character.is_ascii_control()
                || character.is_ascii_whitespace()
                || !(character.is_ascii_alphanumeric() || ".-_".contains(character))
        })
    {
        return Err(Error::config(
            path,
            format!("disk_format '{format}' contains unsafe QEMU option characters"),
        ));
    }
    Ok(())
}

pub fn parse_config(contents: &str) -> BTreeMap<String, String> {
    contents
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with("#!") {
                return None;
            }
            let line = strip_comment(line);
            let line = line.trim();
            let line = line.strip_prefix("export ").unwrap_or(line);
            let (key, value) = line.split_once('=')?;
            let key = key.trim();
            if key.is_empty()
                || !key
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric() || character == '_')
            {
                return None;
            }
            Some((key.to_string(), unquote(value.trim())))
        })
        .collect()
}

pub fn parse_tokens(value: Option<&String>) -> Vec<String> {
    let Some(value) = value else {
        return Vec::new();
    };
    let value = value.trim();
    let value = value
        .strip_prefix('(')
        .and_then(|value| value.strip_suffix(')'))
        .unwrap_or(value);

    let mut tokens = Vec::new();
    let mut token = String::new();
    let mut quote = None;
    let mut escaped = false;

    for character in value.chars() {
        if escaped {
            token.push(character);
            escaped = false;
            continue;
        }
        if character == '\\' {
            escaped = true;
            continue;
        }
        match (quote, character) {
            (Some(current), character) if character == current => quote = None,
            (Some(_), character) => token.push(character),
            (None, '\'' | '"') => quote = Some(character),
            (None, character) if character.is_whitespace() => {
                if !token.is_empty() {
                    tokens.push(std::mem::take(&mut token));
                }
            }
            (None, character) => token.push(character),
        }
    }
    if escaped {
        token.push('\\');
    }
    if !token.is_empty() {
        tokens.push(token);
    }
    tokens
}

pub(super) fn parse_port_forwards(value: Option<&String>, path: &Path) -> Result<Vec<(u16, u16)>> {
    parse_tokens(value)
        .into_iter()
        .map(|forward| {
            let (host, guest) = forward
                .split_once(':')
                .ok_or_else(|| Error::config(path, format!("invalid port forward '{forward}'")))?;
            let host = parse_port(host, path, "host port")?;
            let guest = parse_port(guest, path, "guest port")?;
            Ok((host, guest))
        })
        .collect()
}

pub(super) fn parse_usb_devices(value: Option<&String>, path: &Path) -> Result<Vec<(u16, u16)>> {
    parse_tokens(value)
        .into_iter()
        .map(|device| {
            let (vendor, product) = device
                .split_once(':')
                .ok_or_else(|| Error::config(path, format!("invalid USB device '{device}'")))?;
            let vendor = u16::from_str_radix(vendor, 16)
                .map_err(|_| Error::config(path, format!("invalid USB vendor in '{device}'")))?;
            let product = u16::from_str_radix(product, 16)
                .map_err(|_| Error::config(path, format!("invalid USB product in '{device}'")))?;
            Ok((vendor, product))
        })
        .collect()
}

fn strip_comment(value: &str) -> &str {
    let mut quote = None;
    let mut escaped = false;
    for (index, character) in value.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if character == '\\' {
            escaped = true;
            continue;
        }
        match (quote, character) {
            (Some(current), character) if current == character => quote = None,
            (None, '\'' | '"') => quote = Some(character),
            (None, '#') => return &value[..index],
            _ => {}
        }
    }
    value
}

fn unquote(value: &str) -> String {
    let value = if value.len() >= 2 {
        let quoted = (value.starts_with('"') && value.ends_with('"'))
            || (value.starts_with('\'') && value.ends_with('\''));
        if quoted {
            &value[1..value.len() - 1]
        } else {
            value
        }
    } else {
        value
    };

    let mut result = String::with_capacity(value.len());
    let mut escaped = false;
    for character in value.chars() {
        if escaped {
            result.push(character);
            escaped = false;
        } else if character == '\\' {
            escaped = true;
        } else {
            result.push(character);
        }
    }
    if escaped {
        result.push('\\');
    }
    result
}

pub(super) fn value_or(values: &BTreeMap<String, String>, key: &str, default: &str) -> String {
    values
        .get(key)
        .cloned()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| default.to_string())
}

pub(super) fn optional_string(values: &BTreeMap<String, String>, key: &str) -> Option<String> {
    values
        .get(key)
        .cloned()
        .filter(|value| !value.is_empty() && value != "none")
}

pub(super) fn optional_path(root: &Path, value: Option<&String>) -> Result<Option<PathBuf>> {
    value
        .filter(|value| !value.is_empty() && value.as_str() != "none")
        .map(|value| resolve_path(root, value))
        .transpose()
}

pub(super) fn parse_public_dir(
    root: &Path,
    values: &BTreeMap<String, String>,
) -> Result<Option<PathBuf>> {
    match values.get("public_dir") {
        Some(value) if value == "none" => Ok(None),
        Some(value) if !value.is_empty() => Ok(Some(resolve_path(root, value)?)),
        _ => default_public_dir(),
    }
}

pub(super) fn resolve_path(root: &Path, value: &str) -> Result<PathBuf> {
    if value == "~" {
        return crate::paths::home_dir();
    }
    if let Some(value) = value.strip_prefix("~/") {
        return Ok(crate::paths::home_dir()?.join(value));
    }
    let path = Path::new(value);
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(root.join(path))
    }
}

fn parse_bool(value: &str) -> Option<bool> {
    match value.to_ascii_lowercase().as_str() {
        "1" | "on" | "true" | "yes" => Some(true),
        "0" | "off" | "false" | "no" => Some(false),
        _ => None,
    }
}

pub(super) fn setting_bool(
    values: &BTreeMap<String, String>,
    key: &str,
    default: bool,
    path: &Path,
) -> Result<bool> {
    values.get(key).map_or(Ok(default), |value| {
        parse_bool(value).ok_or_else(|| Error::config(path, format!("{key} must be true or false")))
    })
}

pub(super) fn optional_bool(
    values: &BTreeMap<String, String>,
    key: &str,
    path: &Path,
) -> Result<Option<bool>> {
    values.get(key).map_or(Ok(None), |value| {
        parse_bool(value)
            .map(Some)
            .ok_or_else(|| Error::config(path, format!("{key} must be true or false")))
    })
}

pub(super) fn optional_u32(
    values: &BTreeMap<String, String>,
    key: &str,
    path: &Path,
) -> Result<Option<u32>> {
    values
        .get(key)
        .filter(|value| !value.is_empty())
        .map_or(Ok(None), |value| {
            value
                .parse()
                .map(Some)
                .map_err(|_| Error::config(path, format!("{key} must be a positive number")))
        })
}

pub(super) fn optional_port(
    values: &BTreeMap<String, String>,
    key: &str,
    path: &Path,
) -> Result<Option<u16>> {
    values
        .get(key)
        .filter(|value| !value.is_empty())
        .map(|value| parse_port(value, path, key))
        .transpose()
}

pub(super) fn parse_port(value: &str, path: &Path, label: &str) -> Result<u16> {
    let port = value
        .parse::<u16>()
        .map_err(|_| Error::config(path, format!("{label} must be a valid port number")))?;
    if port == 0 {
        return Err(Error::config(
            path,
            format!("{label} must be greater than zero"),
        ));
    }
    Ok(port)
}

pub(super) fn validate_one_of(path: &Path, key: &str, value: &str, allowed: &[&str]) -> Result<()> {
    if allowed.contains(&value) {
        Ok(())
    } else {
        Err(Error::config(
            path,
            format!("{key} must be one of {}", allowed.join(", ")),
        ))
    }
}

pub(super) fn valid_host_or_address(value: &str) -> bool {
    if value.parse::<std::net::IpAddr>().is_ok() {
        return true;
    }
    !value.is_empty()
        && value.len() <= 253
        && !value.chars().any(|character| {
            character.is_whitespace()
                || character.is_control()
                || matches!(character, ',' | '=' | '/' | '\\' | '[' | ']')
        })
        && value.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric() || character == '-')
        })
}

pub(super) fn valid_network_name(value: &str) -> bool {
    !value.is_empty()
        && !value.chars().any(|character| {
            character.is_whitespace()
                || character.is_control()
                || matches!(character, ',' | '=' | '/' | '\\')
        })
}
