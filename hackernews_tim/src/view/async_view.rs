use std::collections::HashMap;

use super::{article_view, comment_view, result_view::ResultView, story_view, utils};
use crate::client;
use crate::prelude::*;
use anyhow::Context;
use cursive_aligned_view::Alignable;
use cursive_async_view::AsyncView;

pub fn construct_comment_view_async(
    siv: &mut Cursive,
    client: &'static client::HNClient,
    item_id: u32,
) -> impl View {
    AsyncView::new_with_bg_creator(siv, move || Ok(client.get_page_data(item_id)), {
        move |result: Result<_>| {
            ResultView::new(
                result.with_context(|| format!("failed to load comments from item (id={item_id})")),
                |data| comment_view::construct_comment_view(client, data, utils::NavTarget::None),
            )
        }
    })
    .with_animation_fn(animation)
    .align_center()
    .full_screen()
}

pub fn construct_threads_view_async(
    siv: &mut Cursive,
    client: &'static client::HNClient,
    username: String,
    page: usize,
) -> impl View {
    AsyncView::new_with_bg_creator(
        siv,
        {
            let username = username.clone();
            move || Ok(client.get_user_threads_page(&username, page))
        },
        move |result: Result<_>| {
            let username = username.clone();
            ResultView::new(
                result.with_context(move || {
                    format!("failed to load threads for user {username} (page {page})")
                }),
                |data| {
                    comment_view::construct_comment_view(client, data, utils::NavTarget::MyThreads)
                },
            )
        },
    )
    .with_animation_fn(animation)
    .align_center()
    .full_screen()
}

pub fn construct_story_view_async(
    siv: &mut Cursive,
    client: &'static client::HNClient,
    tag: &'static str,
    sort_mode: client::StorySortMode,
    page: usize,
    numeric_filters: client::StoryNumericFilters,
) -> impl View {
    let cb_sink = siv.cb_sink().clone();
    AsyncView::new_with_bg_creator(
        siv,
        move || {
            // Fetch stories and listing-page vote state in parallel so the
            // arrows appear in the first render instead of waiting for a
            // lazy vote-triggered fetch. Vote prefetch failures are
            // swallowed: missing arrows is better than a blank error.
            let (stories_result, vote_state) = rayon::join(
                || client.get_stories_by_tag(tag, sort_mode, page, numeric_filters),
                || match client.get_listing_vote_state(tag, sort_mode, page) {
                    Ok(map) => map,
                    Err(err) => {
                        warn!(
                            "failed to prefetch listing vote state (tag={tag}, sort_mode={sort_mode:?}, page={page}): {err}"
                        );
                        HashMap::new()
                    }
                },
            );
            Ok(stories_result.map(|stories| (stories, vote_state)))
        },
        move |result| {
            let cb_sink = cb_sink.clone();
            ResultView::new(
                result.with_context(|| {
                    format!(
                        "failed to get stories (tag={tag}, sort_mode={sort_mode:?}, page={page}, numeric_filters={{{numeric_filters}}})",
                    )
                }),
                move |(stories, vote_state)| {
                    let cb_sink = cb_sink.clone();
                    story_view::construct_story_view(stories, vote_state, client, tag, sort_mode, page, numeric_filters, cb_sink)
                },
            )
        },
    )
    .with_animation_fn(animation)
    .align_center()
    .full_screen()
}

pub fn construct_article_view_async(
    client: &'static client::HNClient,
    siv: &mut Cursive,
    article_url: &str,
) -> impl View {
    let err_context = format!(
        "Failed to parse an article into readable text:\n\
         `{}`.\n\n\
         Please review your configuration as described in the below link, and try again\n\
         \"https://github.com/aome510/hackernews-TUI/blob/main/docs/config.md#article-parse-command\"",
        article_url);

    AsyncView::new_with_bg_creator(
        siv,
        {
            let article_url = article_url.to_owned();
            move || Ok(client::HNClient::get_article(client, &article_url))
        },
        move |result| {
            let err_context = err_context.clone();
            ResultView::new(result.with_context(|| err_context), |article| {
                article_view::construct_article_view(client, article)
            })
        },
    )
    .with_animation_fn(animation)
    .align_center()
    .full_screen()
}

fn animation(width: usize, _height: usize, frame_idx: usize) -> cursive_async_view::AnimationFrame {
    let n_frames = 120; // number of frames to complete an animation
    let style = ColorStyle::from(config::get_config_theme().component_style.loading_bar);

    if config::get_config().use_pacman_loading {
        let factor = (frame_idx as f64) / (n_frames as f64);
        let x = (factor * width as f64) as usize;

        let content = crate::utils::combine_styled_strings([
            StyledString::styled(repeat_str("- ", x / 2), style),
            StyledString::styled('ᗧ', style),
            StyledString::styled(repeat_str(" o", width.saturating_sub(x + 1) / 2), style),
        ]);

        cursive_async_view::AnimationFrame {
            content,
            next_frame_idx: (frame_idx + 1) % n_frames,
        }
    } else {
        let symbol = "━";
        let factor = (frame_idx as f64) / (n_frames as f64);
        let x = (factor * width as f64) as usize;

        let content = crate::utils::combine_styled_strings([
            StyledString::styled(repeat_str(symbol, x), style.back),
            StyledString::styled(repeat_str(symbol, width - x), style.front),
        ]);

        cursive_async_view::AnimationFrame {
            content,
            next_frame_idx: (frame_idx + 1) % n_frames,
        }
    }
}

fn repeat_str<S: Into<String>>(s: S, n: usize) -> String {
    s.into().repeat(n)
}
