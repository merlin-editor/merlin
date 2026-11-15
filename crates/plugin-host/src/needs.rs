//! What a plugin needs from the computer it runs on: the programs it
//! starts and how to install them, the systems and processors it works
//! on, and the plugins it depends on (plugin-registry.md 12.1 to 12.4).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The systems merlin is released for, as `std::env::consts::OS` names
/// them.
pub const SYSTEMS: [&str; 3] = ["linux", "macos", "windows"];
/// The processors merlin is released for, as `std::env::consts::ARCH`
/// names them.
pub const ARCHES: [&str; 2] = ["x86_64", "aarch64"];

/// How to install a program: one command for every system, or one for
/// each system that has one.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum Install {
    Everywhere(String),
    PerSystem(PerSystem),
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PerSystem {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub linux: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub macos: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub windows: Option<String>,
}

impl Install {
    /// The command for `system`, if there is one.
    pub fn on(&self, system: &str) -> Option<&str> {
        let command = match self {
            Install::Everywhere(c) => Some(c),
            Install::PerSystem(p) => match system {
                "linux" => p.linux.as_ref(),
                "macos" => p.macos.as_ref(),
                "windows" => p.windows.as_ref(),
                _ => None,
            },
        };
        command.map(String::as_str).filter(|c| !c.trim().is_empty())
    }

    /// The command for the system merlin runs on.
    pub fn here(&self) -> Option<&str> {
        self.on(std::env::consts::OS)
    }
}

/// `[programs.<name>]`: a program the plugin starts, what it's for and how
/// to get it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct ManifestProgram {
    /// What the plugin can't do without it, in a few words.
    #[serde(default, rename = "for", skip_serializing_if = "Option::is_none")]
    pub purpose: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub install: Option<Install>,
    /// Where to read how to install it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub systems: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub arch: Vec<String>,
}

impl ManifestProgram {
    /// Whether it's needed on the computer merlin runs on.
    pub fn here(&self) -> bool {
        fits_here(&self.systems, &self.arch)
    }
}

/// `[[agents]]`: an agent the plugin adds to the chat.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct ManifestAgent {
    pub id: String,
    pub name: String,
    /// One character for its tab, such as `◆`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// The program it runs, if it's one of `[programs]`: the chat says
    /// when it isn't installed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub program: Option<String>,
}

/// Whether a list of `systems` and `arch` (empty for all) takes in this
/// computer.
pub fn fits_here(systems: &[String], arch: &[String]) -> bool {
    fits(systems, arch, std::env::consts::OS, std::env::consts::ARCH)
}

pub fn fits(systems: &[String], arch: &[String], os: &str, cpu: &str) -> bool {
    (systems.is_empty() || systems.iter().any(|s| s == os))
        && (arch.is_empty() || arch.iter().any(|a| a == cpu))
}

/// The name people know a system by.
pub fn system_title(system: &str) -> &str {
    match system {
        "linux" => "Linux",
        "macos" => "macOS",
        "windows" => "Windows",
        other => other,
    }
}

/// What a plugin's `systems` and `arch` leave out of this computer, as
/// "not for Windows", "macOS only" or "x86_64 only"; none when it fits.
pub fn not_here(systems: &[String], arch: &[String]) -> Option<String> {
    not_on(systems, arch, std::env::consts::OS, std::env::consts::ARCH)
}

pub fn not_on(systems: &[String], arch: &[String], os: &str, cpu: &str) -> Option<String> {
    if !systems.is_empty() && !systems.iter().any(|s| s == os) {
        return Some(if systems.len() == 1 {
            format!("{} only", system_title(&systems[0]))
        } else {
            format!("not for {}", system_title(os))
        });
    }
    if !arch.is_empty() && !arch.iter().any(|a| a == cpu) {
        return Some(if arch.len() == 1 {
            format!("{} only", arch[0])
        } else {
            format!("not for {cpu}")
        });
    }
    None
}

