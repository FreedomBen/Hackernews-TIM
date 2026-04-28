use super::{article_view, help_view};
use crate::prelude::*;

/// HN story-tag tabs rendered in the nav strip, in F1..F5 order. Story
/// view also uses this for tag-cycling.
pub static STORY_TAGS: [&str; 5] = ["front_page", "story", "ask_hn", "show_hn", "job"];

/// Which entry of the global nav strip a view wants highlighted as
/// "you are here". Views that aren't anchored to a nav target (article
/// view, generic comment view) pass `None`.
#[derive(Debug, Clone, Copy)]
pub enum NavTarget {
    None,
    StoryTag(&'static str),
    MyThreads,
    Search,
}

/// Construct a simple footer view
pub fn construct_footer_view<T: help_view::HasHelpView>() -> impl View {
    LinearLayout::horizontal()
        .child(
            TextView::new(StyledString::styled(
                "Hacker News Terminal UI - made by AOME ©",
                config::get_config_theme().component_style.bold,
            ))
            .align(align::Align::bot_center())
            .full_width(),
        )
        .child(
            LinearLayout::horizontal()
                .child(Button::new_raw(
                    format!("[{}: help] ", config::get_global_keymap().open_help_dialog),
                    |s| s.add_layer(T::construct_on_event_help_view()),
                ))
                .child(Button::new_raw("[back] ", |s| {
                    if s.screen_mut().len() > 1 {
                        s.pop_layer();
                    } else {
                        s.quit();
                    }
                }))
                .child(Button::new_raw("[quit] ", |s| s.quit())),
        )
}

/// Build the "username (karma)" styled text rendered at the right edge of a
/// title bar, or an empty string when there's no logged-in user. The HN
/// website shows the same thing in its top-right nav area.
pub fn build_user_info_text(style: Style) -> StyledString {
    match client::get_user_info() {
        None => StyledString::new(),
        Some(info) => {
            let text = match info.karma {
                Some(k) => format!(" {} ({}) ", info.username, k),
                None => format!(" {} ", info.username),
            };
            StyledString::styled(text, style)
        }
    }
}

/// Render the global nav strip — `[Y] Hacker News | 1.front_page | …`
/// — with the entry matching `active` highlighted. `sort_suffix` is
/// appended to the active label and is only used by the story view to
/// surface the current sort mode (e.g. `" (by_date)"`); other callers
/// pass an empty string.
pub(super) fn nav_strip_styled(active: NavTarget, sort_suffix: &str) -> StyledString {
    let theme = config::get_config_theme();
    let style = theme.component_style.title_bar;
    let active_style = Style::from(style).combine(theme.component_style.current_story_tag);

    let mut out = StyledString::styled(
        "[Y]",
        Style::from(style).combine(ColorStyle::front(theme.palette.light_white)),
    );
    out.append_styled(" Hacker News", style);

    for (i, tag) in STORY_TAGS.iter().enumerate() {
        out.append_styled(" | ", style);
        let label = format!("{}.{tag}", i + 1);
        if matches!(active, NavTarget::StoryTag(t) if t == *tag) {
            out.append_styled(format!("{label}{sort_suffix}"), active_style);
        } else {
            out.append_styled(label, style);
        }
    }

    out.append_styled(" | ", style);
    if matches!(active, NavTarget::MyThreads) {
        out.append_styled("6.threads", active_style);
    } else {
        out.append_styled("6.threads", style);
    }

    out.append_styled(" | ", style);
    if matches!(active, NavTarget::Search) {
        out.append_styled("search (^S)", active_style);
    } else {
        out.append_styled("search (^S)", style);
    }

    out.append_styled(" | ", style);
    out
}

/// The nav-strip-only top bar used by the story view. Other views render
/// the strip plus a description row via [`construct_view_title_bar`].
pub fn construct_story_view_top_bar(
    active_tag: &'static str,
    sort_mode: client::StorySortMode,
) -> impl View {
    let suffix = match sort_mode {
        client::StorySortMode::None => "",
        client::StorySortMode::Date => " (by_date)",
        client::StorySortMode::Points => " (by_point)",
    };
    let style = config::get_config_theme().component_style.title_bar;
    let user_info = build_user_info_text(style.into());
    let nav_text = nav_strip_styled(NavTarget::StoryTag(active_tag), suffix);

    PaddedView::lrtb(
        0,
        0,
        0,
        1,
        Layer::with_color(
            LinearLayout::horizontal()
                .child(TextView::new(nav_text))
                .child(TextView::new(StyledString::new()).full_width())
                .child(TextView::new(user_info)),
            style.into(),
        ),
    )
}

/// Construct a view's title bar (nav strip + centered description).
/// Equivalent to [`construct_view_title_bar_with_nav`] with no nav
/// target highlighted.
pub fn construct_view_title_bar(desc: &str) -> impl View {
    construct_view_title_bar_with_nav(desc, NavTarget::None)
}

/// Two-row title bar: the global nav strip on top (with the matching
/// entry highlighted), and the per-view description centered below.
pub fn construct_view_title_bar_with_nav(desc: &str, nav: NavTarget) -> impl View {
    let style = config::get_config_theme().component_style.title_bar;
    let user_info = build_user_info_text(style.into());
    let nav_text = nav_strip_styled(nav, "");

    let nav_layer = Layer::with_color(
        LinearLayout::horizontal()
            .child(TextView::new(nav_text))
            .child(TextView::new(StyledString::new()).full_width())
            .child(TextView::new(user_info)),
        style.into(),
    );

    let desc_layer = Layer::with_color(
        TextView::new(StyledString::styled(desc, style))
            .h_align(align::HAlign::Center)
            .full_width(),
        style.into(),
    );

    LinearLayout::vertical().child(nav_layer).child(desc_layer)
}

/// Open a given url using a specific command
pub fn open_url_in_browser(url: &str) {
    if url.is_empty() {
        return;
    }

    let url = url.to_string();
    let url_open_command = &config::get_config().url_open_command;
    std::thread::spawn(move || {
        match std::process::Command::new(&url_open_command.command)
            .args(&url_open_command.options)
            .arg(&url)
            .output()
        {
            Err(err) => warn!(
                "failed to execute command `{} {}`: {}",
                url_open_command, url, err
            ),
            Ok(output) => {
                if !output.status.success() {
                    warn!(
                        "failed to execute command `{} {}`: {}",
                        url_open_command,
                        url,
                        std::str::from_utf8(&output.stderr).unwrap(),
                    )
                }
            }
        }
    });
}

/// open in article view the `i`-th link.
/// Note that the link index starts with `1`.
pub fn open_ith_link_in_article_view(
    client: &'static client::HNClient,
    links: &[String],
    i: usize,
) -> Option<EventResult> {
    if i > 0 && i <= links.len() {
        Some(EventResult::with_cb({
            let url = links[i - 1].clone();
            move |s| article_view::construct_and_add_new_article_view(client, s, &url)
        }))
    } else {
        Some(EventResult::Consumed(None))
    }
}

/// open in browser the `i`-th link.
/// Note that the link index starts with `1`.
pub fn open_ith_link_in_browser(links: &[String], i: usize) -> Option<EventResult> {
    if i > 0 && i <= links.len() {
        open_url_in_browser(&links[i - 1]);
        Some(EventResult::Consumed(None))
    } else {
        Some(EventResult::Consumed(None))
    }
}
