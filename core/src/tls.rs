use anyhow::Result;
pub const GROUP_ALPN: &[u8] = b"psychotalk/2";
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{WebPkiSupportedAlgorithms, ring, verify_tls13_signature};
use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer, ServerName, UnixTime};
use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
use rustls::{
    ClientConfig, DigitallySignedStruct, DistinguishedName, Error, ServerConfig, SignatureScheme,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use zeroize::{Zeroize, ZeroizeOnDrop};

#[derive(Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
pub struct Identity {
    pub cert: Vec<u8>,
    pub key: Vec<u8>,
}
impl Identity {
    pub fn generate() -> Result<Self> {
        let c = rcgen::generate_simple_self_signed(vec!["psychotalk.invalid".into()])?;
        Ok(Self {
            cert: c.cert.der().to_vec(),
            key: c.signing_key.serialize_der(),
        })
    }
    pub fn pin(&self) -> [u8; 32] {
        Sha256::digest(&self.cert).into()
    }
}

#[derive(Debug)]
struct Pinned {
    pins: Vec<[u8; 32]>,
    algorithms: WebPkiSupportedAlgorithms,
}
impl Pinned {
    fn check(&self, cert: &CertificateDer<'_>, chain: &[CertificateDer<'_>]) -> Result<(), Error> {
        if chain.is_empty() && self.pins.contains(&fingerprint(cert)) {
            Ok(())
        } else {
            Err(Error::General("unrecognized peer".into()))
        }
    }
    fn sig(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        // Pinning establishes trust; rustls/webpki still proves possession of the private key.
        verify_tls13_signature(message, cert, dss, &self.algorithms)
    }
}
impl ServerCertVerifier for Pinned {
    fn verify_server_cert(
        &self,
        c: &CertificateDer<'_>,
        i: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> Result<ServerCertVerified, Error> {
        self.check(c, i)?;
        Ok(ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        _: &[u8],
        _: &CertificateDer<'_>,
        _: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        Err(Error::General("TLS 1.3 required".into()))
    }
    fn verify_tls13_signature(
        &self,
        m: &[u8],
        c: &CertificateDer<'_>,
        d: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        self.sig(m, c, d)
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algorithms.supported_schemes()
    }
}
impl ClientCertVerifier for Pinned {
    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        &[]
    }
    fn verify_client_cert(
        &self,
        c: &CertificateDer<'_>,
        i: &[CertificateDer<'_>],
        _: UnixTime,
    ) -> Result<ClientCertVerified, Error> {
        self.check(c, i)?;
        Ok(ClientCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        _: &[u8],
        _: &CertificateDer<'_>,
        _: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        Err(Error::General("TLS 1.3 required".into()))
    }
    fn verify_tls13_signature(
        &self,
        m: &[u8],
        c: &CertificateDer<'_>,
        d: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        self.sig(m, c, d)
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algorithms.supported_schemes()
    }
}
pub fn configs(
    identity: &Identity,
    pin: [u8; 32],
) -> Result<(Arc<ClientConfig>, Arc<ServerConfig>)> {
    let provider = Arc::new(ring::default_provider());
    let verifier = Arc::new(Pinned {
        pins: vec![pin],
        algorithms: provider.signature_verification_algorithms,
    });
    let certs = vec![CertificateDer::from(identity.cert.clone())];
    let key = || PrivatePkcs8KeyDer::from(identity.key.clone()).into();
    let mut client = ClientConfig::builder_with_provider(provider.clone())
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .dangerous()
        .with_custom_certificate_verifier(verifier.clone())
        .with_client_auth_cert(certs.clone(), key())?;
    client.resumption = rustls::client::Resumption::disabled();
    client.enable_early_data = false;
    client.alpn_protocols = vec![GROUP_ALPN.to_vec()];
    Ok((Arc::new(client), server_config(identity, vec![pin])?))
}
pub fn fingerprint(cert: &CertificateDer<'_>) -> [u8; 32] {
    Sha256::digest(cert.as_ref()).into()
}
pub fn server_config(identity: &Identity, pins: Vec<[u8; 32]>) -> Result<Arc<ServerConfig>> {
    let provider = Arc::new(ring::default_provider());
    let verifier = Arc::new(Pinned {
        pins,
        algorithms: provider.signature_verification_algorithms,
    });
    let certs = vec![CertificateDer::from(identity.cert.clone())];
    let key = PrivatePkcs8KeyDer::from(identity.key.clone()).into();
    let mut server = ServerConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .with_client_cert_verifier(verifier)
        .with_single_cert(certs, key)?;
    server.session_storage = Arc::new(rustls::server::NoServerSessionStorage {});
    server.send_tls13_tickets = 0;
    server.max_early_data_size = 0;
    server.alpn_protocols = vec![GROUP_ALPN.to_vec()];
    Ok(Arc::new(server))
}
