//! Mutually authenticated TLS 1.3 between devices.
//!
//! Devices use self-signed certificates, so certificate chains mean nothing here.
//! The verifier still checks the handshake signature (proving the peer holds the
//! key for the certificate it presented); the engine then decides whether that
//! certificate's fingerprint belongs to a paired device.

use std::sync::Arc;

use anyhow::{Result, anyhow};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{CryptoProvider, verify_tls12_signature, verify_tls13_signature};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
use rustls::{ClientConfig, DigitallySignedStruct, DistinguishedName, Error, ServerConfig, SignatureScheme};
use tokio_rustls::{TlsAcceptor, TlsConnector};

use crate::identity::{Identity, fingerprint};

#[derive(Debug)]
struct FingerprintVerifier(Arc<CryptoProvider>);

impl ServerCertVerifier for FingerprintVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(&self, msg: &[u8], cert: &CertificateDer<'_>, dss: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, Error> {
        verify_tls12_signature(msg, cert, dss, &self.0.signature_verification_algorithms)
    }

    fn verify_tls13_signature(&self, msg: &[u8], cert: &CertificateDer<'_>, dss: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, Error> {
        verify_tls13_signature(msg, cert, dss, &self.0.signature_verification_algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

impl ClientCertVerifier for FingerprintVerifier {
    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        &[]
    }

    fn verify_client_cert(&self, _end_entity: &CertificateDer<'_>, _intermediates: &[CertificateDer<'_>], _now: UnixTime) -> Result<ClientCertVerified, Error> {
        Ok(ClientCertVerified::assertion())
    }

    fn verify_tls12_signature(&self, msg: &[u8], cert: &CertificateDer<'_>, dss: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, Error> {
        verify_tls12_signature(msg, cert, dss, &self.0.signature_verification_algorithms)
    }

    fn verify_tls13_signature(&self, msg: &[u8], cert: &CertificateDer<'_>, dss: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, Error> {
        verify_tls13_signature(msg, cert, dss, &self.0.signature_verification_algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

fn provider() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

pub fn acceptor(id: &Identity) -> Result<TlsAcceptor> {
    let p = provider();
    let config = ServerConfig::builder_with_provider(p.clone())
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .with_client_cert_verifier(Arc::new(FingerprintVerifier(p)))
        .with_single_cert(vec![id.cert.clone()], id.private_key())?;
    Ok(TlsAcceptor::from(Arc::new(config)))
}

pub fn connector(id: &Identity) -> Result<TlsConnector> {
    let p = provider();
    let config = ClientConfig::builder_with_provider(p.clone())
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(FingerprintVerifier(p)))
        .with_client_auth_cert(vec![id.cert.clone()], id.private_key())?;
    Ok(TlsConnector::from(Arc::new(config)))
}

pub fn server_name() -> ServerName<'static> {
    ServerName::try_from("the-tunnel.local").expect("static name is valid")
}

pub fn peer_id(certs: Option<&[CertificateDer<'_>]>) -> Result<String> {
    certs
        .and_then(|c| c.first())
        .map(|c| fingerprint(c.as_ref()))
        .ok_or_else(|| anyhow!("peer did not present a certificate"))
}
