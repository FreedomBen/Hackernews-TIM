//! TEST_PLAN.md §3.3 — regression test that asserts no e2e request
//! reaches a real production HN host (Linux-only).
//!
//! Phase 3 is built on the structural guarantee that the spawned
//! binary reads `HN_ALGOLIA_BASE`, `HN_FIREBASE_BASE`, and
//! `HN_NEWS_BASE` from the environment when constructing its
//! `HNClient`, and the harness in `tests/e2e/mod.rs` always sets all
//! three vars (defaulting to an unrouteable blackhole when no fake
//! backend is supplied).
//!
//! This test pins that contract: a representative front-page run is
//! driven against a single [`FakeHnServer`], and every request that
//! reached the wiremock listener is checked to ensure its URL host is
//! the local server — never `hn.algolia.com`, `hacker-news.firebaseio.com`,
//! or `news.ycombinator.com`. A regression that hardcodes a production
//! URL would either fail to connect at all (the local server never
//! sees the request, so the front page never renders and the test
//! times out) or arrive with a non-localhost Host header (caught by
//! the explicit assertion below).
//!
//! See also `tests/e2e/mod.rs` for harness conventions and
//! `tests/e2e/fakehn.rs` for the env-var override surface.

#![cfg(target_os = "linux")]

#[path = "e2e/mod.rs"]
mod helpers;

use std::time::Duration;

use serde_json::json;

use helpers::fakehn::FakeHnServer;
use helpers::{spawn_app, SpawnOptions, DEFAULT_WAIT};

const FRONT_PAGE_RENDER_TIMEOUT: Duration = Duration::from_secs(60);

const STORY_ID: u32 = 60001;
const STORY_TITLE: &str = "no-real-network fixture story";

/// Production hostnames that must never appear in any recorded
/// request URL (or `Host` header) while the harness is in use.
const PROD_HOSTS: &[&str] = &[
    "hn.algolia.com",
    "hacker-news.firebaseio.com",
    "news.ycombinator.com",
];

fn mount_front_page(server: &FakeHnServer) {
    server.mount_get_json("/v0/topstories.json", 200, json!([STORY_ID]));
    server.mount_get_json(
        "/api/v1/search",
        200,
        json!({
            "hits": [{
                "objectID": STORY_ID.to_string(),
                "author": "alice",
                "url": format!("https://example.com/{STORY_ID}"),
                "story_text": null,
                "points": 100,
                "num_comments": 5,
                "created_at_i": 1_700_000_000_u64,
                "_highlightResult": {
                    "title": { "value": STORY_TITLE }
                },
                "dead": false,
                "flagged": false
            }]
        }),
    );
    // The unauthenticated front-page render also fires
    // `get_listing_vote_state`, which hits the news_base `/news`
    // path. Mount an empty page so the request resolves cleanly
    // (and is recorded for the host-header assertion below).
    server.mount_get_text("/news", 200, "<html><body></body></html>");
}

#[test]
fn representative_run_never_targets_a_production_hn_host() {
    let server = FakeHnServer::start();
    mount_front_page(&server);

    let opts = SpawnOptions::new()
        .algolia_base(server.algolia_base())
        .firebase_base(server.firebase_base())
        .news_base(server.news_base());
    let mut handle = spawn_app(opts).expect("spawn_app should succeed");

    handle
        .wait_for_text("[l]ight", DEFAULT_WAIT)
        .expect("flavor prompt should print");
    handle.send_keys("l\n").expect("send light flavor");

    handle
        .wait_for_text("Wrote config to", Duration::from_secs(10))
        .expect("binary should announce the freshly-written config");

    handle
        .wait_for_text("No auth file found", DEFAULT_WAIT)
        .expect("auth prompt should print after config write");
    handle.send_keys("\n").expect("skip auth (default = N)");

    handle
        .wait_for_text(STORY_TITLE, FRONT_PAGE_RENDER_TIMEOUT)
        .expect("fixture story should render — if missing, the binary may have leaked the request to a production host");

    let status = handle.shutdown().expect("binary should exit cleanly");
    assert!(status.success(), "expected success exit, got {status:?}");

    let requests = server.received_requests();
    assert!(
        !requests.is_empty(),
        "binary should have made at least one HTTP request to the fake backend; \
         an empty request log usually means the binary couldn't reach the fake \
         server at all (env-var override broken?)"
    );

    for req in &requests {
        let url_host = req.url.host_str().unwrap_or("");
        for prod in PROD_HOSTS {
            assert!(
                !url_host.contains(prod),
                "request URL host should never match a production HN host \
                 (got {url_host:?}, full URL {})",
                req.url
            );
        }
        assert!(
            url_host == "127.0.0.1" || url_host == "localhost",
            "expected request to target localhost, got {url_host:?} (URL {})",
            req.url
        );

        if let Some(host_header) = req.headers.get("host").and_then(|v| v.to_str().ok()) {
            for prod in PROD_HOSTS {
                assert!(
                    !host_header.contains(prod),
                    "Host header should never match a production HN host \
                     (got {host_header:?}, request URL {})",
                    req.url
                );
            }
        }
    }
}
