use super::*;

pub(super) fn create_iso(
    source: &Path,
    destination: &Path,
    volume_label: Option<&str>,
) -> Result<()> {
    let builder = ["mkisofs", "genisoimage", "xorriso"]
        .into_iter()
        .find(|command| command_exists(command))
        .ok_or_else(|| {
            Error::message("creating ISO media requires mkisofs, genisoimage, or xorriso")
        })?;
    let mut command = Command::new(builder);
    if builder == "xorriso" {
        command.args(["-as", "mkisofs"]);
    }
    command.args(["-quiet", "-R", "-J"]);
    if let Some(label) = volume_label {
        command.args(["-V", label]);
    }
    let status = command
        .arg("-o")
        .arg(destination)
        .arg(source)
        .status()
        .map_err(|error| Error::command_unavailable(builder, error))?;
    if status.success() {
        Ok(())
    } else {
        Err(Error::command_failed_status(builder, status))
    }
}
