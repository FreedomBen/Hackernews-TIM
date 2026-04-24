use std::collections::HashMap;

use anyhow::Context;
use rayon::prelude::*;

use model::*;
// re-export
pub use query::{StoryNumericFilters, StorySortMode};

use crate::{prelude::*, utils::decode_html};

// modules
mod model;
mod query;

const HN_ALGOLIA_PREFIX: &str = "https://hn.algolia.com/api/v1";
const HN_OFFICIAL_PREFIX: &str = "https://hacker-news.firebaseio.com/v0";
const HN_SEARCH_QUERY_STRING: &str =
    "tags=story&restrictSearchableAttributes=title,url&typoTolerance=false";
pub const HN_HOST_URL: &str = "https://news.ycombinator.com";

/// Number of items served per page on HN's own listing pages
/// (`/news`, `/ask`, `/show`, `/newest`). The TUI paginates at
/// [`config::page_size`], which is user-configurable, so a single TUI
/// page can span one or more HN listing pages — callers that reconcile
/// the two must sweep the range returned by
/// [`hn_listing_pages_for_tui_page`].
const HN_LISTING_PAGE_SIZE: usize = 30;
pub const SEARCH_LIMIT: usize = 15;

static CLIENT: once_cell::sync::OnceCell<HNClient> = once_cell::sync::OnceCell::new();

/// Global slot for the logged-in user's display info. `None` means either no
/// credentials were configured or login failed — views should treat both the
/// same way and render nothing on the right of the title bar.
static USER_INFO: once_cell::sync::OnceCell<Option<UserInfo>> = once_cell::sync::OnceCell::new();

/// Summary of the logged-in HN user, mirrored into views' title bars.
///
/// `karma` is optional because the profile fetch is best-effort: a network
/// failure or a surprise HTML change shouldn't block startup, so we just
/// render the username on its own in that case. `showdead` mirrors the
/// HN profile preference; when true, page/listing fetches pass
/// `showdead=yes` so HN includes dead comments and stories.
#[derive(Debug, Clone)]
pub struct UserInfo {
    pub username: String,
    pub karma: Option<u32>,
    pub showdead: bool,
}

/// Parsed fields from a HN user profile page. The optionals are optional so
/// the parser can return a useful result even when HN tweaks its markup.
/// `showdead` defaults to `false` when the preference can't be parsed — HN
/// itself defaults new accounts to `no`.
#[derive(Debug, Default, Clone)]
pub struct ProfileInfo {
    pub topcolor: Option<String>,
    pub karma: Option<u32>,
    pub showdead: bool,
}

/// Outcome of the password-login attempt made at startup when no valid
/// cached session cookie was available. Surfaced in the UI so the user
/// doesn't have to tail `hn-tui.log` to know whether voting will work.
#[derive(Debug, Clone)]
pub enum StartupLoginStatus {
    /// No password login was attempted — either because there are no
    /// credentials configured, or because a cached session was still valid.
    NotAttempted,
    /// Password login succeeded; the session cookie was refreshed on disk.
    Success { username: String },
    /// HN replied with `Bad login.` — the stored credentials are wrong.
    BadLogin,
    /// HN served a CAPTCHA challenge. The TUI can't solve it, so the user
    /// needs to fall back to pasting a browser cookie into the auth file.
    Captcha,
    /// Any other error (network failure, unexpected HN response, etc.).
    Other(String),
}

impl StartupLoginStatus {
    /// Classify an error from [`HNClient::login`] by matching on the markers
    /// emitted by [`classify_login_response`]. Relying on substring match is
    /// a little fragile, but we only generate these strings in one place so
    /// any future rename will require touching both sides together.
    pub fn from_login_error(err: &Error) -> Self {
        let msg = err.to_string();
        if msg.contains("Bad login") {
            StartupLoginStatus::BadLogin
        } else if msg.contains("captcha") {
            StartupLoginStatus::Captcha
        } else {
            StartupLoginStatus::Other(msg)
        }
    }
}

/// HNClient is a HTTP client to communicate with Hacker News APIs.
#[derive(Clone)]
pub struct HNClient {
    client: ureq::Agent,
}

/// A macro to log the runtime of an expression
macro_rules! log {
    ($e:expr, $desc:expr) => {{
        let time = std::time::SystemTime::now();
        let result = $e;
        if let Ok(elapsed) = time.elapsed() {
            info!("{} took {}ms", $desc, elapsed.as_millis());
        }
        result
    }};
}

impl HNClient {
    /// Create a new Hacker News Client using the timeout from the loaded
    /// global config. Requires [`config::init_config`] to have been called.
    pub fn new() -> Result<HNClient> {
        Self::with_timeout(config::get_config().client_timeout)
    }

    /// Create a new Hacker News Client with an explicit timeout (in seconds).
    /// Useful during startup when the global config has not been sealed yet.
    pub fn with_timeout(timeout: u64) -> Result<HNClient> {
        Ok(HNClient {
            client: ureq::AgentBuilder::new()
                .timeout(std::time::Duration::from_secs(timeout))
                .build(),
        })
    }

    /// Create a new Hacker News Client whose cookie store already contains a
    /// previously captured HN session cookie. Used at startup to restore a
    /// logged-in session without re-POSTing to `/login` (which HN throttles
    /// with a CAPTCHA). If the cookie parse fails the client is still
    /// returned — the caller will fall back to a password login.
    pub fn with_cached_session(timeout: u64, session: &str) -> Result<HNClient> {
        let mut store = cookie_store::CookieStore::default();
        if let Ok(url) = url::Url::parse(HN_HOST_URL) {
            // Give the cookie an explicit Max-Age so the store doesn't
            // discard it as a session-only cookie when it's re-parsed.
            let cookie_str = format!("user={session}; Path=/; Max-Age=31536000");
            if let Err(err) = store.parse(&cookie_str, &url) {
                warn!("failed to load cached HN session cookie: {err}");
            }
        }
        Ok(HNClient {
            client: ureq::AgentBuilder::new()
                .timeout(std::time::Duration::from_secs(timeout))
                .cookie_store(store)
                .build(),
        })
    }

    /// Check whether this client's current cookie jar produces an
    /// authenticated HN session. We fetch the front page and look for the
    /// `logout` link that HN only renders when a valid `user` cookie is
    /// attached. Returns `false` on any network/parse error — the caller
    /// should treat that the same as an expired cookie.
    pub fn verify_session(&self) -> bool {
        let url = format!("{HN_HOST_URL}/news");
        match self.client.get(&url).call() {
            Ok(resp) => match resp.into_string() {
                Ok(body) => body.contains("href=\"logout"),
                Err(err) => {
                    warn!("failed to read {url}: {err}");
                    false
                }
            },
            Err(err) => {
                warn!("failed to fetch {url}: {err}");
                false
            }
        }
    }

    /// Extract the current value of HN's `user` session cookie from this
    /// client's cookie jar, if any. Called after a successful login (or
    /// session verification) so the caller can persist the latest cookie.
    pub fn current_session_cookie(&self) -> Option<String> {
        let url = url::Url::parse(HN_HOST_URL).ok()?;
        let domain = url.host_str()?;
        self.client
            .cookie_store()
            .get(domain, "/", "user")
            .map(|c| c.value().to_string())
    }

    /// Get data of a HN item based on its id then parse the data
    /// to a corresponding struct representing that item
    pub fn get_item_from_id<T>(&self, id: u32) -> Result<T>
    where
        T: serde::de::DeserializeOwned,
    {
        let request_url = format!("{HN_ALGOLIA_PREFIX}/items/{id}");
        let item = log!(
            self.client.get(&request_url).call()?.into_json::<T>()?,
            format!("get HN item (id={id}) using {request_url}")
        );
        Ok(item)
    }

    pub fn get_page_data(&self, item_id: u32) -> Result<PageData> {
        // get the root item in the page
        let request_url = format!("{HN_OFFICIAL_PREFIX}/item/{item_id}.json");
        let item = log!(
            self.client
                .get(&request_url)
                .call()?
                .into_json::<ItemResponse>()?,
            format!("get item (id={item_id}) using {request_url}")
        );

        let text = decode_html(&item.text.unwrap_or_default());

        // Construct the shortened text to represent the page's title if not exist
        let chars = text.replace('\n', " ").chars().collect::<Vec<_>>();
        let limit = 64;
        let shortened_text = if chars.len() > limit {
            String::from_iter(chars[..limit].iter()) + "..."
        } else {
            text.to_string()
        };

        let url = item
            .url
            .unwrap_or(format!("{HN_HOST_URL}/item?id={item_id}"));
        let title = item.title.unwrap_or(shortened_text);

        // parse the root item of the page
        let root_item: HnItem = match item.typ.as_str() {
            "story" => Story {
                id: item_id,
                url: url.clone(),
                author: item.by.unwrap_or_default(),
                points: item.score.unwrap_or_default(),
                num_comments: item.descendants.unwrap_or_default(),
                time: item.time,
                title: title.clone(),
                content: text,
                dead: item.dead,
                flagged: item.flagged,
            }
            .into(),
            "comment" => Comment {
                id: item_id,
                level: 0,
                n_children: 0,
                author: item.by.unwrap_or_default(),
                time: item.time,
                content: text,
                dead: item.dead,
                flagged: item.flagged,
            }
            .into(),
            typ => {
                anyhow::bail!("unknown item type: {typ}");
            }
        };

        // When the user is logged in, the Algolia API's snapshot lags HN's
        // own HTML by several minutes — long enough that the user can't see
        // their own freshly-posted comment. Route authenticated sessions
        // through a single HN HTML fetch that serves both the vote state and
        // the comment tree. Unauthenticated sessions keep the parallel
        // Algolia path, which is faster for them and carries no freshness
        // penalty (they can't vote or reply anyway).
        let (vote_state, vouch_state, comment_receiver) = if get_user_info().is_some() {
            let content = log!(
                self.get_page_content(item_id)?,
                format!("fetch HN page HTML for comments (id={item_id})")
            );
            let vote_state = self.parse_vote_data(&content)?;
            let vouch_state = self.parse_vouch_data(&content)?;
            let receiver = html_comment_receiver(content);
            (vote_state, vouch_state, receiver)
        } else {
            // Parallelize two tasks using [`rayon::join`](https://docs.rs/rayon/latest/rayon/fn.join.html).
            // Vouch state is only meaningful for logged-in users, so the
            // anonymous path skips parsing it.
            let (vote_state, comment_receiver) = rayon::join(
                || {
                    // get the page's vote state
                    log!(
                        {
                            let content = self.get_page_content(item_id)?;
                            self.parse_vote_data(&content)
                        },
                        format!("get page's vote state of item (id={item_id}) ")
                    )
                },
                // lazily load the page's top comments
                || self.lazy_load_comments(item.kids),
            );
            (vote_state?, HashMap::new(), comment_receiver?)
        };

        Ok(PageData {
            title,
            url,
            root_item,
            comment_receiver,
            vote_state,
            vouch_state,
        })
    }

