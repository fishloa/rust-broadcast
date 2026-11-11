use std::collections::HashMap;

use smooth_runtime::{Action, FetchFailure, Output, RequestId, SmoothClient, SmoothConfig};
use transmux::{Sample, TrackId};
use url::Url;

pub const ORIGIN: &str = "http://origin.test/";
pub const STEP_LIMIT: usize = 2_000;

#[derive(Clone)]
pub enum Reply {
    Body(Vec<u8>),
    Status(u16),
    Transport,
}

pub struct Host {
    pub client: SmoothClient,
    pub now: u64,
    pub outputs: Vec<Output>,
    pub requested: Vec<String>,
    pub request_times: Vec<u64>,
    routes: HashMap<String, Vec<Reply>>,
    hits: HashMap<String, usize>,
}

pub fn manifest_url() -> Url {
    Url::parse(&format!("{ORIGIN}Manifest")).expect("url")
}

pub fn fixture(rel: &str) -> Vec<u8> {
    test_fixtures::read(&format!("smooth/{rel}"))
}

impl Host {
    pub fn new(config: SmoothConfig) -> Self {
        Self {
            client: SmoothClient::new(manifest_url(), config),
            now: 0,
            outputs: Vec::new(),
            requested: Vec::new(),
            request_times: Vec::new(),
            routes: HashMap::new(),
            hits: HashMap::new(),
        }
    }

    pub fn route(&mut self, path: &str, replies: Vec<Reply>) {
        self.routes.insert(format!("{ORIGIN}{path}"), replies);
    }

    pub fn serve_dir(&mut self, fixture_dir: &str) {
        fn walk(host: &mut Host, root: &std::path::Path, dir: &std::path::Path) {
            for entry in std::fs::read_dir(dir).expect("read dir") {
                let path = entry.expect("entry").path();
                if path.is_dir() {
                    walk(host, root, &path);
                } else if path.file_name().is_some_and(|n| n != "PROVENANCE.md") {
                    let rel = path
                        .strip_prefix(root)
                        .expect("prefix")
                        .to_str()
                        .expect("utf8");
                    host.route(rel, vec![Reply::Body(std::fs::read(&path).expect("read"))]);
                }
            }
        }
        let root = test_fixtures::path(&format!("smooth/{fixture_dir}"));
        walk(self, &root, &root);
    }

    fn reply(&mut self, url: &str) -> Reply {
        let n = self.hits.entry(url.to_string()).or_insert(0);
        let i = *n;
        *n += 1;
        match self.routes.get(url) {
            Some(r) if !r.is_empty() => r[i.min(r.len() - 1)].clone(),
            _ => Reply::Status(404),
        }
    }

    fn serve(&mut self, action: Action) {
        let (id, url, is_manifest) = match &action {
            Action::FetchManifest { id, url } => (*id, url.to_string(), true),
            Action::FetchFragment { id, url, .. } => (*id, url.to_string(), false),
            _ => panic!("unexpected action"),
        };
        let decoded = url.replace("%28", "(").replace("%29", ")");
        self.requested.push(decoded.clone());
        self.request_times.push(self.now);
        match self.reply(&decoded) {
            Reply::Body(b) if is_manifest => self.client.on_manifest(self.now, id, &b),
            Reply::Body(b) => self.client.on_fragment(self.now, id, &b),
            Reply::Status(s) => self.fail(id, Some(s)),
            Reply::Transport => self.fail(id, None),
        }
    }

    fn fail(&mut self, id: RequestId, status: Option<u16>) {
        self.client
            .on_failure(self.now, id, FetchFailure { status });
    }

    fn drain(&mut self) {
        while let Some(o) = self.client.next_output() {
            self.outputs.push(o);
        }
    }

    pub fn run(&mut self) {
        self.run_until(|_| false);
    }

    pub fn run_until(&mut self, stop: impl Fn(&[Output]) -> bool) {
        for _ in 0..STEP_LIMIT {
            while let Some(action) = self.client.poll(self.now) {
                self.serve(action);
            }
            self.drain();
            if stop(&self.outputs) {
                return;
            }
            match self.client.next_deadline() {
                Some(t) if t > self.now => self.now = t,
                Some(_) => {}
                None => return,
            }
        }
        panic!("step limit reached");
    }

    pub fn run_for(&mut self, micros: u64) {
        let end = self.now.saturating_add(micros);
        for _ in 0..STEP_LIMIT {
            while let Some(action) = self.client.poll(self.now) {
                self.serve(action);
            }
            self.drain();
            match self.client.next_deadline() {
                Some(t) if t > end => return,
                Some(t) if t > self.now => self.now = t,
                Some(_) => {}
                None => return,
            }
        }
        panic!("step limit reached");
    }

    pub fn times_of(&self, suffix: &str) -> Vec<u64> {
        self.requested
            .iter()
            .zip(&self.request_times)
            .filter(|(u, _)| u.ends_with(suffix))
            .map(|(_, t)| *t)
            .collect()
    }

    pub fn count_requests(&self, suffix: &str) -> usize {
        self.requested
            .iter()
            .filter(|u| u.ends_with(suffix))
            .count()
    }

    pub fn samples(&self) -> Vec<(TrackId, Sample)> {
        self.outputs
            .iter()
            .filter_map(|o| match o {
                Output::Sample { track, sample } => Some((*track, sample.clone())),
                _ => None,
            })
            .collect()
    }

    pub fn has(&self, f: impl Fn(&Output) -> bool) -> bool {
        self.outputs.iter().any(f)
    }
}

pub fn is_ended(o: &Output) -> bool {
    matches!(o, Output::Ended)
}

pub fn is_fatal(o: &Output) -> bool {
    matches!(o, Output::Fatal(_))
}
