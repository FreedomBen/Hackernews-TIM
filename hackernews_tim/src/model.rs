use once_cell::sync::Lazy;
use regex::Regex;
use serde::Deserialize;

use crate::parser::parse_hn_html_text;
use crate::prelude::*;
use crate::utils;

use std::{borrow::Cow, collections::HashMap};

pub type CommentSender = crossbeam_channel::Sender<Vec<Comment>>;
pub type CommentReceiver = crossbeam_channel::Receiver<Vec<Comment>>;

/// a regex that matches a search match in the response from HN Algolia search API
static MATCH_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"<em>(?P<match>.*?)</em>").unwrap());

#[derive(Debug, Clone)]
pub struct Story {
    pub id: u32,
    pub url: String,
    pub author: String,
    pub points: u32,
    pub num_comments: usize,
    pub time: u64,
    pub title: String,
    pub content: String,
    /// True when HN flagged this story as "dead". Only appears when the
    /// viewer has `showdead=yes` set in their HN profile.
    pub dead: bool,
    /// True when HN shows a `[flagged]` badge on the byline (enough
    /// user flags to drop the item's rank, which may or may not also
    /// be dead).
    pub flagged: bool,
}

#[derive(Debug, Clone)]
pub struct Comment {
    pub id: u32,
    pub level: usize,
    pub n_children: usize,
    pub author: String,
    pub time: u64,
    pub content: String,
    /// True when HN flagged this comment as "dead". Only appears when the
    /// viewer has `showdead=yes` set in their HN profile.
    pub dead: bool,
    /// True when HN shows a `[flagged]` badge on the byline (enough
    /// user flags to drop the item's rank, which may or may not also
    /// be dead).
    pub flagged: bool,
    /// Score in HN points. HN only renders `<span class="score">` on the
    /// logged-in viewer's own comments, so this is `Some` only for own
    /// comments fetched via the authenticated HTML path; the Algolia and
    /// Firebase paths always leave it `None`.
    pub points: Option<u32>,
    /// Parent story URL on news.ycombinator.com. Set by the threads view
    /// on every comment in each user-comment subtree (root comment plus
    /// replies) so a bare `o`/`O` from any focused item jumps back to the
    /// parent thread. `None` outside the threads view.
    pub parent_story_url: Option<String>,
}

/// A Hacker News page data.
///
/// The page data is mainly used to construct a comment view.
pub struct PageData {
    pub title: String,
    pub url: String,

    /// the root item in the page
    pub root_item: HnItem,

    /// a channel to lazily load items/comments in the page
    pub comment_receiver: CommentReceiver,
    /// the voting state of items in the page
    pub vote_state: HashMap<String, VoteData>,
    /// per-item vouch state, keyed by item id. Only contains entries for
    /// dead items HN rendered a `[vouch]`/`[unvouch]` link for — i.e.
    /// items the logged-in viewer has the karma to vouch on.
    pub vouch_state: HashMap<String, VouchData>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoteDirection {
    Up,
    Down,
}

impl VoteDirection {
    /// The `how` query parameter value expected by HN's `/vote` endpoint.
    pub fn as_how_param(self) -> &'static str {
        match self {
            Self::Up => "up",
            Self::Down => "down",
        }
    }
}

#[derive(Debug, Clone)]
pub struct VoteData {
    pub auth: String,
    /// The user's current vote on this item, or `None` if they haven't voted.
    pub vote: Option<VoteDirection>,
    /// True when HN rendered a downvote arrow for this item — i.e. the logged-in
    /// user has the karma to downvote and the item is downvote-eligible.
    pub can_downvote: bool,
}

#[derive(Debug, Clone)]
pub struct VouchData {
    pub auth: String,
    /// True when the HN page rendered `[unvouch]` rather than `[vouch]` for
    /// this item — the viewer has already vouched and the unvouch window
    /// hasn't expired. Used to pick `how=un` vs `how=up` when firing the
    /// next toggle.
    pub vouched: bool,
}

#[derive(Debug, Clone)]
/// A Hacker News item which can be either a story or a comment.
///
/// This struct is a shared representation between a story and a comment
/// and is used to render their content.
pub struct HnItem {
    pub id: u32,
    pub level: usize,
    pub display_state: DisplayState,
    pub links: Vec<String>,
    pub author: Option<String>,
    text: StyledString,
    minimized_text: StyledString,
    /// Parent story URL on news.ycombinator.com. Populated by the threads
    /// view on every item in each user-comment subtree so a bare `o`/`O`
    /// jumps back to the parent thread regardless of which item (root or
    /// reply) is focused. `None` for items not constructed by the threads
    /// view path.
    pub parent_story_url: Option<String>,
}

