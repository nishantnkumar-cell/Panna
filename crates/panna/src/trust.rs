//! The certificates the user trusts, kept as certificate files in the config directory: a
//! signature whose certificate leads to one of them shows as trusted.

use std::path::PathBuf;

use panna_core::Cert;

/// More certificate files than this, or larger ones, aren't read.
const MAX_CERTS: usize = 200;
pub const MAX_FILE: u64 = 64 * 1024;

#[derive(Debug, Clone)]
pub enum Event {
    /// Trust this certificate.
    Add(Cert),
    /// Stop trusting the certificate with this fingerprint.
    Remove(String),
    /// Choose a certificate file to trust.
    Import,
    Imported(Option<PathBuf>),
}

#[derive(Debug, Default)]
pub struct Trusted {
    pub certs: Vec<Cert>,
}

fn dir() -> Option<PathBuf> {
    crate::settings::config_dir().map(|d| d.join("trusted"))
}

/// A certificate file's contents, if it isn't too large.
pub fn read(path: &std::path::Path) -> Result<Vec<u8>, String> {
    match std::fs::metadata(path) {
        Ok(m) if m.len() > MAX_FILE => Err("That file is too large to be a certificate.".into()),
        Ok(_) => std::fs::read(path).map_err(|e| e.to_string()),
        Err(e) => Err(e.to_string()),
    }
}

impl Trusted {
    pub fn load() -> Self {
        let mut certs = Vec::new();
        let entries = dir().and_then(|d| std::fs::read_dir(d).ok());
        for entry in entries.into_iter().flatten().flatten().take(MAX_CERTS) {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "cer")
                && let Ok(bytes) = read(&path)
                && let Ok(cert) = Cert::from_file(&bytes)
                && !certs.iter().any(|c: &Cert| c.fingerprint == cert.fingerprint)
            {
                certs.push(cert);
            }
        }
        certs.sort_by(|a, b| a.name.cmp(&b.name));
        Self { certs }
    }

    /// Saves the certificate as `<fingerprint>.cer` and trusts it.
    pub fn add(&mut self, cert: Cert) -> Result<(), String> {
        if self.certs.iter().any(|c| c.fingerprint == cert.fingerprint) {
            return Ok(());
        }
        if self.certs.len() >= MAX_CERTS {
            return Err("Too many certificates are trusted already.".into());
        }
        let dir = dir().ok_or("There's no settings folder to keep it in.")?;
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        std::fs::write(dir.join(format!("{}.cer", cert.fingerprint)), cert.der())
            .map_err(|e| e.to_string())?;
        self.certs.push(cert);
        self.certs.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(())
    }

    pub fn remove(&mut self, fingerprint: &str) -> Result<(), String> {
        // Fingerprints are hex, so this names a file in the folder and nothing else.
        if fingerprint.is_empty() || !fingerprint.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Ok(());
        }
        if let Some(dir) = dir() {
            match std::fs::remove_file(dir.join(format!("{fingerprint}.cer"))) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.to_string()),
            }
        }
        self.certs.retain(|c| c.fingerprint != fingerprint);
        Ok(())
    }
}
