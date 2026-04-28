//! In-memory fake implementation of [`HnApi`] for tests.
//!
//! Tests register fixture responses ahead of the call (one-shot queues for
//! the page-data fetches whose return type owns a channel, simple maps for
//! everything else) and read back the recorded [`FakeCall`] log to assert
//! that the view drove the expected request.
//!
//! Methods that aren't stubbed return sensible empty values rather than
//! erroring, so a test that only cares about a single endpoint doesn't
//! have to scaffold the rest. The opt-in `fail_*` knobs flip individual
//! mutating calls into the failure path.

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

use anyhow::{anyhow, Result};

use crate::client::{HnApi, StoryNumericFilters, StorySortMode};
use crate::model::{Article, PageData, Story, VoteData, VoteDirection, VouchData};

/// One recorded interaction with [`FakeHnApi`].
#[derive(Debug, Clone)]
pub enum FakeCall {
    GetPageData(u32),
    GetUserThreadsPage(String, usize),
    GetStoriesByTag(String, StorySortMode, usize, StoryNumericFilters),
    GetListingVoteState(String, StorySortMode, usize),
    GetListingVouchState(String, StorySortMode, usize),
    GetMatchedStories(String, bool, usize),
    Login(String, String),
    CurrentSessionCookie,
    Vote(u32, String, Option<VoteDirection>),
    Vouch(u32, String, bool),
    GetVoteDataForItem(u32),
    GetVouchDataForItem(u32),
    GetArticle(String),
}

#[derive(Default)]
struct FakeState {
    calls: Vec<FakeCall>,
    page_data: VecDeque<PageData>,
    user_threads: VecDeque<PageData>,
    stories_by_tag: HashMap<String, Vec<Story>>,
    listing_vote_state: HashMap<(String, StorySortMode, usize), HashMap<u32, VoteData>>,
    listing_vouch_state: HashMap<(String, StorySortMode, usize), HashMap<u32, VouchData>>,
    matched_stories: HashMap<(String, bool, usize), Vec<Story>>,
    vote_data: HashMap<u32, Option<VoteData>>,
    vouch_data: HashMap<u32, Option<VouchData>>,
    article: HashMap<String, Article>,
    session_cookie: Option<String>,
    fail_login: bool,
    fail_vote: bool,
    fail_vouch: bool,
}

/// Test double for [`HnApi`]. Construct with [`FakeHnApi::new`] (or
/// [`FakeHnApi::default`]), wire it into a view through `&'static dyn
/// HnApi`, register fixtures with the `set_*` / `enqueue_*` methods, then
/// inspect [`FakeHnApi::calls`] to assert what the view requested.
#[derive(Default)]
pub struct FakeHnApi {
    state: Mutex<FakeState>,
}

impl FakeHnApi {
    pub fn new() -> Self {
        Self::default()
    }

    /// Snapshot of every call the fake has received so far, in the order
    /// they happened.
    pub fn calls(&self) -> Vec<FakeCall> {
        self.state.lock().unwrap().calls.clone()
    }

    /// Number of recorded calls. Convenience wrapper around
    /// [`Self::calls`].
    pub fn call_count(&self) -> usize {
        self.state.lock().unwrap().calls.len()
    }

    pub fn enqueue_page_data(&self, data: PageData) {
        self.state.lock().unwrap().page_data.push_back(data);
    }

    pub fn enqueue_user_threads(&self, data: PageData) {
        self.state.lock().unwrap().user_threads.push_back(data);
    }

    pub fn set_stories_for_tag(&self, tag: &str, stories: Vec<Story>) {
        self.state
            .lock()
            .unwrap()
            .stories_by_tag
            .insert(tag.to_string(), stories);
    }

    pub fn set_listing_vote_state(
        &self,
        tag: &str,
        sort_mode: StorySortMode,
        page: usize,
        state: HashMap<u32, VoteData>,
    ) {
        self.state
            .lock()
            .unwrap()
            .listing_vote_state
            .insert((tag.to_string(), sort_mode, page), state);
    }

    pub fn set_listing_vouch_state(
        &self,
        tag: &str,
        sort_mode: StorySortMode,
        page: usize,
        state: HashMap<u32, VouchData>,
    ) {
        self.state
            .lock()
            .unwrap()
            .listing_vouch_state
            .insert((tag.to_string(), sort_mode, page), state);
    }

    pub fn set_matched_stories(
        &self,
        query: &str,
        by_date: bool,
        page: usize,
        stories: Vec<Story>,
    ) {
        self.state
            .lock()
            .unwrap()
            .matched_stories
            .insert((query.to_string(), by_date, page), stories);
    }

