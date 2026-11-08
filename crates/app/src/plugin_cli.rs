//! `merlin plugin …`: installing, packing, searching, updating and
//! removing plugins from the command line. See `docs/plugin-registry.md`
//! section 7.

use std::cmp::Ordering;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use merlin_plugins::registry::{self, Loaded, Source};
use merlin_plugins::{Config, Manifest, Permissions, package};

use crate::settings;

type Command = fn(&[String], &str) -> Result<()>;

/// The command for `merlin plugin <name>`, if there's one.
pub fn command(name: &str) -> Option<Command> {
    Some(match name {
        "install" => install,
        "pack" => pack,
        "search" => search,
        "update" => update,
        "uninstall" => uninstall,
        _ => return None,
    })
}

/// Where plugins are installed, made if it isn't there.
fn plugins_dir() -> Result<PathBuf> {
    let dir = settings::plugins_dir().context("no home folder")?;
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Fetches a URL over HTTPS, at most `limit` bytes of it.
fn fetch(url: &str, limit: u64) -> Result<Vec<u8>> {
    let mut response = ureq::get(url)
        .header("User-Agent", concat!("merlin/", env!("CARGO_PKG_VERSION")))
        .call()
        .with_context(|| format!("fetching {url}"))?;
    // ureq refuses a body as long as its limit, and a package is exactly
    // as long as the index says: so one more, and the length checked here.
    let bytes = response
        .body_mut()
        .with_config()
        .limit(limit.saturating_add(1))
        .read_to_vec()
        .with_context(|| format!("reading {url}"))?;
    if bytes.len() as u64 > limit {
        bail!("{url} is larger than the {limit} bytes expected");
    }
    Ok(bytes)
}

/// The registries in the user's own settings, saying what's wrong with any
/// it skips.
fn sources() -> Vec<Source> {
    let root = std::env::current_dir().unwrap_or_default();
    let (user, _) = settings::Files::new(&root).each();
    let (sources, problems) = registry::sources(&user);
    for problem in problems {
        eprintln!("Skipping a plugin source: {problem}");
    }
    sources
}

/// Every source's index, fetched and checked, saying which it couldn't
/// get. A source it can't reach still lists its last good index.
fn load_all() -> Vec<Loaded> {
    let data = Config::standard().data_dir;
    let now = registry::now();
    sources()
        .iter()
        .filter_map(|source| match registry::load(source, &data, &fetch, now) {
            Ok(loaded) => {
                if let Some(why) = &loaded.stale {
                    eprintln!("Showing {}'s last index: {why}", source.name);
                }
                Some(loaded)
            }
            Err(err) => {
                eprintln!("Can't read {}'s index: {err:#}", source.name);
                None
            }
        })
        .collect()
}

/// Prints what an installed plugin may do.
fn print_installed(title: &str, how: &str, allowed: &[String]) {
    if allowed.is_empty() {
        println!("Installed {title}{how}");
    } else {
        println!("Installed {title}{how}, allowed to:");
        for what in allowed {
            println!("  • {what}");
        }
    }
}

/// `merlin plugin install <name | dir | file.merlin>...`: installs each,
/// and allows what its manifest asks for, since installing it is the
/// user's say-so.
fn install(items: &[String], usage: &str) -> Result<()> {
    if items.is_empty() {
        bail!("install what?\n\n{usage}");
    }
    let plugins = plugins_dir()?;
    let data = Config::standard().data_dir;
    let mut indexes: Option<Vec<Loaded>> = None;
    for item in items {
        let path = Path::new(item);
        let manifest =
            if path.is_file() && path.extension().is_some_and(|e| e == package::EXTENSION) {
                package::install(path, &plugins, &data)?
            } else if path.is_dir() {
                let (manifest, linked) = link(path, &plugins, &data)?;
                if !linked {
                    let installed = plugins.join(&manifest.name);
                    let (_, allowed) = merlin_plugins::grant(&Config::standard(), &installed)?;
                    print_installed(
                        &format!("{} {}", manifest.title, manifest.version),
                        " as a copy, since this system won't make links (Developer Mode allows \
                     them on Windows); install it again after rebuilding it",
                        &allowed,
                    );
                    continue;
                }
                manifest
            } else {
                // A bad name says so before any registry is asked.
                parse_wanted(item)?;
                let loaded = indexes.get_or_insert_with(load_all);
                from_registry(item, loaded, &plugins, &data)?
            };
        let (_, allowed) =
            merlin_plugins::grant(&Config::standard(), &plugins.join(&manifest.name))?;
        print_installed(
            &format!("{} {}", manifest.title, manifest.version),
            "",
            &allowed,
        );
    }
    Ok(())
}

/// Links a built plugin folder in. `false` when it was copied instead.
fn link(path: &Path, plugins: &Path, data: &Path) -> Result<(Manifest, bool)> {
    let dir = path
        .canonicalize()
        .with_context(|| format!("no plugin at {}", path.display()))?;
    let manifest = merlin_plugins::manifest(&dir)?;
    if let Some(component) = &manifest.component
        && !dir.join(component).exists()
    {
        bail!(
            "{} isn't built: run `cargo build --release` in {}",
            manifest.name,
            dir.display()
        );
    }
    let linked = package::link(&dir, &manifest.name, plugins, data)?;
    Ok((manifest, linked))
}

/// What `install` was asked for by name: `[source:]name[@version]`.
struct Wanted<'a> {
    source: Option<&'a str>,
    name: &'a str,
    version: Option<&'a str>,
}

