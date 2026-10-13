//! A device's long-term identity: a self-signed certificate whose SHA-256
//! fingerprint is the device ID. Names are just labels; this is what peers trust.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use sha2::{Digest, Sha256};

pub struct Identity {
    pub cert: CertificateDer<'static>,
    key: PrivatePkcs8KeyDer<'static>,
    pub device_id: String,
}

impl Identity {
    pub fn load_or_create(dir: &Path) -> Result<Self> {
        let cert_path = dir.join("identity.cert.der");
        let key_path = dir.join("identity.key.der");

        let (cert, key) = if cert_path.exists() && key_path.exists() {
            (
                fs::read(&cert_path).context("reading device certificate")?,
                fs::read(&key_path).context("reading device key")?,
            )
        } else {
            let generated = rcgen::generate_simple_self_signed(vec!["the-tunnel.local".to_string()])
                .context("generating device identity")?;
            let cert = generated.cert.der().to_vec();
            let key = generated.signing_key.serialize_der();
            write_private(&key_path, &key)?;
            fs::write(&cert_path, &cert)?;
            (cert, key)
        };

        Ok(Self {
            device_id: fingerprint(&cert),
            cert: CertificateDer::from(cert),
            key: PrivatePkcs8KeyDer::from(key),
        })
    }

    pub fn private_key(&self) -> PrivateKeyDer<'static> {
        PrivateKeyDer::Pkcs8(self.key.clone_key())
    }
}

pub fn fingerprint(der: &[u8]) -> String {
    hex::encode(Sha256::digest(der))
}

#[cfg(unix)]
fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    f.write_all(bytes)?;
    Ok(())
}

#[cfg(not(unix))]
fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    // On Windows the per-user app data folder is already restricted to the user.
    fs::write(path, bytes)?;
    Ok(())
}
