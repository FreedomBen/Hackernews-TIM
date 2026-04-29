//! Fake HN backend backed by `wiremock`. Linux-only.
//!
//! Wraps a [`wiremock::MockServer`] in a private tokio runtime so that
//! sync test bodies can configure mocks (and inspect recorded
//! requests) without becoming async themselves. The binary under test
//! reaches the server via real ureq HTTP calls — Phase 3 deliberately
//! exercises the live HTTP layer rather than the `HnApi` /
//! `FakeHnApi` trait double from Phase 2.
//!
//! See docs/planning/TEST_PLAN.md §3.1.2.

#![cfg(target_os = "linux")]
#![allow(dead_code)]

use serde_json::Value;
use tokio::runtime::Runtime;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Owns a `wiremock::MockServer` plus the small tokio runtime that
/// drives it. Drop the handle to stop the listener.
pub struct FakeHnServer {
    runtime: Runtime,
    server: MockServer,
}

impl FakeHnServer {
    /// Start a new fake HN backend bound to a random localhost port.
    pub fn start() -> Self {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .expect("failed to build tokio runtime for FakeHnServer");
        let server = runtime.block_on(MockServer::start());
        Self { runtime, server }
    }

    /// Server root URL, e.g. `http://127.0.0.1:54321`. Strips any
    /// trailing slash so callers can interpolate relative paths
    /// directly.
    pub fn base_url(&self) -> String {
        let mut url = self.server.uri();
        if url.ends_with('/') {
            url.pop();
        }
        url
    }

    /// Algolia API base — pass directly as `HN_ALGOLIA_BASE`. Mirrors
    /// the production prefix `https://hn.algolia.com/api/v1`.
    pub fn algolia_base(&self) -> String {
        format!("{}/api/v1", self.base_url())
    }

    /// Firebase API base — pass directly as `HN_FIREBASE_BASE`.
    /// Mirrors the production prefix
    /// `https://hacker-news.firebaseio.com/v0`.
    pub fn firebase_base(&self) -> String {
        format!("{}/v0", self.base_url())
    }

    /// HN news.ycombinator.com host base — pass directly as
    /// `HN_NEWS_BASE`. Mirrors the production host
    /// `https://news.ycombinator.com` (no path suffix). Used by the
    /// binary for `/login`, `/vote`, `/vouch`, `/comment`, `/edit`,
    /// `/xedit`, `/item`, `/threads`, `/user`, and `/news` requests.
    pub fn news_base(&self) -> String {
        self.base_url()
    }

    /// Mount a GET handler at `route` returning `body` as JSON with
    /// the given HTTP status. `route` is server-relative
    /// (e.g. `"/api/v1/items/12345"`).
    pub fn mount_get_json<P: Into<String>>(&self, route: P, status: u16, body: Value) {
        let route = route.into();
        self.runtime.block_on(
            Mock::given(method("GET"))
                .and(path(route))
                .respond_with(ResponseTemplate::new(status).set_body_json(body))
                .mount(&self.server),
        );
    }

    /// Mount a GET handler at `route` returning `body` as a raw
    /// string. Useful for HTML fixture responses (HN scraping).
    pub fn mount_get_text<P: Into<String>, B: Into<String>>(&self, route: P, status: u16, body: B) {
        let route = route.into();
        let body = body.into();
        self.runtime.block_on(
            Mock::given(method("GET"))
                .and(path(route))
                .respond_with(
                    ResponseTemplate::new(status)
                        .set_body_string(body)
                        .insert_header("content-type", "text/html; charset=utf-8"),
                )
                .mount(&self.server),
        );
    }

    /// Mount a POST handler at `route` returning `body` as HTML with
    /// the given HTTP status. `route` is server-relative
    /// (e.g. `"/login"`, `"/comment"`). Useful for HN form-submit
    /// endpoints (`/vote`, `/vouch`, `/comment`, `/xedit`).
    pub fn mount_post_text<P: Into<String>, B: Into<String>>(
        &self,
        route: P,
        status: u16,
        body: B,
    ) {
        let route = route.into();
        let body = body.into();
        self.runtime.block_on(
            Mock::given(method("POST"))
                .and(path(route))
                .respond_with(
                    ResponseTemplate::new(status)
                        .set_body_string(body)
                        .insert_header("content-type", "text/html; charset=utf-8"),
                )
                .mount(&self.server),
        );
    }

    /// Mount a POST handler that additionally issues a `user` session
    /// cookie on the response (`Path=/`), mirroring HN's `/login`
    /// `Set-Cookie` header. The binary's cookie jar then carries
    /// `cookie_value` on every subsequent request to the fake host —
    /// which is what `current_session_cookie()` looks up after a
    /// successful login.
    pub fn mount_post_with_user_cookie<P, B, C>(
        &self,
        route: P,
        status: u16,
        body: B,
        cookie_value: C,
    ) where
        P: Into<String>,
        B: Into<String>,
        C: Into<String>,
    {
        let route = route.into();
        let body = body.into();
        let cookie = format!("user={}; Path=/", cookie_value.into());
        self.runtime.block_on(
            Mock::given(method("POST"))
                .and(path(route))
                .respond_with(
                    ResponseTemplate::new(status)
                        .set_body_string(body)
                        .insert_header("content-type", "text/html; charset=utf-8")
                        .insert_header("set-cookie", cookie.as_str()),
                )
                .mount(&self.server),
        );
    }

    /// All requests received by the server, in chronological order.
    /// Used for assertions like "the binary POSTed the expected
    /// payload to `/login`".
    pub fn received_requests(&self) -> Vec<wiremock::Request> {
        self.runtime
            .block_on(self.server.received_requests())
            .unwrap_or_default()
    }
}
