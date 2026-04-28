use crate::client;
use config_parser2::*;
use cursive::event;
use serde::{de, Deserialize, Deserializer};

#[derive(Default, Debug, Clone, Deserialize, ConfigParse)]
pub struct KeyMap {
    pub edit_keymap: EditKeyMap,
    pub scroll_keymap: ScrollKeyMap,
    pub global_keymap: GlobalKeyMap,
    pub story_view_keymap: StoryViewKeyMap,
    pub search_view_keymap: SearchViewKeyMap,
    pub comment_view_keymap: CommentViewKeyMap,
    pub article_view_keymap: ArticleViewKeyMap,
    pub link_dialog_keymap: LinkDialogKeyMap,

    pub custom_keymaps: Vec<CustomKeyMap>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CustomKeyMap {
    pub key: Keys,
    pub tag: String,
    pub by_date: bool,
    pub numeric_filters: client::StoryNumericFilters,
}

config_parser_impl!(CustomKeyMap);

#[derive(Debug, Clone, Deserialize, ConfigParse)]
pub struct EditKeyMap {
    pub move_cursor_left: Keys,
    pub move_cursor_right: Keys,
    pub move_cursor_to_begin: Keys,
    pub move_cursor_to_end: Keys,
    pub backward_delete_char: Keys,
}

impl Default for EditKeyMap {
    fn default() -> Self {
        EditKeyMap {
            move_cursor_left: Keys::new(vec![event::Key::Left.into(), event::Event::CtrlChar('b')]),
            move_cursor_right: Keys::new(vec![
                event::Key::Right.into(),
                event::Event::CtrlChar('f'),
            ]),
            move_cursor_to_begin: Keys::new(vec![
                event::Key::Home.into(),
                event::Event::CtrlChar('a'),
            ]),
            move_cursor_to_end: Keys::new(vec![
                event::Key::End.into(),
                event::Event::CtrlChar('e'),
            ]),
            backward_delete_char: Keys::new(vec![event::Key::Backspace.into()]),
        }
    }
}

#[derive(Debug, Clone, Deserialize, ConfigParse)]
pub struct ScrollKeyMap {
    pub up: Keys,
    pub down: Keys,
    pub page_up: Keys,
    pub page_down: Keys,
    pub top: Keys,
    pub bottom: Keys,
}

impl Default for ScrollKeyMap {
    fn default() -> Self {
        ScrollKeyMap {
            up: Keys::new(vec!['k'.into(), event::Key::Up.into()]),
            down: Keys::new(vec!['j'.into(), event::Key::Down.into()]),
            page_up: Keys::new(vec![
                'u'.into(),
                event::Key::PageUp.into(),
                event::Event::CtrlChar('u'),
            ]),
            page_down: Keys::new(vec![
                'd'.into(),
                event::Key::PageDown.into(),
                event::Event::CtrlChar('d'),
            ]),
            top: Keys::new(vec!['g'.into(), event::Key::Home.into()]),
            bottom: Keys::new(vec!['G'.into(), event::Key::End.into()]),
        }
    }
}

#[derive(Debug, Clone, Deserialize, ConfigParse)]
pub struct GlobalKeyMap {
    pub open_help_dialog: Keys,
    pub open_login_dialog: Keys,
    pub open_my_threads_in_browser: Keys,
    pub quit: Keys,
    pub close_dialog: Keys,

