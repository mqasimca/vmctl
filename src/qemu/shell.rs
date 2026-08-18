pub fn shell_join(binary: &str, args: &[String]) -> String {
    let mut command = String::new();
    write_shell_quoted(binary, &mut command);
    for argument in args {
        command.push(' ');
        write_shell_quoted(argument, &mut command);
    }
    command
}

pub(super) fn write_shell_quoted(value: &str, output: &mut String) {
    if !value.is_empty()
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "_./:=,-".contains(character))
    {
        output.push_str(value);
        return;
    }
    output.push('\'');
    output.push_str(&value.replace('\'', "'\\''"));
    output.push('\'');
}
