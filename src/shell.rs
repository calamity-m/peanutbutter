use std::path::PathBuf;

/// Return the preferred Bash executable for shell-evaluated commands.
///
/// On Windows, prefer Git for Windows' Bash over the WSL shim named `bash.exe`,
/// because the shim exits unsuccessfully when no WSL distribution is installed.
pub(crate) fn bash_command() -> PathBuf {
    #[cfg(windows)]
    {
        for var in ["ProgramFiles", "ProgramFiles(x86)"] {
            if let Some(path) = std::env::var_os(var) {
                let candidate = PathBuf::from(path).join("Git").join("bin").join("bash.exe");
                if candidate.exists() {
                    return candidate;
                }
            }
        }
    }

    PathBuf::from("bash")
}
