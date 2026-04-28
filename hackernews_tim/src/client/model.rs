use crate::utils::decode_html;

use super::*;
use serde::{de, Deserialize, Deserializer};

fn parse_id<'de, D>(d: D) -> std::result::Result<u32, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(d)?;
    s.parse::<u32>().map_err(de::Error::custom)
}

fn parse_null_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    T: Default + Deserialize<'de>,
    D: Deserializer<'de>,
{
    let opt = Option::deserialize(deserializer)?;
    Ok(opt.unwrap_or_default())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all(deserialize = "camelCase"))]
struct MatchResult {
    value: String,
}

#[derive(Debug, Deserialize)]
struct HighlightResultResponse {
    title: Option<MatchResult>,
}

#[derive(Debug, Deserialize)]
/// StoryResponse represents the story data received from HN_ALGOLIA APIs
pub struct StoryResponse {
    #[serde(default)]
    #[serde(rename(deserialize = "objectID"))]
    #[serde(deserialize_with = "parse_id")]
    id: u32,

    author: Option<String>,
    url: Option<String>,
    #[serde(rename(deserialize = "story_text"))]
    text: Option<String>,

    #[serde(default)]
    #[serde(deserialize_with = "parse_null_default")]
    points: u32,
    #[serde(default)]
    #[serde(deserialize_with = "parse_null_default")]
    num_comments: usize,

    #[serde(rename(deserialize = "created_at_i"))]
    time: u64,

    // search result
    #[serde(rename(deserialize = "_highlightResult"))]
    highlight_result: Option<HighlightResultResponse>,

    #[serde(default)]
    #[serde(deserialize_with = "parse_null_default")]
    dead: bool,

    #[serde(default)]
    #[serde(deserialize_with = "parse_null_default")]
    flagged: bool,
}

#[derive(Debug, Deserialize)]
/// ItemResponse represents the item data received from the official HackerNews APIs
pub struct ItemResponse {
    pub by: Option<String>,
    pub text: Option<String>,
    pub title: Option<String>,
    pub url: Option<String>,

    #[serde(rename(deserialize = "type"))]
    pub typ: String,

    pub descendants: Option<usize>,
    pub score: Option<u32>,
    pub time: u64,

    #[serde(default)]
    pub kids: Vec<u32>,

    #[serde(default)]
    pub dead: bool,

    #[serde(default)]
    pub flagged: bool,
}

#[derive(Debug, Deserialize)]
/// CommentResponse represents the comment data received from HN_ALGOLIA APIs
pub struct CommentResponse {
    id: u32,

    #[serde(default)]
    children: Vec<CommentResponse>,

    text: Option<String>,
    author: Option<String>,

    #[serde(rename(deserialize = "created_at_i"))]
    time: u64,

    #[serde(default)]
    #[serde(deserialize_with = "parse_null_default")]
    dead: bool,

    #[serde(default)]
    #[serde(deserialize_with = "parse_null_default")]
    flagged: bool,
}

#[derive(Debug, Deserialize)]
/// StoriesResponse represents the stories data received from HN_ALGOLIA APIs
pub struct StoriesResponse {
    pub hits: Vec<StoryResponse>,
}

impl From<StoriesResponse> for Vec<Story> {
    fn from(s: StoriesResponse) -> Vec<Story> {
        s.hits
            .into_par_iter()
            .filter(|story| story.highlight_result.is_some())
            .map(|story| story.into())
            .collect()
    }
}

impl From<StoryResponse> for Story {
    fn from(s: StoryResponse) -> Self {
        let title = s
            .highlight_result
            .unwrap()
            .title
            .map(|r| r.value)
            .unwrap_or_default();
        let title = decode_html(&title);

        let content = decode_html(&s.text.unwrap_or_default());

        Story {
            url: s.url.unwrap_or_default(),
            author: s.author.unwrap_or_default(),
            id: s.id,
            points: s.points,
            num_comments: s.num_comments,
            time: s.time,
            title,
            content,
            dead: s.dead,
            flagged: s.flagged,
        }
    }
}

impl From<CommentResponse> for Vec<Comment> {
    fn from(c: CommentResponse) -> Self {
        // recursively parse child comments of the current comment
        let children = c
            .children
            .into_par_iter()
            .filter(|comment| comment.author.is_some() && comment.text.is_some())
            .flat_map(<Vec<Comment>>::from)
            .map(|mut c| {
                c.level += 1; // update the level of every child comment
                c
            })
            .collect::<Vec<_>>();

        // parse current comment
        let comment = {
            Comment {
                id: c.id,
                level: 0,
                n_children: children.len(),
                time: c.time,
                author: c.author.unwrap_or_default(),
                content: decode_html(&c.text.unwrap_or_default()),
                dead: c.dead,
                flagged: c.flagged,
                points: None,
                parent_story_url: None,
            }
        };

        [vec![comment], children].concat()
    }
}

/// One hit returned by HN Algolia's `search_by_date?tags=comment,author_<u>`
/// listing — a comment owned by a specific author. Carries the parent
/// story's id/title so the threads view can render a "re: …" header that
/// links back to the discussion.
#[derive(Debug, Deserialize)]
pub struct UserCommentResponse {
    #[serde(rename(deserialize = "objectID"))]
    #[serde(deserialize_with = "parse_id")]
    id: u32,

    author: Option<String>,
    comment_text: Option<String>,

    #[serde(rename(deserialize = "created_at_i"))]
    time: u64,

    story_id: Option<u32>,
    story_title: Option<String>,