    // view navigation keymaps
    pub goto_previous_view: Keys,
    pub goto_front_page_view: Keys,
    pub goto_search_view: Keys,
    pub goto_all_stories_view: Keys,
    pub goto_ask_hn_view: Keys,
    pub goto_show_hn_view: Keys,
    pub goto_jobs_view: Keys,
    pub goto_my_threads_view: Keys,
}

impl Default for GlobalKeyMap {
    fn default() -> Self {
        GlobalKeyMap {
            open_help_dialog: Keys::new(vec!['?'.into()]),
            open_login_dialog: Keys::new(vec!['L'.into()]),
            open_my_threads_in_browser: Keys::new(vec!['T'.into()]),
            quit: Keys::new(vec!['q'.into(), event::Event::CtrlChar('c')]),
            close_dialog: Keys::new(vec![event::Key::Esc.into()]),

            goto_previous_view: Keys::new(vec![
                event::Key::Backspace.into(),
                event::Event::CtrlChar('p'),
            ]),

            goto_search_view: Keys::new(vec![event::Event::CtrlChar('s')]),

            goto_front_page_view: Keys::new(vec![event::Key::F1.into()]),
            goto_all_stories_view: Keys::new(vec![event::Key::F2.into()]),
            goto_ask_hn_view: Keys::new(vec![event::Key::F3.into()]),
            goto_show_hn_view: Keys::new(vec![event::Key::F4.into()]),
            goto_jobs_view: Keys::new(vec![event::Key::F5.into()]),
            goto_my_threads_view: Keys::new(vec![event::Key::F6.into()]),
        }
    }
}

#[derive(Debug, Clone, Deserialize, ConfigParse)]
pub struct StoryViewKeyMap {
    // story tags navigation keymaps
    pub next_story_tag: Keys,
    pub prev_story_tag: Keys,

    // stories navigation keymaps
    pub next_story: Keys,
    pub prev_story: Keys,
    pub goto_story: Keys,

    // stories paging/filtering keymaps
    pub next_page: Keys,
    pub prev_page: Keys,
    pub cycle_sort_mode: Keys,

    // link keymaps
    pub open_article_in_browser: Keys,
    pub open_article_in_article_view: Keys,
    pub open_story_in_browser: Keys,

    pub goto_story_comment_view: Keys,

    pub upvote: Keys,
    pub downvote: Keys,
    pub vouch: Keys,
    pub reply: Keys,

    pub find_in_view: Keys,
    pub find_next_match: Keys,
    pub find_prev_match: Keys,
}

impl Default for StoryViewKeyMap {
    fn default() -> Self {
        StoryViewKeyMap {
            next_story_tag: Keys::new(vec!['l'.into(), event::Key::Right.into()]),
            prev_story_tag: Keys::new(vec!['h'.into(), event::Key::Left.into()]),
            next_story: Keys::new(vec!['j'.into(), event::Key::Down.into()]),
            prev_story: Keys::new(vec!['k'.into(), event::Key::Up.into()]),
            goto_story: Keys::new(vec!['g'.into()]),

            next_page: Keys::new(vec!['n'.into()]),
            prev_page: Keys::new(vec!['p'.into()]),
            cycle_sort_mode: Keys::new(vec!['d'.into()]),

            open_article_in_browser: Keys::new(vec!['o'.into()]),
            open_article_in_article_view: Keys::new(vec!['O'.into()]),
            open_story_in_browser: Keys::new(vec!['s'.into()]),

            goto_story_comment_view: Keys::new(vec![event::Key::Enter.into()]),

            upvote: Keys::new(vec!['v'.into()]),
            downvote: Keys::new(vec!['V'.into()]),
            vouch: Keys::new(vec!['!'.into()]),
            reply: Keys::new(vec!['r'.into()]),

            find_in_view: Keys::new(vec!['/'.into(), event::Event::CtrlChar('f')]),
            find_next_match: Keys::new(vec!['n'.into()]),
            find_prev_match: Keys::new(vec!['N'.into()]),
        }
    }
}

#[derive(Debug, Clone, Deserialize, ConfigParse)]
pub struct SearchViewKeyMap {
    // switch mode keymaps
    pub to_navigation_mode: Keys,
    pub to_search_mode: Keys,
}

impl Default for SearchViewKeyMap {
    fn default() -> Self {
        SearchViewKeyMap {
            to_navigation_mode: Keys::new(vec![event::Key::Esc.into()]),
            to_search_mode: Keys::new(vec!['i'.into()]),
        }
    }
}

#[derive(Debug, Clone, Deserialize, ConfigParse)]
pub struct CommentViewKeyMap {
    // comments navigation keymaps
    pub next_comment: Keys,
    pub prev_comment: Keys,
    pub next_top_level_comment: Keys,
    pub prev_top_level_comment: Keys,
    pub next_leq_level_comment: Keys,
    pub prev_leq_level_comment: Keys,
    pub parent_comment: Keys,

