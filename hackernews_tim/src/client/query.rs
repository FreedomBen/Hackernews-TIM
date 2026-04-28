use crate::utils;
use serde::Deserialize;

#[derive(Debug, Eq, PartialEq, Hash, Clone, Copy)]
pub enum StorySortMode {
    None,
    Date,
    Points,
}

impl StorySortMode {
    /// Cycle to the next sort mode for the given story tag. The cycle
    /// depends on the tag:
    ///
    /// - `"front_page"` only ever stays at [`StorySortMode::None`].
    /// - `"story"` and `"job"` cycle `Date → Points → Date` (never `None`).
    /// - Any other tag (`"ask_hn"`, `"show_hn"`, custom tags) cycles
    ///   `None → Date → Points → None`.
    ///
    /// ```
    /// use hackernews_tim::client::StorySortMode;
    /// assert_eq!(StorySortMode::None.next("ask_hn"), StorySortMode::Date);
    /// assert_eq!(StorySortMode::Date.next("ask_hn"), StorySortMode::Points);
    /// assert_eq!(StorySortMode::Points.next("ask_hn"), StorySortMode::None);
    ///
    /// // story / job cycles between Date and Points only.
    /// assert_eq!(StorySortMode::Date.next("story"), StorySortMode::Points);
    /// assert_eq!(StorySortMode::Points.next("story"), StorySortMode::Date);
    /// ```
    pub fn next(self, tag: &str) -> Self {
        if tag == "front_page" {
            assert!(
                self == Self::None,
                "`front_page` stories should have no sort mode"
            );
            return Self::None;
        }
        match self {
            Self::None => {
                assert!(
                    tag != "story" && tag != "job",
                    "`story`/`job` stories should have a sort mode"
                );
                Self::Date
            }
            Self::Date => Self::Points,
            Self::Points => {
                if tag == "story" || tag == "job" {
                    Self::Date
                } else {
                    Self::None
                }
            }
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Deserialize)]
pub struct FilterInterval<T> {
    start: Option<T>,
    end: Option<T>,
}

impl<T: std::fmt::Display> FilterInterval<T> {
    pub fn query(&self, field: &str) -> String {
        format!(
            "{}{}",
            match self.start.as_ref() {
                Some(x) => format!(",{field}>={x}"),
                None => "".to_string(),
            },
            match self.end.as_ref() {
                Some(x) => format!(",{field}<{x}"),
                None => "".to_string(),
            },
        )
    }

    pub fn desc(&self, field: &str) -> String {
        format!(
            "{}: [{}:{}]",
            field,
            match self.start.as_ref() {
                Some(x) => x.to_string(),
                None => "".to_string(),
            },
            match self.end.as_ref() {
                Some(x) => x.to_string(),
                None => "".to_string(),
            }
        )
    }
}

#[derive(Debug, Clone, Copy, Default, Deserialize)]
/// `StoryNumericFilters` defines a list of options to filter stories
pub struct StoryNumericFilters {
    #[serde(default)]
    elapsed_days_interval: FilterInterval<u32>,
    #[serde(default)]
    points_interval: FilterInterval<u32>,
    #[serde(default)]
    num_comments_interval: FilterInterval<usize>,
}

impl StoryNumericFilters {
    fn from_elapsed_days_to_unix_time(elapsed_days: Option<u32>) -> Option<u64> {
        match elapsed_days {
            None => None,
            Some(day_offset) => {
                let current_time = std::time::SystemTime::now()
                    .duration_since(std::time::SystemTime::UNIX_EPOCH)
                    .unwrap()
                    .as_secs();
                Some(current_time - utils::from_day_offset_to_time_offset_in_secs(day_offset))
            }
        }
    }

    pub fn desc(&self) -> String {
        format!(
            "{}, {}, {}",
            self.elapsed_days_interval.desc("elapsed_days"),
            self.points_interval.desc("points"),
            self.num_comments_interval.desc("num_comments")
        )
    }

