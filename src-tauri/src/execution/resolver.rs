//! Windows executable resolution for the bounded runner.
//!
//! Two gaps in bare `Command::new("name")` on Windows are closed here:
//!
//! 1. **Stale process PATH.** The GUI app inherits its environment at login
//!    time, so PATH entries added afterwards (rustup's `%USERPROFILE%\.cargo\bin`,
//!    nvm-windows switches) are invisible. The registry is the source of truth:
//!    machine + user `Path` values are read fresh via the Registry API and
//!    merged ahead of the process PATH, for both lookup and the child's own
//!    `PATH`.
//! 2. **PATHEXT.** `CreateProcess` only auto-appends `.exe`, so extension-less
//!    names like `npm` (which is `npm.cmd`) never resolve. Lookup walks each
//!    PATH directory trying every `PATHEXT` extension, like the shell does.
//!
//! Security notes: only bare allowlisted basenames reach this module (policy
//! rejects paths), and unlike `CreateProcess` the workspace cwd is never
//! searched, so a planted executable cannot shadow an approved command.

use std::path::{Path, PathBuf};

/// Default extension order when `PATHEXT` is unset (matches Windows).
const DEFAULT_PATHEXT: &str = ".COM;.EXE;.BAT;.CMD";

/// Expands `%VAR%` references against the current process environment.
/// Unknown or unterminated references are left verbatim.
fn expand_percent_vars(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('%') {
            Some(end) => {
                match std::env::var(&after[..end]) {
                    Ok(value) => out.push_str(&value),
                    // Keep the literal %VAR% text when unknown.
                    Err(_) => out.push_str(&rest[start..=start + end + 1]),
                }
                rest = &after[end + 1..];
            }
            None => {
                out.push('%');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Splits a PATH-style string into non-empty entries.
fn split_path_entries(value: &str) -> Vec<PathBuf> {
    value
        .split(';')
        .filter(|entry| !entry.trim().is_empty())
        .map(PathBuf::from)
        .collect()
}

/// Reads the raw `Path` value from one registry hive's environment key via
/// the Registry API (no subprocess, no parsing). `None` when the key or value
/// does not exist.
#[cfg(windows)]
fn registry_raw_path(subkey: &str) -> Option<String> {
    use windows::core::PCWSTR;
    use windows::Win32::System::Registry::{
        RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE,
        KEY_READ,
    };

    fn to_wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    let hive = if subkey.starts_with("HKEY_LOCAL_MACHINE") {
        HKEY_LOCAL_MACHINE
    } else {
        HKEY_CURRENT_USER
    };
    let sub = subkey.split_once('\\').map(|x| x.1).unwrap_or(subkey);
    let sub_wide = to_wide(sub);
    let name_wide = to_wide("Path");

    unsafe {
        let mut key = HKEY::default();
        if RegOpenKeyExW(hive, PCWSTR(sub_wide.as_ptr()), Some(0), KEY_READ, &mut key).is_err() {
            return None;
        }

        // Size query first, then one bounded read.
        let mut byte_len = u32::MAX;
        let status = RegQueryValueExW(
            key,
            PCWSTR(name_wide.as_ptr()),
            None,
            None,
            None,
            Some(&mut byte_len),
        );
        if status.is_err() || byte_len == u32::MAX || byte_len == 0 || byte_len % 2 != 0 {
            let _ = RegCloseKey(key);
            return None;
        }
        let mut buffer = vec![0u8; byte_len as usize];
        let mut actual = byte_len;
        let status = RegQueryValueExW(
            key,
            PCWSTR(name_wide.as_ptr()),
            None,
            None,
            Some(buffer.as_mut_ptr()),
            Some(&mut actual),
        );
        let _ = RegCloseKey(key);
        if status.is_err() {
            return None;
        }
        buffer.truncate(actual as usize);

        let wide: Vec<u16> = buffer
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .take_while(|&unit| unit != 0)
            .collect();
        Some(String::from_utf16_lossy(&wide))
    }
}

/// Machine + user `Path` read fresh from the registry (percent-expanded), so
/// PATH edits made after this process started are honored.
#[cfg(windows)]
fn registry_path_dirs() -> Vec<PathBuf> {
    const MACHINE_ENV: &str =
        r"HKEY_LOCAL_MACHINE\SYSTEM\CurrentControlSet\Control\Session Manager\Environment";
    const USER_ENV: &str = r"HKEY_CURRENT_USER\Environment";
    [MACHINE_ENV, USER_ENV]
        .iter()
        .filter_map(|subkey| registry_raw_path(subkey))
        .flat_map(|raw| split_path_entries(&expand_percent_vars(&raw)))
        .collect()
}

/// Directories searched for executables: registry-fresh machine PATH, then
/// user PATH, then the process PATH (covers parent-injected entries the
/// registry does not know about). Deduplicated case-insensitively, first
/// occurrence wins.
pub fn search_dirs() -> Vec<PathBuf> {
    let process_dirs = split_path_entries(&std::env::var("PATH").unwrap_or_default());
    #[cfg(windows)]
    let mut dirs = registry_path_dirs();
    #[cfg(not(windows))]
    let mut dirs = Vec::new();
    dirs.extend(process_dirs);
    let mut seen: Vec<String> = Vec::with_capacity(dirs.len());
    dirs.retain(|dir| {
        let key = dir.to_string_lossy().to_ascii_lowercase();
        if seen.contains(&key) {
            return false;
        }
        seen.push(key);
        true
    });
    dirs
}

/// The merged search path as a single `PATH` string for child environments.
pub fn merged_path_string() -> Option<String> {
    let dirs = search_dirs();
    if dirs.is_empty() {
        return None;
    }
    Some(
        dirs.iter()
            .map(|dir| dir.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join(";"),
    )
}

/// Pure lookup: the first existing `dir\name{ext}` for `dir` in `dirs` (PATH
/// order wins across directories) and `ext` in `exts` (PATHEXT order wins
/// within a directory). `name` must be a bare stem with no extension.
fn find_executable_in_dirs(name: &str, dirs: &[PathBuf], exts: &[String]) -> Option<PathBuf> {
    for dir in dirs {
        for ext in exts {
            let candidate = dir.join(format!("{name}{ext}"));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

/// Resolves an allowlisted bare command name to a concrete executable path.
/// `None` means "not found anywhere searchable" — the runner turns that into
/// a precise error instead of CreateProcess's opaque `program not found`.
pub fn resolve_executable(name: &str) -> Option<PathBuf> {
    let exts: Vec<String> = match std::env::var("PATHEXT") {
        Ok(value) if !value.trim().is_empty() => value,
        _ => DEFAULT_PATHEXT.to_string(),
    }
    .split(';')
    .filter(|ext| !ext.is_empty())
    .map(str::to_string)
    .collect();
    let dirs = search_dirs();
    #[cfg(windows)]
    {
        find_executable_in_dirs(name, &dirs, &exts)
    }
    #[cfg(not(windows))]
    {
        let _ = exts;
        dirs.iter()
            .map(|dir| dir.join(name))
            .find(|candidate| candidate.is_file())
    }
}

/// True for paths the resolver may return: absolute, existing files under one
/// of the merged search directories. Defense in depth for callers that want
/// to re-verify before spawning.
pub fn is_under_search_dirs(path: &Path) -> bool {
    search_dirs()
        .iter()
        .any(|dir| path.starts_with(dir) && path.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_path_drops_empty_entries() {
        let dirs = split_path_entries(r"C:\a;;D:\b ;");
        assert_eq!(dirs, vec![PathBuf::from(r"C:\a"), PathBuf::from(r"D:\b ")]);
    }

    #[cfg(windows)]
    #[test]
    fn percent_expansion_uses_process_env() {
        let expanded = expand_percent_vars(r"%SystemRoot%\System32;%USERPROFILE%\bin");
        let system_root = std::env::var("SystemRoot").unwrap();
        assert!(expanded.starts_with(&system_root), "got {expanded}");
        assert!(expanded.contains("\\bin"));
    }

    #[cfg(windows)]
    #[test]
    fn percent_expansion_keeps_unknown_vars_verbatim() {
        assert_eq!(
            expand_percent_vars(r"%NO_SUCH_VAR_XYZ%\x"),
            r"%NO_SUCH_VAR_XYZ%\x"
        );
    }

    /// Case-insensitive path equality — PATHEXT entries may differ in letter
    /// case from the on-disk names and NTFS does not care.
    #[cfg(windows)]
    fn same_path(a: &Path, b: &Path) -> bool {
        a.to_string_lossy().to_ascii_lowercase() == b.to_string_lossy().to_ascii_lowercase()
    }

    #[test]
    fn pathext_order_picks_exe_over_cmd() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("tool.cmd"), b"").unwrap();
        std::fs::write(dir.path().join("tool.exe"), b"").unwrap();
        let exts = [".COM".to_string(), ".EXE".to_string(), ".CMD".to_string()];
        let found = find_executable_in_dirs("tool", &[dir.path().to_path_buf()], &exts).unwrap();
        assert!(same_path(&found, &dir.path().join("tool.exe")));
    }

    #[test]
    fn cmd_only_name_resolves_via_pathext() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("npm.cmd"), b"").unwrap();
        let exts = [".EXE".to_string(), ".CMD".to_string()];
        let found = find_executable_in_dirs("npm", &[dir.path().to_path_buf()], &exts).unwrap();
        assert!(same_path(&found, &dir.path().join("npm.cmd")));
    }

    #[test]
    fn directory_order_beats_extension_order() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        std::fs::write(first.path().join("tool.cmd"), b"").unwrap();
        std::fs::write(second.path().join("tool.exe"), b"").unwrap();
        let exts = [".EXE".to_string(), ".CMD".to_string()];
        let dirs = [first.path().to_path_buf(), second.path().to_path_buf()];
        let found = find_executable_in_dirs("tool", &dirs, &exts).unwrap();
        assert!(same_path(&found, &first.path().join("tool.cmd")));
    }

    #[test]
    fn missing_everywhere_is_none() {
        let dir = tempfile::tempdir().unwrap();
        let exts = [".EXE".to_string()];
        assert!(find_executable_in_dirs("nope", &[dir.path().to_path_buf()], &exts).is_none());
    }

    #[test]
    fn search_dirs_dedupes_case_insensitively() {
        // Registry and process PATH overlap heavily; dedup must collapse them
        // even when letter case differs.
        let dirs = search_dirs();
        let lowered: Vec<String> = dirs
            .iter()
            .map(|d| d.to_string_lossy().to_ascii_lowercase())
            .collect();
        let unique: std::collections::HashSet<&String> = lowered.iter().collect();
        assert_eq!(lowered.len(), unique.len(), "duplicates in {lowered:?}");
    }

    #[cfg(windows)]
    #[test]
    fn search_dirs_includes_system32_and_cargo_bin() {
        let dirs = search_dirs();
        let lowered: Vec<String> = dirs
            .iter()
            .map(|d| d.to_string_lossy().to_ascii_lowercase())
            .collect();
        assert!(
            lowered.iter().any(|d| d.contains("system32")),
            "System32 missing from {lowered:?}"
        );
        // The whole point of the registry read: rustup's user PATH shows up
        // even when the process environment predates it.
        assert!(
            lowered.iter().any(|d| d.contains(r".cargo\bin")),
            ".cargo\\bin missing from {lowered:?}"
        );
    }

    #[cfg(windows)]
    #[test]
    fn merged_path_string_is_semicolon_joined() {
        let merged = merged_path_string().unwrap();
        assert!(merged.contains(';'));
    }

    #[cfg(windows)]
    #[test]
    fn resolves_git_exe_and_npm_cmd_from_real_system() {
        let git = resolve_executable("git").expect("git is required by the test suite");
        assert!(git.is_file(), "resolved path not a file: {git:?}");
        assert!(
            git.to_string_lossy().to_ascii_lowercase().ends_with(".exe"),
            "expected an .exe hit: {git:?}"
        );

        // npm ships as npm.cmd only; bare-name CreateProcess can never run it.
        let npm = resolve_executable("npm").expect("npm ships with node");
        assert!(
            npm.to_string_lossy().to_ascii_lowercase().ends_with(".cmd"),
            "expected npm.cmd, got {npm:?}"
        );
    }

    #[cfg(windows)]
    #[test]
    fn resolved_paths_are_under_search_dirs() {
        let git = resolve_executable("git").unwrap();
        assert!(is_under_search_dirs(&git));
        // A path outside every search dir must be rejected.
        assert!(!is_under_search_dirs(&PathBuf::from(
            r"C:\Program Data\evil\git.exe"
        )));
    }
}
