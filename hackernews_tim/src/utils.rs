use crate::prelude::*;
use std::time::{Duration, SystemTime};

fn format_plural(amount: u64, time: &str) -> String {
    format!("{} {}{}", amount, time, if amount == 1 { "" } else { "s" })
}

fn get_time_offset_in_text(offset: u64) -> String {
    if offset < 60 {
        format_plural(offset, "second")
    } else if offset < 60 * 60 {
        format_plural(offset / 60, "minute")
    } else if offset < 60 * 60 * 24 {
        format_plural(offset / (60 * 60), "hour")
    } else if offset < 60 * 60 * 24 * 30 {
        format_plural(offset / (60 * 60 * 24), "day")
    } else if offset < 60 * 60 * 24 * 365 {
        format_plural(offset / (60 * 60 * 24 * 30), "month")
    } else {
        format_plural(offset / (60 * 60 * 24 * 365), "year")
    }
}

/// Convert a "days ago" offset into the equivalent number of seconds.
///
/// ```
/// use hackernews_tim::utils::from_day_offset_to_time_offset_in_secs;
/// assert_eq!(from_day_offset_to_time_offset_in_secs(0), 0);
/// assert_eq!(from_day_offset_to_time_offset_in_secs(1), 86_400);
/// assert_eq!(from_day_offset_to_time_offset_in_secs(7), 604_800);
/// ```
pub fn from_day_offset_to_time_offset_in_secs(day_offset: u32) -> u64 {
    let day_in_secs: u64 = 24 * 60 * 60;
    day_in_secs * (day_offset as u64)
}

/// Calculate the elapsed time and return the result
/// in an appropriate format depending on the duration
pub fn get_elapsed_time_as_text(time: u64) -> String {
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap();
    let then = Duration::new(time, 0);
    let offset = now.as_secs() - then.as_secs();
    get_time_offset_in_text(offset)
}

/// A simple URL shortening function that reduces the URL length if it
/// exceeds a given threshold. URLs of 50 chars or fewer pass through
/// unchanged; longer ones render as `first40 + "..." + last10`.
///
/// ```
/// use hackernews_tim::utils::shorten_url;
/// assert_eq!(shorten_url("https://example.com/short"), "https://example.com/short");
///
/// let long = "https://example.com/".to_string() + &"a".repeat(60);
/// let short = shorten_url(&long);
/// assert!(short.contains("..."));
/// assert_eq!(short.len(), 53);
/// ```
pub fn shorten_url(url: &str) -> String {
    let chars = url.chars().collect::<Vec<_>>();
    let len = chars.len();
    if len > 50 {
        String::from_iter(chars[..40].iter()) + "..." + &String::from_iter(chars[len - 10..].iter())
    } else {
        url.to_string()
    }
}
/// Combine multiple styled strings into a single styled string
pub fn combine_styled_strings<S>(strings: S) -> StyledString
where
    S: Into<Vec<StyledString>>,
{
    strings
        .into()
        .into_iter()
        .fold(StyledString::new(), |mut acc, s| {
            acc.append(s);
            acc
        })
}