    // link keymaps
    pub open_story_in_browser: Keys,
    pub open_comment_in_browser: Keys,
    pub open_article_in_browser: Keys,
    pub open_article_in_article_view: Keys,
    pub open_link_in_browser: Keys,
    pub open_link_in_article_view: Keys,

    pub upvote: Keys,
    pub downvote: Keys,
    pub vouch: Keys,
    pub reply: Keys,
    pub edit: Keys,

    pub toggle_collapse_comment: Keys,

    pub find_in_view: Keys,
    pub find_next_match: Keys,
    pub find_prev_match: Keys,
}

impl Default for CommentViewKeyMap {
    fn default() -> Self {
        CommentViewKeyMap {
            next_comment: Keys::new(vec!['j'.into(), event::Key::Down.into()]),
            prev_comment: Keys::new(vec!['k'.into(), event::Key::Up.into()]),
            next_top_level_comment: Keys::new(vec!['n'.into()]),
            prev_top_level_comment: Keys::new(vec!['p'.into()]),
            next_leq_level_comment: Keys::new(vec!['l'.into(), event::Key::Right.into()]),
            prev_leq_level_comment: Keys::new(vec!['h'.into(), event::Key::Left.into()]),
            parent_comment: Keys::new(vec!['u'.into()]),

            open_comment_in_browser: Keys::new(vec!['c'.into()]),
            open_story_in_browser: Keys::new(vec!['s'.into()]),
            open_article_in_browser: Keys::new(vec!['a'.into()]),
            open_article_in_article_view: Keys::new(vec!['A'.into()]),
            open_link_in_browser: Keys::new(vec!['o'.into()]),
            open_link_in_article_view: Keys::new(vec!['O'.into()]),

            upvote: Keys::new(vec!['v'.into()]),
            downvote: Keys::new(vec!['V'.into()]),
            vouch: Keys::new(vec!['!'.into()]),
            reply: Keys::new(vec!['r'.into()]),
            edit: Keys::new(vec!['e'.into()]),

            toggle_collapse_comment: Keys::new(vec![event::Key::Tab.into()]),

            find_in_view: Keys::new(vec!['/'.into(), event::Event::CtrlChar('f')]),
            find_next_match: Keys::new(vec!['n'.into()]),
            find_prev_match: Keys::new(vec!['N'.into()]),
        }
    }
}

#[derive(Debug, Clone, Deserialize, ConfigParse)]
pub struct ArticleViewKeyMap {
    pub open_link_dialog: Keys,
    pub open_article_in_browser: Keys,
    pub open_link_in_browser: Keys,
    pub open_link_in_article_view: Keys,

