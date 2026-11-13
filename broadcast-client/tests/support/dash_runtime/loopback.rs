//! A sans-IO loopback host for `DashClient`: serves the files of
//! `fixtures/dash/**` by URL and byte range, with fault injection, and drives
//! the client to a terminal state the way an adapter would (zenith-style
//! loopback servers of `broadcast-client/tests/support`).

use std::collections::BTreeMap;

use broadcast_common::Unpackage;
use dash_runtime::client::Failure;

use transmux::{Fmp4Demux, Sample};
use url::Url;

pub const HOST: &str = "dash.test";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Req {
    pub url: String,
    pub range: Option<(u64, u64)>,
}

#[derive(Debug, Clone)]
pub struct Resp {
    pub status: u16,
    pub body: Vec<u8>,
}

#[derive(Default)]
pub struct Origin {
    docs: BTreeMap<String, Vec<u8>>,
    mounts: BTreeMap<String, String>,
}

impl Origin {
    pub fn new() -> Self {
        let mut o = Self::default();
        o.mounts.insert(HOST.to_owned(), String::new());
        o
    }

    pub fn mount(mut self, host: &str, dir: &str) -> Self {
        self.mounts.insert(host.to_owned(), dir.to_owned());
        self
    }

    pub fn doc(mut self, url: &str, bytes: Vec<u8>) -> Self {
        self.docs.insert(url.to_owned(), bytes);
        self
    }

    fn bytes(&self, url: &str) -> Option<Vec<u8>> {
        if let Some(b) = self.docs.get(url) {
            return Some(b.clone());
        }
        let u = Url::parse(url).ok()?;
        let dir = self.mounts.get(u.host_str()?)?;
        let rel = u.path().trim_start_matches('/');
        let rel = if dir.is_empty() {
            format!("dash/{rel}")
        } else {
            format!("dash/{dir}/{rel}")
        };
        let path = test_fixtures::path(&rel);
        std::fs::read(path).ok()
    }

    pub fn serve(&mut self, req: &Req) -> Result<Resp, Failure> {
        let Some(all) = self.bytes(&req.url) else {
            return Ok(Resp {
                status: 404,
                body: b"not found".to_vec(),
            });
        };
        let (status, body) = match req.range {
            Some((off, len)) => {
                let off = off as usize;
                if off >= all.len() {
                    return Ok(Resp {
                        status: 416,
                        body: Vec::new(),
                    });
                }
                let end = (off + len as usize).min(all.len());
                (206, all[off..end].to_vec())
            }
            None => (200, all),
        };
        Ok(Resp { status, body })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sig {
    pub data: Vec<u8>,
    pub pts: Option<i64>,
    pub dts: Option<i64>,
    pub duration: Option<u32>,
    pub flags: String,
}

pub fn sig(s: &Sample) -> Sig {
    Sig {
        data: s.data.to_vec(),
        pts: s.pts,
        dts: s.dts,
        duration: s.duration,
        flags: format!("{:?}", s.flags),
    }
}

pub fn sigs(samples: &[Sample]) -> Vec<Sig> {
    samples.iter().map(sig).collect()
}

/// The samples `Fmp4Demux` produces from the concatenation of `files` (an init
/// first), per track in the init's order. This is the oracle: it never sees
/// the client.
pub fn oracle(files: &[&str]) -> Vec<Vec<Sig>> {
    let mut bytes = Vec::new();
    for f in files {
        bytes.extend_from_slice(&test_fixtures::read(&format!("dash/{f}")));
    }
    let media = Fmp4Demux::new()
        .unpackage(bytes.as_slice())
        .expect("oracle demux");
    media.tracks.iter().map(|t| sigs(&t.samples)).collect()
}