    /// lazily loads comments of a Hacker News item
    fn lazy_load_comments(&self, mut comment_ids: Vec<u32>) -> Result<CommentReceiver> {
        let (sender, receiver) = crossbeam_channel::bounded(32);

        // loads the first 5 top comments to ensure the corresponding `CommentView` has data to render
        self.load_comments(&sender, &mut comment_ids, 5)?;
        std::thread::spawn({
            let client = self.clone();
            let sleep_dur = std::time::Duration::from_millis(1000);
            move || {
                while !comment_ids.is_empty() {
                    if let Err(err) = client.load_comments(&sender, &mut comment_ids, 5) {
                        warn!("encountered an error when loading comments: {}", err);
                        break;
                    }
                    std::thread::sleep(sleep_dur);
                }
            }
        });
        Ok(receiver)
    }

    /// Load the first `size` comments from a list of comment IDs.
    fn load_comments(&self, sender: &CommentSender, ids: &mut Vec<u32>, size: usize) -> Result<()> {
        let size = std::cmp::min(ids.len(), size);
        if size == 0 {
            return Ok(());
        }

        let responses = ids
            .drain(0..size)
            .collect::<Vec<_>>()
            .into_par_iter()
            .map(|id| match self.get_item_from_id::<CommentResponse>(id) {
                Ok(response) => Some(response),
                Err(err) => {
                    warn!("failed to get comment (id={}): {}", id, err);
                    None
                }
            })
            .flatten()
            .collect::<Vec<_>>();

        for response in responses {
            sender.send(response.into())?;
        }

        Ok(())
    }

    /// Get a story based on its id
    pub fn get_story_from_story_id(&self, id: u32) -> Result<Story> {
        let request_url = format!("{HN_ALGOLIA_PREFIX}/search?tags=story,story_{id}");
        let response = log!(
            self.client
                .get(&request_url)
                .call()?
                .into_json::<StoriesResponse>()?,
            format!("get story (id={id}) using {request_url}")
        );

        match <Vec<Story>>::from(response).pop() {
            Some(story) => Ok(story),
            None => Err(anyhow::anyhow!("failed to get story with id {}", id)),
        }
    }

    /// Get a list of stories matching certain conditions
    pub fn get_matched_stories(
        &self,
        query: &str,
        by_date: bool,
        page: usize,
    ) -> Result<Vec<Story>> {
        let request_url = format!(
            "{}/{}?{}&hitsPerPage={}&page={}",
            HN_ALGOLIA_PREFIX,
            if by_date { "search_by_date" } else { "search" },
            HN_SEARCH_QUERY_STRING,
            SEARCH_LIMIT,
            page
        );
        let response = log!(
            self.client
                .get(&request_url)
                .query("query", query)
                .call()?
                .into_json::<StoriesResponse>()?,
            format!(
                "get matched stories with query {query} (by_date={by_date}, page={page}) using {request_url}"
            )
        );

        Ok(response.into())
    }

    /// Reorder a list of stories to follow the same order as another list of story IDs.
    ///
    /// Needs to do this because stories returned by Algolia APIs are sorted by `points`,
    /// reoder those stories to match the list shown up in the HackerNews website,
    /// which has the same order as the list of IDs returned from the official API.
    fn reorder_stories_based_on_ids(&self, stories: Vec<Story>, ids: &[u32]) -> Vec<Story> {
        let mut stories = stories;
        stories.sort_by(|story_x, story_y| {
            let story_x_pos = ids
                .iter()
                .enumerate()
                .find(|&(_, story_id)| *story_id == story_x.id)
                .unwrap()
                .0;
            let story_y_pos = ids
                .iter()
                .enumerate()
                .find(|&(_, story_id)| *story_id == story_y.id)
                .unwrap()
                .0;

            story_x_pos.cmp(&story_y_pos)
        });
        stories
    }

    /// Retrieve a list of story IDs given a story tag using the HN Official API
    /// then compose a HN Algolia API to retrieve the corresponding stories' data.
    fn get_stories_no_sort(
        &self,
        tag: &str,
        page: usize,
        numeric_filters: query::StoryNumericFilters,
    ) -> Result<Vec<Story>> {
        // get the HN official API's endpoint based on query's story tag
        let endpoint = match tag {
            "front_page" => "/topstories.json",
            "ask_hn" => "/askstories.json",
            "show_hn" => "/showstories.json",
            _ => {
                anyhow::bail!("unsupported story tag {tag}");
            }
        };
        let request_url = format!("{HN_OFFICIAL_PREFIX}{endpoint}");
        let stories = log!(
            self.client
                .get(&request_url)
                .call()?
                .into_json::<Vec<u32>>()?,
            format!("get {tag} story IDs using {request_url}")
        );

        let page_size = config::page_size();
        let start_id = page_size * page;
        if start_id >= stories.len() {
            return Ok(vec![]);
        }

        let end_id = std::cmp::min(start_id + page_size, stories.len());
        let ids = &stories[start_id..end_id];

        let request_url = format!(
            "{}/search?tags=story,({}){}&hitsPerPage={}",
            HN_ALGOLIA_PREFIX,
            ids.iter().fold("".to_owned(), |tags, story_id| format!(
                "{tags}story_{story_id},"
            )),
            numeric_filters.query(),
            page_size,
        );

        let response = log!(
            self.client
                .get(&request_url)
                .call()?
                .into_json::<StoriesResponse>()?,
            format!("get stories (tag={tag}, page={page}) using {request_url}",)
        );

        Ok(self.reorder_stories_based_on_ids(response.into(), ids))
    }

    /// Get a list of stories filtering on a specific tag.
    ///
    /// Depending on the specifed `sort_mode`, stories are retrieved based on
    /// the Algolia API or a combination of Algolia API and the Official API.
    pub fn get_stories_by_tag(
        &self,
        tag: &str,
        sort_mode: StorySortMode,
        page: usize,
        numeric_filters: query::StoryNumericFilters,
    ) -> Result<Vec<Story>> {
        let search_op = match sort_mode {
            StorySortMode::None => {
                return self.get_stories_no_sort(tag, page, numeric_filters);
            }
            StorySortMode::Date => "search_by_date",
            StorySortMode::Points => "search", // Algolia API default search is sorted by points
        };

        let request_url = format!(
            "{}/{}?tags={}&hitsPerPage={}&page={}{}",
            HN_ALGOLIA_PREFIX,
            search_op,
            tag,
            config::page_size(),
            page,
            numeric_filters.query(),
        );

        let response = log!(
            self.client
                .get(&request_url)
                .call()?
                .into_json::<StoriesResponse>()?,
            format!(
                "get stories (tag={}, sort_mode={:?}, page={}, numeric_filters={}) using {}",
                tag,
                sort_mode,
                page,
                numeric_filters.query(),
                request_url
            )
        );

        Ok(response.into())
    }

    pub fn get_article(&self, url: &str) -> Result<Article> {
        let article_parse_command = &config::get_config().article_parse_command;
        let output = std::process::Command::new(&article_parse_command.command)
            .args(&article_parse_command.options)
            .arg(url)
            .output();

        match output {
            Ok(output) => {
                if output.status.success() {
                    match serde_json::from_slice::<Article>(&output.stdout) {
                        Ok(mut article) => {
                            // Replace a tab character by 4 spaces as it's possible
                            // that the terminal cannot render the tab character.
                            article.content = article.content.replace('\t', "    ");

                            article.url = url.to_string();
                            Ok(article)
                        }
                        Err(err) => {
                            let stdout = std::str::from_utf8(&output.stdout)?;
                            warn!("failed to deserialize {} into an `Article` struct:", stdout);
                            Err(anyhow::anyhow!(err))
                        }
                    }
                } else {
                    let stderr = std::str::from_utf8(&output.stderr)?.to_string();
                    Err(anyhow::anyhow!(stderr))
                }
            }
            Err(_) => {
                // fallback to the `readable-readability` crate if the command fails
                let html = self
                    .client
                    .get(url)
                    .call()
                    .with_context(|| "failed to get url")?
                    .into_string()
                    .with_context(|| "failed to turn the response into string")?;
                let (nodes, metadata) = readable_readability::Readability::new()
                    .base_url(url::Url::parse(url).with_context(|| "failed to parse url")?)
                    .parse(&html);

                let mut text = vec![];
                nodes
                    .serialize(&mut text)
                    .with_context(|| "failed to serialize nodes")?;
                let title = metadata
                    .page_title
                    .or(metadata.article_title)
                    .unwrap_or("(no title)".to_string());
                let content = std::str::from_utf8(&text)
                    .with_context(|| "failed to turn the text into string")?
                    .replace('\t', "    ")
                    .to_string();

                Ok(Article {
                    title,
                    content,
                    author: metadata.byline,
                    url: url.to_string(),
                    date_published: None,
                })
            }
        }
    }

    pub fn login(&self, username: &str, password: &str) -> Result<()> {
        info!("Trying to login, user={username}...");

        let res = self
            .client
            .post(&format!("{HN_HOST_URL}/login"))
            .set("mode", "no-cors")
            .set("credentials", "include")
            .set("Access-Control-Allow-Origin", "*")
            .send_form(&[("acct", username), ("pw", password)])?
            .into_string()?;

        classify_login_response(&res)
    }

    /// gets the HTML page content of a Hacker News item
    pub fn get_page_content(&self, item_id: u32) -> Result<String> {
        let morelink_rg = regex::Regex::new("<a.*?href='(?P<link>.*?)'.*class='morelink'.*?>")?;

        let url = format!(
            "{HN_HOST_URL}/item?id={item_id}{}",
            showdead_query_suffix("&")
        );
        let mut content = self.client.get(&url).call()?.into_string()?;

        // A Hacker News item can have multiple pages, so
        // we need to make additional requests for each page and concatenate all the responses.
        let mut curr_page_content = content.clone();

        while let Some(cap) = morelink_rg.captures(&curr_page_content) {
            let next_page_link = cap.name("link").unwrap().as_str().replace("&amp;", "&");

            let next_page_content = self
                .client
                .get(&format!("{HN_HOST_URL}/{next_page_link}"))
                .call()?
                .into_string()?;

            content.push_str(&next_page_content);
            curr_page_content = next_page_content;
        }

        Ok(content)
    }

    /// Parse vote data of items in a page.
    ///
    /// Returns a map from item id to a [`VoteData`] describing the user's
    /// current vote (if any), the available downvote privilege, and the
    /// auth token needed to submit future vote requests.
    pub fn parse_vote_data(&self, page_content: &str) -> Result<HashMap<String, VoteData>> {
        parse_vote_data_from_content(page_content)
    }

    /// Fetch the HN item page for a single item and return its [`VoteData`].
    ///
    /// Used by views that don't pre-load vote state (e.g. the story list)
    /// to lazily discover the auth token + current vote direction the first
    /// time the user tries to vote on a given item. Returns `Ok(None)` when
    /// HN doesn't render any vote links for the item (typically: the user
    /// isn't logged in, or the item doesn't accept votes).
    pub fn get_vote_data_for_item(&self, item_id: u32) -> Result<Option<VoteData>> {
        let content = self.get_page_content(item_id)?;
        let mut map = self.parse_vote_data(&content)?;
        Ok(map.remove(&item_id.to_string()))
    }