    pub fn query(&self) -> String {
        // convert elapsed_days to unix time (in seconds)
        let time_interval = FilterInterval {
            end: Self::from_elapsed_days_to_unix_time(self.elapsed_days_interval.start),
            start: Self::from_elapsed_days_to_unix_time(self.elapsed_days_interval.end),
        };

        let mut query = format!(
            "{}{}{}",
            time_interval.query("created_at_i"),
            self.points_interval.query("points"),
            self.num_comments_interval.query("num_comments")
        );
        if !query.is_empty() {
            query.remove(0); // remove trailing ,
            format!("&numericFilters={query}")
        } else {
            "".to_string()
        }
    }
}

impl std::fmt::Display for StoryNumericFilters {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.desc())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- StorySortMode::next ---

    #[test]
    fn sort_mode_front_page_stays_none() {
        assert_eq!(StorySortMode::None.next("front_page"), StorySortMode::None);
    }

    #[test]
    #[should_panic(expected = "`front_page` stories should have no sort mode")]
    fn sort_mode_front_page_date_panics() {
        let _ = StorySortMode::Date.next("front_page");
    }

    #[test]
    #[should_panic(expected = "`front_page` stories should have no sort mode")]
    fn sort_mode_front_page_points_panics() {
        let _ = StorySortMode::Points.next("front_page");
    }

    #[test]
    fn sort_mode_story_cycles_date_points_only() {
        assert_eq!(StorySortMode::Date.next("story"), StorySortMode::Points);
        assert_eq!(StorySortMode::Points.next("story"), StorySortMode::Date);
    }

    #[test]
    fn sort_mode_job_cycles_date_points_only() {
        assert_eq!(StorySortMode::Date.next("job"), StorySortMode::Points);
        assert_eq!(StorySortMode::Points.next("job"), StorySortMode::Date);
    }

    #[test]
    #[should_panic(expected = "`story`/`job` stories should have a sort mode")]
    fn sort_mode_story_none_panics() {
        let _ = StorySortMode::None.next("story");
    }

    #[test]
    #[should_panic(expected = "`story`/`job` stories should have a sort mode")]
    fn sort_mode_job_none_panics() {
        let _ = StorySortMode::None.next("job");
    }

    #[test]
    fn sort_mode_ask_hn_cycles_none_date_points() {
        assert_eq!(StorySortMode::None.next("ask_hn"), StorySortMode::Date);
        assert_eq!(StorySortMode::Date.next("ask_hn"), StorySortMode::Points);
        assert_eq!(StorySortMode::Points.next("ask_hn"), StorySortMode::None);
    }

    #[test]
    fn sort_mode_show_hn_cycles_like_ask_hn() {
        assert_eq!(StorySortMode::None.next("show_hn"), StorySortMode::Date);
        assert_eq!(StorySortMode::Date.next("show_hn"), StorySortMode::Points);
        assert_eq!(StorySortMode::Points.next("show_hn"), StorySortMode::None);
    }

    #[test]
    fn sort_mode_custom_tag_cycles_like_ask_hn() {
        assert_eq!(StorySortMode::None.next("custom"), StorySortMode::Date);
        assert_eq!(StorySortMode::Date.next("custom"), StorySortMode::Points);
        assert_eq!(StorySortMode::Points.next("custom"), StorySortMode::None);
    }

    // --- FilterInterval::query ---

    fn interval(start: Option<u32>, end: Option<u32>) -> FilterInterval<u32> {
        FilterInterval { start, end }
    }

    #[test]
    fn filter_interval_query_empty_when_both_bounds_none() {
        assert_eq!(interval(None, None).query("points"), "");
    }

    #[test]
    fn filter_interval_query_start_only() {
        assert_eq!(interval(Some(10), None).query("points"), ",points>=10");
    }

    #[test]
    fn filter_interval_query_end_only() {
        assert_eq!(interval(None, Some(50)).query("points"), ",points<50");
    }

    #[test]
    fn filter_interval_query_both_bounds() {
        assert_eq!(
            interval(Some(10), Some(50)).query("points"),
            ",points>=10,points<50"
        );
    }

    // --- FilterInterval::desc ---

    #[test]
    fn filter_interval_desc_empty_bounds() {
        assert_eq!(interval(None, None).desc("points"), "points: [:]");
    }

