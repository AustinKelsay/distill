//! Backups (ADR 0009): copies of the backup set ([`Home::keep`]) at targets the owner chooses,
//! encrypted on this machine before an adapter sees them. This part is the backup key (issue
//! #147): it encrypts every object with age and names it by a keyed hash, so a target learns only
//! how many objects it holds, their sizes and when they arrive.
//!
//! - **The key** is an age X25519 identity in `keys/backup.key`, owner-only. It is made at backup
//!   setup and replaced only when the owner asks ([`BackupKey::replace`]): a new key starts every
//!   target over.
//! - **The owner's copy** is the same key as text, shown to the owner to keep somewhere other
//!   than this machine. Without it or the home's copy, no one can open the backups.
//! - **Object names** are an HMAC-SHA256, under a key derived from the backup key, of what the
//!   object is (a blob's capture hash, a file's path). A plain SHA-256 would let a target test
//!   whether it holds a file it already knows.

pub mod restore;
pub mod run;
pub mod status;
pub mod store;
pub mod target;

pub use restore::{BackupCheck, RestoreReport, restore, verify};
pub use run::{BackupReport, run};
pub use status::{TargetStatus, TickBackups, targets};
pub use target::{Place, add_folder, places, reused_name, synced, test};

/// The backup key's text, for a face to show the owner.
pub fn expose(recovery: &SecretString) -> &str {
    recovery.expose_secret()
}

use std::fs;
use std::io::{Read, Write};

use age::secrecy::{ExposeSecret, SecretString};
use anyhow::{Context, Result, anyhow, bail};
use hmac::{Hmac, Mac};
use sha2::Sha256;

use crate::home::Home;
use crate::util::{create_dir_private, private_file, set_private, sync_dir};

/// This home's backup key.
pub struct BackupKey {
    identity: age::x25519::Identity,
    names: [u8; 32],
}

impl BackupKey {
    /// Make this home's backup key. Refused when it has one: a new key would start every target
    /// over, so that is never done by accident.
    pub fn create(home: &Home) -> Result<Self> {
        let key = Self::from_identity(age::x25519::Identity::generate());
        key.install(home)?;
        Ok(key)
    }

