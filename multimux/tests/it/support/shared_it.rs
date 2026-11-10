//! Helpers shared by more than one integration test file under `tests/it/`.
//! Each one exists here and nowhere else under `tests/it/`.

use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::time::Duration;

pub const HANG_GUARD: Duration = Duration::from_secs(20);
pub const OBSERVE_CEILING: Duration = Duration::from_secs(30);

pub fn nz(n: usize) -> NonZeroUsize {
    NonZeroUsize::new(n).expect("non-zero")
}

pub fn fixture_path(name: &str) -> PathBuf {
    test_fixtures::path(name)
}

pub fn fixture_string(name: &str) -> String {
    fixture_path(name).to_string_lossy().into_owned()
}

pub async fn poll_until_extinf(client: &reqwest::Client, playlist_url: &str) -> String {
    let deadline = tokio::time::Instant::now() + HANG_GUARD;
    loop {
        if let Ok(resp) = client.get(playlist_url).send().await
            && resp.status().is_success()
            && let Ok(body) = resp.text().await
            && body.contains("#EXTINF:")
        {
            return body;
        }
        if tokio::time::Instant::now() >= deadline {
            panic!("no #EXTINF: line appeared in {playlist_url} within the hang guard");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

pub async fn poll_until_extinf_yield(client: &reqwest::Client, playlist_url: &str) -> String {
    let deadline = tokio::time::Instant::now() + HANG_GUARD;
    loop {
        if let Ok(resp) = client.get(playlist_url).send().await
            && resp.status().is_success()
            && let Ok(body) = resp.text().await
            && body.contains("#EXTINF:")
        {
            return body;
        }
        if tokio::time::Instant::now() >= deadline {
            panic!("no #EXTINF: line appeared in {playlist_url} within the hang guard");
        }
        tokio::task::yield_now().await;
    }
}

pub async fn observe_extinf(client: &reqwest::Client, playlist_url: &str) -> Option<String> {
    let deadline = tokio::time::Instant::now() + OBSERVE_CEILING;
    let mut tick = tokio::time::interval(Duration::from_millis(50));
    while tokio::time::Instant::now() < deadline {
        tick.tick().await;
        if let Ok(resp) = client.get(playlist_url).send().await
            && resp.status().is_success()
            && let Ok(body) = resp.text().await
            && body.contains("#EXTINF:")
        {
            return Some(body);
        }
    }
    None
}
