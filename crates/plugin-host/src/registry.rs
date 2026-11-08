//! Plugin registries: where packages come from, as `docs/plugin-registry.md`
//! sections 6 and 7 lay out. A source is a URL serving `index.json` and
//! its minisign signature; the index lists each plugin's versions with
//! the URL and SHA-256 of each package.
//!
//! Nothing here touches the network itself: callers pass `fetch`, so the
//! app chooses how, and tests serve bytes from memory.

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use crate::package::{self, MAX_PACKED, sha256_hex};
use crate::{API_VERSION, Manifest, Permissions, valid_name};

/// The registry merlin comes with.
pub const DEFAULT_SOURCE: &str = "merlin";
pub const DEFAULT_URL: &str = "https://merlin-editor.github.io/plugins/index.json";
/// Its public keys: the one it signs with, and the next, for rotating to.
pub const DEFAULT_KEYS: [&str; 2] = [
    "RWSOaZXj0KSbbxIQCKOCNyg+9hxPpudxVWt+NUb+c3iAJ/Ivft6Yqbpv",
    "RWQDtqLDqdomJVzQvJHLQx3nZVFZRx9YpNvgHKdAAsHImTXo/pEDnYiv",
];
/// The index format this merlin reads.
pub const SCHEMA: u32 = 1;
/// The largest index it reads.
pub const MAX_INDEX: u64 = 16 * 1024 * 1024;

/// Fetches a URL's bytes, at most `limit` of them.
pub type Fetch<'a> = &'a dyn Fn(&str, u64) -> Result<Vec<u8>>;

/// A registry to install from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Source {
    pub name: String,
    /// Its `index.json`; the signature is beside it, with `.minisig` added.
    pub url: String,
    /// minisign public keys, base64, any of which may have signed it.
    pub keys: Vec<String>,
}

impl Source {
    pub fn default_source() -> Self {
        Source {
            name: DEFAULT_SOURCE.into(),
            url: DEFAULT_URL.into(),
            keys: DEFAULT_KEYS.iter().map(|k| k.to_string()).collect(),
        }
    }
}