    /// Make this key `home`'s backup key, as a restore does. Refused when it has one.
    pub fn install(&self, home: &Home) -> Result<()> {
        let dir = home.keys_dir();
        let made_root = !home.root.exists();
        create_dir_private(&dir).with_context(|| format!("create {}", dir.display()))?;
        set_private(&home.root);
        set_private(&dir);
        let mut nonce = [0u8; 8];
        getrandom::fill(&mut nonce).map_err(|e| anyhow!("generate a file name: {e}"))?;
        let tmp = dir.join(format!(".backup.key.{}", hex::encode(nonce)));
        let placed = (|| -> Result<()> {
            let mut f = private_file()
                .write(true)
                .create_new(true)
                .open(&tmp)
                .with_context(|| format!("create {}", tmp.display()))?;
            f.write_all(self.recovery().expose_secret().as_bytes())?;
            f.sync_all()?;
            // The `keys/` entry must be durable before the key is put in place: a crash must not
            // lose the key the owner is about to be shown, and a failure here leaves no key.
            sync_dir(&home.root)?;
            match fs::hard_link(&tmp, home.backup_key_path()) {
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    bail!("this home already has a backup key")
                }
                r => r.context("put the backup key in place"),
            }
        })();
        let _ = fs::remove_file(&tmp);
        placed?;
        sync_dir(&dir)?;
        // A home made here (before `distill init`) must not vanish with its key either.
        if made_root && let Some(parent) = home.root.parent() {
            // A relative home's parent is empty: the current folder.
            let parent = if parent.as_os_str().is_empty() {
                std::path::Path::new(".")
            } else {
                parent
            };
            sync_dir(parent)?;
        }
        Ok(())
    }

    /// Replace this home's backup key with a new one, and log it, so every target starts over
    /// under the new key: its runs are named and sealed with it. What a target holds already
    /// stays, and the old key's copy still opens it; the home keeps no old key. Refused while a
    /// backup runs, and when the home has no key to replace.
    pub fn replace(home: &Home) -> Result<Self> {
        let _backup = home
            .try_lock("backup")?
            .context("a backup is running; make a new key when it has finished")?;
        if Self::existing(home)?.is_none() {
            bail!("this home has no backup key to replace: make one first");
        }
        let key = Self::from_identity(age::x25519::Identity::generate());
        let dir = home.keys_dir();
        let mut nonce = [0u8; 8];
        getrandom::fill(&mut nonce).map_err(|e| anyhow!("generate a file name: {e}"))?;
        let tmp = dir.join(format!(".backup.key.{}", hex::encode(nonce)));
        let placed = (|| -> Result<()> {
            let mut f = private_file()
                .write(true)
                .create_new(true)
                .open(&tmp)
                .with_context(|| format!("create {}", tmp.display()))?;
            f.write_all(key.recovery().expose_secret().as_bytes())?;
            f.sync_all()?;
            // Logged first: a new key the log doesn't name would leave every target refusing
            // the tick (a target that had runs and holds none under the key), while a logged one
            // that didn't land only has the old key start its runs over, which finds them.
            // On a line of its own, even after a run's line a crash cut short.
            let log = home.backup_log_path();
            if fs::read(&log).is_ok_and(|b| b.last().is_some_and(|c| *c != b'\n')) {
                let mut f = private_file().append(true).open(&log)?;
                f.write_all(b"\n")?;
                f.sync_data()?;
            }
            crate::util::append_jsonl(
                &log,
                &run::BackupReport {
                    at: crate::util::now_rfc3339(),
                    new_key: true,
                    ..Default::default()
                },
            )?;
            fs::rename(&tmp, home.backup_key_path()).context("put the new backup key in place")
        })();
        if placed.is_err() {
            let _ = fs::remove_file(&tmp);
        }
        placed?;
        // The new key is in place, so the owner must see it. A rename a crash loses brings the
        // old key back, whose runs then start over from what the target holds.
        let _ = sync_dir(&dir);
        Ok(key)
    }

    /// This home's backup key, if it has one.
    pub fn existing(home: &Home) -> Result<Option<Self>> {
        let path = home.backup_key_path();
        match fs::read_to_string(&path).map(SecretString::from) {
            Ok(text) => Self::from_recovery(text.expose_secret())
                .map(Some)
                .with_context(|| format!("{} is not a backup key", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e).with_context(|| format!("read {}", path.display())),
        }
    }

    /// The key from its recovery text, as the owner kept it.
    pub fn from_recovery(text: &str) -> Result<Self> {
        let identity = text
            .trim()
            .parse::<age::x25519::Identity>()
            .map_err(|e| anyhow!("not a backup key: {e}"))?;
        Ok(Self::from_identity(identity))
    }

    fn from_identity(identity: age::x25519::Identity) -> Self {
        let mut m = Hmac::<Sha256>::new_from_slice(identity.to_string().expose_secret().as_bytes())
            .expect("HMAC takes any key length");
        m.update(b"\0distill backup object names v1");
        Self {
            identity,
            names: m.finalize().into_bytes().into(),
        }
    }

    /// The key as text, for the owner to keep somewhere other than this machine. Kept secret
    /// (no `Debug`, wiped when dropped) until a caller exposes it to write or show it.
    pub fn recovery(&self) -> SecretString {
        self.identity.to_string()
    }

    /// The name a target stores the object `what` under: a keyed hash, the same for the same
    /// object under the same key and unlinkable to it without the key.
    pub fn name(&self, what: &str) -> String {
        let mut m = Hmac::<Sha256>::new_from_slice(&self.names).expect("HMAC takes any key length");
        m.update(what.as_bytes());
        hex::encode(m.finalize().into_bytes())
    }

    /// Encrypt everything `from` yields into `to`, as an age file only this key opens.
    pub fn seal(&self, mut from: impl Read, to: impl Write) -> Result<()> {
        let recipient = self.identity.to_public();
        let encryptor =
            age::Encryptor::with_recipients(std::iter::once(&recipient as &dyn age::Recipient))
                .map_err(|e| anyhow!("encrypt: {e}"))?;
        let mut w = encryptor.wrap_output(to).context("encrypt")?;
        std::io::copy(&mut from, &mut w).context("encrypt")?;
        w.finish().context("encrypt")?.flush().context("encrypt")?;
        Ok(())
    }

    /// Decrypt the age file `from` yields into `to`. Fails when another key made it or it was
    /// changed; chunks before a changed one have already been written, so a caller writes to a
    /// temporary place and keeps it only when this succeeds.
    pub fn open(&self, from: impl Read, mut to: impl Write) -> Result<()> {
        let decryptor = age::Decryptor::new(from).map_err(|e| anyhow!("decrypt: {e}"))?;
        let mut r = decryptor
            .decrypt(std::iter::once(&self.identity as &dyn age::Identity))
            .map_err(|e| anyhow!("decrypt: {e}"))?;
        std::io::copy(&mut r, &mut to).context("decrypt")?;
        to.flush().context("decrypt")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sealed(key: &BackupKey, bytes: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        key.seal(bytes, &mut out).unwrap();
        out
    }

    fn opened(key: &BackupKey, bytes: &[u8]) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        key.open(bytes, &mut out)?;
        Ok(out)
    }

    #[test]
    fn a_home_gets_one_owner_only_backup_key_that_its_recovery_text_restores() {
        let dir = tempfile::tempdir().unwrap();
        let home = Home::new(dir.path().join("home"));
        assert!(BackupKey::existing(&home).unwrap().is_none());
        let key = BackupKey::create(&home).unwrap();
        let err = BackupKey::create(&home).err().unwrap().to_string();
        assert!(err.contains("already has a backup key"), "{err}");

        let mode = |p: &std::path::Path| {
            use std::os::unix::fs::PermissionsExt;
            fs::metadata(p).unwrap().permissions().mode() & 0o777
        };
        assert_eq!(mode(&home.backup_key_path()), 0o600);
        assert_eq!(mode(&home.keys_dir()), 0o700);
        let leftovers: Vec<_> = fs::read_dir(home.keys_dir())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(leftovers, ["backup.key"]);

        let again = BackupKey::existing(&home).unwrap().unwrap();
        let recovered =
            BackupKey::from_recovery(&format!("  {}\n", key.recovery().expose_secret())).unwrap();
        for k in [&again, &recovered] {
            assert_eq!(k.recovery().expose_secret(), key.recovery().expose_secret());
            assert_eq!(k.name("blob/ab12"), key.name("blob/ab12"));
        }
        assert!(BackupKey::from_recovery("not a key").is_err());
    }

    #[test]
    fn objects_round_trip_and_only_their_key_opens_them() {
        let dir = tempfile::tempdir().unwrap();
        let key = BackupKey::create(&Home::new(dir.path().join("a"))).unwrap();
        let other = BackupKey::create(&Home::new(dir.path().join("b"))).unwrap();
        let plain = b"a session, secrets included".repeat(10_000);
        let box_ = sealed(&key, &plain);
        assert!(
            !box_
                .windows(b"secrets included".len())
                .any(|w| w == b"secrets included"),
            "ciphertext only"
        );
        assert_eq!(opened(&key, &box_).unwrap(), plain);
        assert!(opened(&other, &box_).is_err(), "another key");
        let mut tampered = box_.clone();
        let last = tampered.len() - 1;
        tampered[last] ^= 1;
        assert!(opened(&key, &tampered).is_err(), "a changed object");
        assert_eq!(opened(&key, &sealed(&key, b"")).unwrap(), b"");
    }

    #[test]
    fn names_are_keyed_so_a_target_cannot_test_for_a_known_file() {
        let dir = tempfile::tempdir().unwrap();
        let key = BackupKey::create(&Home::new(dir.path().join("a"))).unwrap();
        let other = BackupKey::create(&Home::new(dir.path().join("b"))).unwrap();
        let hash = "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08";
        let name = key.name(hash);
        assert_eq!(name.len(), 64);
        assert_ne!(name, hash, "not the plain hash");
        assert_ne!(name, other.name(hash), "another key, another name");
        assert_ne!(name, key.name("9f86d081"), "another object, another name");
    }

    #[test]
    fn a_writer_that_cannot_save_fails_seal_and_open() {
        use std::io::BufWriter;
        struct Refuses;
        impl Write for Refuses {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("disk full"))
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let key = BackupKey::create(&Home::new(dir.path().join("a"))).unwrap();
        assert!(key.seal(&b"x"[..], BufWriter::new(Refuses)).is_err());
        let box_ = sealed(&key, b"x");
        assert!(key.open(box_.as_slice(), BufWriter::new(Refuses)).is_err());
    }
}