fn parse_wanted(item: &str) -> Result<Wanted<'_>> {
    let (source, rest) = match item.split_once(':') {
        Some((s, r)) => (Some(s), r),
        None => (None, item),
    };
    let (name, version) = match rest.split_once('@') {
        Some((n, v)) => (n, Some(v)),
        None => (rest, None),
    };
    if !merlin_plugins::valid_name(name) {
        bail!("there's no folder or package at {item}, and {name:?} isn't a plugin's name");
    }
    Ok(Wanted {
        source,
        name,
        version,
    })
}

/// Installs a plugin by name from the registry that has it.
fn from_registry(item: &str, indexes: &[Loaded], plugins: &Path, data: &Path) -> Result<Manifest> {
    let wanted = parse_wanted(item)?;
    let installed = package::installed(data);
    let current = installed.get(wanted.name);
    let having: Vec<&Loaded> = indexes
        .iter()
        .filter(|l| l.index.plugins.contains_key(wanted.name))
        .filter(|l| wanted.source.is_none_or(|s| s == l.source.name))
        .collect();
    let loaded = match having.as_slice() {
        [] => match wanted.source {
            Some(source) => bail!("{source} has no plugin called {}", wanted.name),
            None => bail!("no registry has a plugin called {}", wanted.name),
        },
        [one] => *one,
        several => {
            // One it came from wins; otherwise the user says which.
            match current.and_then(|c| several.iter().find(|l| l.source.name == c.source)) {
                Some(l) => *l,
                None => bail!(
                    "{} is in {}; say which, as {}:{}",
                    wanted.name,
                    several
                        .iter()
                        .map(|l| l.source.name.as_str())
                        .collect::<Vec<_>>()
                        .join(" and "),
                    several[0].source.name,
                    wanted.name
                ),
            }
        }
    };
    if let Some(current) = current
        && !["file", "folder"].contains(&current.source.as_str())
        && current.source != loaded.source.name
        && wanted.source.is_none()
    {
        bail!(
            "{} came from {}; to replace it with {}'s, install {}:{}",
            wanted.name,
            current.source,
            loaded.source.name,
            loaded.source.name,
            wanted.name
        );
    }
    let entry = &loaded.index.plugins[wanted.name];
    let version = match wanted.version {
        Some(v) => entry.version(v).with_context(|| {
            format!(
                "{} has no version {v} of {}",
                loaded.source.name, wanted.name
            )
        })?,
        None => entry.newest().with_context(|| why_none(entry))?,
    };
    if let Some(why) = entry.blocked(&version.version) {
        bail!("{} {} is blocked: {why}", wanted.name, version.version);
    }
    if version.api != merlin_plugins::API_VERSION {
        bail!(
            "{} {} is built for plugin API {}, and this merlin has {}",
            wanted.name,
            version.version,
            version.api,
            merlin_plugins::API_VERSION
        );
    }
    if let Some(why) = &version.yanked {
        eprintln!(
            "Note: {} {} was yanked: {why}",
            wanted.name, version.version
        );
    }
    registry::install(loaded, wanted.name, version, plugins, data, &fetch)
}