    /// Fetch vote state for every item on an HN listing page.
    ///
    /// Maps a TUI view (tag + sort mode) to its `news.ycombinator.com`
    /// equivalent (`front_page` → `/news`, `ask_hn` → `/ask`, `show_hn` →
    /// `/show`, `story` by date → `/newest`), so opening the story list
    /// surfaces the user's existing up/down arrows without waiting for
    /// the lazy per-item fetch. Views that don't have a stable HN
    /// listing URL (Algolia-sorted results outside the mappings above,
    /// `job`, custom keymaps, search) return an empty map; those views
    /// keep the existing lazy behavior. Errors are non-fatal for the
    /// caller — we'd rather render stories without vote arrows than
    /// fail the whole page load.
    ///
    /// Because the TUI paginates at [`config::page_size`] but HN paginates
    /// its listings at [`HN_LISTING_PAGE_SIZE`] (30), a single TUI page
    /// can straddle multiple HN pages. All of them are fetched and merged
    /// so every row that has vote data available gets an arrow.
    pub fn get_listing_vote_state(
        &self,
        tag: &str,
        sort_mode: StorySortMode,
        page: usize,
    ) -> Result<HashMap<u32, VoteData>> {
        let Some(path) = listing_path_for_view(tag, sort_mode) else {
            return Ok(HashMap::new());
        };
        let (first_hn_page, last_hn_page) =
            hn_listing_pages_for_tui_page(page, config::page_size());
        let mut merged = HashMap::new();
        for hn_page in first_hn_page..=last_hn_page {
            let url = format!(
                "{HN_HOST_URL}/{path}?p={hn_page}{}",
                showdead_query_suffix("&")
            );
            let content = log!(
                self.client.get(&url).call()?.into_string()?,
                format!(
                    "fetch listing vote state (tag={tag}, sort_mode={sort_mode:?}, tui_page={page}, hn_page={hn_page}) using {url}"
                )
            );
            let map = parse_vote_data_from_content(&content)?;
            merged.extend(
                map.into_iter()
                    .filter_map(|(k, v)| k.parse::<u32>().ok().map(|id| (id, v))),
            );
        }
        Ok(merged)
    }

    /// Apply (or rescind) a vote on a HN item.
    ///
    /// `new_vote = Some(dir)` sends `how=up|down` to HN. `new_vote = None`
    /// sends `how=un` to rescind whatever vote the user currently holds.
    pub fn vote(&self, id: u32, auth: &str, new_vote: Option<VoteDirection>) -> Result<()> {
        log!(
            {
                let how = match new_vote {
                    Some(dir) => dir.as_how_param(),
                    None => "un",
                };
                let vote_url = format!("{HN_HOST_URL}/vote?id={id}&how={how}&auth={auth}");
                self.client.get(&vote_url).call()?;
            },
            format!("vote HN item (id={id})")
        );
        Ok(())
    }

    /// Parse vouch data of items in a page.
    ///
    /// Returns a map from item id to a [`VouchData`] describing whether the
    /// logged-in user has vouched for the item and the auth token needed to
    /// submit a vouch/unvouch request. Only contains entries for items HN
    /// rendered a vouch link for — dead items the viewer has the karma to
    /// vouch on.
    pub fn parse_vouch_data(&self, page_content: &str) -> Result<HashMap<String, VouchData>> {
        parse_vouch_data_from_content(page_content)
    }

    /// Fetch the HN item page for a single item and return its [`VouchData`].
    ///
    /// Used by the story list to lazily discover the auth token the first
    /// time the user tries to vouch on a given dead item. Returns `Ok(None)`
    /// when HN doesn't render a vouch link for the item (not dead, not
    /// logged in, insufficient karma, or the viewer authored the item).
    pub fn get_vouch_data_for_item(&self, item_id: u32) -> Result<Option<VouchData>> {
        let content = self.get_page_content(item_id)?;
        let mut map = self.parse_vouch_data(&content)?;
        Ok(map.remove(&item_id.to_string()))
    }

    /// Fetch vouch state for every item on an HN listing page in one request.
    ///
    /// Mirrors [`get_listing_vote_state`](Self::get_listing_vote_state). HN
    /// only renders a vouch link on the listing for dead items the viewer
    /// has privilege to vouch on, so most listings will return an empty map
    /// — that's the common case and is expected.
    pub fn get_listing_vouch_state(
        &self,
        tag: &str,
        sort_mode: StorySortMode,
        page: usize,
    ) -> Result<HashMap<u32, VouchData>> {
        let Some(path) = listing_path_for_view(tag, sort_mode) else {
            return Ok(HashMap::new());
        };
        let (first_hn_page, last_hn_page) =
            hn_listing_pages_for_tui_page(page, config::page_size());
        let mut merged = HashMap::new();
        for hn_page in first_hn_page..=last_hn_page {
            let url = format!(
                "{HN_HOST_URL}/{path}?p={hn_page}{}",
                showdead_query_suffix("&")
            );
            let content = log!(
                self.client.get(&url).call()?.into_string()?,
                format!(
                    "fetch listing vouch state (tag={tag}, sort_mode={sort_mode:?}, tui_page={page}, hn_page={hn_page}) using {url}"
                )
            );
            let map = parse_vouch_data_from_content(&content)?;
            merged.extend(
                map.into_iter()
                    .filter_map(|(k, v)| k.parse::<u32>().ok().map(|id| (id, v))),
            );
        }
        Ok(merged)
    }

    /// Vouch for (or unvouch) a dead HN item.
    ///
    /// `rescind = false` sends `how=up` to restore karma to the item.
    /// `rescind = true` sends `how=un` to take that vouch back. Only
    /// meaningful on dead items the viewer has the karma to vouch on — the
    /// caller should gate on that before calling.
    pub fn vouch(&self, id: u32, auth: &str, rescind: bool) -> Result<()> {
        log!(
            {
                let how = if rescind { "un" } else { "up" };
                let url = format!("{HN_HOST_URL}/vouch?id={id}&how={how}&auth={auth}");
                self.client.get(&url).call()?;
            },
            format!("vouch HN item (id={id}, rescind={rescind})")
        );
        Ok(())
    }

    /// Fetch the prefilled edit form for a comment.
    ///
    /// HN only renders the edit form for comments the logged-in user owns
    /// and only inside the edit window (~2 hours). Outside either gate the
    /// page renders without a `hmac` input and this method errors — the UI
    /// should pre-gate the common case (ownership) so users don't see that
    /// failure path after typing a full edit.
    pub fn fetch_edit_form(&self, comment_id: u32) -> Result<EditForm> {
        let url = format!("{HN_HOST_URL}/edit?id={comment_id}");
        let body = self
            .client
            .get(&url)
            .call()
            .with_context(|| format!("fetching {url}"))?
            .into_string()?;
        let hmac = extract_hidden_input(&body, "hmac").ok_or_else(|| {
            let dump_path =
                std::env::temp_dir().join(format!("hn-edit-response-{comment_id}.html"));
            let hint = match std::fs::write(&dump_path, &body) {
                Ok(()) => {
                    format!(
                        " (response body saved to {} for inspection)",
                        dump_path.display()
                    )
                }
                Err(_) => String::new(),
            };
            anyhow::anyhow!(
                "no edit form on {url} — not your comment, edit window closed, \
                 or HN changed its markup{hint}"
            )
        })?;
        let text = extract_textarea(&body, "text")
            .map(|raw| decode_html(&raw).to_string())
            .unwrap_or_default();
        Ok(EditForm { hmac, text })
    }

    /// Submit an edit for a comment the user owns.
    ///
    /// `hmac` must be the token scraped from the same `/edit?id=<comment_id>`
    /// page the caller is operating against — HN ties the token to the id
    /// and rejects cross-edits.
    pub fn submit_comment_edit(&self, comment_id: u32, hmac: &str, new_text: &str) -> Result<()> {
        let id_str = comment_id.to_string();
        let url = format!("{HN_HOST_URL}/xedit");
        let response_body = self
            .client
            .post(&url)
            .send_form(&[("id", id_str.as_str()), ("hmac", hmac), ("text", new_text)])
            .with_context(|| format!("POST {url}"))?
            .into_string()?;
        classify_post_reply_response(&response_body)
    }

    /// Post a reply to a HN item.
    ///
    /// HN's reply flow is cookie-authenticated and CSRF-protected: we GET
    /// `/reply?id=<parent>` to scrape the per-request `hmac` token (plus the
    /// `goto` redirect target), then POST the body to `/comment` along with
    /// the session cookies carried by [`self.client`]. If the user isn't
    /// logged in, HN redirects the GET to its login page and the form lookup
    /// fails — we surface that as an explicit error instead of a silent noop.
    pub fn post_reply(&self, parent_id: u32, text: &str) -> Result<()> {
        let page_url = format!("{HN_HOST_URL}/reply?id={parent_id}");
        let page_body = self
            .client
            .get(&page_url)
            .call()
            .with_context(|| format!("fetching {page_url}"))?
            .into_string()?;
        let hmac = parse_reply_form(&page_body).ok_or_else(|| {
            let dump_path =
                std::env::temp_dir().join(format!("hn-reply-response-{parent_id}.html"));
            let hint = match std::fs::write(&dump_path, &page_body) {
                Ok(()) => {
                    format!(
                        " (response body saved to {} for inspection)",
                        dump_path.display()
                    )
                }
                Err(_) => String::new(),
            };
            let looks_like_login =
                page_body.contains(r#"name="acct""#) || page_body.contains(r#"name='acct'"#);
            let cause = if looks_like_login {
                "HN redirected the GET to its login page — the cached session is probably stale. \
                 Try deleting the `session` line in hn-auth.toml and restarting, or re-paste a \
                 fresh cookie"
            } else {
                "hmac field missing, or HN changed its markup"
            };
            anyhow::anyhow!("no reply form on {page_url} — {cause}{hint}")
        })?;
        let parent = parent_id.to_string();
        let comment_url = format!("{HN_HOST_URL}/comment");
        let response_body = self
            .client
            .post(&comment_url)
            .send_form(&[
                ("parent", parent.as_str()),
                ("goto", ""),
                ("hmac", hmac.as_str()),
                ("text", text),
            ])
            .with_context(|| format!("POST {comment_url}"))?
            .into_string()?;
        classify_post_reply_response(&response_body)
    }

    /// Fetch the logged-in user's profile page and return the parsed
    /// `topcolor` and `karma` together.
    ///
    /// Requires that [`login`](Self::login) has already succeeded on this
    /// client so session cookies are attached (the `topcolor` field only
    /// renders on the user's own profile). Both fields are best-effort:
    /// request failures and missing/malformed values are logged and
    /// swallowed, so a failed parse still lets the rest of the app start
    /// up cleanly.
    pub fn fetch_profile_info(&self, username: &str) -> ProfileInfo {
        let url = format!("{HN_HOST_URL}/user?id={username}");
        let body = log!(
            match self.client.get(&url).call() {
                Ok(resp) => match resp.into_string() {
                    Ok(body) => body,
                    Err(err) => {
                        warn!("failed to read {url}: {err}");
                        return ProfileInfo::default();
                    }
                },
                Err(err) => {
                    warn!("failed to fetch {url}: {err}");
                    return ProfileInfo::default();
                }
            },
            format!("fetch HN profile (user={username})")
        );
        ProfileInfo {
            topcolor: parse_topcolor_from_profile(&body),
            karma: parse_karma_from_profile(&body),
            showdead: parse_showdead_from_profile(&body),
        }
    }
}

/// Classify an HN `/login` response body as success or failure.
///
/// HN's `/login` POST behaves as follows:
///
/// - **Success** — 302 to `goto` (defaults to `/news`). `ureq` follows the
///   redirect, so the body we see is the logged-in HN page, which carries a
///   `<a href="logout?...">` link in its nav bar.
/// - **Bad credentials** — 200 with a body starting `Bad login.` that
///   re-renders the login form. No `logout` substring appears anywhere.
/// - **Captcha required** — 200 with a body starting `Validation required.`
///   that embeds a Google reCAPTCHA. HN serves this after repeated failed
///   attempts from the same IP; we can't solve it from a TUI, so surface
///   a specific error telling the user to log in via the web UI first.
///
/// Previously we only checked for the success marker (`href="logout`) and
/// treated its absence as failure — which was fine in theory, but fragile
/// enough that a wrong-password attempt once got through and was written to
/// the auth file. Check for explicit failure markers first so any unexpected
/// body also fails closed.
fn classify_login_response(body: &str) -> Result<()> {
    if body.contains("Bad login") {
        return Err(anyhow::anyhow!("Bad login"));
    }
    if body.contains("Validation required") {
        return Err(anyhow::anyhow!(
            "Hacker News requires a captcha — sign in once via news.ycombinator.com, then retry"
        ));
    }
    if body.contains("href=\"logout") {
        return Ok(());
    }
    Err(anyhow::anyhow!(
        "login failed: unexpected response from Hacker News"
    ))
}

/// Build the `showdead=yes` query-string fragment to append to an HN URL,
/// or an empty string when the logged-in user has left the preference off
/// (or when there's no logged-in user at all). `sep` is the character that
/// prefixes the fragment — pass `"?"` for URLs that don't already carry a
/// query string, `"&"` otherwise.
///
/// HN ignores `showdead=yes` on unauthenticated requests, so it's safe to
/// append unconditionally, but we still gate on the profile preference so
/// opting out in HN's settings propagates to this TUI as well.
fn showdead_query_suffix(sep: &str) -> String {
    if get_user_info().map(|u| u.showdead).unwrap_or(false) {
        format!("{sep}showdead=yes")
    } else {
        String::new()
    }
}

/// Map an internal story-view tag + sort mode to the HN listing path that
/// shows the same set of items with vote links attached. Only the views
/// backed by a stable HN-side page are mapped; Algolia-only views (search,
/// `story` sorted by points, `job`, custom keymaps) return `None` and fall
/// back to per-item lazy fetches.
///
/// The `story` tag sorted by date aligns with HN's `/newest` stream, so the
/// F2 "story (by_date)" view picks up arrows via the listing sweep too.
fn listing_path_for_view(tag: &str, sort_mode: StorySortMode) -> Option<&'static str> {
    match (tag, sort_mode) {
        ("front_page", _) => Some("news"),
        ("ask_hn", _) => Some("ask"),
        ("show_hn", _) => Some("show"),
        ("story", StorySortMode::Date) => Some("newest"),
        _ => None,
    }
}