#[derive(Debug, Clone)]
pub enum DisplayState {
    Hidden,
    Minimized,
    Normal,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Article {
    pub title: String,
    pub url: String,
    pub content: String,
    pub author: Option<String>,
    pub date_published: Option<String>,
}

/// HN decorates stories and comments authored by the logged-in user with an
/// orange `*` to the left of the byline. Returns that prefix when the viewer
/// is the author, or an empty styled string otherwise.
fn own_item_prefix(author: &str, me: Option<&str>, style: config::Style) -> StyledString {
    if me == Some(author) {
        StyledString::styled("* ", style)
    } else {
        StyledString::new()
    }
}

/// HN renders `<span class="score">N points</span>` on the viewer's own
/// comments. Returns the matching " by"-suffixed byline fragment, or an
/// empty styled string when the score is unknown. The trailing " by " is
/// included because the username follows immediately.
fn comment_points_prefix(points: Option<u32>, style: config::Style) -> StyledString {
    match points {
        Some(n) => {
            let unit = if n == 1 { "point" } else { "points" };
            StyledString::styled(format!("{n} {unit} by "), style)
        }
        None => StyledString::new(),
    }
}

/// Mirrors HN's `[flagged]` / `[dead]` badges on the byline. Either, both,
/// or neither can be present — HN orders them `[flagged] [dead]`, and we
/// do the same. Returns an empty styled string for plain items.
fn status_prefix(flagged: bool, dead: bool, style: config::Style) -> StyledString {
    let mut s = StyledString::new();
    if flagged {
        s.append_styled("[flagged] ", style);
    }
    if dead {
        s.append_styled("[dead] ", style);
    }
    s
}

impl From<Story> for HnItem {
    fn from(story: Story) -> Self {
        let component_style = &config::get_config_theme().component_style;
        let me = client::get_user_info().map(|u| u.username.as_str());
        let is_faded = story.flagged || story.dead;

        let metadata = utils::combine_styled_strings([
            status_prefix(story.flagged, story.dead, component_style.metadata),
            story.styled_title(),
            StyledString::plain("\n"),
            own_item_prefix(&story.author, me, component_style.own_item_indicator),
            StyledString::styled(
                format!(
                    "{} points | by {} | {} ago | {} comments\n",
                    story.points,
                    story.author,
                    utils::get_elapsed_time_as_text(story.time),
                    story.num_comments,
                ),
                component_style.metadata,
            ),
        ]);

        // parse story's HTML content; fade the body when HN has marked
        // the story dead/flagged so it mirrors the site's gray rendering.
        let body_style: Style = if is_faded {
            component_style.faded.into()
        } else {
            Style::default()
        };
        let result = parse_hn_html_text(story.content, body_style, 0);

        // construct a minimized text representing the collapsed story's content
        let minimized_text = if result.content.source().is_empty() {
            metadata.clone()
        } else {
            utils::combine_styled_strings([metadata.clone(), StyledString::plain("... (more)")])
        };

        let text =
            utils::combine_styled_strings([metadata, StyledString::plain("\n"), result.content]);

        HnItem {
            id: story.id,
            level: 0, // story is at level 0 by default
            display_state: DisplayState::Normal,
            links: result.links,
            author: Some(story.author.clone()),
            text,
            minimized_text,
            parent_story_url: None,
        }
    }
}

impl From<Comment> for HnItem {
    fn from(comment: Comment) -> Self {
        let component_style = &config::get_config_theme().component_style;
        let me = client::get_user_info().map(|u| u.username.as_str());
        let author = comment.author.clone();
        let is_faded = comment.flagged || comment.dead;

        // When HN has grayed out the comment, render the username in the
        // same faded style rather than bold, so the whole comment reads as
        // de-emphasized the way it does on news.ycombinator.com.
        let username_style = if is_faded {
            component_style.faded
        } else {
            component_style.username
        };

        let metadata = utils::combine_styled_strings([
            status_prefix(comment.flagged, comment.dead, component_style.metadata),
            own_item_prefix(&comment.author, me, component_style.own_item_indicator),
            comment_points_prefix(comment.points, component_style.metadata),
            StyledString::styled(comment.author, username_style),
            StyledString::styled(
                format!(" {} ago ", utils::get_elapsed_time_as_text(comment.time)),
                component_style.metadata,
            ),
        ]);

        // constructs a minimized text representing the collapsed comment's content
        let minimized_text = utils::combine_styled_strings([
            metadata.clone(),
            StyledString::styled(
                format!("({} more)", comment.n_children + 1),
                component_style.metadata,
            ),
        ]);

        // parse the comment's content; fade the body when HN has marked
        // the comment dead/flagged so it mirrors the site's gray rendering.
        let body_style: Style = if is_faded {
            component_style.faded.into()
        } else {
            Style::default()
        };
        let result = parse_hn_html_text(comment.content, body_style, 0);

        let text =
            utils::combine_styled_strings([metadata, StyledString::plain("\n"), result.content]);

        HnItem {
            id: comment.id,
            level: comment.level,
            display_state: DisplayState::Normal,
            links: result.links,
            author: Some(author),
            text,
            minimized_text,
            parent_story_url: comment.parent_story_url,
        }
    }
}

impl Story {
    /// get the story's article URL.
    /// If the article URL is empty (in case of "AskHN" stories), fallback to the HN story's URL
    pub fn get_url(&self) -> Cow<'_, str> {
        if self.url.is_empty() {
            Cow::from(self.story_url())
        } else {
            Cow::from(&self.url)
        }
    }

    pub fn story_url(&self) -> String {
        format!("{}/item?id={}", client::HN_HOST_URL, self.id)
    }