/// Decode an HTML-encoded string. Handles named entities (`&amp;`,
/// `&lt;`, `&gt;`) and numeric/hex entities (`&#x27;` etc.). Plain text
/// passes through unchanged.
///
/// ```
/// use hackernews_tim::utils::decode_html;
/// assert_eq!(decode_html("&amp;&lt;&gt;"), "&<>");
/// assert_eq!(decode_html("&#x27;"), "'");
/// assert_eq!(decode_html("hello world"), "hello world");
/// ```
pub fn decode_html(s: &str) -> String {
    html_escape::decode_html_entities(s).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- get_time_offset_in_text (private) ---

    #[test]
    fn time_offset_seconds_branch() {
        assert_eq!(get_time_offset_in_text(0), "0 seconds");
        assert_eq!(get_time_offset_in_text(1), "1 second");
        assert_eq!(get_time_offset_in_text(59), "59 seconds");
    }

    #[test]
    fn time_offset_minutes_branch() {
        // 60s is the boundary into the minutes branch.
        assert_eq!(get_time_offset_in_text(60), "1 minute");
        assert_eq!(get_time_offset_in_text(120), "2 minutes");
        assert_eq!(get_time_offset_in_text(60 * 60 - 1), "59 minutes");
    }

    #[test]
    fn time_offset_hours_branch() {
        assert_eq!(get_time_offset_in_text(60 * 60), "1 hour");
        assert_eq!(get_time_offset_in_text(2 * 60 * 60), "2 hours");
    }

    #[test]
    fn time_offset_days_branch() {
        let day = 60 * 60 * 24;
        assert_eq!(get_time_offset_in_text(day), "1 day");
        assert_eq!(get_time_offset_in_text(2 * day), "2 days");
    }

    #[test]
    fn time_offset_months_branch() {
        let month = 60 * 60 * 24 * 30;
        assert_eq!(get_time_offset_in_text(month), "1 month");
        assert_eq!(get_time_offset_in_text(2 * month), "2 months");
    }

    #[test]
    fn time_offset_years_branch() {
        let year = 60 * 60 * 24 * 365;
        assert_eq!(get_time_offset_in_text(year), "1 year");
        assert_eq!(get_time_offset_in_text(3 * year), "3 years");
    }

    // --- get_elapsed_time_as_text smoke test (uses SystemTime::now) ---

    #[test]
    fn elapsed_time_against_now_yields_seconds() {
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let text = get_elapsed_time_as_text(now);
        assert!(
            text.ends_with("second") || text.ends_with("seconds"),
            "expected a 'seconds' bucket for now-relative input; got {text:?}"
        );
    }

    // --- from_day_offset_to_time_offset_in_secs ---

    #[test]
    fn day_offset_zero_is_zero_seconds() {
        assert_eq!(from_day_offset_to_time_offset_in_secs(0), 0);
    }

    #[test]
    fn day_offset_one_is_a_day_in_seconds() {
        assert_eq!(from_day_offset_to_time_offset_in_secs(1), 86_400);
    }

    #[test]
    fn day_offset_seven_is_a_week_in_seconds() {
        assert_eq!(from_day_offset_to_time_offset_in_secs(7), 604_800);
    }

    // --- shorten_url ---

    #[test]
    fn short_url_returned_unchanged() {
        let url = "https://example.com/short";
        assert_eq!(shorten_url(url), url);
    }

    #[test]
    fn fifty_char_url_returned_unchanged() {
        // The cutoff is `> 50`, so a 50-char URL must come back as-is.
        let url = "a".repeat(50);
        assert_eq!(shorten_url(&url), url);
    }

    #[test]
    fn long_url_keeps_prefix_ellipsis_suffix() {
        // 60 chars trips shortening to `first40 + "..." + last10`.
        let url = "a".repeat(40) + &"b".repeat(20);
        let shortened = shorten_url(&url);
        assert_eq!(shortened.len(), 40 + 3 + 10);
        assert!(shortened.starts_with(&"a".repeat(40)));
        assert!(shortened.contains("..."));
        assert!(shortened.ends_with(&"b".repeat(10)));
    }

    // --- decode_html ---

    #[test]
    fn decode_html_named_entities() {
        assert_eq!(decode_html("&amp;&lt;&gt;"), "&<>");
    }

    #[test]
    fn decode_html_numeric_entity() {
        assert_eq!(decode_html("&#x27;"), "'");
    }

    #[test]
    fn decode_html_passes_plain_text_through() {
        assert_eq!(decode_html("hello world"), "hello world");
    }

    // --- combine_styled_strings ---

    #[test]
    fn combine_styled_strings_concatenates_in_order() {
        let a = StyledString::plain("hello ");
        let b = StyledString::plain("world");
        let combined = combine_styled_strings(vec![a, b]);
        assert_eq!(combined.source(), "hello world");
    }

    #[test]
    fn combine_styled_strings_handles_empty_input() {
        let combined: StyledString = combine_styled_strings(Vec::<StyledString>::new());
        assert_eq!(combined.source(), "");
    }
}