/// The systems in words, as the Versions tab shows them: "all", or
/// "Linux, macOS".
pub fn systems_words(systems: &[String]) -> String {
    if systems.is_empty() {
        "all".into()
    } else {
        systems
            .iter()
            .map(|s| system_title(s))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// Checks `systems` and `arch` hold only names merlin knows, so a typo
/// doesn't hide a plugin everywhere.
pub fn check_systems(what: &str, systems: &[String], arch: &[String]) -> anyhow::Result<()> {
    for s in systems {
        if !SYSTEMS.contains(&s.as_str()) {
            anyhow::bail!("{what}: systems has {s:?}; it can be linux, macos or windows");
        }
    }
    for a in arch {
        if !ARCHES.contains(&a.as_str()) {
            anyhow::bail!("{what}: arch has {a:?}; it can be x86_64 or aarch64");
        }
    }
    Ok(())
}

/// Checks a network host pattern: a host name, or `*.` and a domain.
pub fn valid_host(host: &str) -> bool {
    let name = host.strip_prefix("*.").unwrap_or(host);
    let label = |l: &str| {
        !l.is_empty()
            && l.len() <= 63
            && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
            && !l.starts_with('-')
            && !l.ends_with('-')
    };
    name.contains('.') && name == name.to_ascii_lowercase() && name.split('.').all(label)
}

/// Whether `host` is one `patterns` allows: an exact name, or under a
/// `*.` domain (not the domain itself).
pub fn host_allowed(patterns: &[String], host: &str) -> bool {
    let host = host.to_ascii_lowercase();
    patterns.iter().any(|p| match p.strip_prefix("*.") {
        Some(domain) => host
            .strip_suffix(domain)
            .is_some_and(|rest| rest.len() > 1 && rest.ends_with('.')),
        None => *p == host,
    })
}

/// A dependency's requirement, read as Cargo reads one: `0.2` is `^0.2`.
pub fn requirement(text: &str) -> anyhow::Result<semver::VersionReq> {
    semver::VersionReq::parse(text.trim())
        .map_err(|e| anyhow::anyhow!("{text:?} isn't a version requirement: {e}"))
}

/// Whether `version` meets `req`. A version that isn't semver meets
/// nothing.
pub fn meets(req: &str, version: &str) -> bool {
    match (requirement(req), semver::Version::parse(version)) {
        (Ok(r), Ok(v)) => r.matches(&v),
        _ => false,
    }
}

/// Where programs were found on `PATH`, until [`look_again`].
static FOUND: std::sync::Mutex<Option<BTreeMap<String, Option<std::path::PathBuf>>>> =
    std::sync::Mutex::new(None);

/// [`on_path`], remembered: the window asks often, and `PATH` rarely
/// changes. [`look_again`] forgets, as when the window gets focus.
pub fn found(program: &str) -> Option<std::path::PathBuf> {
    let mut cache = FOUND.lock().unwrap_or_else(|e| e.into_inner());
    cache
        .get_or_insert_with(BTreeMap::new)
        .entry(program.to_string())
        .or_insert_with(|| on_path(program))
        .clone()
}

/// Forgets where programs were found, so they're looked for again.
pub fn look_again() {
    *FOUND.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

/// Where `program` is on `PATH`, as the shell would find it: a path with
/// a folder in it as it is, else each `PATH` folder in turn, with Windows'
/// `.exe`, `.cmd` and `.bat` tried too.
pub fn on_path(program: &str) -> Option<std::path::PathBuf> {
    let p = std::path::Path::new(program);
    let runnable = |path: &std::path::Path| {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            path.metadata()
                .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        }
        #[cfg(not(unix))]
        {
            path.is_file()
        }
    };
    if p.components().count() > 1 {
        return runnable(p).then(|| p.to_path_buf());
    }
    let exts: &[&str] = if cfg!(windows) {
        &["", ".exe", ".cmd", ".bat"]
    } else {
        &[""]
    };
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).find_map(|dir| {
        exts.iter().find_map(|ext| {
            let candidate = dir.join(format!("{program}{ext}"));
            runnable(&candidate).then_some(candidate)
        })
    })
}

/// A program a plugin starts, on this computer: where it is, if it's on
/// `PATH`, and what to show when it isn't.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProgramStatus {
    pub name: String,
    pub found: Option<std::path::PathBuf>,
    /// What the plugin can't do without it.
    pub purpose: Option<String>,
    /// The command to install it on this system.
    pub install: Option<String>,
    pub url: Option<String>,
}

/// The programs a manifest's plugin starts here, in its order: those in
/// `processes`, then its debuggers', each with its `[programs]` entry.
/// Ones left out for this computer aren't listed.
pub fn program_status(m: &crate::Manifest) -> Vec<ProgramStatus> {
    let mut names: Vec<(String, Option<Install>, Option<String>)> = Vec::new();
    for p in &m.permissions.processes {
        names.push((p.clone(), None, None));
    }
    for d in m
        .debuggers
        .iter()
        .filter(|d| fits_here(&d.systems, &d.arch))
    {
        if let Some(cmd) = d.command.first()
            && !names.iter().any(|(n, ..)| n == cmd)
        {
            names.push((cmd.clone(), d.install.clone(), d.url.clone()));
        }
    }
    names
        .into_iter()
        .filter_map(|(name, install, url)| {
            let entry = m.programs.get(&name);
            if entry.is_some_and(|e| !e.here()) {
                return None;
            }
            let install = entry.and_then(|e| e.install.clone()).or(install);
            Some(ProgramStatus {
                found: found(&name),
                purpose: entry.and_then(|e| e.purpose.clone()),
                install: install.as_ref().and_then(|i| i.here()).map(str::to_string),
                url: entry.and_then(|e| e.url.clone()).or(url),
                name,
            })
        })
        .collect()
}