/// Why an entry has nothing to install.
fn why_none(entry: &registry::Entry) -> String {
    if let Some(why) = &entry.blocked {
        return format!("{} is blocked: {why}", entry.title);
    }
    let api = merlin_plugins::API_VERSION;
    if entry.versions.iter().any(|v| v.api == api) {
        format!(
            "{} has no version to install: they're yanked, blocked or pre-releases",
            entry.title
        )
    } else {
        format!(
            "{} isn't yet updated for this version of merlin (plugin API {api})",
            entry.title
        )
    }
}

/// `merlin plugin search [text]`: what the registries have.
fn search(args: &[String], _usage: &str) -> Result<()> {
    let text = args.join(" ").to_lowercase();
    let installed = package::installed(&Config::standard().data_dir);
    let mut found = 0;
    for loaded in load_all() {
        for (name, entry) in &loaded.index.plugins {
            let haystack = format!(
                "{name} {} {}",
                entry.title,
                entry.description.as_deref().unwrap_or("")
            )
            .to_lowercase();
            if !text.split_whitespace().all(|w| haystack.contains(w)) {
                continue;
            }
            found += 1;
            let newest = entry.newest();
            let version = newest.map_or("-", |v| v.version.as_str());
            println!(
                "{name} {version}  {}  [{}]",
                entry.title, loaded.source.name
            );
            if let Some(about) = &entry.description {
                println!("    {about}");
            }
            if newest.is_none() {
                println!("    {}", why_none(entry));
            }
            if let Some(mine) = installed.get(name.as_str()) {
                println!("    Installed: {}", mine.version);
            }
        }
    }
    if found == 0 {
        println!("Nothing found");
    }
    Ok(())
}

/// `merlin plugin update [name...]`: brings plugins installed from a
/// registry up to its newest version for this merlin. One that asks for
/// more than it was allowed waits for `merlin plugin install`.
fn update(names: &[String], _usage: &str) -> Result<()> {
    let config = Config::standard();
    let plugins = plugins_dir()?;
    let installed = package::installed(&config.data_dir);
    for name in names {
        match installed.get(name.as_str()) {
            None => bail!("{name} isn't installed from a registry or a package"),
            Some(i) if i.source == "file" => {
                bail!("{name} was installed from a .merlin file: install a newer one to update it")
            }
            Some(i) if i.source == "folder" => {
                bail!("{name} was copied from a folder: install the folder again to update it")
            }
            Some(_) => {}
        }
    }
    let from_registry: Vec<_> = installed
        .iter()
        .filter(|(name, _)| names.is_empty() || names.contains(name))
        .filter(|(_, i)| !["file", "folder"].contains(&i.source.as_str()))
        .collect();
    if from_registry.is_empty() {
        println!("Nothing installed from a registry");
        return Ok(());
    }
    let indexes = load_all();
    let mut updated = 0;
    for (name, mine) in from_registry {
        let Some(loaded) = indexes.iter().find(|l| l.source.name == mine.source) else {
            eprintln!("{name}: can't reach {}", mine.source);
            continue;
        };
        let Some(entry) = loaded.index.plugins.get(name.as_str()) else {
            eprintln!("{name}: {} no longer lists it", mine.source);
            continue;
        };
        if let Some(why) = entry.blocked(&mine.version) {
            eprintln!(
                "{name} {} is blocked by {}: {why}. Remove it with `merlin plugin uninstall {name}`.",
                mine.version, mine.source
            );
        } else if let Some(why) = entry.version(&mine.version).and_then(|v| v.yanked.as_ref()) {
            eprintln!("{name} {} was yanked: {why}", mine.version);
        }
        let Some(newest) = entry.newest() else {
            eprintln!("{name}: {}", why_none(entry));
            continue;
        };
        if registry::compare_versions(&newest.version, &mine.version) != Ordering::Greater {
            continue;
        }
        let dir = plugins.join(name);
        let allowed = merlin_plugins::granted(&config, &dir).unwrap_or_default();
        if !allowed.covers(&newest.permissions) {
            println!(
                "{name} {} asks to do more than {} could:",
                newest.version, mine.version
            );
            for (what, _) in more(&allowed, &newest.permissions) {
                println!("  • {what}");
            }
            println!("  Install it with `merlin plugin install {name}` to allow that.");
            continue;
        }
        let manifest = registry::install(loaded, name, newest, &plugins, &config.data_dir, &fetch)?;
        merlin_plugins::grant(&config, &dir)?;
        println!(
            "Updated {} from {} to {}",
            manifest.title, mine.version, manifest.version
        );
        updated += 1;
    }
    if updated == 0 {
        println!("Everything's up to date");
    }
    Ok(())
}