/// The sources the user's own settings give: merlin's, unless
/// `[plugin-sources.merlin] enabled = false`, then each
/// `[plugin-sources.<name>]` with a `url` and a `key` (or `keys`). Workspace
/// settings never add sources: a project can't choose where code comes from.
/// Also says what was wrong with any it skipped.
pub fn sources(user_settings: &toml::Table) -> (Vec<Source>, Vec<String>) {
    let mut sources = Vec::new();
    let mut problems = Vec::new();
    let table = user_settings
        .get("plugin-sources")
        .and_then(|t| t.as_table())
        .cloned()
        .unwrap_or_default();
    let enabled = |t: &toml::Table| t.get("enabled").and_then(|e| e.as_bool()) != Some(false);
    let merlin = table.get(DEFAULT_SOURCE).and_then(|t| t.as_table());
    if merlin.is_none_or(enabled) {
        sources.push(Source::default_source());
    }
    for (name, value) in &table {
        if name == DEFAULT_SOURCE {
            continue;
        }
        let Some(t) = value.as_table() else {
            problems.push(format!("plugin-sources.{name} should be a table"));
            continue;
        };
        if !enabled(t) {
            continue;
        }
        if !valid_name(name) {
            problems.push(format!(
                "plugin-sources.{name}: a source's name is lowercase letters, digits and -"
            ));
            continue;
        }
        let Some(url) = t.get("url").and_then(|u| u.as_str()) else {
            problems.push(format!("plugin-sources.{name} needs a url"));
            continue;
        };
        if !url.starts_with("https://") {
            problems.push(format!(
                "plugin-sources.{name}: the url must start with https://"
            ));
            continue;
        }
        let mut keys: Vec<String> = t
            .get("keys")
            .and_then(|k| k.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|k| k.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        keys.extend(t.get("key").and_then(|k| k.as_str()).map(String::from));
        if keys.is_empty() {
            problems.push(format!(
                "plugin-sources.{name} needs a key: the minisign public key its index is signed with"
            ));
            continue;
        }
        if let Some(bad) = keys
            .iter()
            .find(|k| minisign_verify::PublicKey::from_base64(k).is_err())
        {
            problems.push(format!(
                "plugin-sources.{name}: {bad:?} isn't a minisign public key"
            ));
            continue;
        }
        sources.push(Source {
            name: name.clone(),
            url: url.to_string(),
            keys,
        });
    }
    (sources, problems)
}

#[derive(Clone, Debug, Deserialize)]
pub struct Index {
    pub schema: u32,
    pub serial: u64,
    pub generated: String,
    pub expires: String,
    #[serde(default)]
    pub plugins: BTreeMap<String, Entry>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Entry {
    pub title: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub badge: Option<String>,
    #[serde(default)]
    pub hue: Option<String>,
    #[serde(default)]
    pub license: Option<String>,
    #[serde(default)]
    pub repository: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub owners: Vec<String>,
    /// A whole plugin a registry's maintainers blocked, and why.
    #[serde(default)]
    pub blocked: Option<String>,
    #[serde(default)]
    pub versions: Vec<Version>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Version {
    pub version: String,
    pub api: String,
    #[serde(default)]
    pub commit: Option<String>,
    #[serde(default)]
    pub published: Option<String>,
    pub url: String,
    pub sha256: String,
    pub size: u64,
    #[serde(default)]
    pub permissions: Permissions,
    #[serde(default)]
    pub depends: Vec<String>,
    #[serde(default)]
    pub yanked: Option<String>,
    #[serde(default)]
    pub blocked: Option<String>,
}

impl Entry {
    /// The version to install: the newest one that isn't a pre-release,
    /// yanked or blocked, and is built for this merlin's API.
    pub fn newest(&self) -> Option<&Version> {
        if self.blocked.is_some() {
            return None;
        }
        self.versions
            .iter()
            .filter(|v| {
                v.yanked.is_none()
                    && v.blocked.is_none()
                    && v.api == API_VERSION
                    && !v.version.contains('-')
            })
            .max_by(|a, b| compare_versions(&a.version, &b.version))
    }

    pub fn version(&self, version: &str) -> Option<&Version> {
        self.versions.iter().find(|v| v.version == version)
    }

    /// Why `version` mustn't be used, if a maintainer blocked it.
    pub fn blocked(&self, version: &str) -> Option<&str> {
        self.blocked
            .as_deref()
            .or_else(|| self.version(version).and_then(|v| v.blocked.as_deref()))
    }
}

/// Orders semver versions. A pre-release comes before its release, and
/// pre-releases compare part by part, numbers as numbers.
pub fn compare_versions(a: &str, b: &str) -> Ordering {
    fn split(v: &str) -> (Vec<u64>, Option<&str>) {
        let v = v.split('+').next().unwrap_or(v);
        let (core, pre) = match v.split_once('-') {
            Some((c, p)) => (c, Some(p)),
            None => (v, None),
        };
        (
            core.split('.').map(|n| n.parse().unwrap_or(0)).collect(),
            pre,
        )
    }
    let ((ca, pa), (cb, pb)) = (split(a), split(b));
    ca.cmp(&cb).then_with(|| match (pa, pb) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (Some(x), Some(y)) => {
            let (xs, ys): (Vec<_>, Vec<_>) = (x.split('.').collect(), y.split('.').collect());
            for (p, q) in xs.iter().zip(&ys) {
                let o = match (p.parse::<u64>(), q.parse::<u64>()) {
                    (Ok(m), Ok(n)) => m.cmp(&n),
                    (Ok(_), Err(_)) => Ordering::Less,
                    (Err(_), Ok(_)) => Ordering::Greater,
                    _ => p.cmp(q),
                };
                if o != Ordering::Equal {
                    return o;
                }
            }
            xs.len().cmp(&ys.len())
        }
    })
}

/// Seconds since 1970 for a UTC time written `2026-11-09T12:00:00Z`.
pub fn parse_time(text: &str) -> Option<i64> {
    let b = text.as_bytes();
    if b.len() < 20
        || b[4] != b'-'
        || b[7] != b'-'
        || b[10] != b'T'
        || b[13] != b':'
        || b[16] != b':'
    {
        return None;
    }
    let n = |r: std::ops::Range<usize>| text.get(r)?.parse::<i64>().ok();
    let (y, mo, d) = (n(0..4)?, n(5..7)?, n(8..10)?);
    let (h, mi, s) = (n(11..13)?, n(14..16)?, n(17..19)?);
    // Fractions of a second are allowed and ignored; the zone must be Z.
    let rest = &text[19..];
    let rest = rest
        .strip_prefix('.')
        .map_or(rest, |r| r.trim_start_matches(|c: char| c.is_ascii_digit()));
    if rest != "Z"
        || !(1..=12).contains(&mo)
        || !(1..=31).contains(&d)
        || h > 23
        || mi > 59
        || s > 60
    {
        return None;
    }
    // Days from the civil date, after Howard Hinnant's days_from_civil.
    let y = if mo <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (mo + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400 + h * 3600 + mi * 60 + s)
}

/// Seconds since 1970, now.
pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Checks an index against `source`'s keys, its schema and expiry, and
/// that its serial is at least `last_serial`, and reads it.
pub fn check(
    source: &Source,
    index: &[u8],
    signature: &[u8],
    now: i64,
    last_serial: u64,
) -> Result<Index> {
    let sig_text = std::str::from_utf8(signature).context("the signature isn't text")?;
    let sig = minisign_verify::Signature::decode(sig_text)
        .map_err(|e| anyhow::anyhow!("the signature can't be read: {e}"))?;
    let signed = source.keys.iter().any(|k| {
        minisign_verify::PublicKey::from_base64(k)
            .is_ok_and(|key| key.verify(index, &sig, false).is_ok())
    });
    if !signed {
        bail!("{}'s index isn't signed by any of its keys", source.name);
    }
    let parsed: Index = serde_json::from_slice(index).context("the index can't be read")?;
    if parsed.schema != SCHEMA {
        bail!(
            "{}'s index is format {}, and this merlin reads format {SCHEMA}; a newer merlin may read it",
            source.name,
            parsed.schema
        );
    }
    let expires = parse_time(&parsed.expires)
        .with_context(|| format!("the index's expiry {:?} can't be read", parsed.expires))?;
    if expires <= now {
        bail!(
            "{}'s index expired at {}; the registry hasn't re-signed it",
            source.name,
            parsed.expires
        );
    }
    if parsed.serial < last_serial {
        bail!(
            "{}'s index is number {}, older than number {last_serial} already seen; someone may be serving an old one",
            source.name,
            parsed.serial
        );
    }
    Ok(parsed)
}

/// Where a source's last good index is kept.
fn cache_dir(data_dir: &Path, source: &str) -> PathBuf {
    data_dir.join("sources").join(source)
}

fn last_serial(data_dir: &Path, source: &str) -> u64 {
    fs::read_to_string(cache_dir(data_dir, source).join("serial"))
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0)
}

/// A source's index, and whether it's fresh.
#[derive(Debug)]
pub struct Loaded {
    pub source: Source,
    pub index: Index,
    /// Why it couldn't fetch a new index, when this is the last good one.
    pub stale: Option<String>,
}

/// Fetches `source`'s index and checks it, keeping it as the last good
/// one. If that fails, falls back to the last good one, still checked
/// against the keys, with why in `stale`.
pub fn load(source: &Source, data_dir: &Path, fetch: Fetch, now: i64) -> Result<Loaded> {
    let last = last_serial(data_dir, &source.name);
    let fresh = (|| {
        let index = fetch(&source.url, MAX_INDEX)?;
        let signature = fetch(&format!("{}.minisig", source.url), 64 * 1024)?;
        let parsed = check(source, &index, &signature, now, last)?;
        let dir = cache_dir(data_dir, &source.name);
        fs::create_dir_all(&dir)?;
        fs::write(dir.join("index.json"), &index)?;
        fs::write(dir.join("index.json.minisig"), &signature)?;
        fs::write(dir.join("serial"), parsed.serial.to_string())?;
        anyhow::Ok(parsed)
    })();
    match fresh {
        Ok(index) => Ok(Loaded {
            source: source.clone(),
            index,
            stale: None,
        }),
        Err(err) => {
            let why = format!("{err:#}");
            let dir = cache_dir(data_dir, &source.name);
            let cached = fs::read(dir.join("index.json"))
                .and_then(|i| Ok((i, fs::read(dir.join("index.json.minisig"))?)));
            let Ok((index, signature)) = cached else {
                bail!("{why}");
            };
            // The last good one is still checked against the keys, but may
            // have expired since.
            let index = check(source, &index, &signature, i64::MIN, last)
                .map_err(|e| anyhow::anyhow!("{why}; and the last good index: {e:#}"))?;
            Ok(Loaded {
                source: source.clone(),
                index,
                stale: Some(why),
            })
        }
    }
}

/// Downloads `version` of `name` from `loaded`, checks it against the
/// index, and installs it. The caller grants its permissions.
pub fn install(
    loaded: &Loaded,
    name: &str,
    version: &Version,
    plugins_dir: &Path,
    data_dir: &Path,
    fetch: Fetch,
) -> Result<Manifest> {
    if let Some(why) = &loaded.stale {
        bail!(
            "can't reach {} to install from it: {why}",
            loaded.source.name
        );
    }
    if !version.url.starts_with("https://") {
        bail!("{name} {} would download over plain http", version.version);
    }
    if version.size > MAX_PACKED {
        bail!("{name} {} is larger than a package can be", version.version);
    }
    let bytes = fetch(&version.url, version.size.max(1))
        .with_context(|| format!("downloading {name} {}", version.version))?;
    if bytes.len() as u64 != version.size
        || sha256_hex(&bytes) != version.sha256.to_ascii_lowercase()
    {
        bail!(
            "the download of {name} {} doesn't match the index; nothing was installed",
            version.version
        );
    }
    package::install_bytes(
        &bytes,
        &loaded.source.name,
        Some((name, &version.version)),
        plugins_dir,
        data_dir,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_order_as_semver() {
        let mut v = [
            "1.0.0",
            "0.9.0",
            "1.0.0-beta.2",
            "1.0.0-beta.10",
            "1.0.0-alpha",
            "0.10.0",
            "1.0.0-beta",
        ];
        v.sort_by(|a, b| compare_versions(a, b));
        assert_eq!(
            v,
            [
                "0.9.0",
                "0.10.0",
                "1.0.0-alpha",
                "1.0.0-beta",
                "1.0.0-beta.2",
                "1.0.0-beta.10",
                "1.0.0"
            ]
        );
    }

    #[test]
    fn times_parse_as_utc() {
        assert_eq!(parse_time("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_time("2000-03-01T00:00:00Z"), Some(951_868_800));
        assert_eq!(parse_time("2026-11-09T12:00:00.5Z"), Some(1_794_225_600));
        assert_eq!(parse_time("2026-11-09T12:00:00+01:00"), None);
        assert_eq!(parse_time("2026-13-09T12:00:00Z"), None);
    }

    fn table(text: &str) -> toml::Table {
        text.parse().unwrap()
    }

    #[test]
    fn sources_come_from_user_settings() {
        let (s, problems) = sources(&toml::Table::new());
        assert_eq!(s, [Source::default_source()]);
        assert!(problems.is_empty());

        let (s, problems) = sources(&table(&format!(
            "[plugin-sources.merlin]\nenabled = false\n\
             [plugin-sources.acme]\nurl = \"https://plugins.acme.dev/index.json\"\nkey = \"{}\"\n\
             [plugin-sources.plain]\nurl = \"http://x/index.json\"\nkey = \"{}\"\n\
             [plugin-sources.nokey]\nurl = \"https://x/index.json\"\n",
            DEFAULT_KEYS[0], DEFAULT_KEYS[0]
        )));
        assert_eq!(
            s.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
            ["acme"]
        );
        assert_eq!(problems.len(), 2, "{problems:?}");
    }

    /// A registry in memory: a key, and the files it serves.
    struct Registry {
        keys: minisign::KeyPair,
        files: std::cell::RefCell<BTreeMap<String, Vec<u8>>>,
        source: Source,
    }

    const URL: &str = "https://test.invalid/index.json";
    const FUTURE: &str = "2099-01-01T00:00:00Z";

    impl Registry {
        fn new() -> Self {
            let keys = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
            let source = Source {
                name: "test".into(),
                url: URL.into(),
                keys: vec![keys.pk.to_base64()],
            };
            Registry {
                keys,
                files: Default::default(),
                source,
            }
        }

        fn sign(&self, bytes: &[u8]) -> Vec<u8> {
            minisign::sign(Some(&self.keys.pk), &self.keys.sk, bytes, None, None)
                .unwrap()
                .to_bytes()
        }

        /// Serves a signed index of `plugins` (JSON object text).
        fn publish(&self, serial: u64, expires: &str, plugins: &str) -> Vec<u8> {
            let index = format!(
                r#"{{"schema":1,"serial":{serial},"generated":"2026-11-09T00:00:00Z","expires":"{expires}","plugins":{plugins}}}"#
            )
            .into_bytes();
            let mut files = self.files.borrow_mut();
            files.insert(format!("{URL}.minisig"), self.sign(&index));
            files.insert(URL.into(), index.clone());
            index
        }

        fn fetch(&self, url: &str, limit: u64) -> Result<Vec<u8>> {
            let bytes = self.files.borrow().get(url).cloned();
            let bytes = bytes.with_context(|| format!("{url}: not found"))?;
            if bytes.len() as u64 > limit {
                bail!("{url}: too large");
            }
            Ok(bytes)
        }
    }

    struct Temp(PathBuf);
    impl Temp {
        fn new(what: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "merlin-registry-{what}-{}-{}",
                std::process::id(),
                now()
            ));
            fs::create_dir_all(&dir).unwrap();
            Temp(dir)
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn an_index_must_be_signed_current_and_not_older() {
        let reg = Registry::new();
        let index = reg.publish(5, FUTURE, "{}");
        let sig = reg.sign(&index);
        assert_eq!(
            check(&reg.source, &index, &sig, now(), 0).unwrap().serial,
            5
        );
        assert_eq!(
            check(&reg.source, &index, &sig, now(), 5).unwrap().serial,
            5
        );

        let mut tampered = index.clone();
        tampered[20] ^= 1;
        let err = format!(
            "{:#}",
            check(&reg.source, &tampered, &sig, now(), 0).unwrap_err()
        );
        assert!(err.contains("isn't signed"), "{err}");

        let other = Registry::new();
        let err = format!(
            "{:#}",
            check(&other.source, &index, &sig, now(), 0).unwrap_err()
        );
        assert!(err.contains("isn't signed"), "{err}");

        let err = format!(
            "{:#}",
            check(&reg.source, &index, &sig, now(), 6).unwrap_err()
        );
        assert!(err.contains("older than number 6"), "{err}");

        let old = reg.publish(7, "2001-01-01T00:00:00Z", "{}");
        let err = format!(
            "{:#}",
            check(&reg.source, &old, &reg.sign(&old), now(), 0).unwrap_err()
        );
        assert!(err.contains("expired"), "{err}");

        let newer = br#"{"schema":2,"serial":1,"generated":"x","expires":"2099-01-01T00:00:00Z"}"#;
        let err = format!(
            "{:#}",
            check(&reg.source, newer, &reg.sign(newer), now(), 0).unwrap_err()
        );
        assert!(err.contains("format 2"), "{err}");
    }

    /// A built-looking plugin folder for `version`.
    fn plugin(root: &Path, version: &str) -> PathBuf {
        let dir = root.join(format!("demo-{version}"));
        fs::create_dir_all(dir.join("target")).unwrap();
        fs::write(
            dir.join("plugin.toml"),
            format!(
                "name = \"demo\"\ntitle = \"Demo\"\nversion = \"{version}\"\napi = \"{API_VERSION}\"\n\
                 component = \"target/demo.wasm\"\n[permissions]\neditor-read = true\n"
            ),
        )
        .unwrap();
        fs::write(dir.join("target/demo.wasm"), "\0asm").unwrap();
        dir
    }

    /// Packs `version` and serves it, returning its index entry's JSON.
    fn serve(reg: &Registry, root: &Path, version: &str) -> String {
        let packed = package::pack(&plugin(root, version), &root.join("out"), 0).unwrap();
        let url = format!("https://test.invalid/demo-{version}.merlin");
        reg.files
            .borrow_mut()
            .insert(url.clone(), fs::read(&packed.path).unwrap());
        format!(
            r#"{{"version":"{version}","api":"{API_VERSION}","url":"{url}","sha256":"{}","size":{},"permissions":{{"editor-read":true}}}}"#,
            packed.sha256, packed.size
        )
    }

    #[test]
    fn installing_from_a_registry_checks_the_download() {
        let tmp = Temp::new("install");
        let (plugins, data) = (tmp.0.join("plugins"), tmp.0.join("data"));
        let reg = Registry::new();
        let fetch = |u: &str, l: u64| reg.fetch(u, l);
        let v1 = serve(&reg, &tmp.0, "0.1.0");
        reg.publish(
            1,
            FUTURE,
            &format!(r#"{{"demo":{{"title":"Demo","versions":[{v1}]}}}}"#),
        );

        let loaded = load(&reg.source, &data, &fetch, now()).unwrap();
        assert!(loaded.stale.is_none());
        let entry = &loaded.index.plugins["demo"];
        let manifest = install(
            &loaded,
            "demo",
            entry.newest().unwrap(),
            &plugins,
            &data,
            &fetch,
        )
        .unwrap();
        assert_eq!(manifest.version, "0.1.0");
        assert_eq!(package::installed(&data)["demo"].source, "test");

        // A download that isn't what the index says installs nothing.
        let v2 = serve(&reg, &tmp.0, "0.2.0");
        reg.publish(
            2,
            FUTURE,
            &format!(r#"{{"demo":{{"title":"Demo","versions":[{v1},{v2}]}}}}"#),
        );
        reg.files
            .borrow_mut()
            .get_mut("https://test.invalid/demo-0.2.0.merlin")
            .unwrap()[100] ^= 1;
        let loaded = load(&reg.source, &data, &fetch, now()).unwrap();
        let entry = &loaded.index.plugins["demo"];
        let err = format!(
            "{:#}",
            install(
                &loaded,
                "demo",
                entry.newest().unwrap(),
                &plugins,
                &data,
                &fetch
            )
            .unwrap_err()
        );
        assert!(err.contains("doesn't match the index"), "{err}");
        assert_eq!(
            crate::manifest(&plugins.join("demo")).unwrap().version,
            "0.1.0"
        );
    }

    #[test]
    fn an_old_index_served_again_is_refused() {
        let tmp = Temp::new("rollback");
        let reg = Registry::new();
        let fetch = |u: &str, l: u64| reg.fetch(u, l);
        let old_index = reg.publish(3, FUTURE, "{}");
        let old_sig = reg.sign(&old_index);
        reg.publish(4, FUTURE, "{}");
        assert_eq!(
            load(&reg.source, &tmp.0, &fetch, now())
                .unwrap()
                .index
                .serial,
            4
        );

        // Someone serves the older, still validly signed index.
        reg.files.borrow_mut().insert(URL.into(), old_index);
        reg.files
            .borrow_mut()
            .insert(format!("{URL}.minisig"), old_sig);
        let loaded = load(&reg.source, &tmp.0, &fetch, now()).unwrap();
        assert_eq!(loaded.index.serial, 4, "the last good one stays");
        assert!(loaded.stale.unwrap().contains("older than number 4"));
    }

    #[test]
    fn offline_the_last_good_index_shows_but_installs_nothing() {
        let tmp = Temp::new("offline");
        let reg = Registry::new();
        let fetch = |u: &str, l: u64| reg.fetch(u, l);
        let v1 = serve(&reg, &tmp.0, "0.1.0");
        reg.publish(
            1,
            FUTURE,
            &format!(r#"{{"demo":{{"title":"Demo","versions":[{v1}]}}}}"#),
        );
        load(&reg.source, &tmp.0, &fetch, now()).unwrap();

        let offline = |u: &str, _: u64| -> Result<Vec<u8>> { bail!("{u}: no network") };
        let loaded = load(&reg.source, &tmp.0, &offline, now()).unwrap();
        assert!(loaded.stale.as_deref().unwrap().contains("no network"));
        let entry = &loaded.index.plugins["demo"];
        let err = install(
            &loaded,
            "demo",
            entry.newest().unwrap(),
            &tmp.0.join("p"),
            &tmp.0,
            &fetch,
        )
        .unwrap_err();
        assert!(format!("{err:#}").contains("can't reach test"), "{err:#}");
    }

    #[test]
    fn the_newest_usable_version_is_chosen() {
        let v = |version: &str, api: &str| {
            format!(
                r#"{{"version":"{version}","api":"{api}","url":"https://x","sha256":"","size":1}}"#
            )
        };
        let json = format!(
            r#"{{"title":"T","versions":[{},{},{},{},{}]}}"#,
            v("0.1.0", API_VERSION),
            v("0.3.0", "0.1"),
            v("0.2.0", API_VERSION),
            v("0.2.1", API_VERSION).replace("}", r#","yanked":"broken"}"#),
            v("0.4.0-beta.1", API_VERSION),
        );
        let entry: Entry = serde_json::from_str(&json).unwrap();
        assert_eq!(entry.newest().unwrap().version, "0.2.0");
    }
}