/// The same for a registry version, from what the index says of it.
pub fn index_program_status(
    programs: &BTreeMap<String, ManifestProgram>,
    processes: &[String],
) -> Vec<ProgramStatus> {
    let mut names: Vec<String> = processes.to_vec();
    for name in programs.keys() {
        if !names.contains(name) {
            names.push(name.clone());
        }
    }
    names
        .into_iter()
        .filter_map(|name| {
            let entry = programs.get(&name);
            if entry.is_some_and(|e| !e.here()) {
                return None;
            }
            Some(ProgramStatus {
                found: found(&name),
                purpose: entry.and_then(|e| e.purpose.clone()),
                install: entry
                    .and_then(|e| e.install.as_ref())
                    .and_then(|i| i.here())
                    .map(str::to_string),
                url: entry.and_then(|e| e.url.clone()),
                name,
            })
        })
        .collect()
}

/// `[depends]`, by plugin name.
pub type Depends = BTreeMap<String, String>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_picks_this_systems_command() {
        let one = Install::Everywhere("rustup component add rust-analyzer".into());
        assert_eq!(
            one.on("windows"),
            Some("rustup component add rust-analyzer")
        );
        let each = Install::PerSystem(PerSystem {
            linux: Some("sudo apt install clangd".into()),
            macos: Some("brew install llvm".into()),
            windows: None,
        });
        assert_eq!(each.on("macos"), Some("brew install llvm"));
        assert_eq!(each.on("windows"), None);
    }

    #[test]
    fn install_reads_as_a_string_or_a_table() {
        #[derive(Deserialize)]
        struct T {
            a: Install,
            b: Install,
        }
        let t: T = toml::from_str(
            "a = \"pip install debugpy\"\nb = { linux = \"apt\", windows = \"winget\" }",
        )
        .unwrap();
        assert_eq!(t.a.on("linux"), Some("pip install debugpy"));
        assert_eq!(t.b.on("windows"), Some("winget"));
        assert!(toml::from_str::<T>("a = \"x\"\nb = { beos = \"x\" }").is_err());
    }

    #[test]
    fn systems_fit_or_say_why_not() {
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert!(fits(&[], &[], "windows", "x86_64"));
        assert!(!fits(&s(&["linux", "macos"]), &[], "windows", "x86_64"));
        assert!(!fits(&[], &s(&["x86_64"]), "macos", "aarch64"));
        assert_eq!(
            not_on(&s(&["linux", "macos"]), &[], "windows", "x86_64").as_deref(),
            Some("not for Windows")
        );
        assert_eq!(
            not_on(&s(&["macos"]), &[], "windows", "x86_64").as_deref(),
            Some("macOS only")
        );
        assert_eq!(
            not_on(&[], &s(&["x86_64"]), "macos", "aarch64").as_deref(),
            Some("x86_64 only")
        );
        assert_eq!(systems_words(&s(&["linux", "macos"])), "Linux, macOS");
        assert!(check_systems("p", &s(&["linx"]), &[]).is_err());
        assert!(check_systems("p", &s(&["linux"]), &s(&["arm"])).is_err());
    }

    #[test]
    fn hosts_match_exactly_or_below_a_domain() {
        let p = vec![
            "api.github.com".to_string(),
            "*.githubusercontent.com".to_string(),
        ];
        assert!(host_allowed(&p, "api.github.com"));
        assert!(host_allowed(&p, "raw.githubusercontent.com"));
        assert!(!host_allowed(&p, "githubusercontent.com"));
        assert!(!host_allowed(&p, "evilgithubusercontent.com"));
        assert!(!host_allowed(&p, "github.com"));
        assert!(valid_host("*.githubusercontent.com"));
        assert!(!valid_host("localhost"));
        assert!(!valid_host("http://x.com"));
        assert!(!valid_host("*.com.*"));
    }

    #[test]
    fn requirements_read_as_cargo_reads_them() {
        assert!(meets("0.2", "0.2.7"));
        assert!(!meets("0.2", "0.3.0"));
        assert!(meets(">=0.3", "0.4.1"));
        assert!(requirement("not a req").is_err());
    }
}