    #[test]
    fn filter_interval_desc_start_only() {
        assert_eq!(interval(Some(10), None).desc("points"), "points: [10:]");
    }

    #[test]
    fn filter_interval_desc_end_only() {
        assert_eq!(interval(None, Some(50)).desc("points"), "points: [:50]");
    }

    #[test]
    fn filter_interval_desc_both_bounds() {
        assert_eq!(
            interval(Some(10), Some(50)).desc("points"),
            "points: [10:50]"
        );
    }

    // --- StoryNumericFilters ---

    #[test]
    fn story_numeric_filters_query_empty_when_no_intervals() {
        assert_eq!(StoryNumericFilters::default().query(), "");
    }

    #[test]
    fn story_numeric_filters_query_points_only() {
        let filters = StoryNumericFilters {
            elapsed_days_interval: FilterInterval::default(),
            points_interval: FilterInterval {
                start: Some(100),
                end: None,
            },
            num_comments_interval: FilterInterval::default(),
        };
        assert_eq!(filters.query(), "&numericFilters=points>=100");
    }

    #[test]
    fn story_numeric_filters_query_combines_points_and_comments() {
        let filters = StoryNumericFilters {
            elapsed_days_interval: FilterInterval::default(),
            points_interval: FilterInterval {
                start: Some(100),
                end: None,
            },
            num_comments_interval: FilterInterval {
                start: Some(5),
                end: None,
            },
        };
        assert_eq!(
            filters.query(),
            "&numericFilters=points>=100,num_comments>=5"
        );
    }

    #[test]
    fn story_numeric_filters_query_strips_leading_comma() {
        // Internal accumulator uses leading commas; the prefix path must
        // strip the very first one before emitting `&numericFilters=`.
        let filters = StoryNumericFilters {
            elapsed_days_interval: FilterInterval::default(),
            points_interval: FilterInterval::default(),
            num_comments_interval: FilterInterval {
                start: Some(10),
                end: Some(100),
            },
        };
        let query = filters.query();
        assert_eq!(query, "&numericFilters=num_comments>=10,num_comments<100");
        assert!(!query.contains("=,"));
    }

    #[test]
    fn story_numeric_filters_query_reverses_elapsed_days_into_created_at_i() {
        // elapsed_days {start: 1, end: 7} = "stories from 1 to 7 days ago".
        // That maps to created_at_i lower=now-7d, upper=now-1d — start/end
        // swap when crossing from "elapsed" to "absolute" time.
        let filters = StoryNumericFilters {
            elapsed_days_interval: FilterInterval {
                start: Some(1),
                end: Some(7),
            },
            points_interval: FilterInterval::default(),
            num_comments_interval: FilterInterval::default(),
        };
        let query = filters.query();
        assert!(query.starts_with("&numericFilters=created_at_i>="));
        let parts: Vec<&str> = query
            .trim_start_matches("&numericFilters=")
            .split(',')
            .collect();
        let lower: u64 = parts[0]
            .trim_start_matches("created_at_i>=")
            .parse()
            .unwrap();
        let upper: u64 = parts[1]
            .trim_start_matches("created_at_i<")
            .parse()
            .unwrap();
        assert_eq!(upper - lower, 6 * 24 * 60 * 60);
    }

    #[test]
    fn story_numeric_filters_desc_concatenates_subdescriptions() {
        let filters = StoryNumericFilters {
            elapsed_days_interval: FilterInterval {
                start: Some(1),
                end: None,
            },
            points_interval: FilterInterval {
                start: None,
                end: Some(50),
            },
            num_comments_interval: FilterInterval::default(),
        };
        assert_eq!(
            filters.desc(),
            "elapsed_days: [1:], points: [:50], num_comments: [:]"
        );
    }

    #[test]
    fn story_numeric_filters_display_matches_desc() {
        let filters = StoryNumericFilters {
            elapsed_days_interval: FilterInterval::default(),
            points_interval: FilterInterval {
                start: Some(100),
                end: None,
            },
            num_comments_interval: FilterInterval::default(),
        };
        assert_eq!(format!("{filters}"), filters.desc());
    }
}