    pub fn set_vote_data(&self, item_id: u32, data: Option<VoteData>) {
        self.state.lock().unwrap().vote_data.insert(item_id, data);
    }

    pub fn set_vouch_data(&self, item_id: u32, data: Option<VouchData>) {
        self.state.lock().unwrap().vouch_data.insert(item_id, data);
    }

    pub fn set_article(&self, url: &str, article: Article) {
        self.state
            .lock()
            .unwrap()
            .article
            .insert(url.to_string(), article);
    }

    pub fn set_session_cookie(&self, cookie: Option<String>) {
        self.state.lock().unwrap().session_cookie = cookie;
    }

    pub fn fail_login(&self) {
        self.state.lock().unwrap().fail_login = true;
    }

    pub fn fail_vote(&self) {
        self.state.lock().unwrap().fail_vote = true;
    }

    pub fn fail_vouch(&self) {
        self.state.lock().unwrap().fail_vouch = true;
    }
}

impl HnApi for FakeHnApi {
    fn get_page_data(&self, item_id: u32) -> Result<PageData> {
        let mut s = self.state.lock().unwrap();
        s.calls.push(FakeCall::GetPageData(item_id));
        s.page_data
            .pop_front()
            .ok_or_else(|| anyhow!("FakeHnApi: no page data queued for item {item_id}"))
    }

    fn get_user_threads_page(&self, username: &str, page: usize) -> Result<PageData> {
        let mut s = self.state.lock().unwrap();
        s.calls
            .push(FakeCall::GetUserThreadsPage(username.to_string(), page));
        s.user_threads.pop_front().ok_or_else(|| {
            anyhow!("FakeHnApi: no user-threads page queued for {username} (page {page})")
        })
    }

    fn get_stories_by_tag(
        &self,
        tag: &str,
        sort_mode: StorySortMode,
        page: usize,
        numeric_filters: StoryNumericFilters,
    ) -> Result<Vec<Story>> {
        let mut s = self.state.lock().unwrap();
        s.calls.push(FakeCall::GetStoriesByTag(
            tag.to_string(),
            sort_mode,
            page,
            numeric_filters,
        ));
        Ok(s.stories_by_tag.get(tag).cloned().unwrap_or_default())
    }

    fn get_listing_vote_state(
        &self,
        tag: &str,
        sort_mode: StorySortMode,
        page: usize,
    ) -> Result<HashMap<u32, VoteData>> {
        let mut s = self.state.lock().unwrap();
        s.calls.push(FakeCall::GetListingVoteState(
            tag.to_string(),
            sort_mode,
            page,
        ));
        Ok(s.listing_vote_state
            .get(&(tag.to_string(), sort_mode, page))
            .cloned()
            .unwrap_or_default())
    }

    fn get_listing_vouch_state(
        &self,
        tag: &str,
        sort_mode: StorySortMode,
        page: usize,
    ) -> Result<HashMap<u32, VouchData>> {
        let mut s = self.state.lock().unwrap();
        s.calls.push(FakeCall::GetListingVouchState(
            tag.to_string(),
            sort_mode,
            page,
        ));
        Ok(s.listing_vouch_state
            .get(&(tag.to_string(), sort_mode, page))
            .cloned()
            .unwrap_or_default())
    }

    fn get_matched_stories(&self, query: &str, by_date: bool, page: usize) -> Result<Vec<Story>> {
        let mut s = self.state.lock().unwrap();
        s.calls
            .push(FakeCall::GetMatchedStories(query.to_string(), by_date, page));
        Ok(s.matched_stories
            .get(&(query.to_string(), by_date, page))
            .cloned()
            .unwrap_or_default())
    }

    fn login(&self, username: &str, password: &str) -> Result<()> {
        let mut s = self.state.lock().unwrap();
        s.calls
            .push(FakeCall::Login(username.to_string(), password.to_string()));
        if s.fail_login {
            Err(anyhow!("FakeHnApi: login failed (stub)"))
        } else {
            Ok(())
        }
    }

    fn current_session_cookie(&self) -> Option<String> {
        let mut s = self.state.lock().unwrap();
        s.calls.push(FakeCall::CurrentSessionCookie);
        s.session_cookie.clone()
    }

    fn vote(&self, id: u32, auth: &str, new_vote: Option<VoteDirection>) -> Result<()> {
        let mut s = self.state.lock().unwrap();
        s.calls
            .push(FakeCall::Vote(id, auth.to_string(), new_vote));
        if s.fail_vote {
            Err(anyhow!("FakeHnApi: vote failed (stub)"))
        } else {
            Ok(())
        }
    }

