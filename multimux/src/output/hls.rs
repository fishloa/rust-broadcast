//! Shared helpers for the HLS [`crate::output::Output`] implementations
//! (`llhls` and `ts_hls`). Each behaviour lives once here; the two output
//! modules are thin adapters over these helpers, parameterised by what
//! actually differs (segment container and the LL-HLS-only
//! parts/preload/iframe machinery).

use std::sync::Arc;

use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};

use crate::http;
use crate::origin::resource::BlockingRequestGuard;
use crate::output::llhls::BlockingReloadQuery;
use crate::route::RouteHandle;
use hls_runtime::server::{DEFAULT_TRACK_ID, HlsBody, HlsRequest};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum HlsFlavour {
    LowLatency,
    Segmented,
}

pub(crate) async fn media_playlist(
    route: Arc<RouteHandle>,
    flavour: HlsFlavour,
    q: BlockingReloadQuery,
) -> Response {
    let serving = if flavour == HlsFlavour::LowLatency {
        match http::resolve_route_program_held(&route).await {
            Ok(s) => s,
            Err(resp) => return *resp,
        }
    } else {
        match http::resolve_route_program(&route) {
            Ok(s) => s,
            Err(resp) => return *resp,
        }
    };
    let trunk = serving.trunk();
    let origin = serving.ll_hls();
    let request = HlsRequest::Playlist {
        track_id: DEFAULT_TRACK_ID,
        query: q.into(),
    };
    let priority = origin.http_priority(&request);
    let (resp, timed_out) = http::resolve_blocking_timed(
        &trunk,
        origin.as_ref(),
        request,
        http::playlist_reload_timeout(&origin),
        BlockingRequestGuard::new,
    )
    .await;
    http::into_playlist_response(resp, timed_out, |body| match body {
        HlsBody::Playlist(m) => {
            let mut resp =
                ([(header::CONTENT_TYPE, http::PLAYLIST_CONTENT_TYPE)], m).into_response();
            if flavour == HlsFlavour::LowLatency
                && let Ok(value) = header::HeaderValue::from_str(&format!("u={}", priority.urgency))
            {
                resp.headers_mut().insert("priority", value);
            }
            resp
        }
        HlsBody::Resource(_) => StatusCode::NOT_FOUND.into_response(),
        _ => StatusCode::NOT_FOUND.into_response(),
    })
}