    pub find_in_view: Keys,
    pub find_next_match: Keys,
    pub find_prev_match: Keys,
}

impl Default for ArticleViewKeyMap {
    fn default() -> Self {
        ArticleViewKeyMap {
            open_link_dialog: Keys::new(vec!['l'.into()]),
            open_article_in_browser: Keys::new(vec!['a'.into()]),
            open_link_in_browser: Keys::new(vec!['o'.into()]),
            open_link_in_article_view: Keys::new(vec!['O'.into()]),

            find_in_view: Keys::new(vec!['/'.into(), event::Event::CtrlChar('f')]),
            find_next_match: Keys::new(vec!['n'.into()]),
            find_prev_match: Keys::new(vec!['N'.into()]),
        }
    }
}

#[derive(Debug, Clone, Deserialize, ConfigParse)]
pub struct LinkDialogKeyMap {
    pub next: Keys,
    pub prev: Keys,
    pub open_link_in_browser: Keys,
    pub open_link_in_article_view: Keys,
}

impl Default for LinkDialogKeyMap {
    fn default() -> Self {
        LinkDialogKeyMap {
            next: Keys::new(vec!['j'.into(), event::Key::Down.into()]),
            prev: Keys::new(vec!['k'.into(), event::Key::Up.into()]),
            open_link_in_browser: Keys::new(vec!['o'.into(), event::Key::Enter.into()]),
            open_link_in_article_view: Keys::new(vec!['O'.into()]),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Keys {
    events: Vec<event::Event>,
}

impl From<Keys> for event::EventTrigger {
    fn from(k: Keys) -> Self {
        event::EventTrigger::from_fn(move |e| k.has_event(e))
    }
}

/// Render a `Keys` value back to its TOML-config-style string form. Single
/// events render as their own canonical name (`"q"`, `"C-c"`, `"esc"`,
/// `"f1"`, `"page_up"`); multi-event bindings render as a bracketed
/// comma-separated list.
///
/// ```
/// use hackernews_tim::config::Keys;
/// use cursive::event::{Event, Key};
///
/// assert_eq!(format!("{}", Keys::new(vec!['q'.into()])), "q");
/// assert_eq!(format!("{}", Keys::new(vec![Event::CtrlChar('c')])), "C-c");
/// assert_eq!(format!("{}", Keys::new(vec![Key::Esc.into()])), "esc");
/// assert_eq!(
///     format!("{}", Keys::new(vec!['q'.into(), Event::CtrlChar('c')])),
///     "[q, C-c]"
/// );
/// ```
impl std::fmt::Display for Keys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        fn fmt_event(e: &event::Event, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match e {
                event::Event::Char(c) => write!(f, "{c}"),
                event::Event::CtrlChar(c) => write!(f, "C-{c}"),
                event::Event::AltChar(c) => write!(f, "M-{c}"),
                event::Event::Key(k) => match k {
                    event::Key::Enter => write!(f, "enter"),
                    event::Key::Tab => write!(f, "tab"),
                    event::Key::Backspace => write!(f, "backspace"),
                    event::Key::Esc => write!(f, "esc"),

                    event::Key::Left => write!(f, "left"),
                    event::Key::Right => write!(f, "right"),
                    event::Key::Up => write!(f, "up"),
                    event::Key::Down => write!(f, "down"),

                    event::Key::Ins => write!(f, "ins"),
                    event::Key::Del => write!(f, "del"),
                    event::Key::Home => write!(f, "home"),
                    event::Key::End => write!(f, "end"),
                    event::Key::PageUp => write!(f, "page_up"),
                    event::Key::PageDown => write!(f, "page_down"),

                    event::Key::F1 => write!(f, "f1"),
                    event::Key::F2 => write!(f, "f2"),
                    event::Key::F3 => write!(f, "f3"),
                    event::Key::F4 => write!(f, "f4"),
                    event::Key::F5 => write!(f, "f5"),
                    event::Key::F6 => write!(f, "f6"),
                    event::Key::F7 => write!(f, "f7"),
                    event::Key::F8 => write!(f, "f8"),
                    event::Key::F9 => write!(f, "f9"),
                    event::Key::F10 => write!(f, "f10"),
                    event::Key::F11 => write!(f, "f11"),
                    event::Key::F12 => write!(f, "f12"),

                    _ => panic!("unknown key: {k:?}"),
                },
                _ => panic!("unknown event: {e:?}"),
            }
        }

        if self.events.is_empty() {
            return Ok(());
        }

        if self.events.len() == 1 {
            fmt_event(&self.events[0], f)
        } else {
            write!(f, "[")?;
            fmt_event(&self.events[0], f)?;
            for e in &self.events[1..] {
                write!(f, ", ")?;
                fmt_event(e, f)?;
            }
            write!(f, "]")?;
            Ok(())
        }
    }
}

impl Keys {
    /// Build a `Keys` from a list of events. The list acts as an OR — any
    /// one of the events triggers the binding.
    ///
    /// ```
    /// use hackernews_tim::config::Keys;
    /// use cursive::event::{Event, Key};
    ///
    /// let quit = Keys::new(vec!['q'.into(), Event::CtrlChar('c')]);
    /// assert!(quit.has_event(&Event::Char('q')));
    /// assert!(quit.has_event(&Event::CtrlChar('c')));
    /// assert!(!quit.has_event(&Event::Key(Key::Esc)));
    /// ```
    pub fn new(events: Vec<event::Event>) -> Self {
        Keys { events }
    }

    /// True when `e` is one of the events this `Keys` was built from.
    /// Used as the trigger predicate when converting into a
    /// [`cursive::event::EventTrigger`].
    pub fn has_event(&self, e: &event::Event) -> bool {
        self.events.contains(e)
    }
}

config_parser_impl!(Keys);

impl<'de> de::Deserialize<'de> for Keys {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        /// an enum representing either
        /// - a single key string \[1\]
        /// - an array of multiple key strings
        ///
        /// \[1\]: "key string" denotes the string representation of a key
        enum StringOrVec {
            String(String),
            Vec(Vec<String>),
        }

