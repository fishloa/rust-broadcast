//! Client identity (mutual TLS) for sessions whose host performs TLS.
//!
//! The session never performs TLS. It carries an identity on every action the host executes over
//! TLS ([`crate::Action::HttpRequest`] with an `https` URL, [`crate::Action::TcpConnect`] with
//! `tls: true`), and the host presents it when the server asks for a client certificate.

use alloc::string::String;
use alloc::vec::Vec;

use rustls_pki_types::pem::PemObject;
use rustls_pki_types::{CertificateDer, PrivateKeyDer};
use zeroize::Zeroize;

const DER_SEQUENCE_TAG: u8 = 0x30;

#[derive(Clone, PartialEq, Eq)]
enum Material {
    Handle(u64),
    Pem {
        cert_chain: String,
        private_key: String,
    },
    Der {
        cert_chain: Vec<Vec<u8>>,
        private_key: Vec<u8>,
    },
}

/// A TLS client certificate and its private key, or an opaque handle to one the host owns.
///
/// A [`handle`](Self::handle) keeps key material out of the session entirely (the host resolves
/// the id against its keychain or secure enclave); [`pem`](Self::pem) and [`der`](Self::der) carry
/// the material, which is wiped on drop and never shown by `Debug`.
#[derive(Clone, PartialEq, Eq)]
pub struct ClientIdentity {
    material: Material,
}

impl ClientIdentity {
    /// An identity the host holds under `id`; the session only passes the id back.
    #[must_use]
    pub fn handle(id: u64) -> Self {
        Self {
            material: Material::Handle(id),
        }
    }

    /// A PEM certificate chain (leaf first) and a PEM private key (PKCS#8, PKCS#1 or SEC1).
    #[must_use]
    pub fn pem(cert_chain: impl Into<String>, private_key: impl Into<String>) -> Self {
        Self {
            material: Material::Pem {
                cert_chain: cert_chain.into(),
                private_key: private_key.into(),
            },
        }
    }

    /// A DER certificate chain (leaf first) and a DER private key (PKCS#8, PKCS#1 or SEC1).
    #[must_use]
    pub fn der(cert_chain: Vec<Vec<u8>>, private_key: Vec<u8>) -> Self {
        Self {
            material: Material::Der {
                cert_chain,
                private_key,
            },
        }
    }

    /// The host handle id, for an identity made by [`handle`](Self::handle).
    #[must_use]
    pub fn handle_id(&self) -> Option<u64> {
        match &self.material {
            Material::Handle(id) => Some(*id),
            _ => None,
        }
    }

    /// The PEM certificate chain and private key, for an identity made by [`pem`](Self::pem).
    #[must_use]
    pub fn pem_parts(&self) -> Option<(&str, &str)> {
        match &self.material {
            Material::Pem {
                cert_chain,
                private_key,
            } => Some((cert_chain, private_key)),
            _ => None,
        }
    }

    /// The DER certificate chain and private key, for an identity made by [`der`](Self::der).
    #[must_use]
    pub fn der_parts(&self) -> Option<(&[Vec<u8>], &[u8])> {
        match &self.material {
            Material::Der {
                cert_chain,
                private_key,
            } => Some((cert_chain, private_key)),
            _ => None,
        }
    }

    pub(crate) fn validate(&self) -> Result<(), crate::Error> {
        let invalid = || {
            Err(crate::Error::InvalidConfig {
                field: "client_identity",
            })
        };
        match &self.material {
            Material::Handle(_) => Ok(()),
            Material::Pem {
                cert_chain,
                private_key,
            } => {
                let certs: Result<Vec<_>, _> =
                    CertificateDer::pem_slice_iter(cert_chain.as_bytes()).collect();
                let Ok(certs) = certs else { return invalid() };
                if certs.is_empty()
                    || !certs.iter().all(|c| is_der_sequence(c.as_ref()))
                    || PrivateKeyDer::from_pem_slice(private_key.as_bytes()).is_err()
                {
                    return invalid();
                }
                Ok(())
            }
            Material::Der {
                cert_chain,
                private_key,
            } => {
                if cert_chain.is_empty()
                    || !cert_chain.iter().all(|c| is_der_sequence(c))
                    || PrivateKeyDer::try_from(private_key.as_slice()).is_err()
                {
                    return invalid();
                }
                Ok(())
            }
        }
    }
}

fn is_der_sequence(der: &[u8]) -> bool {
    der.first() == Some(&DER_SEQUENCE_TAG) && der.len() > 2
}

impl Drop for ClientIdentity {
    fn drop(&mut self) {
        match &mut self.material {
            Material::Handle(_) => {}
            Material::Pem { private_key, .. } => private_key.zeroize(),
            Material::Der { private_key, .. } => private_key.zeroize(),
        }
    }
}

impl core::fmt::Debug for ClientIdentity {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match &self.material {
            Material::Handle(id) => f.debug_tuple("ClientIdentity::Handle").field(id).finish(),
            Material::Pem { cert_chain, .. } => f
                .debug_struct("ClientIdentity::Pem")
                .field("cert_chain_bytes", &cert_chain.len())
                .field("private_key", &format_args!("<redacted>"))
                .finish(),
            Material::Der { cert_chain, .. } => f
                .debug_struct("ClientIdentity::Der")
                .field("certs", &cert_chain.len())
                .field("private_key", &format_args!("<redacted>"))
                .finish(),
        }
    }
}

/// Which TLS connections of a session offer the [`ClientIdentity`].
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ClientIdentityScope {
    /// Only connections to the session URL's own origin (scheme, host, port). A redirect to
    /// another origin, or a manifest that names a CDN host, does not receive the identity. The
    /// default: a server only learns the client's certificate when it asks for it, but an
    /// untrusted redirect target can ask.
    #[default]
    SessionOrigin,
    /// Every `https` request and every `tls: true` connection of the session, whatever the host.
    /// For deployments where the CDN or key server behind the manifest also requires the
    /// certificate.
    AnyHttps,
}

impl ClientIdentityScope {
    /// Short label.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::SessionOrigin => "session-origin",
            Self::AnyHttps => "any-https",
        }
    }
}

broadcast_common::impl_spec_display!(ClientIdentityScope);
