//! Route-level `audio_renditions` config and its effect on `/master.m3u8`
//! (issue #1428 slice 1).

use std::collections::HashMap;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use hls_runtime::server::AudioRenditionDecl;
use tower::ServiceExt;

use multimux::config::Config;
use multimux::origin::{AppState, router};
use multimux::output::llhls::LlHlsOutput;
use multimux::route::RouteHandle;

fn decl(name: &str, lang: &str, default: bool, uri: &str) -> AudioRenditionDecl {
    AudioRenditionDecl {
        group_id: "aac".into(),
        name: name.into(),
        language: lang.into(),
        default,
        autoselect: true,
        channels: Some("2".into()),
        uri: uri.into(),
        characteristics: Vec::new(),
    }
}

fn decls() -> Vec<AudioRenditionDecl> {
    vec![
        decl("English", "en", true, "audio/en.m3u8"),
        decl("Deutsch", "de", false, "audio/de.m3u8"),
    ]
}

async fn get_master(route: Arc<RouteHandle>) -> (StatusCode, String) {
    let mut streams = HashMap::new();
    streams.insert(
        "cam".to_string(),
        (
            route,
            vec![Arc::new(LlHlsOutput::default()) as Arc<dyn multimux::output::Output>],
        ),
    );
    let resp = router(Arc::new(AppState::new(streams)))
        .oneshot(
            Request::builder()
                .uri("/cam/master.m3u8")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status();
    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    (status, String::from_utf8(body.to_vec()).unwrap())
}

fn route(audio: Option<Vec<AudioRenditionDecl>>) -> Arc<RouteHandle> {
    let mut r = RouteHandle::new(4.0, 500, 4).with_declared_bandwidth_bps(7_000_000u64);
    if let Some(a) = audio {
        r = r.with_audio_renditions(a);
    }
    let r = Arc::new(r);
    r.publish_new_program(media_plane::ProgramId(0));
    r
}

#[test]
fn config_audio_renditions_json_round_trip() {
    let json = r#"{
        "routes": [{
            "name": "cam",
            "input": { "type": "rtsp", "url": "rtsp://host/s1" },
            "outputs": [ "llhls" ],
            "audio_renditions": [
                { "group_id": "aac", "name": "English", "language": "en", "default": true,
                  "autoselect": true, "channels": "2", "uri": "audio/en.m3u8", "characteristics": [] },
                { "group_id": "aac", "name": "Deutsch", "language": "de", "default": false,
                  "autoselect": true, "channels": "2", "uri": "audio/de.m3u8", "characteristics": [] }
            ]
        }]
    }"#;
    let cfg: Config = serde_json::from_str(json).expect("config parses");
    cfg.validate().expect("config validates");
    assert_eq!(cfg.routes[0].audio_renditions, decls());

    let out = serde_json::to_string(&cfg.routes[0].audio_renditions).expect("serialise");
    let back: Vec<AudioRenditionDecl> = serde_json::from_str(&out).expect("re-parse");
    assert_eq!(back, decls());
}

#[test]
fn config_without_audio_renditions_defaults_to_empty() {
    let json = r#"{ "routes": [{ "name": "cam",
        "input": { "type": "rtsp", "url": "rtsp://host/s1" }, "outputs": [ "llhls" ] }] }"#;
    let cfg: Config = serde_json::from_str(json).expect("config parses");
    assert!(cfg.routes[0].audio_renditions.is_empty());
}

#[tokio::test]
async fn master_lists_audio_renditions_when_configured() {
    let (status, text) = get_master(route(Some(decls()))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        text,
        "#EXTM3U\n\
         #EXT-X-MEDIA:TYPE=AUDIO,URI=\"audio/en.m3u8\",GROUP-ID=\"aac\",LANGUAGE=\"en\",NAME=\"English\",DEFAULT=YES,AUTOSELECT=YES,CHANNELS=\"2\"\n\
         #EXT-X-MEDIA:TYPE=AUDIO,URI=\"audio/de.m3u8\",GROUP-ID=\"aac\",LANGUAGE=\"de\",NAME=\"Deutsch\",DEFAULT=NO,AUTOSELECT=YES,CHANNELS=\"2\"\n\
         #EXT-X-STREAM-INF:BANDWIDTH=7000000,AUDIO=\"aac\"\n\
         media.m3u8\n"
    );
}

#[tokio::test]
async fn master_is_unchanged_without_audio_renditions() {
    let (status, text) = get_master(route(None)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        text,
        "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=7000000\nmedia.m3u8\n"
    );
}