        /// a helper function that converts a key string into `cursive::event::Event`
        fn from_key_string_to_event(ks: String) -> Result<event::Event> {
            let chars: Vec<char> = ks.chars().collect();

            let event = if chars.len() == 1 {
                // a single character
                event::Event::Char(chars[0])
            } else if chars.len() == 3 && chars[1] == '-' {
                // M-<c> for alt-<c> and C-<c> for ctrl-<c>, with <c> denotes a single character
                match chars[0] {
                    'C' => event::Event::CtrlChar(chars[2]),
                    'M' => event::Event::AltChar(chars[2]),
                    _ => {
                        return Err(anyhow::anyhow!(
                            "failed to parse key: unknown/invalid key {}",
                            ks
                        ))
                    }
                }
            } else {
                let key = match ks.as_str() {
                    "enter" => event::Key::Enter,
                    "tab" => event::Key::Tab,
                    "backspace" => event::Key::Backspace,
                    "esc" => event::Key::Esc,

                    "left" => event::Key::Left,
                    "right" => event::Key::Right,
                    "up" => event::Key::Up,
                    "down" => event::Key::Down,

                    "ins" => event::Key::Ins,
                    "del" => event::Key::Del,
                    "home" => event::Key::Home,
                    "end" => event::Key::End,
                    "page_up" => event::Key::PageUp,
                    "page_down" => event::Key::PageDown,

                    "f1" => event::Key::F1,
                    "f2" => event::Key::F2,
                    "f3" => event::Key::F3,
                    "f4" => event::Key::F4,
                    "f5" => event::Key::F5,
                    "f6" => event::Key::F6,
                    "f7" => event::Key::F7,
                    "f8" => event::Key::F8,
                    "f9" => event::Key::F9,
                    "f10" => event::Key::F10,
                    "f11" => event::Key::F11,
                    "f12" => event::Key::F12,

                    _ => {
                        return Err(anyhow::anyhow!(
                            "failed to parse key: unknown/invalid key {}",
                            ks
                        ))
                    }
                };

                event::Event::Key(key)
            };

            Ok(event)
        }

        let key_strings = match StringOrVec::deserialize(deserializer)? {
            StringOrVec::String(v) => vec![v],
            StringOrVec::Vec(v) => v,
        };

        let events = key_strings
            .into_iter()
            .map(from_key_string_to_event)
            .collect::<Result<Vec<_>>>()
            .map_err(serde::de::Error::custom)?;

        Ok(Keys::new(events))
    }
}

pub fn get_edit_keymap() -> &'static EditKeyMap {
    &super::get_config().keymap.edit_keymap
}

pub fn get_scroll_keymap() -> &'static ScrollKeyMap {
    &super::get_config().keymap.scroll_keymap
}

pub fn get_global_keymap() -> &'static GlobalKeyMap {
    &super::get_config().keymap.global_keymap
}

pub fn get_story_view_keymap() -> &'static StoryViewKeyMap {
    &super::get_config().keymap.story_view_keymap
}

pub fn get_search_view_keymap() -> &'static SearchViewKeyMap {
    &super::get_config().keymap.search_view_keymap
}

pub fn get_comment_view_keymap() -> &'static CommentViewKeyMap {
    &super::get_config().keymap.comment_view_keymap
}

pub fn get_article_view_keymap() -> &'static ArticleViewKeyMap {
    &super::get_config().keymap.article_view_keymap
}

pub fn get_link_dialog_keymap() -> &'static LinkDialogKeyMap {
    &super::get_config().keymap.link_dialog_keymap
}

#[cfg(test)]
mod tests {
    use super::*;
    use config_parser2::ConfigParser;
    use cursive::event::{Event, EventTrigger, Key};

    fn parse_keys(toml_value: &str) -> Keys {
        // Wrap in a key=… table so we get a serde Deserialize entry point
        // for the inner value (which can be a string or array).
        let raw = format!("key = {toml_value}");
        let value: toml::Value = toml::from_str(&raw).expect("test toml should parse");
        let key_value = value.get("key").expect("key field").clone();
        key_value.try_into().expect("Keys should deserialize")
    }

