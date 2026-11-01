use tshttp_runtime::{Config, Event, TsHttpClient, TsHttpPolicy};

const URL: &str = "http://example/live.ts";
const CHUNK: usize = 1316;
const CHUNK_SPACING_MS: u64 = 10;
const NOISE_BYTE: u8 = 0xFF;
const NOISE_CHUNKS: usize = 8;
const NOISE_SPACING_MS: u64 = 1_000;

enum Step {
    Response(u16, Vec<(String, String)>),
    Chunk(usize),
    Eof,
    Error,
    Reconnect,
    Timeout,
}

struct Timed {
    at: u64,
    step: Step,
}

type Trace = Vec<String>;

fn describe(
    events: &mut Vec<Event>,
    actions: &mut Vec<tshttp_runtime::Action>,
    deadline: Option<u64>,
) -> String {
    let ev: Vec<String> = events
        .drain(..)
        .filter(|e| !matches!(e, Event::Demux(_)))
        .map(|e| format!("{e:?}"))
        .collect();
    let ac: Vec<String> = actions.drain(..).map(|a| format!("{a:?}")).collect();
    format!("events={ev:?} actions={ac:?} deadline={deadline:?}")
}

fn drain_client(c: &mut TsHttpClient) -> (Vec<Event>, Vec<tshttp_runtime::Action>, bool) {
    let mut events = Vec::new();
    let mut sampled = false;
    while let Some(e) = c.poll_event() {
        sampled |= matches!(e, Event::Demux(transmux::DemuxEvent::Sample { .. }));
        events.push(e);
    }
    let mut actions = Vec::new();
    while let Some(a) = c.poll() {
        actions.push(a);
    }
    (events, actions, sampled)
}

fn run_client(script: &[Timed], payload: &[u8]) -> (Trace, Vec<bool>) {
    let mut c = TsHttpClient::new(URL, Config::default(), 0);
    let mut trace = Vec::new();
    let mut sampled_per_step = Vec::new();
    let (mut ev, mut ac, _) = drain_client(&mut c);
    trace.push(describe(&mut ev, &mut ac, c.next_deadline()));
    sampled_per_step.push(false);
    let mut offset = 0usize;
    for t in script {
        match &t.step {
            Step::Response(status, headers) => c.on_response(*status, headers, t.at),
            Step::Chunk(len) => {
                let end = (offset + len).min(payload.len());
                let chunk = &payload[offset..end];
                offset = end;
                c.feed(chunk, t.at);
            }
            Step::Eof => c.on_eof(t.at),
            Step::Error => c.on_error(t.at),
            Step::Reconnect => c.reconnect(t.at),
            Step::Timeout => c.poll_timeout(t.at),
        }
        let (mut ev, mut ac, sampled) = drain_client(&mut c);
        trace.push(describe(&mut ev, &mut ac, c.next_deadline()));
        sampled_per_step.push(sampled);
    }
    (trace, sampled_per_step)
}

fn run_policy(script: &[Timed], sampled_per_step: &[bool]) -> Trace {
    let mut p = TsHttpPolicy::new(URL, Config::default(), 0);
    let mut trace = Vec::new();
    let drain = |p: &mut TsHttpPolicy| {
        let mut events = Vec::new();
        while let Some(e) = p.poll_event() {
            events.push(e);
        }
        let mut actions = Vec::new();
        while let Some(a) = p.poll() {
            actions.push(a);
        }
        (events, actions)
    };
    let (mut ev, mut ac) = drain(&mut p);
    trace.push(describe(&mut ev, &mut ac, p.next_deadline()));
    for (i, t) in script.iter().enumerate() {
        match &t.step {
            Step::Response(status, headers) => p.on_response(*status, headers, t.at),
            Step::Chunk(len) => p.on_bytes(*len, t.at, sampled_per_step[i + 1]),
            Step::Eof => p.on_eof(t.at),
            Step::Error => p.on_error(t.at),
            Step::Reconnect => p.reconnect(t.at),
            Step::Timeout => p.poll_timeout(t.at),
        }
        let (mut ev, mut ac) = drain(&mut p);
        trace.push(describe(&mut ev, &mut ac, p.next_deadline()));
    }
    trace
}