/// The inclusive range of HN listing pages (1-indexed,
/// [`HN_LISTING_PAGE_SIZE`] items per page) that together cover the
/// items shown on a given TUI page (0-indexed, `page_size` items per
/// page). When `page_size` exceeds [`HN_LISTING_PAGE_SIZE`] the range
/// spans multiple HN pages; all of them need to be fetched by the
/// caller.
fn hn_listing_pages_for_tui_page(page: usize, page_size: usize) -> (usize, usize) {
    // Clamp so a config entry of 0 (or a buggy caller) doesn't wrap.
    let page_size = page_size.max(1);
    let start_item = page_size * page;
    let end_item = page_size * (page + 1) - 1;
    let first = start_item / HN_LISTING_PAGE_SIZE + 1;
    let last = end_item / HN_LISTING_PAGE_SIZE + 1;
    (first, last)
}

/// A snapshot of HN's edit form for a comment: the per-request `hmac`
/// token plus the comment's current text, decoded from its HTML-escaped
/// representation.
pub struct EditForm {
    pub hmac: String,
    pub text: String,
}

/// Extract the inner content of `<textarea name="NAME">...</textarea>`.
/// Uses `(?s)` so the `.` matches newlines — HN's textarea spans multiple
/// lines when the comment does. Accommodates either attribute quote style.
fn extract_textarea(body: &str, name: &str) -> Option<String> {
    let pattern = format!(
        r#"(?s)<textarea[^>]*name=['"]{}['"][^>]*>(.*?)</textarea>"#,
        regex::escape(name)
    );
    regex::Regex::new(&pattern)
        .ok()?
        .captures(body)?
        .get(1)
        .map(|m| m.as_str().to_string())
}

/// Scrape the per-request `hmac` token out of HN's reply form. Returns
/// `None` if the input is missing — typically because the user isn't
/// logged in (HN serves the login page instead). The `goto` input on the
/// reply form is always rendered empty (`<input name="goto">` with no
/// `value=`), so we don't bother parsing it; the POST just sends `goto=""`
/// which is what a browser does too.
fn parse_reply_form(body: &str) -> Option<String> {
    extract_hidden_input(body, "hmac")
}

/// Extract the `value="..."` of an `<input name="NAME" ...>` element. HN
/// uses both single- and double-quoted attribute values on different pages
/// so the regex accepts either.
fn extract_hidden_input(body: &str, name: &str) -> Option<String> {
    let pattern = format!(
        r#"name=['"]{}['"][^>]*value=['"]([^'"]+)['"]"#,
        regex::escape(name)
    );
    regex::Regex::new(&pattern)
        .ok()?
        .captures(body)?
        .get(1)
        .map(|m| m.as_str().to_string())
}

/// Classify an HN `/comment` response body after a reply POST.
///
/// HN renders errors as HTML pages with distinctive phrases; treat any of
/// them as failure. A missing error marker is treated as success — HN
/// redirects to the item page on a successful post and `ureq` follows the
/// redirect so we see that page's body.
fn classify_post_reply_response(body: &str) -> Result<()> {
    if body.contains("Unknown or expired link") {
        return Err(anyhow::anyhow!("reply link expired — try again"));
    }
    if body.contains("Validation required") {
        return Err(anyhow::anyhow!(
            "HN requires a CAPTCHA — log in via the web UI first, then retry"
        ));
    }
    if body.contains("You broke the rate limit") {
        return Err(anyhow::anyhow!("HN rate-limited the reply"));
    }
    Ok(())
}

/// Spawn a background thread that parses `page_content` into comments and
/// pushes them through a fresh channel, grouped by top-level thread so the
/// view can render each subtree as it arrives (matching the progressive
/// feel of the Algolia loader). The parse is off the UI thread because large
/// threads can take a few ms and we'd rather the page land before the parse
/// finishes.
fn html_comment_receiver(page_content: String) -> CommentReceiver {
    let (sender, receiver) = crossbeam_channel::bounded(32);
    std::thread::spawn(move || {
        let comments = parse_comments_from_content(&page_content);
        let mut group: Vec<Comment> = Vec::new();
        for comment in comments {
            if comment.level == 0
                && !group.is_empty()
                && sender.send(std::mem::take(&mut group)).is_err()
            {
                return;
            }
            group.push(comment);
        }
        if !group.is_empty() {
            let _ = sender.send(group);
        }
    });
    receiver
}