    fn try_parse_keys(toml_value: &str) -> Result<Keys, toml::de::Error> {
        let raw = format!("key = {toml_value}");
        let value: toml::Value = toml::from_str(&raw).expect("test toml should parse");
        let key_value = value.get("key").expect("key field").clone();
        key_value.try_into()
    }

    // --- Keys::deserialize ---

    #[test]
    fn keys_deserialize_single_char() {
        let keys = parse_keys("\"q\"");
        assert!(keys.has_event(&Event::Char('q')));
        assert!(!keys.has_event(&Event::Char('Q')));
    }

    #[test]
    fn keys_deserialize_ctrl_modifier() {
        let keys = parse_keys("\"C-c\"");
        assert!(keys.has_event(&Event::CtrlChar('c')));
    }

    #[test]
    fn keys_deserialize_alt_modifier() {
        let keys = parse_keys("\"M-x\"");
        assert!(keys.has_event(&Event::AltChar('x')));
    }

    #[test]
    fn keys_deserialize_special_keys() {
        for (s, expected) in [
            ("\"esc\"", Key::Esc),
            ("\"backspace\"", Key::Backspace),
            ("\"enter\"", Key::Enter),
            ("\"tab\"", Key::Tab),
            ("\"page_up\"", Key::PageUp),
            ("\"page_down\"", Key::PageDown),
            ("\"f1\"", Key::F1),
            ("\"f12\"", Key::F12),
        ] {
            let keys = parse_keys(s);
            assert!(
                keys.has_event(&Event::Key(expected)),
                "{s} should resolve to {expected:?}"
            );
        }
    }

