use std::cell::RefCell;
use std::rc::Rc;

use super::text_view::EditableTextView;
use crate::prelude::*;

/// Shared find-on-page state. A view that supports find-on-page owns a
/// `FindStateRef` and passes a clone to `construct_find_dialog`. The
/// dialog mutates the state on keystrokes; the owning view polls the
/// `pending` signal on each layout pass. Tracked `match_ids` live on
/// the state so outer-layer keymaps (e.g. a story view's paging
/// wrapper) can check "is find active" without reaching into the view.
pub struct FindState {
    pub query: String,
    pub pending: Option<FindSignal>,
    pub match_ids: Vec<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FindSignal {
    /// Re-apply highlights using the current `query`.
    Update,
    /// Drop tracked matches and restore original item text.
    Clear,
    /// Jump focus to the next match at or after the current focus.
    JumpNext,
    /// Jump focus to the previous match at or before the current focus.
    JumpPrev,
}

pub type FindStateRef = Rc<RefCell<FindState>>;

impl FindState {
    pub fn new_ref() -> FindStateRef {
        Rc::new(RefCell::new(FindState {
            query: String::new(),
            pending: None,
            match_ids: Vec::new(),
        }))
    }
}

/// Apply `match_style` on top of each existing span in `content` wherever
/// `query` occurs (ASCII case-insensitive). Returns the rebuilt string
/// and the source-byte ranges of every match (useful for callers that
/// need to scroll to a specific match, e.g. article view jump-to-match).
pub fn highlight_matches(
    content: &StyledString,
    query: &str,
    match_style: Style,
) -> (StyledString, Vec<(usize, usize)>) {
    if query.is_empty() {
        return (content.clone(), Vec::new());
    }
    let src = content.source();
    let src_lower = src.to_ascii_lowercase();
    let q_lower = query.to_ascii_lowercase();
    if q_lower.is_empty() || q_lower.len() > src_lower.len() {
        return (content.clone(), Vec::new());
    }

    let mut ranges: Vec<(usize, usize)> = Vec::new();
    let mut pos = 0usize;
    while let Some(idx) = src_lower[pos..].find(&q_lower) {
        let start = pos + idx;
        let end = start + q_lower.len();
        // Skip matches that straddle multi-byte codepoint boundaries.
        if !src.is_char_boundary(start) || !src.is_char_boundary(end) {
            pos = start + 1;
            continue;
        }
        ranges.push((start, end));
        pos = end;
    }

    if ranges.is_empty() {
        return (content.clone(), Vec::new());
    }

    let mut out = StyledString::new();
    let mut offset = 0usize;
    let mut range_idx = 0usize;
    for span in content.spans() {
        let span_text: &str = span.content;
        let span_style: Style = *span.attr;
        let span_start = offset;
        let span_end = offset + span_text.len();
        let mut cursor = 0usize;

        while range_idx < ranges.len() {
            let (ms, me) = ranges[range_idx];
            if me <= span_start {
                range_idx += 1;
                continue;
            }
            if ms >= span_end {
                break;
            }
            let local_start = ms.saturating_sub(span_start);
            let local_end = me.min(span_end).saturating_sub(span_start);
            if cursor < local_start {
                out.append_styled(&span_text[cursor..local_start], span_style);
            }
            out.append_styled(&span_text[local_start..local_end], match_style);
            cursor = local_end;
            if me > span_end {
                break;
            }
            range_idx += 1;
        }
        if cursor < span_text.len() {
            out.append_styled(&span_text[cursor..], span_style);
        }
        offset = span_end;
    }

    (out, ranges)
}

/// Construct the find-on-page dialog overlay. Typing updates the shared
/// `state` and signals the owning view; Enter commits (jump to next
/// match); Esc clears highlights. The owning view reacts via
/// `wrap_layout` polling `state.pending`.
pub fn construct_find_dialog(state: FindStateRef) -> impl View {
    let edit_keymap = config::get_edit_keymap().clone();
    let close_keymap = config::get_global_keymap().close_dialog.clone();

    let dialog = Dialog::around(EditableTextView::new().fixed_width(40)).title("Find");

    let esc_state = state.clone();
    let enter_state = state.clone();
    let type_state = state;

    OnEventView::new(dialog)
        .on_pre_event_inner(close_keymap, move |_, _| {
            esc_state.borrow_mut().pending = Some(FindSignal::Clear);
            Some(EventResult::with_cb(|s| {
                s.pop_layer();
            }))
        })
        .on_pre_event_inner(Event::Key(Key::Enter), move |_, _| {
            enter_state.borrow_mut().pending = Some(FindSignal::JumpNext);
            Some(EventResult::with_cb(|s| {
                s.pop_layer();
            }))
        })
        .on_pre_event_inner(EventTrigger::from_fn(|_| true), move |dialog, e| {
            let input = find_input_mut(dialog)?;
            let query_changed = match *e {
                Event::Char(c) => {
                    input.add_char(c);
                    true
                }
                _ if edit_keymap.backward_delete_char.has_event(e) => {
                    input.del_char();
                    true
                }
                _ if edit_keymap.move_cursor_left.has_event(e) => {
                    input.move_cursor_left();
                    false
                }
                _ if edit_keymap.move_cursor_right.has_event(e) => {
                    input.move_cursor_right();
                    false
                }
                _ if edit_keymap.move_cursor_to_begin.has_event(e) => {
                    input.move_cursor_to_begin();
                    false
                }
                _ if edit_keymap.move_cursor_to_end.has_event(e) => {
                    input.move_cursor_to_end();
                    false
                }
                _ => return None,
            };
            if query_changed {
                let query = input.get_text();
                let mut s = type_state.borrow_mut();
                s.query = query;
                s.pending = Some(FindSignal::Update);
            }
            Some(EventResult::Consumed(None))
        })
}

fn find_input_mut(dialog: &mut Dialog) -> Option<&mut EditableTextView> {
    let resized = dialog
        .get_content_mut()
        .downcast_mut::<ResizedView<EditableTextView>>()?;
    Some(resized.get_inner_mut())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_style() -> Style {
        Style::default()
    }

    #[test]
    fn empty_query_returns_input_unchanged() {
        let input = StyledString::plain("hello world");
        let (out, ranges) = highlight_matches(&input, "", make_style());
        assert!(ranges.is_empty());
        assert_eq!(out.source(), "hello world");
    }

    #[test]
    fn counts_all_occurrences_case_insensitively() {
        let input = StyledString::plain("Foo foo FOO");
        let (_, ranges) = highlight_matches(&input, "foo", make_style());
        assert_eq!(ranges.len(), 3);
    }

    #[test]
    fn preserves_source_text_byte_for_byte() {
        let input = StyledString::plain("search within a longer sentence");
        let (out, _) = highlight_matches(&input, "within", make_style());
        assert_eq!(out.source(), input.source());
    }

    #[test]
    fn zero_matches_when_query_absent() {
        let input = StyledString::plain("hello");
        let (_, ranges) = highlight_matches(&input, "zzz", make_style());
        assert!(ranges.is_empty());
    }

    #[test]
    fn handles_multibyte_text_without_panicking() {
        let input = StyledString::plain("café matcha café");
        let (_, ranges) = highlight_matches(&input, "café", make_style());
        // ASCII-only lowering won't match "café" against "CAFÉ"; just
        // verify it matches itself and produces a valid string.
        assert_eq!(ranges.len(), 2);
    }

    #[test]
    fn returns_source_byte_ranges_for_each_match() {
        let input = StyledString::plain("foo bar foo");
        let (_, ranges) = highlight_matches(&input, "foo", make_style());
        assert_eq!(ranges, vec![(0, 3), (8, 11)]);
    }
}