/// Extract every comment from a rendered HN item page.
///
/// HN renders comments as a flat `<tr class="athing comtr">` list — nesting
/// is encoded by the `indent="N"` attribute on each row's `<td class="ind">`
/// cell rather than by DOM nesting — so one pass over the rows, in document
/// order, reconstructs the depth-first tree. `n_children` (total descendants)
/// is derived from that order by counting subsequent rows whose indent is
/// greater, which mirrors how the Algolia branch builds it out of nested
/// `CommentResponse` structures.
///
/// Rows with a missing author or missing comment text are skipped — HN uses
/// those shapes for dead/deleted comments, and the Algolia branch filters
/// them identically via `CommentResponse -> Vec<Comment>`.
fn parse_comments_from_content(page_content: &str) -> Vec<Comment> {
    let anchor_rg = match regex::Regex::new(r#"<tr class="athing comtr" id="(\d+)">"#) {
        Ok(rg) => rg,
        Err(_) => return Vec::new(),
    };
    let indent_rg = regex::Regex::new(r#"<td class="ind" indent="(\d+)">"#).unwrap();
    let author_rg = regex::Regex::new(r#"<a href="user\?id=([^"]+)" class="hnuser">"#).unwrap();
    // The age span's title is "YYYY-MM-DDTHH:MM:SS <unix_time>"; grab the
    // trailing integer so we don't have to parse the human-readable half.
    let time_rg = regex::Regex::new(r#"<span class="age" title="[^"]* (\d+)">"#).unwrap();
    let text_rg = regex::Regex::new(r#"(?s)<div class="commtext[^"]*">(.*?)</div>"#).unwrap();
    // HN marks dead / flagged comments with literal ` [flagged] ` / ` [dead] `
    // tokens in the comhead, right after the empty `unv_<id>` span. A comment
    // can carry either, both, or neither. With `showdead=yes` the row itself
    // still renders, so we surface those flags to the view layer.
    let dead_rg = regex::Regex::new(r#"<span id="unv_[^"]*"></span>[^<]*\[dead\]"#).unwrap();
    let flagged_rg = regex::Regex::new(r#"<span id="unv_[^"]*"></span>[^<]*\[flagged\]"#).unwrap();

    let anchors: Vec<(u32, usize, usize)> = anchor_rg
        .captures_iter(page_content)
        .filter_map(|c| {
            let id: u32 = c.get(1)?.as_str().parse().ok()?;
            let m = c.get(0)?;
            Some((id, m.start(), m.end()))
        })
        .collect();

    let mut comments: Vec<Comment> = Vec::with_capacity(anchors.len());
    for i in 0..anchors.len() {
        let (id, _, body_start) = anchors[i];
        let body_end = anchors
            .get(i + 1)
            .map(|a| a.1)
            .unwrap_or(page_content.len());
        let body = &page_content[body_start..body_end];

        let Some(level) = indent_rg
            .captures(body)
            .and_then(|c| c.get(1))
            .and_then(|m| m.as_str().parse::<usize>().ok())
        else {
            continue;
        };
        let Some(author) = author_rg
            .captures(body)
            .and_then(|c| c.get(1))
            .map(|m| decode_html(m.as_str()).to_string())
        else {
            continue;
        };
        let Some(content) = text_rg
            .captures(body)
            .and_then(|c| c.get(1))
            .map(|m| decode_html(m.as_str()).to_string())
        else {
            continue;
        };
        let time: u64 = time_rg
            .captures(body)
            .and_then(|c| c.get(1))
            .and_then(|m| m.as_str().parse().ok())
            .unwrap_or(0);

        let dead = dead_rg.is_match(body);
        let flagged = flagged_rg.is_match(body);

        comments.push(Comment {
            id,
            level,
            n_children: 0,
            author,
            time,
            content,
            dead,
            flagged,
        });
    }

    // Fill in n_children by walking forward from each row: its descendants
    // are the contiguous run of following rows with a strictly greater level.
    let levels: Vec<usize> = comments.iter().map(|c| c.level).collect();
    for (i, comment) in comments.iter_mut().enumerate() {
        let level = comment.level;
        comment.n_children = levels[i + 1..].iter().take_while(|&&l| l > level).count();
    }

    comments
}

/// Parse vote data out of a rendered HN item page.
///
/// HN's HTML exposes three anchor ids per voteable item:
///
/// - `up_<id>`   — present when the user hasn't upvoted (can still upvote)
/// - `down_<id>` — present when the user hasn't downvoted and has the
///   karma/privilege to downvote this particular item
/// - `un_<id>`   — present when the user has already voted; clicking it
///   rescinds the vote (direction is implicit)
///
/// We reconstruct the user's current vote by checking which of the up/down
/// arrows got "consumed" (replaced by the `un` link): if `un_<id>` and
/// `down_<id>` are both present but `up_<id>` is not, the user upvoted; the
/// mirror case means they downvoted. `can_downvote` tracks whether HN
/// rendered a downvote link for this item — i.e. the user has the privilege.
fn parse_vote_data_from_content(page_content: &str) -> Result<HashMap<String, VoteData>> {
    // `[^>]*?` keeps each match inside a single `<a ...>` open tag. The
    // earlier `.*?` pattern happily crossed tag boundaries, which would
    // let a neighbouring anchor's attributes leak into the captured tag
    // and defeat the `nosee` check below.
    let upvote_rg =
        regex::Regex::new("<a[^>]*?id='up_(?P<id>[^']*?)'[^>]*?auth=(?P<auth>[0-9a-z]*)[^>]*?>")?;
    let downvote_rg =
        regex::Regex::new("<a[^>]*?id='down_(?P<id>[^']*?)'[^>]*?auth=(?P<auth>[0-9a-z]*)[^>]*?>")?;
    // Capture the un_ anchor's inner text too: HN sets it to `unvote`
    // for an upvote and `undown` for a downvote — the only reliable
    // signal in the DOM for the user's vote direction.
    let unvote_rg = regex::Regex::new(
        "<a[^>]*?id='un_(?P<id>[^']*?)'[^>]*?auth=(?P<auth>[0-9a-z]*)[^>]*?>(?P<text>[^<]*)</a>",
    )?;

    #[derive(Default)]
    struct Flags {
        // `<a id='up_...'>` rendered without `class='nosee'` — the arrow
        // is live and the user hasn't used their upvote on this item.
        up_clickable: bool,
        // `<a id='up_...'>` rendered at all (with or without nosee). HN
        // omits the tag entirely only for items the user cannot upvote
        // (typically: their own post).
        up_present: bool,
        down_clickable: bool,
        // `<a id='down_...'>` rendered at all. HN omits it for users who
        // lack the karma to downvote, so presence implies downvote
        // privilege on this item — independent of whether a vote was
        // already cast.
        down_present: bool,
        // `<a id='un_...'>` present. Rendered for recent votes that are
        // still within HN's unvote window; always clickable (no nosee).
        has_un: bool,
        // Vote direction recovered from the un_ anchor's link text, if any.
        un_direction: Option<VoteDirection>,
        auth: String,
    }

    let mut flags: HashMap<String, Flags> = HashMap::new();
    // HN does not remove the used-up arrow after voting — it keeps both
    // `<a id='up_...'>` and `<a id='down_...'>` tags and adds
    // `class='nosee'` to hide them via CSS. The vote direction is
    // recovered from the un_ anchor's link text: `unvote` for an
    // upvote, `undown` for a downvote. Older votes past the unvote
    // window render neither an un_ anchor nor a direction-bearing
    // cue; for those we fall back to assuming an upvote, since
    // downvotes on comments are rare and gated on high karma.
    for c in upvote_rg.captures_iter(page_content) {
        let whole = c.get(0).unwrap().as_str();
        let id = c.name("id").unwrap().as_str().to_owned();
        let auth = c.name("auth").unwrap().as_str().to_owned();
        let entry = flags.entry(id).or_default();
        entry.up_present = true;
        if !whole.contains("nosee") {
            entry.up_clickable = true;
        }
        if !auth.is_empty() {
            entry.auth = auth;
        }
    }
    for c in downvote_rg.captures_iter(page_content) {
        let whole = c.get(0).unwrap().as_str();
        let id = c.name("id").unwrap().as_str().to_owned();
        let auth = c.name("auth").unwrap().as_str().to_owned();
        let entry = flags.entry(id).or_default();
        entry.down_present = true;
        if !whole.contains("nosee") {
            entry.down_clickable = true;
        }
        if !auth.is_empty() {
            entry.auth = auth;
        }
    }
    for c in unvote_rg.captures_iter(page_content) {
        let id = c.name("id").unwrap().as_str().to_owned();
        let auth = c.name("auth").unwrap().as_str().to_owned();
        let text = c.name("text").map(|m| m.as_str().trim()).unwrap_or("");
        let entry = flags.entry(id).or_default();
        entry.has_un = true;
        entry.un_direction = match text {
            "undown" => Some(VoteDirection::Down),
            // `unvote` is the upvote case. Anything else (unexpected text,
            // trimmed-to-empty) falls back to the "voted, direction
            // unknown" branch below.
            "unvote" => Some(VoteDirection::Up),
            _ => None,
        };
        if !auth.is_empty() {
            entry.auth = auth;
        }
    }

    let hm = flags
        .into_iter()
        .map(|(id, f)| {
            let voted = f.has_un
                || (f.up_present && !f.up_clickable)
                || (f.down_present && !f.down_clickable);
            let vote = if voted {
                // Prefer the un_ text (authoritative for recent votes).
                // Fall back to Up for older votes past the unvote window,
                // whose DOM carries no direction cue.
                Some(f.un_direction.unwrap_or(VoteDirection::Up))
            } else {
                None
            };
            (
                id,
                VoteData {
                    auth: f.auth,
                    vote,
                    can_downvote: f.down_present,
                },
            )
        })
        .collect();

    Ok(hm)
}

/// Parse vouch data out of a rendered HN item or listing page.
///
/// HN only renders a vouch anchor for dead items the viewer has the karma
/// to vouch on (30+ as of this writing) and isn't the author of. The anchor
/// looks like:
///
/// ```html
/// <a id='vouch_<id>' ... href='vouch?id=<id>&how=up&auth=<token>&goto=...'>vouch</a>
/// ```
///
/// Once the viewer has vouched, HN rewrites the same anchor to advertise
/// the reverse action: the inner text becomes `unvouch` and the `how`
/// parameter flips to `un`. We key off the inner text because it's the
/// same signal HN's own JavaScript uses, and it doesn't require us to
/// understand the `goto` query the anchor carries. Items without any
/// `vouch_<id>` anchor are omitted from the returned map — callers can
/// treat absence as "vouching is unavailable for this item".
fn parse_vouch_data_from_content(page_content: &str) -> Result<HashMap<String, VouchData>> {
    let vouch_rg = regex::Regex::new(
        "<a[^>]*?id='vouch_(?P<id>[^']*?)'[^>]*?auth=(?P<auth>[0-9a-z]*)[^>]*?>(?P<text>[^<]*)</a>",
    )?;

    let mut hm: HashMap<String, VouchData> = HashMap::new();
    for c in vouch_rg.captures_iter(page_content) {
        let id = c.name("id").unwrap().as_str().to_owned();
        let auth = c.name("auth").unwrap().as_str().to_owned();
        let text = c.name("text").map(|m| m.as_str().trim()).unwrap_or("");
        // HN renders `vouch` before the viewer has vouched and `unvouch`
        // after. Any other text (empty, unexpected) falls through to the
        // "not vouched" branch — it's the safe default, since a stale
        // `vouched=true` would turn the next keypress into an unintended
        // unvouch.
        let vouched = text.eq_ignore_ascii_case("unvouch");
        if auth.is_empty() {
            continue;
        }
        hm.insert(id, VouchData { auth, vouched });
    }

    Ok(hm)
}

/// Extract the `topcolor` value from the HTML of a HN user's profile page.
///
/// The edit form on `news.ycombinator.com/user?id=<self>` renders the
/// preference as a labelled row: `<td>topcolor:</td><td><input type="text"
/// name="topc" value="ff6600" size="20"></td>`. Note HN shortens the form
/// input's `name` to `topc` — only the user-facing label cell spells it out
/// in full. Anchoring on the label text rather than the short input name
/// keeps the match tied to something visible on the page.
/// Extract the `karma` value from the HTML of a HN user's profile page.
///
/// HN renders karma in a two-cell table row: `<td>karma:</td><td>123</td>`.
/// The number may be surrounded by whitespace and the `<td>` tags may carry
/// extra attributes, so we match loosely. Returns `None` if the pattern
/// isn't present or the number doesn't fit in a `u32`.
fn parse_karma_from_profile(html: &str) -> Option<u32> {
    let rg =
        regex::Regex::new(r"(?is)<td[^>]*>\s*karma:\s*</td>\s*<td[^>]*>\s*(\d+)\s*</td>").ok()?;
    let cap = rg.captures(html)?;
    cap.get(1)?.as_str().parse::<u32>().ok()
}

fn parse_topcolor_from_profile(html: &str) -> Option<String> {
    // Anchor on the visible `topcolor:` label cell, then capture the 6-char
    // hex value of the `<input>` in the following cell. Quoting is optional
    // to tolerate HN's minimal-HTML quirks. Don't key off the input's `name`
    // attribute — HN shortens it to `topc`, which is easy to mistake for a
    // typo and easy for HN to change without warning.
    let rg = regex::Regex::new(
        r#"(?is)topcolor:\s*</td>\s*<td[^>]*>\s*<input\b[^>]*?\bvalue=["']?([0-9a-f]{6})["']?"#,
    )
    .ok()?;
    let cap = rg.captures(html)?;
    Some(cap.get(1)?.as_str().to_ascii_lowercase())
}

/// Read the `showdead` preference from the logged-in user's profile page.
///
/// HN renders the control as a `<select name="showd">` with two `<option>`
/// elements whose inner text is `yes` or `no`; the currently-saved value is
/// marked with a `selected` attribute. We anchor on the short input name
/// (`showd`) to isolate the right `<select>`, then scan its options for the
/// one carrying `selected`. Absence of the field — which happens when we
/// end up on someone else's profile, or when HN tweaks its markup — falls
/// back to `false` since that matches HN's default for new accounts.
fn parse_showdead_from_profile(html: &str) -> bool {
    let select_rg =
        match regex::Regex::new(r#"(?is)<select\b[^>]*\bname=["']?showd["']?[^>]*>(.*?)</select>"#)
        {
            Ok(rg) => rg,
            Err(_) => return false,
        };
    let Some(body) = select_rg.captures(html).and_then(|c| c.get(1)) else {
        return false;
    };
    let option_rg = match regex::Regex::new(
        r#"(?is)<option\b[^>]*\bselected\b[^>]*>\s*([A-Za-z]+)\s*</option>"#,
    ) {
        Ok(rg) => rg,
        Err(_) => return false,
    };
    option_rg
        .captures(body.as_str())
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().eq_ignore_ascii_case("yes"))
        .unwrap_or(false)
}

/// Record the logged-in user's display info for views to read later.
///
/// Must be called at most once during startup, after the login attempt has
/// resolved. Pass `None` when the user isn't logged in; views use that to
/// decide whether to render anything on the right side of the title bar.
pub fn init_user_info(info: Option<UserInfo>) {
    USER_INFO.set(info).unwrap_or_else(|_| {
        panic!("user info has already been initialised");
    });
}

/// Returns the logged-in user's display info, or `None` if there's no
/// authenticated session (either no credentials configured or login failed).
pub fn get_user_info() -> Option<&'static UserInfo> {
    USER_INFO.get().and_then(|opt| opt.as_ref())
}

pub fn init_client() -> &'static HNClient {
    let client = HNClient::new().unwrap();
    install_client(client)
}

/// Seal an already-built [`HNClient`] into the global slot and return a
/// `'static` reference. Used when the client is built before the global
/// config is sealed (e.g. so the startup login request can happen before the
/// HN `topcolor` is applied to the theme).
pub fn install_client(client: HNClient) -> &'static HNClient {
    CLIENT.set(client).unwrap_or_else(|_| {
        panic!("failed to set up the application's HackerNews Client");
    });
    CLIENT.get().unwrap()
}

/// Verify a username/password pair against Hacker News without touching the
/// global client, and return the resulting session cookie so the caller can
/// persist it. Used by the first-run prompt so credentials can be checked
/// before they're written to disk. `Ok(None)` means the login succeeded but
/// we couldn't extract the session cookie — the caller should still save the
/// credentials.
pub fn verify_credentials(username: &str, password: &str) -> Result<Option<String>> {
    let client = HNClient::new()?;
    client.login(username, password)?;
    Ok(client.current_session_cookie())
}

#[cfg(test)]
mod tests {
    use super::{
        classify_login_response, hn_listing_pages_for_tui_page, listing_path_for_view,
        parse_comments_from_content, parse_karma_from_profile, parse_reply_form,
        parse_showdead_from_profile, parse_topcolor_from_profile, parse_vote_data_from_content,
        parse_vouch_data_from_content, StartupLoginStatus, StorySortMode,
    };
    use crate::model::VoteDirection;

    #[test]
    fn listing_path_maps_hn_backed_views() {
        assert_eq!(
            listing_path_for_view("front_page", StorySortMode::None),
            Some("news")
        );
        assert_eq!(
            listing_path_for_view("ask_hn", StorySortMode::None),
            Some("ask")
        );
        assert_eq!(
            listing_path_for_view("show_hn", StorySortMode::None),
            Some("show")
        );
        // `story` by date corresponds to HN's /newest stream, so F2 picks
        // up listing vote state just like the other tag views.
        assert_eq!(
            listing_path_for_view("story", StorySortMode::Date),
            Some("newest")
        );
    }

    #[test]
    fn listing_path_returns_none_for_algolia_only_views() {
        // These views come from Algolia search with arbitrary sort/filter
        // that has no direct HN listing equivalent, so callers fall back
        // to the lazy per-item vote fetch.
        assert_eq!(listing_path_for_view("story", StorySortMode::Points), None);
        assert_eq!(listing_path_for_view("story", StorySortMode::None), None);
        assert_eq!(listing_path_for_view("job", StorySortMode::Date), None);
        assert_eq!(
            listing_path_for_view("custom_whatever", StorySortMode::None),
            None
        );
        assert_eq!(listing_path_for_view("", StorySortMode::None), None);
    }

    #[test]
    fn hn_listing_pages_cover_tui_window_default_size() {
        // Default TUI page size is 20 items; HN listing pages are 30. Every
        // TUI page spans 1 or 2 HN pages — sweeping this range is how
        // pagination beyond page 0 keeps vote arrows visible.
        assert_eq!(hn_listing_pages_for_tui_page(0, 20), (1, 1)); // items 0-19
        assert_eq!(hn_listing_pages_for_tui_page(1, 20), (1, 2)); // items 20-39 straddle HN p1+p2
        assert_eq!(hn_listing_pages_for_tui_page(2, 20), (2, 2)); // items 40-59
        assert_eq!(hn_listing_pages_for_tui_page(3, 20), (3, 3)); // items 60-79
        assert_eq!(hn_listing_pages_for_tui_page(4, 20), (3, 4)); // items 80-99 straddle p3+p4
        assert_eq!(hn_listing_pages_for_tui_page(5, 20), (4, 4)); // items 100-119
        assert_eq!(hn_listing_pages_for_tui_page(6, 20), (5, 5)); // items 120-139
    }

    #[test]
    fn hn_listing_pages_handle_small_page_size() {
        // A TUI page smaller than a HN listing page always fits inside a
        // single HN page (except when it straddles a boundary).
        assert_eq!(hn_listing_pages_for_tui_page(0, 10), (1, 1)); // items 0-9
        assert_eq!(hn_listing_pages_for_tui_page(1, 10), (1, 1)); // items 10-19
        assert_eq!(hn_listing_pages_for_tui_page(2, 10), (1, 1)); // items 20-29
        assert_eq!(hn_listing_pages_for_tui_page(3, 10), (2, 2)); // items 30-39
    }

    #[test]
    fn hn_listing_pages_handle_page_size_equal_to_hn_page() {
        // A TUI page of exactly 30 aligns with HN's own listing pages.
        assert_eq!(hn_listing_pages_for_tui_page(0, 30), (1, 1));
        assert_eq!(hn_listing_pages_for_tui_page(1, 30), (2, 2));
        assert_eq!(hn_listing_pages_for_tui_page(2, 30), (3, 3));
    }

    #[test]
    fn hn_listing_pages_handle_large_page_size() {
        // A TUI page larger than HN's 30-per-page listing must sweep
        // multiple HN pages to collect every row's vote state.
        // page_size=50 on TUI page 0 covers items 0-49, which spans HN p1+p2.
        assert_eq!(hn_listing_pages_for_tui_page(0, 50), (1, 2));
        // TUI page 1 covers items 50-99 -> HN p2+p3+p4 (item 50 in p2, item 99 in p4).
        assert_eq!(hn_listing_pages_for_tui_page(1, 50), (2, 4));
        // page_size=100 on TUI page 0 covers items 0-99 -> HN p1..=p4.
        assert_eq!(hn_listing_pages_for_tui_page(0, 100), (1, 4));
        // page_size=100 on TUI page 1 covers items 100-199 -> HN p4..=p7.
        assert_eq!(hn_listing_pages_for_tui_page(1, 100), (4, 7));
    }

    #[test]
    fn hn_listing_pages_handle_zero_page_size_gracefully() {
        // A page_size of 0 would wrap/divide-by-zero; the helper clamps it
        // to 1 so the caller still gets a sane range rather than a panic.
        assert_eq!(hn_listing_pages_for_tui_page(0, 0), (1, 1));
    }

    #[test]
    fn clamp_page_size_keeps_values_within_bounds() {
        use crate::config::{clamp_page_size, MAX_PAGE_SIZE, MIN_PAGE_SIZE};
        assert_eq!(clamp_page_size(0), MIN_PAGE_SIZE);
        assert_eq!(clamp_page_size(1), MIN_PAGE_SIZE);
        assert_eq!(clamp_page_size(4), MIN_PAGE_SIZE);
        assert_eq!(clamp_page_size(5), 5);
        assert_eq!(clamp_page_size(20), 20);
        assert_eq!(clamp_page_size(100), 100);
        assert_eq!(clamp_page_size(101), MAX_PAGE_SIZE);
        assert_eq!(clamp_page_size(10_000), MAX_PAGE_SIZE);
    }

    #[test]
    fn classify_login_accepts_logged_in_page_with_logout_link() {
        // HN's logged-in nav bar includes a double-quoted logout anchor.
        let body = r#"<html><body><span><a href="logout?auth=abc&amp;goto=news">logout</a></span></body></html>"#;
        assert!(classify_login_response(body).is_ok());
    }

    #[test]
    fn classify_login_rejects_bad_login_body() {
        // Verbatim shape of HN's response for wrong password (non-existent user).
        let body = r#"<html lang="en"><body>Bad login.<br><br><b>Login</b><br><br><form action="login" method="post"></form></body></html>"#;
        let err = classify_login_response(body).unwrap_err().to_string();
        assert!(err.contains("Bad login"), "got: {err}");
    }

    #[test]
    fn classify_login_rejects_captcha_challenge() {
        // HN serves this after repeated failed attempts; no "logout" anywhere.
        let body = r#"<html lang="en"><body>Validation required. If this doesn't work, you can email hn@ycombinator.com<br><div class="g-recaptcha"></div></body></html>"#;
        let err = classify_login_response(body).unwrap_err().to_string();
        assert!(err.contains("captcha"), "got: {err}");
    }

    #[test]
    fn classify_login_rejects_empty_body() {
        assert!(classify_login_response("").is_err());
    }

    #[test]
    fn classify_login_rejects_unexpected_body() {
        // Anything without the success marker and without a known failure
        // marker must still fail — we don't want to accidentally persist
        // credentials on some unrecognised HN response.
        let body = "<html><body>welcome</body></html>";
        assert!(classify_login_response(body).is_err());
    }

    #[test]
    fn parses_unvoted_item_with_downvote_privilege() {
        let html = concat!(
            "<a id='up_1' href='vote?id=1&amp;how=up&amp;auth=aaa111'>x</a>",
            "<a id='down_1' href='vote?id=1&amp;how=down&amp;auth=aaa111'>x</a>",
        );
        let data = parse_vote_data_from_content(html).unwrap();
        let v = data.get("1").expect("expected vote data for id=1");
        assert_eq!(v.vote, None);
        assert!(v.can_downvote);
        assert_eq!(v.auth, "aaa111");
    }

    #[test]
    fn parses_unvoted_item_without_downvote_privilege() {
        let html = "<a id='up_2' href='vote?id=2&amp;how=up&amp;auth=bbb222'>x</a>";
        let data = parse_vote_data_from_content(html).unwrap();
        let v = data.get("2").expect("expected vote data for id=2");
        assert_eq!(v.vote, None);
        assert!(!v.can_downvote);
    }

    #[test]
    fn parses_upvoted_item_with_downvote_privilege() {
        // Real HN shape after upvoting: both arrows stay in the DOM with
        // `class='clicky nosee'` hiding them via CSS, and the `un_` link
        // renders with text `unvote`.
        let html = concat!(
            "<a id='up_3' class='clicky nosee' href='vote?id=3&amp;how=up&amp;auth=ccc333'>x</a>",
            "<a id='down_3' class='clicky nosee' href='vote?id=3&amp;how=down&amp;auth=ccc333'>x</a>",
            "<a id='un_3' class='clicky' href='vote?id=3&amp;how=un&amp;auth=ccc333'>unvote</a>",
        );
        let data = parse_vote_data_from_content(html).unwrap();
        let v = data.get("3").expect("expected vote data for id=3");
        assert_eq!(v.vote, Some(VoteDirection::Up));
        assert!(v.can_downvote);
        assert_eq!(v.auth, "ccc333");
    }

    #[test]
    fn parses_downvoted_item() {
        // Real HN shape after downvoting: identical to upvoted except the
        // `un_` link text reads `undown`. That text is the only signal in
        // the DOM that distinguishes a downvote from an upvote.
        let html = concat!(
            "<a id='up_4' class='clicky nosee' href='vote?id=4&amp;how=up&amp;auth=ddd444'>x</a>",
            "<a id='down_4' class='clicky nosee' href='vote?id=4&amp;how=down&amp;auth=ddd444'>x</a>",
            "<a id='un_4' class='clicky' href='vote?id=4&amp;how=un&amp;auth=ddd444'>undown</a>",
        );
        let data = parse_vote_data_from_content(html).unwrap();
        let v = data.get("4").expect("expected vote data for id=4");
        assert_eq!(v.vote, Some(VoteDirection::Down));
        assert!(v.can_downvote);
    }

    #[test]
    fn parses_upvoted_item_without_downvote_privilege() {
        // User without downvote karma who already upvoted: `up_` stays in
        // the DOM with nosee, no `down_` anchor is rendered at all, and
        // the `un_` link carries the `unvote` text.
        let html = concat!(
            "<a id='up_5' class='clicky nosee' href='vote?id=5&amp;how=up&amp;auth=eee555'>x</a>",
            "<a id='un_5' class='clicky' href='vote?id=5&amp;how=un&amp;auth=eee555'>unvote</a>",
        );
        let data = parse_vote_data_from_content(html).unwrap();
        let v = data.get("5").expect("expected vote data for id=5");
        assert_eq!(v.vote, Some(VoteDirection::Up));
        assert!(!v.can_downvote);
    }

    #[test]
    fn parses_vote_past_unvote_window() {
        // Older votes no longer render an `un_` anchor, but HN still
        // hides both vote arrows with nosee. Without un_ text we can't
        // tell direction, so fall back to an upvote (the common case).
        let html = concat!(
            "<a id='up_6' class='clicky nosee' href='vote?id=6&amp;how=up&amp;auth=fff666'>x</a>",
            "<a id='down_6' class='clicky nosee' href='vote?id=6&amp;how=down&amp;auth=fff666'>x</a>",
        );
        let data = parse_vote_data_from_content(html).unwrap();
        let v = data.get("6").expect("expected vote data for id=6");
        assert_eq!(v.vote, Some(VoteDirection::Up));
        assert!(v.can_downvote);
    }

    #[test]
    fn parses_own_comment_has_no_vote_entry() {
        // HN omits every vote anchor for the logged-in user's own items,
        // so the parser should produce no entry for them.
        let html = concat!(
            "<tr class='athing comtr' id='7'>",
            "<td><span>my own comment, no vote links</span></td>",
            "</tr>",
        );
        let data = parse_vote_data_from_content(html).unwrap();
        assert!(!data.contains_key("7"));
    }

    #[test]
    fn neighbouring_nosee_does_not_leak_across_tags() {
        // Regression guard: an unrelated `nosee`-classed anchor sitting
        // next to a live upvote arrow must not mark the upvote arrow as
        // hidden. The old `.*?` regex could consume characters across
        // tag boundaries and trip this up.
        let html = concat!(
            "<a class='clicky nosee' href='something'>decoy</a>",
            "<a id='up_8' class='clicky' href='vote?id=8&amp;how=up&amp;auth=888888'>x</a>",
            "<a id='down_8' class='clicky' href='vote?id=8&amp;how=down&amp;auth=888888'>x</a>",
        );
        let data = parse_vote_data_from_content(html).unwrap();
        let v = data.get("8").expect("expected vote data for id=8");
        assert_eq!(v.vote, None);
        assert!(v.can_downvote);
    }

    #[test]
    fn ignores_items_without_any_vote_link() {
        let html = "<tr><td>just a comment row with no vote links</td></tr>";
        let data = parse_vote_data_from_content(html).unwrap();
        assert!(data.is_empty());
    }

    #[test]
    fn parses_vouch_link_as_unvouched() {
        // Dead item the viewer can vouch for: inner text `vouch`, href
        // carries `how=up`. Parser should record `vouched=false` so the
        // next keypress fires a fresh vouch.
        let html = concat!(
            "<a class='clicky' id='vouch_100' ",
            "href='vouch?id=100&amp;how=up&amp;auth=abcdef&amp;goto=item%3Fid%3D100'>vouch</a>",
        );
        let data = parse_vouch_data_from_content(html).unwrap();
        let vd = data.get("100").expect("expected vouch data for id=100");
        assert_eq!(vd.auth, "abcdef");
        assert!(!vd.vouched);
    }

    #[test]
    fn parses_unvouch_link_as_already_vouched() {
        // Same item after the viewer vouched: HN flips the inner text to
        // `unvouch` and the `how` query to `un`. Parser should flag it so
        // the next keypress rescinds rather than re-vouches.
        let html = concat!(
            "<a class='clicky' id='vouch_101' ",
            "href='vouch?id=101&amp;how=un&amp;auth=abcdef&amp;goto=item%3Fid%3D101'>unvouch</a>",
        );
        let data = parse_vouch_data_from_content(html).unwrap();
        let vd = data.get("101").expect("expected vouch data for id=101");
        assert!(vd.vouched);
    }

    #[test]
    fn ignores_pages_without_vouch_links() {
        // A non-dead item, or one the viewer lacks privilege to vouch on,
        // has no `vouch_<id>` anchor at all — the map stays empty.
        let html = concat!(
            "<a id='up_5' class='clicky' href='vote?id=5&amp;how=up&amp;auth=555555'>▲</a>",
            "<a id='down_5' class='clicky' href='vote?id=5&amp;how=down&amp;auth=555555'>▽</a>",
        );
        let data = parse_vouch_data_from_content(html).unwrap();
        assert!(data.is_empty());
    }

    #[test]
    fn skips_vouch_anchors_missing_auth_token() {
        // Without an auth token we can't fire the request, so a malformed
        // entry should be dropped rather than surfaced with an empty auth.
        let html = "<a id='vouch_102' href='vouch?id=102&amp;how=up'>vouch</a>";
        let data = parse_vouch_data_from_content(html).unwrap();
        assert!(!data.contains_key("102"));
    }

    #[test]
    fn extracts_topcolor_from_real_hn_profile_shape() {
        // Literal slice of `news.ycombinator.com/user?id=<self>` — the input
        // name is `topc`, not `topcolor`. Only the preceding label cell
        // spells the word out in full.
        let html = r#"<tr><td valign="top">topcolor:</td><td><input type="text" name="topc" value="33cc33" size="20"></td></tr>"#;
        assert_eq!(parse_topcolor_from_profile(html).as_deref(), Some("33cc33"));
    }

    #[test]
    fn extracts_topcolor_without_quotes() {
        let html = "<td>topcolor:</td><td><input type=text name=topc size=20 value=336699></td>";
        assert_eq!(parse_topcolor_from_profile(html).as_deref(), Some("336699"));
    }

    #[test]
    fn normalises_uppercase_to_lowercase() {
        let html = r#"<td>topcolor:</td><td><input name="topc" value="FF6600"></td>"#;
        assert_eq!(parse_topcolor_from_profile(html).as_deref(), Some("ff6600"));
    }

    #[test]
    fn returns_none_when_profile_has_no_topcolor_field() {
        // Someone else's profile (not your own) doesn't render the edit form.
        let html = r#"<tr><td>user:</td><td>pg</td></tr><tr><td>karma:</td><td>12345</td></tr>"#;
        assert_eq!(parse_topcolor_from_profile(html), None);
    }

    #[test]
    fn returns_none_when_value_is_malformed() {
        let html = r#"<td>topcolor:</td><td><input name="topc" value="not-a-color"></td>"#;
        assert_eq!(parse_topcolor_from_profile(html), None);
    }

    #[test]
    fn parses_karma_from_profile_row() {
        // Shape of HN's profile page: two `<td>` cells, the value optionally
        // padded with whitespace.
        let html = r#"<tr><td>user:</td><td>pg</td></tr><tr><td>karma:</td><td> 12345 </td></tr>"#;
        assert_eq!(parse_karma_from_profile(html), Some(12345));
    }

    #[test]
    fn parses_karma_with_attribute_on_td() {
        // HN sometimes renders `<td valign=top>` or similar — match loosely.
        let html = r#"<tr><td valign="top">karma:</td><td>67</td></tr>"#;
        assert_eq!(parse_karma_from_profile(html), Some(67));
    }

    #[test]
    fn returns_none_when_karma_missing() {
        let html = r#"<tr><td>user:</td><td>pg</td></tr>"#;
        assert_eq!(parse_karma_from_profile(html), None);
    }

    #[test]
    fn parses_showdead_yes_when_selected() {
        // Verbatim shape of the HN profile edit form when the user has the
        // preference switched on — note the short `name="showd"` and the
        // `selected` flag on the `yes` option.
        let html = r#"<tr><td>showdead:</td><td>    <select name="showd">
      <option>no</option>
      <option selected="selected">yes</option>
    </select>
  </td></tr>"#;
        assert!(parse_showdead_from_profile(html));
    }

    #[test]
    fn parses_showdead_no_when_selected() {
        let html = r#"<tr><td>showdead:</td><td><select name="showd">
      <option selected="selected">no</option>
      <option>yes</option>
    </select></td></tr>"#;
        assert!(!parse_showdead_from_profile(html));
    }

    #[test]
    fn parses_showdead_with_bare_selected_attribute() {
        // Some HN pages render `selected` without a value.
        let html = r#"<select name=showd>
      <option>no</option>
      <option selected>yes</option>
    </select>"#;
        assert!(parse_showdead_from_profile(html));
    }

    #[test]
    fn showdead_defaults_false_when_select_missing() {
        // Someone else's profile (no edit form) or an HTML-shape drift both
        // land here. `false` matches HN's default for new accounts.
        let html = r#"<tr><td>user:</td><td>pg</td></tr>"#;
        assert!(!parse_showdead_from_profile(html));
    }

    #[test]
    fn showdead_defaults_false_when_no_option_selected() {
        let html = r#"<select name="showd"><option>no</option><option>yes</option></select>"#;
        assert!(!parse_showdead_from_profile(html));
    }

    /// Build a minimal comhead row with whatever status token string appears
    /// between the `unv_<id>` span and the `navs` span — keeps the dead /
    /// flagged tests focused on the tokens rather than the surrounding tags.
    fn comment_row_with_status(id: u32, author: &str, status: &str) -> String {
        format!(
            concat!(
                r#"<tr class="athing comtr" id="{id}">"#,
                r#"<td><table><tr>"#,
                r#"<td class="ind" indent="0"></td>"#,
                r#"<td class="default"><div><span class="comhead">"#,
                r#"<a href="user?id={author}" class="hnuser">{author}</a> "#,
                r#"<span class="age" title="2025-01-01T00:00:00 1735689600">on Jan 1, 2025</span> "#,
                r#"<span id="unv_{id}"></span>{status}"#,
                r#"<span class="navs">"#,
                r#"</span></span></div>"#,
                r#"<div class="commtext c00">body</div>"#,
                r#"</td></tr></table></td>"#,
                r#"</tr>"#,
            ),
            id = id,
            author = author,
            status = status,
        )
    }

    #[test]
    fn parses_dead_and_flagged_comment_row_and_sets_both_flags() {
        // With `?showdead=yes` HN keeps the row but tags the comhead with
        // literal ` [flagged] ` / ` [dead] ` tokens between the empty
        // `unv_<id>` span and the `navs` span. A comment can carry either,
        // both, or neither.
        let html = comment_row_with_status(99, "deaduser", " [flagged]  [dead] ");
        let comments = parse_comments_from_content(&html);
        assert_eq!(comments.len(), 1);
        assert_eq!(comments[0].id, 99);
        assert_eq!(comments[0].author, "deaduser");
        assert!(comments[0].dead);
        assert!(comments[0].flagged);
    }

    #[test]
    fn parses_flagged_only_comment_row() {
        // A comment can be flagged without being dead — enough user flags
        // to carry a `[flagged]` badge but not enough (or not moderator-
        // killed) to go dead.
        let html = comment_row_with_status(101, "flaggeduser", " [flagged] ");
        let comments = parse_comments_from_content(&html);
        assert_eq!(comments.len(), 1);
        assert!(comments[0].flagged);
        assert!(!comments[0].dead);
    }

    #[test]
    fn parses_dead_only_comment_row() {
        let html = comment_row_with_status(102, "deaduser", " [dead] ");
        let comments = parse_comments_from_content(&html);
        assert_eq!(comments.len(), 1);
        assert!(comments[0].dead);
        assert!(!comments[0].flagged);
    }

    #[test]
    fn parses_live_comment_row_leaves_status_flags_unset() {
        let html = comment_row_with_status(100, "liveuser", " ");
        let comments = parse_comments_from_content(&html);
        assert_eq!(comments.len(), 1);
        assert!(!comments[0].dead);
        assert!(!comments[0].flagged);
    }

    #[test]
    fn classifies_bad_login_error_from_classify_response() {
        let err = classify_login_response(r#"<html lang="en"><body>Bad login.<br></body></html>"#)
            .unwrap_err();
        assert!(matches!(
            StartupLoginStatus::from_login_error(&err),
            StartupLoginStatus::BadLogin
        ));
    }

    #[test]
    fn classifies_captcha_error_from_classify_response() {
        let err = classify_login_response(
            r#"<html lang="en"><body>Validation required.<div class="g-recaptcha"></div></body></html>"#,
        )
        .unwrap_err();
        assert!(matches!(
            StartupLoginStatus::from_login_error(&err),
            StartupLoginStatus::Captcha
        ));
    }

    #[test]
    fn classifies_unknown_error_as_other() {
        let err = anyhow::anyhow!("connection reset");
        match StartupLoginStatus::from_login_error(&err) {
            StartupLoginStatus::Other(msg) => assert!(msg.contains("connection reset")),
            other => panic!("expected Other(..), got {other:?}"),
        }
    }

    #[test]
    fn returns_none_when_karma_not_a_number() {
        let html = r#"<tr><td>karma:</td><td>abc</td></tr>"#;
        assert_eq!(parse_karma_from_profile(html), None);
    }

    // Captured from a real /reply?id=X response. If HN ever drifts its
    // form markup this regression trips; refresh the fixture rather than
    // loosening the assertion.
    const REPLY_FORM_HTML: &str = include_str!("../../tests/fixtures/reply_form.html");

    #[test]
    fn parse_reply_form_extracts_hmac_from_real_sample() {
        let hmac = parse_reply_form(REPLY_FORM_HTML)
            .expect("hmac should be present in the captured reply form");
        assert!(!hmac.is_empty());
        assert!(
            hmac.chars().all(|c| c.is_ascii_hexdigit()),
            "hmac should be lowercase hex; got {hmac:?}"
        );
    }

    #[test]
    fn parse_reply_form_returns_none_without_hmac() {
        assert!(parse_reply_form("<html><body>no form here</body></html>").is_none());
    }

    // Captured from a real item page. Covers 26 comments across 6 indent
    // levels, so the fixture exercises the n_children walk and the
    // depth-first ordering at once. Refresh the fixture rather than
    // loosening these assertions if HN's markup ever drifts.
    const COMMENT_PAGE_HTML: &str = include_str!("../../tests/fixtures/comment_page.html");

    #[test]
    fn parse_comments_extracts_all_rows_in_document_order() {
        let comments = parse_comments_from_content(COMMENT_PAGE_HTML);
        assert_eq!(comments.len(), 26);

        // Top-level anchor: only indent=0 row in the fixture.
        let first = &comments[0];
        assert_eq!(first.id, 43500020);
        assert_eq!(first.level, 0);
        assert_eq!(first.author, "mechagodzilla");
        assert_eq!(first.time, 1743122540);
        assert!(first.content.starts_with("None of that means"));

        // Second row is the first reply — indent=1.
        assert_eq!(comments[1].id, 43500174);
        assert_eq!(comments[1].level, 1);
    }

    #[test]
    fn parse_comments_reaches_deepest_indent() {
        let comments = parse_comments_from_content(COMMENT_PAGE_HTML);
        let deepest = comments.iter().max_by_key(|c| c.level).unwrap();
        assert_eq!(deepest.level, 5);
        assert_eq!(deepest.id, 43501529);
        assert_eq!(deepest.author, "brookst");
    }

    #[test]
    fn parse_comments_counts_descendants_via_indent_walk() {
        let comments = parse_comments_from_content(COMMENT_PAGE_HTML);

        // Only one indent=0 comment; every other row is its descendant.
        assert_eq!(comments[0].n_children, 25);

        // Leaf rows have no descendants — sanity check a handful.
        let leaves: Vec<_> = comments.iter().filter(|c| c.n_children == 0).collect();
        assert!(leaves.len() >= 10);

        // Mid-tree: id=43500139 has three descendants (parent + two grandchildren).
        let mid = comments.iter().find(|c| c.id == 43500139).unwrap();
        assert_eq!(mid.level, 2);
        assert_eq!(mid.n_children, 4);
    }

    #[test]
    fn parse_comments_decodes_html_entities_in_content() {
        let comments = parse_comments_from_content(COMMENT_PAGE_HTML);
        let with_entity = comments.iter().find(|c| c.id == 43500174).unwrap();
        // Fixture has `Dot-com boom&#x2F;bubble ...`; the parser should
        // decode the `&#x2F;` slash so downstream styling sees real text.
        assert!(with_entity.content.starts_with("Dot-com boom/bubble"));
    }

    #[test]
    fn parse_comments_skips_rows_without_required_fields() {
        // A row shaped like a deleted comment: no hnuser link and no
        // commtext div. The Algolia branch also drops these.
        let html = concat!(
            r#"<tr class="athing comtr" id="1">"#,
            r#"<td class="ind" indent="0"></td>"#,
            r#"<span class="age" title="2025-01-01T00:00:00 1735689600"></span>"#,
            r#"</tr>"#,
        );
        assert!(parse_comments_from_content(html).is_empty());
    }

    #[test]
    fn parse_comments_returns_empty_when_page_has_no_comments() {
        assert!(parse_comments_from_content("<html><body></body></html>").is_empty());
    }

    // Captured from an authenticated session on `/item?id=47882645`. Covers
    // (a) the root story, upvoted with no downvote arrow; (b) a recently
    // upvoted comment (47889861) whose `un_` link carries the `unvote`
    // text; (c) older upvoted comments past the unvote window whose only
    // cue is both arrows carrying `nosee`; (d) an unvoted not-own comment
    // (47890764); (e) the user's own comment (47891300) with no vote
    // links at all. Auth tokens have been redacted; refresh the fixture
    // rather than loosening these assertions if HN's markup drifts.
    const ITEM_PAGE_AUTHENTICATED_HTML: &str =
        include_str!("../../tests/fixtures/item_page_authenticated.html");

    // Captured from an authenticated session on `/item?id=47889387` — a
    // single comment the user downvoted. The `un_` anchor's `undown`
    // text is the only cue in the DOM for the direction.
    const DOWNVOTED_COMMENT_PAGE_HTML: &str =
        include_str!("../../tests/fixtures/downvoted_comment_page.html");

    #[test]
    fn fixture_marks_recently_upvoted_comment_as_upvote() {
        let data = parse_vote_data_from_content(ITEM_PAGE_AUTHENTICATED_HTML).unwrap();
        let v = data
            .get("47889861")
            .expect("upvoted comment should have vote data");
        assert_eq!(v.vote, Some(VoteDirection::Up));
        assert!(v.can_downvote);
    }

    #[test]
    fn fixture_marks_old_upvoted_comments_past_unvote_window() {
        // These ids appear on the page with both arrows nosee'd but no
        // `un_` anchor — HN's shape for votes past the unvote window.
        // Direction can't be recovered from the DOM, so the parser falls
        // back to `Up`.
        let data = parse_vote_data_from_content(ITEM_PAGE_AUTHENTICATED_HTML).unwrap();
        for id in ["47885819", "47886877", "47887119", "47887934"] {
            let v = data
                .get(id)
                .unwrap_or_else(|| panic!("expected vote data for id={id}"));
            assert_eq!(
                v.vote,
                Some(VoteDirection::Up),
                "id={id} should parse as upvoted"
            );
            assert!(v.can_downvote, "id={id} should retain downvote privilege");
        }
    }

    #[test]
    fn fixture_leaves_unvoted_not_own_comment_alone() {
        let data = parse_vote_data_from_content(ITEM_PAGE_AUTHENTICATED_HTML).unwrap();
        let v = data
            .get("47890764")
            .expect("unvoted comment should still have vote data");
        assert_eq!(v.vote, None);
        assert!(v.can_downvote);
    }

    #[test]
    fn fixture_skips_own_comment_entirely() {
        // HN omits every vote anchor for the logged-in user's own items,
        // so there should be no entry at all for 47891300.
        let data = parse_vote_data_from_content(ITEM_PAGE_AUTHENTICATED_HTML).unwrap();
        assert!(
            !data.contains_key("47891300"),
            "own comment should not appear in vote state"
        );
    }

    #[test]
    fn fixture_marks_downvoted_comment_as_down() {
        let data = parse_vote_data_from_content(DOWNVOTED_COMMENT_PAGE_HTML).unwrap();
        let v = data
            .get("47889387")
            .expect("downvoted comment should have vote data");
        assert_eq!(v.vote, Some(VoteDirection::Down));
        assert!(v.can_downvote);
    }

    #[test]
    fn fixture_sets_status_flags_from_comhead_tokens() {
        // The authenticated fixture was captured with `showdead=yes`, so a
        // handful of its rows carry ` [flagged] ` / ` [dead] ` tokens in the
        // comhead. Spot-check a few combinations so regressions trip.
        let comments = parse_comments_from_content(ITEM_PAGE_AUTHENTICATED_HTML);
        let find = |id: u32| {
            comments
                .iter()
                .find(|c| c.id == id)
                .unwrap_or_else(|| panic!("comment {id} should appear in tree"))
        };
        // 47891323: bare `[dead]`, no `[flagged]`.
        let dead_only = find(47891323);
        assert!(dead_only.dead);
        assert!(!dead_only.flagged);
        // 47888617: both `[flagged]` and `[dead]`.
        let both = find(47888617);
        assert!(both.dead);
        assert!(both.flagged);
        // 47885819: neither.
        let live = find(47885819);
        assert!(!live.dead);
        assert!(!live.flagged);
    }
}
