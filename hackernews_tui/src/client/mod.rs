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
pub const STORY_LIMIT: usize = 20;
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
/// render the username on its own in that case.
#[derive(Debug, Clone)]
pub struct UserInfo {
    pub username: String,
    pub karma: Option<u32>,
}

/// Parsed fields from a HN user profile page. Both are optional so the parser
/// can return a useful result even when HN tweaks its markup.
#[derive(Debug, Default, Clone)]
pub struct ProfileInfo {
    pub topcolor: Option<String>,
    pub karma: Option<u32>,
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
            }
            .into(),
            "comment" => Comment {
                id: item_id,
                level: 0,
                n_children: 0,
                author: item.by.unwrap_or_default(),
                time: item.time,
                content: text,
            }
            .into(),
            typ => {
                anyhow::bail!("unknown item type: {typ}");
            }
        };

        // Parallelize two tasks using [`rayon::join`](https://docs.rs/rayon/latest/rayon/fn.join.html)
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
        let vote_state = vote_state?;
        let comment_receiver = comment_receiver?;

        Ok(PageData {
            title,
            url,
            root_item,
            comment_receiver,
            vote_state,
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

        let start_id = STORY_LIMIT * page;
        if start_id >= stories.len() {
            return Ok(vec![]);
        }

        let end_id = std::cmp::min(start_id + STORY_LIMIT, stories.len());
        let ids = &stories[start_id..end_id];

        let request_url = format!(
            "{}/search?tags=story,({}){}&hitsPerPage={}",
            HN_ALGOLIA_PREFIX,
            ids.iter().fold("".to_owned(), |tags, story_id| format!(
                "{tags}story_{story_id},"
            )),
            numeric_filters.query(),
            STORY_LIMIT,
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
            STORY_LIMIT,
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

        let mut content = self
            .client
            .get(&format!("{HN_HOST_URL}/item?id={item_id}"))
            .call()?
            .into_string()?;

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
    let upvote_rg = regex::Regex::new("<a.*?id='up_(?P<id>.*?)'.*?auth=(?P<auth>[0-9a-z]*).*?>")?;
    let downvote_rg =
        regex::Regex::new("<a.*?id='down_(?P<id>.*?)'.*?auth=(?P<auth>[0-9a-z]*).*?>")?;
    let unvote_rg = regex::Regex::new("<a.*?id='un_(?P<id>.*?)'.*?auth=(?P<auth>[0-9a-z]*).*?>")?;

    #[derive(Default)]
    struct Flags {
        has_up: bool,
        has_down: bool,
        has_un: bool,
        auth: String,
    }

    let mut flags: HashMap<String, Flags> = HashMap::new();
    let mut record = |rg: &regex::Regex, mark: fn(&mut Flags)| {
        for c in rg.captures_iter(page_content) {
            let id = c.name("id").unwrap().as_str().to_owned();
            let auth = c.name("auth").unwrap().as_str().to_owned();
            let entry = flags.entry(id).or_default();
            mark(entry);
            if !auth.is_empty() {
                entry.auth = auth;
            }
        }
    };
    record(&upvote_rg, |f| f.has_up = true);
    record(&downvote_rg, |f| f.has_down = true);
    record(&unvote_rg, |f| f.has_un = true);

    let hm = flags
        .into_iter()
        .map(|(id, f)| {
            let (vote, can_downvote) = match (f.has_up, f.has_down, f.has_un) {
                // Not voted, both arrows rendered → can upvote or downvote.
                (true, true, false) => (None, true),
                // Not voted, only upvote arrow → no downvote privilege.
                (true, false, false) => (None, false),
                // Voted, downvote arrow still rendered → upvoted (up arrow
                // was consumed by the `un` link).
                (false, true, true) => (Some(VoteDirection::Up), true),
                // Voted, upvote arrow still rendered → downvoted.
                (true, false, true) => (Some(VoteDirection::Down), true),
                // Voted, no arrows left → user lacks downvote privilege, so
                // the vote must be an upvote.
                (false, false, true) => (Some(VoteDirection::Up), false),
                // Any other combination is unexpected; fall back to the
                // conservative "not voted" reading.
                _ => (None, f.has_down),
            };
            (
                id,
                VoteData {
                    auth: f.auth,
                    vote,
                    can_downvote,
                },
            )
        })
        .collect();

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
        classify_login_response, parse_karma_from_profile, parse_topcolor_from_profile,
        parse_vote_data_from_content, StartupLoginStatus,
    };
    use crate::model::VoteDirection;

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
        // After upvoting, HN removes the `up_<id>` link, leaves `down_<id>`,
        // and adds the `un_<id>` rescind link.
        let html = concat!(
            "<a id='down_3' href='vote?id=3&amp;how=down&amp;auth=ccc333'>x</a>",
            "<a id='un_3' href='vote?id=3&amp;how=un&amp;auth=ccc333'>unvote</a>",
        );
        let data = parse_vote_data_from_content(html).unwrap();
        let v = data.get("3").expect("expected vote data for id=3");
        assert_eq!(v.vote, Some(VoteDirection::Up));
        assert!(v.can_downvote);
    }

    #[test]
    fn parses_downvoted_item() {
        // Mirror of the upvoted case: the `down_<id>` arrow was replaced by
        // the `un_<id>` link while `up_<id>` is still available.
        let html = concat!(
            "<a id='up_4' href='vote?id=4&amp;how=up&amp;auth=ddd444'>x</a>",
            "<a id='un_4' href='vote?id=4&amp;how=un&amp;auth=ddd444'>unvote</a>",
        );
        let data = parse_vote_data_from_content(html).unwrap();
        let v = data.get("4").expect("expected vote data for id=4");
        assert_eq!(v.vote, Some(VoteDirection::Down));
        assert!(v.can_downvote);
    }

    #[test]
    fn parses_upvoted_item_without_downvote_privilege() {
        // User without downvote karma who already upvoted: only `un_<id>`.
        let html = "<a id='un_5' href='vote?id=5&amp;how=un&amp;auth=eee555'>unvote</a>";
        let data = parse_vote_data_from_content(html).unwrap();
        let v = data.get("5").expect("expected vote data for id=5");
        assert_eq!(v.vote, Some(VoteDirection::Up));
        assert!(!v.can_downvote);
    }

    #[test]
    fn ignores_items_without_any_vote_link() {
        let html = "<tr><td>just a comment row with no vote links</td></tr>";
        let data = parse_vote_data_from_content(html).unwrap();
        assert!(data.is_empty());
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
}
