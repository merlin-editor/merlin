//! Updating merlin itself.
//!
//! Each release carries `SHA256SUMS`, the SHA-256 of each of its archives,
//! and `SHA256SUMS.minisig`, a minisign signature over it by the release
//! key, whose trusted comment is `merlin <tag>`. An archive counts as that
//! release's only when the list is signed by the key merlin was built with,
//! the comment names the release asked for, and the archive's hash is on
//! the list. A checksum beside the archive on the same page would prove
//! nothing against someone who can change the page.

// The updater (finding a release and installing it) is built on this next.
#![cfg_attr(not(test), allow(dead_code))]

use std::collections::BTreeMap;

use anyhow::{Context, Result, bail};

/// The release key's public half, as `minisign -G` wrote it. The repository
/// keeps it at `keys/release.pub`; the release workflow checks with it too.
const RELEASE_PUB: &str = include_str!("../../../keys/release.pub");

/// The base64 public key from [`RELEASE_PUB`]: its line after the comment.
pub fn release_key() -> &'static str {
    RELEASE_PUB
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with("untrusted comment:"))
        .unwrap_or_default()
}

/// Checks `sums` against `signature` with `key` and that it was signed for
/// `tag`, and reads it: each archive's file name to its SHA-256, in hex.
pub fn check_sums(
    key: &str,
    sums: &[u8],
    signature: &[u8],
    tag: &str,
) -> Result<BTreeMap<String, String>> {
    let key = minisign_verify::PublicKey::from_base64(key)
        .map_err(|e| anyhow::anyhow!("the release key can't be read: {e}"))?;
    let sig_text = std::str::from_utf8(signature).context("the signature isn't text")?;
    let sig = minisign_verify::Signature::decode(sig_text)
        .map_err(|e| anyhow::anyhow!("the signature can't be read: {e}"))?;
    if key.verify(sums, &sig, false).is_err() {
        bail!("{tag}'s SHA256SUMS isn't signed by merlin's release key");
    }
    let want = format!("merlin {tag}");
    if sig.trusted_comment() != want {
        bail!(
            "{tag}'s SHA256SUMS was signed for {:?}, not {want:?}",
            sig.trusted_comment()
        );
    }
    let text = std::str::from_utf8(sums).context("SHA256SUMS isn't text")?;
    let mut out = BTreeMap::new();
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        // `sha256sum` writes `<hex>  <name>`, or `<hex> *<name>` in binary mode.
        let (hex, name) = line
            .split_once(' ')
            .with_context(|| format!("SHA256SUMS has a line it can't read: {line:?}"))?;
        let name = name.trim_start_matches([' ', '*']);
        if hex.len() != 64 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) || name.is_empty() {
            bail!("SHA256SUMS has a line it can't read: {line:?}");
        }
        out.insert(name.to_string(), hex.to_ascii_lowercase());
    }
    Ok(out)
}

/// Checks that `bytes` are the archive `name` from a list [`check_sums`]
/// read.
pub fn check_archive(sums: &BTreeMap<String, String>, name: &str, bytes: &[u8]) -> Result<()> {
    let want = sums
        .get(name)
        .with_context(|| format!("the release's SHA256SUMS doesn't list {name}"))?;
    let got = merlin_plugins::package::sha256_hex(bytes);
    if &got != want {
        bail!("{name} doesn't match the release's SHA256SUMS: expected {want}, got {got}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Signer {
        keys: minisign::KeyPair,
    }

    impl Signer {
        fn new() -> Self {
            Signer {
                keys: minisign::KeyPair::generate_unencrypted_keypair().unwrap(),
            }
        }
        fn key(&self) -> String {
            self.keys.pk.to_base64()
        }
        fn sign(&self, bytes: &[u8], comment: &str) -> Vec<u8> {
            let sig = minisign::sign(
                Some(&self.keys.pk),
                &self.keys.sk,
                std::io::Cursor::new(bytes),
                Some(comment),
                None,
            )
            .unwrap();
            sig.to_string().into_bytes()
        }
    }

    fn sums_for(archive: &[u8]) -> Vec<u8> {
        let hex = merlin_plugins::package::sha256_hex(archive);
        format!(
            "{hex}  merlin-linux-x86_64.tar.gz\n{} *merlin-windows-x86_64.zip\n",
            "a".repeat(64)
        )
        .into_bytes()
    }

    #[test]
    fn the_embedded_key_is_the_repositorys() {
        let key = release_key();
        assert!(key.starts_with("RW"), "{key}");
        minisign_verify::PublicKey::from_base64(key).unwrap();
    }

    #[test]
    fn a_signed_list_for_the_tag_reads_and_vouches_for_its_archives() {
        let s = Signer::new();
        let archive = b"an archive".as_slice();
        let sums = sums_for(archive);
        let sig = s.sign(&sums, "merlin v0.2.0");
        let read = check_sums(&s.key(), &sums, &sig, "v0.2.0").unwrap();
        assert_eq!(read.len(), 2);
        assert!(read.contains_key("merlin-windows-x86_64.zip"));
        check_archive(&read, "merlin-linux-x86_64.tar.gz", archive).unwrap();
        let err = check_archive(&read, "merlin-linux-x86_64.tar.gz", b"changed").unwrap_err();
        assert!(err.to_string().contains("doesn't match"), "{err}");
        let err = check_archive(&read, "merlin-macos-aarch64.tar.gz", archive).unwrap_err();
        assert!(err.to_string().contains("doesn't list"), "{err}");
    }

    #[test]
    fn another_key_a_changed_list_or_another_tag_is_refused() {
        let s = Signer::new();
        let sums = sums_for(b"x");
        let sig = s.sign(&sums, "merlin v0.2.0");

        let other = Signer::new();
        let err = check_sums(&other.key(), &sums, &sig, "v0.2.0").unwrap_err();
        assert!(err.to_string().contains("isn't signed"), "{err}");

        let mut changed = sums.clone();
        changed[0] = if changed[0] == b'0' { b'1' } else { b'0' };
        let err = check_sums(&s.key(), &changed, &sig, "v0.2.0").unwrap_err();
        assert!(err.to_string().contains("isn't signed"), "{err}");

        // An older release's signed list served as the newest one.
        let err = check_sums(&s.key(), &sums, &sig, "v0.3.0").unwrap_err();
        assert!(err.to_string().contains("signed for"), "{err}");
    }

    #[test]
    fn a_list_it_cant_read_is_refused() {
        let s = Signer::new();
        let sums = b"not a checksum line\n".to_vec();
        let sig = s.sign(&sums, "merlin v0.2.0");
        let err = check_sums(&s.key(), &sums, &sig, "v0.2.0").unwrap_err();
        assert!(err.to_string().contains("can't read"), "{err}");
    }
}
