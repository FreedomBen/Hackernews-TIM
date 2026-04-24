use super::{article_view, help_view};
use crate::prelude::*;

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

/// Construct a view's title bar
pub fn construct_view_title_bar(desc: &str) -> impl View {
    let style = config::get_config_theme().component_style.title_bar;
    let user_info = build_user_info_text(style.into());

    Layer::with_color(
        LinearLayout::horizontal()
            .child(
                TextView::new(StyledString::styled(desc, style))
                    .h_align(align::HAlign::Center)
                    .full_width(),
            )
            .child(TextView::new(user_info)),
        style.into(),
    )
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