fn chunked(payload_len: usize, from_ms: u64) -> Vec<Timed> {
    let mut out = Vec::new();
    let mut sent = 0usize;
    let mut at = from_ms;
    while sent < payload_len {
        let len = CHUNK.min(payload_len - sent);
        out.push(Timed {
            at,
            step: Step::Chunk(len),
        });
        sent += len;
        at += CHUNK_SPACING_MS;
    }
    out
}

fn response(status: u16, headers: Vec<(String, String)>) -> Timed {
    Timed {
        at: 0,
        step: Step::Response(status, headers),
    }
}

fn content_length(len: usize) -> Vec<(String, String)> {
    vec![("Content-Length".to_string(), len.to_string())]
}

fn assert_same(name: &str, script: &[Timed], payload: &[u8]) -> Trace {
    let (client, sampled) = run_client(script, payload);
    let policy = run_policy(script, &sampled);
    assert_eq!(client.len(), policy.len());
    for (i, (c, p)) in client.iter().zip(&policy).enumerate() {
        assert_eq!(c, p, "{name}: step {i} diverged");
    }
    client
}

fn fixture() -> Vec<u8> {
    test_fixtures::read("ts/h264_aac.ts")
}

fn contains(trace: &Trace, needle: &str) -> bool {
    trace.iter().any(|s| s.contains(needle))
}

#[test]
fn live_then_silence_matches_client() {
    let payload = fixture();
    let mut script = vec![response(200, vec![])];
    script.extend(chunked(payload.len(), 1));
    let last = script.last().map_or(0, |t| t.at);
    script.push(Timed {
        at: last + 5_000,
        step: Step::Timeout,
    });
    script.push(Timed {
        at: last + 6_000,
        step: Step::Timeout,
    });
    script.push(Timed {
        at: last + 30_000,
        step: Step::Timeout,
    });
    let trace = assert_same("live", &script, &payload);
    assert!(contains(&trace, "Starved(NoData)"));
    assert!(contains(&trace, "StallNotice"));
    assert!(contains(&trace, "Abort"));
}

#[test]
fn finite_complete_matches_client() {
    let payload = fixture();
    let mut script = vec![response(200, content_length(payload.len()))];
    script.extend(chunked(payload.len(), 1));
    script.push(Timed {
        at: 60_000,
        step: Step::Eof,
    });
    let trace = assert_same("finite", &script, &payload);
    assert!(contains(&trace, "Finite"));
    assert!(contains(&trace, "Ended"));
    assert!(!contains(&trace, "Reconnecting"));
}

#[test]
fn finite_truncated_matches_client() {
    let payload = fixture();
    let mut script = vec![response(200, content_length(payload.len()))];
    let mut body = chunked(payload.len(), 1);
    body.truncate(body.len() / 2);
    script.extend(body);
    script.push(Timed {
        at: 5_000,
        step: Step::Eof,
    });
    let trace = assert_same("truncated", &script, &payload);
    assert!(contains(&trace, "Reconnecting"));
    assert!(!contains(&trace, "Ended"));
}

#[test]
fn start_timeout_matches_client() {
    let script = vec![
        response(200, vec![]),
        Timed {
            at: 15_000,
            step: Step::Timeout,
        },
    ];
    let trace = assert_same("start", &script, &[]);
    assert!(contains(&trace, "Abort"));
    assert!(contains(&trace, "Reconnecting"));
}

#[test]
fn bytes_without_samples_starve_matches_client() {
    let payload = vec![NOISE_BYTE; CHUNK * NOISE_CHUNKS];
    let mut script = vec![response(200, vec![])];
    for i in 0..NOISE_CHUNKS {
        script.push(Timed {
            at: 1 + i as u64 * NOISE_SPACING_MS,
            step: Step::Chunk(CHUNK),
        });
    }
    let trace = assert_same("starve", &script, &payload);
    assert!(contains(&trace, "Starved(Data)"));
}

#[test]
fn error_reconnect_and_not_found_match_client() {
    let payload = fixture();
    let mut script = vec![response(404, vec![])];
    script.push(Timed {
        at: 2_000,
        step: Step::Timeout,
    });
    script.push(response(200, vec![]));
    script.extend(chunked(payload.len(), 2_001));
    script.push(Timed {
        at: 9_000,
        step: Step::Error,
    });
    script.push(Timed {
        at: 9_001,
        step: Step::Reconnect,
    });
    let trace = assert_same("mixed", &script, &payload);
    assert!(contains(&trace, "NotFound"));
}