    #[test]
    fn keys_deserialize_array_acts_as_or() {
        let keys = parse_keys(r#"["q", "C-c"]"#);
        assert!(keys.has_event(&Event::Char('q')));
        assert!(keys.has_event(&Event::CtrlChar('c')));
        assert!(!keys.has_event(&Event::Char('x')));
    }

    #[test]
    fn keys_deserialize_unknown_string_errors() {
        // "ctrl-c" is not a valid key string — modifier syntax is "C-c".
        let err = try_parse_keys("\"ctrl-c\"").unwrap_err();
        assert!(
            err.to_string().contains("unknown/invalid key"),
            "expected key parse error, got {err}"
        );
    }

    #[test]
    fn keys_deserialize_unknown_special_key_errors() {
        let err = try_parse_keys("\"f99\"").unwrap_err();
        assert!(
            err.to_string().contains("unknown/invalid key"),
            "expected key parse error, got {err}"
        );
    }

    // --- Display for Keys ---

    #[test]
    fn keys_display_single_char() {
        let keys = Keys::new(vec!['q'.into()]);
        assert_eq!(format!("{keys}"), "q");
    }

    #[test]
    fn keys_display_ctrl_uses_capital_c_prefix() {
        let keys = Keys::new(vec![Event::CtrlChar('c')]);
        assert_eq!(format!("{keys}"), "C-c");
    }

    #[test]
    fn keys_display_alt_uses_capital_m_prefix() {
        let keys = Keys::new(vec![Event::AltChar('x')]);
        assert_eq!(format!("{keys}"), "M-x");
    }

    #[test]
    fn keys_display_special_keys() {
        assert_eq!(format!("{}", Keys::new(vec![Key::Esc.into()])), "esc");
        assert_eq!(
            format!("{}", Keys::new(vec![Key::Backspace.into()])),
            "backspace"
        );
        assert_eq!(format!("{}", Keys::new(vec![Key::F1.into()])), "f1");
        assert_eq!(
            format!("{}", Keys::new(vec![Key::PageUp.into()])),
            "page_up"
        );
    }

    #[test]
    fn keys_display_multi_event_uses_brackets() {
        let keys = Keys::new(vec!['q'.into(), Event::CtrlChar('c')]);
        assert_eq!(format!("{keys}"), "[q, C-c]");
    }

    #[test]
    fn keys_display_empty_renders_nothing() {
        let keys = Keys::new(vec![]);
        assert_eq!(format!("{keys}"), "");
    }

    // --- Round-trip: deserialize then display ---

    #[test]
    fn keys_round_trip_single_to_canonical() {
        for canonical in ["q", "C-c", "M-x", "esc", "f1", "page_down"] {
            let keys = parse_keys(&format!("\"{canonical}\""));
            assert_eq!(format!("{keys}"), canonical);
        }
    }

    // --- From<Keys> for EventTrigger ---

    #[test]
    fn keys_to_event_trigger_matches_listed_events() {
        let keys = Keys::new(vec!['q'.into(), Event::CtrlChar('c')]);
        let trigger: EventTrigger = keys.into();
        assert!(trigger.apply(&Event::Char('q')));
        assert!(trigger.apply(&Event::CtrlChar('c')));
        assert!(!trigger.apply(&Event::Char('Q')));
        assert!(!trigger.apply(&Event::Key(Key::Esc)));
    }

    // --- KeyMap defaults ---

    #[test]
    fn global_keymap_default_has_quit_binding() {
        let g = GlobalKeyMap::default();
        // `quit` defaults to {q, C-c}
        assert!(g.quit.has_event(&Event::Char('q')));
        assert!(g.quit.has_event(&Event::CtrlChar('c')));
    }

    #[test]
    fn global_keymap_default_has_help_dialog_binding() {
        let g = GlobalKeyMap::default();
        assert!(g.open_help_dialog.has_event(&Event::Char('?')));
    }

    #[test]
    fn global_keymap_default_function_keys_route_to_views() {
        let g = GlobalKeyMap::default();
        assert!(g.goto_front_page_view.has_event(&Event::Key(Key::F1)));
        assert!(g.goto_all_stories_view.has_event(&Event::Key(Key::F2)));
        assert!(g.goto_ask_hn_view.has_event(&Event::Key(Key::F3)));
        assert!(g.goto_show_hn_view.has_event(&Event::Key(Key::F4)));
        assert!(g.goto_jobs_view.has_event(&Event::Key(Key::F5)));
        assert!(g.goto_my_threads_view.has_event(&Event::Key(Key::F6)));
    }

    #[test]
    fn keymap_default_populates_every_section() {
        let km = KeyMap::default();
        // Spot-check at least one binding from each section to prove the
        // top-level Default propagates all the way down.
        assert!(km.global_keymap.quit.has_event(&Event::Char('q')));
        assert!(km.scroll_keymap.up.has_event(&Event::Char('k')));
        assert!(km.custom_keymaps.is_empty());
    }

    // --- CustomKeyMap deserialization ---

    #[test]
    fn custom_keymap_deserializes_from_toml_block() {
        // Same shape used by the [[keymap.custom_keymaps]] examples in
        // examples/config.toml.
        let toml_src = r#"
            key = "M-1"
            tag = "story"
            by_date = false
            [numeric_filters]
            elapsed_days_interval = {start = 0, end = 3}
            points_interval = {start = 10}
            num_comments_interval = {}
        "#;
        let custom: CustomKeyMap = toml::from_str(toml_src).expect("custom keymap parse");
        assert_eq!(custom.tag, "story");
        assert!(!custom.by_date);
        assert!(custom.key.has_event(&Event::AltChar('1')));
        // Numeric filter description ignores num_comments because it's empty.
        let desc = custom.numeric_filters.desc();
        assert!(desc.contains("elapsed_days: [0:3]"));
        assert!(desc.contains("points: [10:]"));
    }

    // --- ConfigParse merge ---

    #[test]
    fn global_keymap_parse_overlays_only_present_fields() {
        // A partial overlay should change just `quit` and leave every other
        // default field intact.
        let mut g = GlobalKeyMap::default();
        let original_help = g.open_help_dialog.clone();
        let overlay: toml::Value = toml::from_str(r#"quit = "Q""#).unwrap();
        g.parse(overlay).expect("partial overlay should apply");
        assert!(g.quit.has_event(&Event::Char('Q')));
        // The previous {q, C-c} default has been replaced wholesale by "Q".
        assert!(!g.quit.has_event(&Event::Char('q')));
        // Other defaults stay.
        assert_eq!(
            format!("{}", g.open_help_dialog),
            format!("{original_help}")
        );
        assert!(g.goto_front_page_view.has_event(&Event::Key(Key::F1)));
    }
}