    #[serde(default)]
    #[serde(deserialize_with = "parse_null_default")]
    dead: bool,

    #[serde(default)]
    #[serde(deserialize_with = "parse_null_default")]
    flagged: bool,
}

#[derive(Debug, Deserialize)]
pub struct UserCommentsResponse {
    pub hits: Vec<UserCommentResponse>,
}

impl UserCommentResponse {
    pub fn id(&self) -> u32 {
        self.id
    }

    /// Parent thread URL on news.ycombinator.com, propagated to every
    /// comment in the threads-view subtree (root + replies) so a bare
    /// `o`/`O` from any focused item jumps back to the parent thread.
    /// Returns `None` when the parent story id isn't known.
    pub fn story_url(&self) -> Option<String> {
        self.story_id
            .map(|sid| format!("{}/item?id={sid}", super::HN_HOST_URL))
    }

    /// HTML snippet linking back to the parent thread, used as a header
    /// prefix on the user's own comment in the threads view. The link is
    /// plain HTML so it flows through `parse_hn_html_text` and ends up
    /// in the `CommentView`'s link dialog (default `o`/`O`). Returns an
    /// empty string when the parent story id isn't known.
    pub fn story_header_html(&self) -> String {
        let Some(sid) = self.story_id else {
            return String::new();
        };
        let title = self
            .story_title
            .as_deref()
            .map(html_escape::encode_text)
            .map(|s| s.into_owned())
            .unwrap_or_else(|| "parent thread".to_string());
        format!(
            "<p><i>re: <a href=\"{}/item?id={sid}\">{title}</a></i></p>",
            super::HN_HOST_URL
        )
    }

    /// Convert the hit into a single level-0 [`Comment`] without fetching
    /// its replies. Used as a fallback when the per-comment subtree fetch
    /// fails — we still want the user to see their own comment, just
    /// without the reply tree.
    pub fn into_root_comment(self) -> Option<Comment> {
        let parent_story_url = self.story_url();
        let header = self.story_header_html();
        let author = self.author?;
        let text = self.comment_text?;
        let content = format!("{header}{text}");
        Some(Comment {
            id: self.id,
            level: 0,
            n_children: 0,
            time: self.time,
            author,
            content: decode_html(&content),
            dead: self.dead,
            flagged: self.flagged,
            points: None,
            parent_story_url,
        })
    }
}

impl From<UserCommentsResponse> for Vec<Comment> {
    fn from(r: UserCommentsResponse) -> Self {
        r.hits
            .into_iter()
            .filter_map(UserCommentResponse::into_root_comment)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_comments_response_parses_and_prepends_story_header() {
        let json = r#"{
            "hits": [
                {
                    "objectID": "12345",
                    "author": "freedomben",
                    "comment_text": "the body",
                    "created_at_i": 100,
                    "story_id": 9999,
                    "story_title": "A <em>great</em> story"
                }
            ]
        }"#;
        let parsed: UserCommentsResponse = serde_json::from_str(json).unwrap();
        let comments: Vec<Comment> = parsed.into();
        assert_eq!(comments.len(), 1);
        let c = &comments[0];
        assert_eq!(c.id, 12345);
        assert_eq!(c.author, "freedomben");
        assert_eq!(c.level, 0);
        // Header links to the parent story and the body is preserved.
        assert!(c.content.contains("/item?id=9999"));
        assert!(c.content.contains("the body"));
    }

    #[test]
    fn user_comments_response_drops_hits_missing_required_fields() {
        let json = r#"{
            "hits": [
                { "objectID": "1", "author": null,    "comment_text": "x", "created_at_i": 1 },
                { "objectID": "2", "author": "alice", "comment_text": null, "created_at_i": 2 }
            ]
        }"#;
        let parsed: UserCommentsResponse = serde_json::from_str(json).unwrap();
        let comments: Vec<Comment> = parsed.into();
        assert!(comments.is_empty());
    }

    #[test]
    fn user_comment_response_story_url_uses_story_id() {
        let json = r#"{
            "hits": [
                {
                    "objectID": "1",
                    "author": "alice",
                    "comment_text": "x",
                    "created_at_i": 1,
                    "story_id": 4242,
                    "story_title": "t"
                }
            ]
        }"#;
        let parsed: UserCommentsResponse = serde_json::from_str(json).unwrap();
        let hit = &parsed.hits[0];
        assert_eq!(
            hit.story_url(),
            Some(format!("{}/item?id=4242", super::HN_HOST_URL))
        );
    }

    #[test]
    fn user_comment_response_story_url_none_when_story_id_missing() {
        let json = r#"{
            "hits": [
                {
                    "objectID": "1",
                    "author": "alice",
                    "comment_text": "x",
                    "created_at_i": 1
                }
            ]
        }"#;
        let parsed: UserCommentsResponse = serde_json::from_str(json).unwrap();
        let hit = &parsed.hits[0];
        assert!(hit.story_url().is_none());
    }

    #[test]
    fn into_root_comment_populates_parent_story_url() {
        let json = r#"{
            "hits": [
                {
                    "objectID": "7",
                    "author": "alice",
                    "comment_text": "x",
                    "created_at_i": 1,
                    "story_id": 4242,
                    "story_title": "t"
                }
            ]
        }"#;
        let mut parsed: UserCommentsResponse = serde_json::from_str(json).unwrap();
        let hit = parsed.hits.remove(0);
        let expected = hit.story_url();
        let comment = hit.into_root_comment().unwrap();
        assert_eq!(comment.parent_story_url, expected);
    }
}
