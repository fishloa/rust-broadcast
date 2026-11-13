//! The values multimux shares between tasks of a multi-thread runtime stay `Send + Sync`: a
//! `Cell`/`Rc` added to one of them fails to compile here instead of at a distant `spawn`.

use multimux::config::Config;
use multimux::origin::AppState;
use multimux::output::catchup::CatchupOutput;
use multimux::output::dash::DashOutput;
use multimux::output::hls::HlsOutput;
use multimux::output::ll_dash::LlDashOutput;
use multimux::output::smooth::SmoothOutput;
use multimux::registry::SchemeRegistry;
use multimux::route::RouteHandle;
use multimux::source::IngestTimeouts;
use multimux::source::dash_pull::DashPullRoute;
use multimux::source::hls_pull::HlsPullRoute;
use multimux::source::rtsp::RtspRoute;
use multimux::source::smooth_pull::SmoothPullRoute;
use multimux::source::ts_http::TsHttpRoute;
use multimux::{MultimuxError, Output};

fn assert_send_sync<T: Send + Sync>() {}

#[test]
fn the_config_and_registries_are_shared_across_tasks() {
    assert_send_sync::<Config>();
    assert_send_sync::<SchemeRegistry>();
    assert_send_sync::<AppState>();
    assert_send_sync::<RouteHandle>();
    assert_send_sync::<MultimuxError>();
    assert_send_sync::<IngestTimeouts>();
}

#[test]
fn every_output_is_shared_behind_an_arc() {
    assert_send_sync::<HlsOutput>();
    assert_send_sync::<DashOutput>();
    assert_send_sync::<LlDashOutput>();
    assert_send_sync::<SmoothOutput>();
    assert_send_sync::<CatchupOutput>();
    assert_send_sync::<std::sync::Arc<dyn Output>>();
}

#[test]
fn every_pull_route_moves_into_its_own_task() {
    assert_send_sync::<HlsPullRoute>();
    assert_send_sync::<DashPullRoute>();
    assert_send_sync::<SmoothPullRoute>();
    assert_send_sync::<RtspRoute>();
    assert_send_sync::<TsHttpRoute>();
}
