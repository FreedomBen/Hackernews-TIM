mod async_view;
mod find_bar;
mod fn_view_wrapper;
mod link_dialog;
mod login_dialog;
mod result_view;
mod text_view;
mod traits;
mod utils;

pub mod article_view;
pub mod comment_view;
pub mod help_view;
pub mod search_view;
pub mod story_view;

use crate::view::help_view::HasHelpView;

use crate::prelude::*;

fn set_up_switch_story_view_shortcut(
    keys: config::Keys,
    tag: &'static str,
    s: &mut Cursive,
    client: &'static client::HNClient,
    numeric_filters: Option<client::StoryNumericFilters>,
) {
    s.set_on_post_event(keys, move |s| {
        story_view::construct_and_add_new_story_view(
            s,
            client,
            tag,
            if tag == "story" || tag == "job" {
                client::StorySortMode::Date
            } else {
                client::StorySortMode::None
            },
            0,
            numeric_filters.unwrap_or_default(),
            false,
        );
    });
}

fn set_up_global_callbacks(
    s: &mut Cursive,
    client: &'static client::HNClient,
    auth_file: std::path::PathBuf,
    auth_storage: config::AuthStorage,
) {
    s.clear_global_callbacks(Event::CtrlChar('c'));

    let global_keymap = config::get_global_keymap().clone();

    // .............................................................
    // global shortcuts for switching between different Story Views
    // .............................................................

    set_up_switch_story_view_shortcut(
        global_keymap.goto_front_page_view,
        "front_page",
        s,
        client,
        None,
    );
    set_up_switch_story_view_shortcut(
        global_keymap.goto_all_stories_view,
        "story",
        s,
        client,
        None,
    );
    set_up_switch_story_view_shortcut(global_keymap.goto_ask_hn_view, "ask_hn", s, client, None);
    set_up_switch_story_view_shortcut(global_keymap.goto_show_hn_view, "show_hn", s, client, None);
    set_up_switch_story_view_shortcut(global_keymap.goto_jobs_view, "job", s, client, None);

    // custom navigation shortcuts
    config::get_config()
        .keymap
        .custom_keymaps
        .iter()
        .for_each(|data| {
            s.set_on_post_event(data.key.clone(), move |s| {
                story_view::construct_and_add_new_story_view(
                    s,
                    client,
                    &data.tag,
                    if data.by_date {
                        client::StorySortMode::Date
                    } else {
                        client::StorySortMode::Points
                    },
                    0,
                    data.numeric_filters,
                    false,
                );
            });
        });

    // ............................................
    // end of navigation shortcuts for Story Views
    // ............................................

    s.set_on_post_event(global_keymap.goto_previous_view, |s| {
        if s.screen_mut().len() > 1 {
            s.pop_layer();
        }
    });

    s.set_on_post_event(global_keymap.goto_search_view, move |s| {
        search_view::construct_and_add_new_search_view(s, client);
    });

    s.set_on_post_event(global_keymap.open_help_dialog, |s| {
        s.add_layer(help_view::DefaultHelpView::construct_on_event_help_view())
    });

    s.set_on_post_event(global_keymap.open_login_dialog, move |s| {
        s.add_layer(login_dialog::get_login_dialog(
            client,
            auth_file.clone(),
            auth_storage,
        ));
    });

    s.set_on_post_event(
        global_keymap.open_my_threads_in_browser,
        |s| match client::get_user_info() {
            Some(info) => {
                let url = format!("{}/threads?id={}", client::HN_HOST_URL, info.username);
                utils::open_url_in_browser(&url);
            }
            None => {
                s.add_layer(
                    Dialog::info(
                        "Log in first (press `L`) to view your comments on \
                         Hacker News.",
                    )
                    .title("Not logged in"),
                );
            }
        },
    );

    s.set_on_post_event(
        global_keymap.goto_my_threads_view,
        move |s| match client::get_user_info() {
            Some(info) => {
                comment_view::construct_and_add_new_threads_view(
                    s,
                    client,
                    info.username.clone(),
                    0,
                );
            }
            None => {
                s.add_layer(
                    Dialog::info(
                        "Log in first (press `L`) to view your comments on \
                         Hacker News.",
                    )
                    .title("Not logged in"),
                );
            }
        },
    );

    s.set_on_post_event(global_keymap.quit, |s| s.quit());
}

/// Build a dialog summarising a startup password-login attempt, or `None`
/// when there's nothing worth interrupting the user with (no login was
/// attempted, or a cached session restored seamlessly).
fn build_login_status_dialog(status: client::StartupLoginStatus) -> Option<Dialog> {
    use client::StartupLoginStatus;
    let (title, body) = match status {
        StartupLoginStatus::NotAttempted => return None,
        StartupLoginStatus::Success { username } => (
            "Login successful",
            format!(
                "Logged in to Hacker News as {username}. A fresh session \
                 cookie was saved to your auth file."
            ),
        ),
        StartupLoginStatus::BadLogin => (
            "Login failed",
            "Hacker News rejected the stored credentials (`Bad login.`). \
             Update `username`/`password` in hn-auth.toml and restart."
                .to_string(),
        ),
        StartupLoginStatus::Captcha => (
            "CAPTCHA required",
            "Hacker News asked for a CAPTCHA before letting us log in, and \
             the TUI can't solve it. Sign in at https://news.ycombinator.com/ \
             in a browser, copy the `user` cookie from DevTools, and paste \
             its value into the `session = \"\"` line in hn-auth.toml."
                .to_string(),
        ),
        StartupLoginStatus::Other(msg) => {
            ("Login failed", format!("Hacker News login failed: {msg}"))
        }
    };
    Some(Dialog::info(body).title(title))
}

/// Initialize the application's UI
pub fn init_ui(
    client: &'static client::HNClient,
    start_id: Option<u32>,
    auth_file: std::path::PathBuf,
    auth_storage: config::AuthStorage,
    login_status: client::StartupLoginStatus,
) -> cursive::CursiveRunnable {
    let mut s = cursive::default();

    // initialize `cursive` color palette which is determined by the application's theme
    let theme = config::get_config_theme();
    s.update_theme(|t| {
        t.palette.set_color("view", theme.palette.background.into());
        t.palette
            .set_color("primary", theme.palette.foreground.into());
        t.palette
            .set_color("title_primary", theme.palette.foreground.into());
        t.palette
            .set_color("highlight", theme.palette.selection_background.into());
        t.palette
            .set_color("highlight_text", theme.palette.selection_foreground.into());

        // `cursive_core` uses `Effect::Reverse` for highlighting focused views
        // since the version `v0.3.7`. The below changes are to remove the reverse effect.
        t.palette[PaletteStyle::Highlight] = ColorStyle::highlight().into();
        t.palette[PaletteStyle::HighlightInactive] = ColorStyle::highlight_inactive().into();
    });

    set_up_global_callbacks(&mut s, client, auth_file, auth_storage);

    match start_id {
        Some(id) => {
            comment_view::construct_and_add_new_comment_view(&mut s, client, id, false);
        }
        None => {
            // render `front_page` story view as the application's startup view if no start id is specified
            story_view::construct_and_add_new_story_view(
                &mut s,
                client,
                "front_page",
                client::StorySortMode::None,
                0,
                client::StoryNumericFilters::default(),
                false,
            );
        }
    }

    if let Some(dialog) = build_login_status_dialog(login_status) {
        s.add_layer(dialog);
    }

    s
}