/// What `wanted` asks for that `allowed` doesn't give, in words.
fn more(allowed: &Permissions, wanted: &Permissions) -> Vec<(String, bool)> {
    let had = allowed.described();
    wanted
        .described()
        .into_iter()
        .filter(|w| !had.contains(w))
        .collect()
}

/// `merlin plugin uninstall <name>...`: removes each, and what it was
/// allowed. A linked folder stays where it is; a packaged plugin's folder
/// goes to the bin.
fn uninstall(names: &[String], usage: &str) -> Result<()> {
    if names.is_empty() {
        bail!("uninstall what?\n\n{usage}");
    }
    let config = Config::standard();
    let plugins = plugins_dir()?;
    for name in names {
        let dir = plugins.join(name);
        let Ok(meta) = dir.symlink_metadata() else {
            bail!("{name} isn't installed");
        };
        let _ = merlin_plugins::revoke(&config, &dir);
        if meta.file_type().is_symlink() {
            let from = dir.read_link().unwrap_or_default();
            remove_link(&dir)?;
            println!(
                "Uninstalled {name}. Its files are still in {}.",
                from.display()
            );
        } else {
            trash::delete(&dir).with_context(|| format!("moving {} to the bin", dir.display()))?;
            println!("Uninstalled {name}. Its folder is in the bin.");
        }
        package::record(&config.data_dir, name, None)?;
    }
    Ok(())
}

/// Removes a link to a folder: a file on Unix, a folder link on Windows.
fn remove_link(link: &Path) -> Result<()> {
    fs::remove_file(link)
        .or_else(|_| fs::remove_dir(link))
        .with_context(|| format!("removing {}", link.display()))
}

/// `merlin plugin pack [dir] [--out <dir>]`: packs a plugin into
/// `<name>-<version>.merlin`, the same bytes every time from the same
/// source and `SOURCE_DATE_EPOCH`.
fn pack(args: &[String], usage: &str) -> Result<()> {
    let mut dir = None;
    let mut out = None;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--out" => out = Some(PathBuf::from(it.next().context("--out needs a folder")?)),
            flag if flag.starts_with('-') => bail!("unknown option: {flag}\n\n{usage}"),
            _ if dir.is_some() => bail!("pack takes one plugin folder\n\n{usage}"),
            path => dir = Some(PathBuf::from(path)),
        }
    }
    let dir = dir.unwrap_or_else(|| PathBuf::from("."));
    let out = out.unwrap_or_else(|| PathBuf::from("."));
    let packed = package::pack(&dir, &out, package::source_epoch())?;
    println!("Packed {}", packed.path.display());
    println!(
        "  {} KB, sha256 {}",
        packed.size.div_ceil(1024),
        packed.sha256
    );
    Ok(())
}
