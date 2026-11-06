//! OS boundaries for the otherwise shared UI and translation engine.

use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Os {
    Windows,
    MacOs,
    Other,
}

impl Os {
    pub fn current() -> Self {
        if cfg!(windows) {
            Os::Windows
        } else if cfg!(target_os = "macos") {
            Os::MacOs
        } else {
            Os::Other
        }
    }
}

type Lookup<'a> = Box<dyn Fn(&str) -> Option<String> + 'a>;

/// The process environment, injectable for tests.
pub struct Env<'a> {
    pub os: Os,
    pub home: PathBuf,
    var: Lookup<'a>,
}

fn process_var(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.is_empty())
}

impl Env<'static> {
    pub fn process() -> Self {
        let os = Os::current();
        let home = process_var(if os == Os::Windows { "USERPROFILE" } else { "HOME" })
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        Env::new(os, home, process_var)
    }
}

impl<'a> Env<'a> {
    pub fn new(os: Os, home: impl Into<PathBuf>, var: impl Fn(&str) -> Option<String> + 'a) -> Self {
        Env { os, home: home.into(), var: Box::new(var) }
    }

    fn var(&self, name: &str) -> Option<String> {
        (self.var)(name)
    }

    fn expand_user(&self, path: &str) -> PathBuf {
        match path.strip_prefix('~') {
            Some("") => self.home.clone(),
            Some(rest) if rest.starts_with(['/', '\\']) => self.home.join(&rest[1..]),
            _ => PathBuf::from(path),
        }
    }

    pub fn application_home(&self) -> PathBuf {
        if let Some(home) = self.var("HERDR_BRIDGE_HOME") {
            return self.expand_user(&home);
        }
        match self.os {
            Os::Windows => self
                .var("APPDATA")
                .map(PathBuf::from)
                .unwrap_or_else(|| self.home.join("AppData/Roaming"))
                .join("HerdrBridge"),
            Os::MacOs => self.home.join("Library/Application Support/HerdrBridge"),
            Os::Other => self
                .var("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| self.home.join(".config"))
                .join("herdr-bridge"),
        }
    }

    /// Where CLIs live when a GUI launch (Finder, Explorer) has a minimal PATH.
    pub fn search_directories(&self) -> Vec<PathBuf> {
        match self.os {
            Os::Windows => vec![
                self.var("LOCALAPPDATA")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| self.home.join("AppData/Local"))
                    .join("Programs/herdr-app"),
                self.var("APPDATA").map(PathBuf::from).unwrap_or_else(|| self.home.join("AppData/Roaming")).join("npm"),
                self.home.join(".local/bin"),
                self.home.join(".cargo/bin"),
            ],
            _ => vec![
                self.home.join(".local/bin"),
                self.home.join(".cargo/bin"),
                PathBuf::from("/opt/homebrew/bin"),
                PathBuf::from("/usr/local/bin"),
            ],
        }
    }

    /// Resolves a program name the way the shell would, then falls back to the
    /// GUI search directories. Explicit paths are returned unchanged.
    pub fn executable(&self, name: &str) -> String {
        if name.starts_with('~') || name.contains(['/', '\\']) || Path::new(name).is_absolute() {
            return self.expand_user(name).to_string_lossy().into_owned();
        }
        if let Some(found) = self.which(name) {
            return found.to_string_lossy().into_owned();
        }
        let suffixes: &[&str] = if self.os == Os::Windows { &["", ".exe", ".cmd"] } else { &[""] };
        for directory in self.search_directories() {
            for suffix in suffixes {
                let candidate = directory.join(format!("{name}{suffix}"));
                if self.is_executable(&candidate) {
                    return candidate.to_string_lossy().into_owned();
                }
            }
        }
        name.to_string()
    }

    fn which(&self, name: &str) -> Option<PathBuf> {
        let path = self.var("PATH")?;
        let separator = if self.os == Os::Windows { ';' } else { ':' };
        let extensions: Vec<String> = if self.os == Os::Windows {
            let pathext = self.var("PATHEXT").unwrap_or_else(|| ".COM;.EXE;.BAT;.CMD".into());
            let extensions: Vec<String> =
                pathext.split(';').filter(|e| !e.is_empty()).map(|e| e.to_ascii_lowercase()).collect();
            if extensions.iter().any(|e| name.to_ascii_lowercase().ends_with(e.as_str())) {
                vec![String::new()]
            } else {
                extensions
            }
        } else {
            vec![String::new()]
        };
        path.split(separator).filter(|d| !d.is_empty()).find_map(|directory| {
            extensions.iter().find_map(|extension| {
                let candidate = Path::new(directory).join(format!("{name}{extension}"));
                self.is_executable(&candidate).then_some(candidate)
            })
        })
    }

