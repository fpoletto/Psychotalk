//! Loopback-only adversarial TLS tests, not a substitute for the onion probe.
use psychotalk_core::{
    tls::{Identity, configs},
    transport::{read_frame, write_frame},
};
use rustls::{
    ClientConfig, ClientConnection, ServerConnection, SignatureScheme, StreamOwned,
    pki_types::{CertificateDer, PrivatePkcs8KeyDer, ServerName},
};
use std::{
    net::{TcpListener, TcpStream},
    sync::Arc,
    thread,
    time::Duration,
};

fn exchange(client: Arc<ClientConfig>, server: Arc<rustls::ServerConfig>) -> (bool, bool) {
    let expected = client
        .alpn_protocols
        .iter()
        .find(|p| server.alpn_protocols.contains(p))
        .cloned();
    let server_expected = expected.clone();
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = l.local_addr().unwrap();
    let receive = thread::spawn(move || {
        let (socket, _) = l.accept().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut s = StreamOwned::new(ServerConnection::new(server).unwrap(), socket);
        if read_frame(&mut s).is_err() {
            return false;
        }
        assert_eq!(
            s.conn.protocol_version(),
            Some(rustls::ProtocolVersion::TLSv1_3)
        );
        assert_eq!(s.conn.alpn_protocol(), server_expected.as_deref());
        write_frame(&mut s, b"persisted").is_ok()
    });
    let socket = TcpStream::connect(address).unwrap();
    socket
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let mut s = StreamOwned::new(
        ClientConnection::new(client, ServerName::try_from("psychotalk.invalid").unwrap()).unwrap(),
        socket,
    );
    let ok = write_frame(&mut s, b"hello")
        .and_then(|_| read_frame(&mut s))
        .is_ok();
    if ok {
        assert_eq!(s.conn.alpn_protocol(), expected.as_deref());
    }
    (ok, receive.join().unwrap())
}
#[test]
fn expected_peer_and_pin_required_both_ways() {
    let a = Identity::generate().unwrap();
    let b = Identity::generate().unwrap();
    let stranger = Identity::generate().unwrap();
    let (client, _) = configs(&a, b.pin()).unwrap();
    let (_, server) = configs(&b, a.pin()).unwrap();
    assert_eq!(exchange(client, server), (true, true));
    let (client, _) = configs(&stranger, b.pin()).unwrap();
    let (_, server) = configs(&b, a.pin()).unwrap();
    assert_eq!(exchange(client, server), (false, false));
    let (client, _) = configs(&a, stranger.pin()).unwrap();
    let (_, server) = configs(&b, a.pin()).unwrap();
    assert_eq!(exchange(client, server), (false, false));
}

#[derive(Debug)]
struct WrongSigner(Arc<rustls::sign::CertifiedKey>);
impl rustls::client::ResolvesClientCert for WrongSigner {
    fn resolve(
        &self,
        _: &[&[u8]],
        _: &[SignatureScheme],
    ) -> Option<Arc<rustls::sign::CertifiedKey>> {
        Some(self.0.clone())
    }
    fn has_certs(&self) -> bool {
        true
    }
}
#[test]
fn pinned_certificate_without_private_key_is_rejected() {
    let a = Identity::generate().unwrap();
    let b = Identity::generate().unwrap();
    let thief = Identity::generate().unwrap();
    let (good, _) = configs(&a, b.pin()).unwrap();
    let key = rustls::crypto::ring::sign::any_supported_type(
        &PrivatePkcs8KeyDer::from(thief.key.clone()).into(),
    )
    .unwrap();
    let fake = rustls::sign::CertifiedKey::new(vec![CertificateDer::from(a.cert.clone())], key);
    let mut attack = (*good).clone();
    attack.client_auth_cert_resolver = Arc::new(WrongSigner(Arc::new(fake)));
    let (_, server) = configs(&b, a.pin()).unwrap();
    assert_eq!(exchange(Arc::new(attack), server), (false, false));
}

#[test]
fn allowlist_accepts_registered_peers_and_rejects_unknown_certificates() {
    let server = Identity::generate().unwrap();
    let a = Identity::generate().unwrap();
    let b = Identity::generate().unwrap();
    let stranger = Identity::generate().unwrap();
    let allow = || rustls_server(&server, vec![a.pin(), b.pin()]);
    assert_eq!(
        exchange(configs(&a, server.pin()).unwrap().0, allow()),
        (true, true)
    );
    assert_eq!(
        exchange(configs(&b, server.pin()).unwrap().0, allow()),
        (true, true)
    );
    assert_eq!(
        exchange(configs(&stranger, server.pin()).unwrap().0, allow()),
        (false, false)
    );
    assert_eq!(
        exchange(
            configs(&a, server.pin()).unwrap().0,
            rustls_server(&server, vec![b.pin()])
        ),
        (false, false)
    );
}
fn rustls_server(identity: &Identity, pins: Vec<[u8; 32]>) -> Arc<rustls::ServerConfig> {
    psychotalk_core::tls::server_config(identity, pins).unwrap()
}

#[test]
fn group_alpn_negotiates_only_between_capable_peers_and_legacy_dm_still_works() {
    let a = Identity::generate().unwrap();
    let b = Identity::generate().unwrap();
    let (client, _) = configs(&a, b.pin()).unwrap();
    let (_, server) = configs(&b, a.pin()).unwrap();
    assert_eq!(exchange(client.clone(), server.clone()), (true, true));
    let mut legacy_server = (*server).clone();
    legacy_server.alpn_protocols.clear();
    assert_eq!(exchange(client, Arc::new(legacy_server)), (true, true));
    let mut legacy_client = (*configs(&a, b.pin()).unwrap().0).clone();
    legacy_client.alpn_protocols.clear();
    assert_eq!(exchange(Arc::new(legacy_client), server), (true, true));
}