    fn vouch(&self, id: u32, auth: &str, rescind: bool) -> Result<()> {
        let mut s = self.state.lock().unwrap();
        s.calls
            .push(FakeCall::Vouch(id, auth.to_string(), rescind));
        if s.fail_vouch {
            Err(anyhow!("FakeHnApi: vouch failed (stub)"))
        } else {
            Ok(())
        }
    }

    fn get_vote_data_for_item(&self, item_id: u32) -> Result<Option<VoteData>> {
        let mut s = self.state.lock().unwrap();
        s.calls.push(FakeCall::GetVoteDataForItem(item_id));
        Ok(s.vote_data.get(&item_id).cloned().flatten())
    }

    fn get_vouch_data_for_item(&self, item_id: u32) -> Result<Option<VouchData>> {
        let mut s = self.state.lock().unwrap();
        s.calls.push(FakeCall::GetVouchDataForItem(item_id));
        Ok(s.vouch_data.get(&item_id).cloned().flatten())
    }

    fn get_article(&self, url: &str) -> Result<Article> {
        let mut s = self.state.lock().unwrap();
        s.calls.push(FakeCall::GetArticle(url.to_string()));
        s.article
            .get(url)
            .cloned()
            .ok_or_else(|| anyhow!("FakeHnApi: no article queued for {url}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Story;

    fn story(id: u32, title: &str) -> Story {
        Story {
            id,
            url: format!("https://example.com/{id}"),
            author: "alice".into(),
            points: 10,
            num_comments: 0,
            time: 0,
            title: title.into(),
            content: String::new(),
            dead: false,
            flagged: false,
        }
    }

    #[test]
    fn unstubbed_calls_return_empty_and_record_the_call() {
        let fake = FakeHnApi::new();

        let stories = fake
            .get_stories_by_tag(
                "front_page",
                StorySortMode::None,
                0,
                StoryNumericFilters::default(),
            )
            .unwrap();

        assert!(stories.is_empty());
        assert_eq!(fake.call_count(), 1);
        assert!(matches!(
            fake.calls()[0],
            FakeCall::GetStoriesByTag(ref tag, StorySortMode::None, 0, _) if tag == "front_page"
        ));
    }

    #[test]
    fn stubbed_stories_are_returned_for_their_tag() {
        let fake = FakeHnApi::new();
        fake.set_stories_for_tag("front_page", vec![story(1, "hi"), story(2, "bye")]);

        let result = fake
            .get_stories_by_tag(
                "front_page",
                StorySortMode::None,
                0,
                StoryNumericFilters::default(),
            )
            .unwrap();

        assert_eq!(result.len(), 2);
        assert_eq!(result[0].id, 1);
        assert_eq!(result[1].title, "bye");
    }

    #[test]
    fn other_tags_get_empty_response_independently() {
        let fake = FakeHnApi::new();
        fake.set_stories_for_tag("front_page", vec![story(1, "hi")]);

        let result = fake
            .get_stories_by_tag(
                "ask_hn",
                StorySortMode::None,
                0,
                StoryNumericFilters::default(),
            )
            .unwrap();

        assert!(result.is_empty());
    }

    #[test]
    fn login_records_credentials_and_honours_failure_flag() {
        let fake = FakeHnApi::new();
        assert!(fake.login("alice", "hunter2").is_ok());

        fake.fail_login();
        let err = fake.login("alice", "wrong").unwrap_err();
        assert!(err.to_string().contains("login failed"));
        assert_eq!(fake.call_count(), 2);
        assert!(matches!(
            fake.calls()[1],
            FakeCall::Login(ref u, ref p) if u == "alice" && p == "wrong"
        ));
    }

    #[test]
    fn vote_records_direction_and_returns_failure_when_flagged() {
        let fake = FakeHnApi::new();
        assert!(fake.vote(123, "auth-token", Some(VoteDirection::Up)).is_ok());
        fake.fail_vote();
        assert!(fake.vote(123, "auth-token", None).is_err());

        let calls = fake.calls();
        assert!(matches!(
            calls[0],
            FakeCall::Vote(123, ref a, Some(VoteDirection::Up)) if a == "auth-token"
        ));
        assert!(matches!(calls[1], FakeCall::Vote(123, _, None)));
    }

    #[test]
    fn dyn_dispatch_through_hn_api_trait_works() {
        let fake = FakeHnApi::new();
        fake.set_session_cookie(Some("user=alice&...".into()));

        let api: &dyn HnApi = &fake;
        assert_eq!(api.current_session_cookie().as_deref(), Some("user=alice&..."));
        assert!(matches!(fake.calls()[0], FakeCall::CurrentSessionCookie));
    }
}
