use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rcgen::{
    BasicConstraints, CertificateParams, DnType, ExtendedKeyUsagePurpose, IsCa, Issuer, KeyPair,
    KeyUsagePurpose,
};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName};
use rustls::server::WebPkiClientVerifier;
use rustls::{ClientConfig, RootCertStore, ServerConfig, ServerConnection, StreamOwned};

use broadcast_client::ClientIdentity;

pub struct Ca {
    pub cert_der: CertificateDer<'static>,
    issuer: Issuer<'static, KeyPair>,
}

pub struct Leaf {
    pub chain_pem: String,
    pub key_pem: String,
    pub chain_der: Vec<Vec<u8>>,
    pub key_der: Vec<u8>,
}

fn provider() -> Arc<rustls::crypto::CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

impl Ca {
    pub fn new(name: &str) -> Self {
        let key = KeyPair::generate().unwrap();
        let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
        params.distinguished_name.push(DnType::CommonName, name);
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
        let cert = params.self_signed(&key).unwrap();
        Self {
            cert_der: cert.der().clone(),
            issuer: Issuer::new(params, key),
        }
    }

    pub fn issue(&self, subject: &str, client: bool) -> Leaf {
        let key = KeyPair::generate().unwrap();
        let mut params = CertificateParams::new(vec!["localhost".to_string()]).unwrap();
        params.distinguished_name.push(DnType::CommonName, subject);
        params.extended_key_usages = vec![if client {
            ExtendedKeyUsagePurpose::ClientAuth
        } else {
            ExtendedKeyUsagePurpose::ServerAuth
        }];
        let cert = params.signed_by(&key, &self.issuer).unwrap();
        Leaf {
            chain_pem: cert.pem(),
            key_pem: key.serialize_pem(),
            chain_der: vec![cert.der().to_vec()],
            key_der: key.serialize_der(),
        }
    }
}

impl Leaf {
    pub fn pem_identity(&self) -> ClientIdentity {
        ClientIdentity::pem(self.chain_pem.clone(), self.key_pem.clone())
    }

    pub fn der_identity(&self) -> ClientIdentity {
        ClientIdentity::der(self.chain_der.clone(), self.key_der.clone())
    }
}

#[derive(Default)]
pub struct ServerLog {
    pub client_certs: Vec<Option<Vec<u8>>>,
    pub handshake_failures: usize,
    pub requests: usize,
}

pub struct TlsServer {
    pub addr: SocketAddr,
    pub log: Arc<Mutex<ServerLog>>,
    stop: Arc<std::sync::atomic::AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl TlsServer {
    pub fn start(server_ca: &Ca, client_ca: &Ca, body: Vec<u8>) -> Self {
        let server_leaf = server_ca.issue("localhost", false);
        let mut roots = RootCertStore::empty();
        roots.add(client_ca.cert_der.clone()).unwrap();
        let verifier = WebPkiClientVerifier::builder_with_provider(Arc::new(roots), provider())
            .build()
            .unwrap();
        let config = Arc::new(
            ServerConfig::builder_with_provider(provider())
                .with_safe_default_protocol_versions()
                .unwrap()
                .with_client_cert_verifier(verifier)
                .with_single_cert(
                    server_leaf
                        .chain_der
                        .iter()
                        .map(|c| CertificateDer::from(c.clone()))
                        .collect(),
                    PrivateKeyDer::try_from(server_leaf.key_der.clone()).unwrap(),
                )
                .unwrap(),
        );
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let addr = listener.local_addr().unwrap();
        let log = Arc::new(Mutex::new(ServerLog::default()));
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (log2, stop2) = (log.clone(), stop.clone());
        let thread = std::thread::spawn(move || {
            while !stop2.load(std::sync::atomic::Ordering::Relaxed) {
                let Ok((tcp, _)) = listener.accept() else {
                    std::thread::sleep(Duration::from_millis(2));
                    continue;
                };
                tcp.set_nonblocking(false).unwrap();
                tcp.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                let conn = ServerConnection::new(config.clone()).unwrap();
                let mut tls = StreamOwned::new(conn, tcp);
                let mut head = Vec::new();
                let mut buf = [0u8; 512];
                let mut failed = false;
                while !head.windows(4).any(|w| w == b"\r\n\r\n") {
                    match tls.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => head.extend_from_slice(&buf[..n]),
                        Err(_) => {
                            failed = true;
                            break;
                        }
                    }
                }
                let mut l = log2.lock().unwrap();
                if failed || head.is_empty() {
                    l.handshake_failures += 1;
                    continue;
                }
                l.requests += 1;
                l.client_certs.push(
                    tls.conn
                        .peer_certificates()
                        .and_then(|c| c.first())
                        .map(|c| c.to_vec()),
                );
                drop(l);
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: video/MP2T\r\nConnection: close\r\n\r\n"
                );
                let _ = tls.write_all(response.as_bytes());
                let _ = tls.write_all(&body);
                let _ = tls.flush();
                tls.conn.send_close_notify();
                let _ = tls.flush();
            }
        });
        Self {
            addr,
            log,
            stop,
            thread: Some(thread),
        }
    }
}

impl Drop for TlsServer {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

pub fn identity_material(
    id: &ClientIdentity,
) -> (Vec<CertificateDer<'static>>, PrivateKeyDer<'static>) {
    if let Some((chain, key)) = id.pem_parts() {
        let chain = CertificateDer::pem_slice_iter(chain.as_bytes())
            .collect::<Result<_, _>>()
            .unwrap();
        (
            chain,
            PrivateKeyDer::from_pem_slice(key.as_bytes()).unwrap(),
        )
    } else {
        let (chain, key) = id.der_parts().expect("a PEM or DER identity");
        (
            chain
                .iter()
                .map(|c| CertificateDer::from(c.clone()))
                .collect(),
            PrivateKeyDer::try_from(key.to_vec()).unwrap(),
        )
    }
}

pub struct Response {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

pub fn https_get(
    addr: SocketAddr,
    path: &str,
    server_ca: &Ca,
    identity: Option<&ClientIdentity>,
) -> Result<Response, String> {
    let mut roots = RootCertStore::empty();
    roots.add(server_ca.cert_der.clone()).unwrap();
    let builder = ClientConfig::builder_with_provider(provider())
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_root_certificates(roots);
    let config = match identity {
        Some(id) => {
            let (chain, key) = identity_material(id);
            builder.with_client_auth_cert(chain, key).unwrap()
        }
        None => builder.with_no_client_auth(),
    };
    let tcp = TcpStream::connect(addr).map_err(|e| e.to_string())?;
    tcp.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let conn =
        rustls::ClientConnection::new(Arc::new(config), ServerName::try_from("localhost").unwrap())
            .map_err(|e| e.to_string())?;
    let mut tls = StreamOwned::new(conn, tcp);
    write!(
        tls,
        "GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
    )
    .map_err(|e| e.to_string())?;
    let mut raw = Vec::new();
    let mut buf = [0u8; 16 * 1024];
    loop {
        match tls.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => raw.extend_from_slice(&buf[..n]),
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof && !raw.is_empty() => break,
            Err(e) => return Err(e.to_string()),
        }
    }
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or("no response head")?;
    let head = String::from_utf8_lossy(&raw[..split]).to_string();
    let mut lines = head.lines();
    let status: u16 = lines
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse().ok())
        .ok_or("bad status line")?;
    let headers = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect();
    Ok(Response {
        status,
        headers,
        body: raw[split + 4..].to_vec(),
    })
}