    /// Get the decorated story's title
    pub fn styled_title(&self) -> StyledString {
        let mut parsed_title = StyledString::new();
        let mut title = self.title.clone();

        let component_style = &config::get_config_theme().component_style;

        // decorate the story title based on the story category
        {
            let categories = ["Ask HN", "Tell HN", "Show HN", "Launch HN"];
            let styles = [
                component_style.ask_hn,
                component_style.tell_hn,
                component_style.show_hn,
                component_style.launch_hn,
            ];

            assert!(categories.len() == styles.len());

            for i in 0..categories.len() {
                if let Some(t) = title.strip_prefix(categories[i]) {
                    parsed_title.append_styled(categories[i], styles[i]);
                    title = t.to_string();
                }
            }
        }

        // The story title may contain search matches wrapped inside `<em>` tags.
        // The matches are decorated with a corresponding style.
        {
            // an index represents the part of the text that hasn't been parsed (e.g `title[curr_pos..]` )
            let mut curr_pos = 0;
            for caps in MATCH_RE.captures_iter(&title) {
                let whole_match = caps.get(0).unwrap();
                // the part that doesn't match any patterns should be rendered in the default style
                if curr_pos < whole_match.start() {
                    parsed_title.append_plain(&title[curr_pos..whole_match.start()]);
                }
                curr_pos = whole_match.end();

                parsed_title.append_styled(
                    caps.name("match").unwrap().as_str(),
                    component_style.matched_highlight,
                );
            }
            if curr_pos < title.len() {
                parsed_title.append_plain(&title[curr_pos..]);
            }
        }

        parsed_title
    }

    /// Get the story's plain title
    pub fn plain_title(&self) -> String {
        self.title.replace("<em>", "").replace("</em>", "") // story's title from the search view can have `<em>` inside it
    }
}

impl HnItem {
    /// Build a placeholder root item for views that don't have a real story
    /// or comment to anchor on (e.g. the in-TUI threads view). The given
    /// `text` is rendered verbatim — no byline, points, or `N comments`
    /// suffix is generated.
    pub fn synthetic_root(text: StyledString) -> Self {
        HnItem {
            id: 0,
            level: 0,
            display_state: DisplayState::Normal,
            links: Vec::new(),
            author: None,
            text: text.clone(),
            minimized_text: text,
            parent_story_url: None,
        }
    }

    /// Plain-text source of the item's rendered body, stripped of styling.
    /// Used when quoting the item into `$EDITOR` for replies.
    pub fn plain_text(&self) -> String {
        self.text.source().to_string()
    }

    /// gets the dispay text of the item, which depends on the item's states
    /// (e.g `vote`, `display_state`, etc)
    pub fn text(&self, vote: Option<&VoteData>) -> StyledString {
        let theme = config::get_config_theme();
        let component_style = &theme.component_style;

        let text = match self.display_state {
            DisplayState::Hidden => unreachable!("Hidden item's text shouldn't be accessed"),
            DisplayState::Minimized => self.minimized_text.clone(),
            DisplayState::Normal => self.text.clone(),
        };
        let vote_text = match vote {
            None => StyledString::plain(""),
            Some(v) => {
                let mut s = StyledString::new();
                if v.vote == Some(VoteDirection::Up) {
                    s.append_styled("▲", component_style.upvote);
                } else {
                    s.append_plain("▲");
                }
                // Render the down arrow only when the user has downvote
                // privileges for this item (or already downvoted it).
                if v.can_downvote || v.vote == Some(VoteDirection::Down) {
                    if v.vote == Some(VoteDirection::Down) {
                        s.append_styled("▼", component_style.downvote);
                    } else {
                        s.append_plain("▼");
                    }
                }
                s.append_plain(" ");
                s
            }
        };

        utils::combine_styled_strings([vote_text, text])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn own_item_prefix_marks_self_author() {
        let s = own_item_prefix("freedomben", Some("freedomben"), config::Style::default());
        assert_eq!(s.source(), "* ");
    }

    #[test]
    fn own_item_prefix_empty_for_other_author() {
        let s = own_item_prefix("nkrisc", Some("freedomben"), config::Style::default());
        assert_eq!(s.source(), "");
    }

    #[test]
    fn own_item_prefix_empty_when_logged_out() {
        let s = own_item_prefix("freedomben", None, config::Style::default());
        assert_eq!(s.source(), "");
    }

    #[test]
    fn comment_points_prefix_uses_plural_for_zero_and_many() {
        let style = config::Style::default();
        assert_eq!(
            comment_points_prefix(Some(0), style).source(),
            "0 points by "
        );
        assert_eq!(
            comment_points_prefix(Some(42), style).source(),
            "42 points by "
        );
    }

    #[test]
    fn comment_points_prefix_uses_singular_for_one() {
        let s = comment_points_prefix(Some(1), config::Style::default());
        assert_eq!(s.source(), "1 point by ");
    }

    #[test]
    fn comment_points_prefix_empty_when_none() {
        let s = comment_points_prefix(None, config::Style::default());
        assert_eq!(s.source(), "");
    }
}