    fn is_executable(&self, path: &Path) -> bool {
        if !path.is_file() {
            return false;
        }
        #[cfg(unix)]
        if self.os != Os::Windows {
            use std::os::unix::fs::PermissionsExt;
            return path.metadata().is_ok_and(|m| m.permissions().mode() & 0o111 != 0);
        }
        true
    }

    /// PATH for child processes: the current PATH plus the GUI search
    /// directories, so `#!/usr/bin/env node` style CLIs still start from Finder.
    pub fn child_path(&self) -> Option<String> {
        let separator = if self.os == Os::Windows { ";" } else { ":" };
        let current = self.var("PATH").unwrap_or_default();
        let mut entries: Vec<String> = current.split(separator).filter(|e| !e.is_empty()).map(String::from).collect();
        let before = entries.len();
        for directory in self.search_directories() {
            let directory = directory.to_string_lossy().into_owned();
            if directory.is_empty() || entries.contains(&directory) {
                continue;
            }
            if Path::new(&directory).is_dir() {
                entries.push(directory);
            }
        }
        (entries.len() != before).then(|| entries.join(separator))
    }
}

pub fn application_home() -> PathBuf {
    Env::process().application_home()
}

pub fn executable(name: &str) -> String {
    Env::process().executable(name)
}

/// (interface, monospace) font families with CJK coverage.
pub fn font_families() -> (&'static str, &'static str) {
    match Os::current() {
        Os::MacOs => ("PingFang SC", "Menlo"),
        Os::Windows => ("Microsoft YaHei UI", "Consolas"),
        Os::Other => ("Noto Sans CJK SC", "DejaVu Sans Mono"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn env<'a>(os: Os, home: &str, vars: &'a HashMap<&str, String>) -> Env<'a> {
        Env::new(os, home, |name: &str| vars.get(name).cloned())
    }

    #[test]
    fn configuration_folders_are_separate_per_platform() {
        let none = HashMap::new();
        assert_eq!(
            env(Os::MacOs, "/Users/demo", &none).application_home(),
            PathBuf::from("/Users/demo/Library/Application Support/HerdrBridge")
        );
        let windows = HashMap::from([("APPDATA", "C:/Roaming".to_string())]);
        assert_eq!(
            env(Os::Windows, "C:/Users/demo", &windows).application_home(),
            PathBuf::from("C:/Roaming/HerdrBridge")
        );
        let custom = HashMap::from([("HERDR_BRIDGE_HOME", "~/bridge".to_string())]);
        assert_eq!(env(Os::MacOs, "/Users/demo", &custom).application_home(), PathBuf::from("/Users/demo/bridge"));
    }

    #[test]
    fn gui_path_fallback_and_explicit_paths() {
        let folder = tempfile::tempdir().unwrap();
        let directory = folder.path().join(".local/bin");
        std::fs::create_dir_all(&directory).unwrap();
        let binary = directory.join("codex");
        std::fs::write(&binary, "test").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let none = HashMap::new();
        let os = if cfg!(windows) { Os::Windows } else { Os::MacOs };
        let e = env(os, folder.path().to_str().unwrap(), &none);
        assert_eq!(e.executable("codex"), binary.to_string_lossy());
        assert_eq!(e.executable(binary.to_str().unwrap()), binary.to_string_lossy());
        assert_eq!(e.executable("missing-tool"), "missing-tool");
    }

    #[test]
    fn path_lookup_prefers_path_entries() {
        let folder = tempfile::tempdir().unwrap();
        let binary = folder.path().join(if cfg!(windows) { "hc.exe" } else { "hc" });
        std::fs::write(&binary, "test").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let vars = HashMap::from([("PATH", folder.path().to_string_lossy().into_owned())]);
        let os = if cfg!(windows) { Os::Windows } else { Os::MacOs };
        assert_eq!(env(os, "/nonexistent", &vars).executable("hc"), binary.to_string_lossy());
    }
}
