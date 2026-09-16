//! Time windows to report cost over, and the difference between the two kinds.
//!
//! A dashboard asks for "the last 7 days" and "this month" in the same breath, which hides that
//! they are not the same sort of question:
//!
//! - **Rolling** windows are pure instant arithmetic. "The last 7 days" means the same thing
//!   everywhere on earth, and no time zone can make it wrong.
//! - **Calendar** windows are not. "This month" begins at local midnight on the first, and which
//!   instant that is depends on where the Tower is standing.
//!
//! Conflating them is not a theoretical hazard. A sibling project gated its daily spend on a UTC
//! boundary while reporting the ledger in local time, so for the hours between local midnight and
//! UTC midnight the budget it enforced and the number it displayed described different days. The
//! bug is invisible until it matters, at which point the factory has spent a day's money twice.
//!
//! So [`Window`] makes the distinction part of the type, and every [`Span`] carries the zone it
//! was reckoned in — `None` for rolling windows, because there was nothing to reckon.

use std::fmt;

use jiff::civil::Date;
use jiff::tz::TimeZone;
use jiff::{Timestamp, ToSpan, Zoned};

/// How long history is kept before it is deleted.
///
/// Reporting cannot see past this, and a window that reaches further back says so rather than
/// quietly returning a smaller number. See [`Span::truncated_by_retention`].
pub const RETENTION_DAYS: i32 = 90;

/// A period to total spend over.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Window {
    /// Since local midnight. Calendar.
    Today,
    /// The 24 hours ending now. Rolling.
    Last24Hours,
    /// The 7 days ending now. Rolling.
    Last7Days,
    /// The 30 days ending now. Rolling.
    Last30Days,
    /// The 90 days ending now, which is everything retention keeps. Rolling.
    Last90Days,
    /// Since local midnight on the first of the current month. Calendar.
    MonthToDate,
    /// Everything still on disk.
    AllTime,
}

impl Window {
    /// Every window, in the order a dashboard should offer them.
    ///
    /// Narrowest first: the question people ask most often is "what is it doing right now", and
    /// the further down the list you read the more you are asking about a trend.
    pub const ALL: [Self; 7] = [
        Self::Today,
        Self::Last24Hours,
        Self::Last7Days,
        Self::Last30Days,
        Self::MonthToDate,
        Self::Last90Days,
        Self::AllTime,
    ];

    /// The identifier used in query strings and JSON.
    #[must_use]
    pub fn slug(self) -> &'static str {
        match self {
            Self::Today => "today",
            Self::Last24Hours => "last_24h",
            Self::Last7Days => "last_7d",
            Self::Last30Days => "last_30d",
            Self::Last90Days => "last_90d",
            Self::MonthToDate => "month_to_date",
            Self::AllTime => "all_time",
        }
    }

    /// A human label.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Today => "Today",
            Self::Last24Hours => "Last 24 hours",
            Self::Last7Days => "Last 7 days",
            Self::Last30Days => "Last 30 days",
            Self::Last90Days => "Last 90 days",
            Self::MonthToDate => "Month to date",
            Self::AllTime => "All time",
        }
    }

    /// Returns `true` when the window's start depends on a time zone.
    ///
    /// Only calendar windows do. It is worth surfacing, because it decides whether two people
    /// comparing dashboards in different places should expect the same number.
    #[must_use]
    pub fn is_calendar(self) -> bool {
        matches!(self, Self::Today | Self::MonthToDate)
    }

    /// Parses a slug, as used in a query string.
    #[must_use]
    pub fn from_slug(slug: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|w| w.slug() == slug)
    }

    /// Resolves the window against a moment in a place.
    ///
    /// Calendar windows need the zone; rolling ones ignore it, and record that they did so.
    #[must_use]
    pub fn resolve(self, now: &Zoned) -> Span {
        let end = now.timestamp();
        let zone = now.time_zone().clone();

        let (start, reckoned_in) = match self {
            Self::Today => (Some(start_of_day(now)), Some(zone)),
            Self::MonthToDate => (Some(start_of_month(now)), Some(zone)),
            Self::Last24Hours => (Some(rolling_back(end, 24)), None),
            Self::Last7Days => (Some(rolling_back(end, 7 * 24)), None),
            Self::Last30Days => (Some(rolling_back(end, 30 * 24)), None),
            Self::Last90Days => (Some(rolling_back(end, 90 * 24)), None),
            Self::AllTime => (None, None),
        };

        Span {
            window: self,
            start,
            end,
            reckoned_in,
            horizon: rolling_back(end, RETENTION_DAYS * 24),
        }
    }
}

impl fmt::Display for Window {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// Steps `hours` back from an instant, saturating at the extreme rather than failing.
///
/// Hours, not days, and that is deliberate. `jiff` refuses to subtract days from a bare timestamp
/// because a day is not always 24 hours — across a DST transition it is 23 or 25 — and without a
/// zone there is no way to know which. Refusing is the right answer: a rolling window that
/// silently changed length twice a year would be a calendar window wearing a disguise. "The last
/// 7 days" here means 168 hours exactly, in every zone, which is what makes it comparable.
fn rolling_back(from: Timestamp, hours: i32) -> Timestamp {
    from.checked_sub(hours.hours()).unwrap_or(Timestamp::MIN)
}

/// Local midnight at the start of `now`'s day.
fn start_of_day(now: &Zoned) -> Timestamp {
    now.start_of_day()
        .map_or_else(|_| now.timestamp(), |zoned| zoned.timestamp())
}

/// Local midnight on the first of `now`'s month.
///
/// Falls back to the start of the current day if the first does not exist as a local midnight,
/// which real zones have done: Samoa skipped 30 December 2011 entirely when it changed side of
/// the date line.
fn start_of_month(now: &Zoned) -> Timestamp {
    let first = Date::new(now.year(), now.month(), 1).unwrap_or_else(|_| now.date());
    first
        .to_zoned(now.time_zone().clone())
        .map_or_else(|_| start_of_day(now), |zoned| zoned.timestamp())
}

/// A resolved [`Window`]: two instants, and an honest account of how they were arrived at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    /// The window this came from.
    pub window: Window,
    /// When the period starts. `None` means "from the beginning of what is kept".
    pub start: Option<Timestamp>,
    /// When the period ends, which is the moment it was resolved.
    pub end: Timestamp,
    /// The zone the start was reckoned in, or `None` for a rolling window.
    pub reckoned_in: Option<TimeZone>,
    /// The oldest instant retention still keeps.
    pub horizon: Timestamp,
}

impl Span {
    /// Returns `true` when `at` falls inside the period.
    ///
    /// Half-open: the start is included and the end is not, so consecutive windows tile without
    /// counting a run at the boundary twice.
    #[must_use]
    pub fn contains(&self, at: Timestamp) -> bool {
        self.start.is_none_or(|start| at >= start) && at < self.end
    }

    /// Returns `true` when the period reaches further back than retention keeps.
    ///
    /// A total over such a window is a lower bound, not a total, and saying so is the same
    /// principle that makes [`crate::cost::CostSource`] a field: a number whose provenance is
    /// unstated will eventually be trusted more than it deserves.
    #[must_use]
    pub fn truncated_by_retention(&self) -> bool {
        match self.window {
            // Nobody reading "all time" believes it predates the install.
            Window::AllTime => false,
            _ => self.start.is_some_and(|start| start < self.horizon),
        }
    }

    /// The name of the zone the start was reckoned in, when one was needed.
    #[must_use]
    pub fn zone_name(&self) -> Option<&str> {
        self.reckoned_in.as_ref().and_then(TimeZone::iana_name)
    }

    /// How long the period is, in whole seconds, or `None` for an open start.
    #[must_use]
    pub fn duration_secs(&self) -> Option<i64> {
        // Second arithmetic directly, rather than converting a `Span` through `f64`: a window can
        // be ninety days, and a float is a strange way to count something already an integer.
        Some(self.end.as_second() - self.start?.as_second())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fixed moment, well inside a month, in a zone an hour ahead of UTC.
    ///
    /// Deliberately after local midnight but before UTC midnight is irrelevant here; the case
    /// that matters is tested separately below.
    fn now() -> Zoned {
        "2026-09-16T13:10:00+03:00[Europe/Tallinn]"
            .parse()
            .expect("valid zoned timestamp")
    }

    #[test]
    fn rolling_windows_go_back_exactly_that_far() {
        let span = Window::Last7Days.resolve(&now());

        assert_eq!(span.duration_secs(), Some(7 * 24 * 3_600));
        assert!(!span.window.is_calendar());
    }

    #[test]
    fn a_calendar_month_starts_at_local_midnight_on_the_first() {
        let span = Window::MonthToDate.resolve(&now());
        let start = span.start.expect("month to date has a start");

        assert_eq!(
            start.to_string(),
            "2026-08-31T21:00:00Z",
            "local midnight on 1 September in UTC+3 is 21:00 the previous day in UTC"
        );
        assert_eq!(span.zone_name(), Some("Europe/Tallinn"));
    }

    #[test]
    fn a_calendar_window_reckoned_in_two_zones_covers_different_instants() {
        // This is the whole reason the distinction is in the type. The same instant, asked "when
        // did today begin?", gives two different answers, and a budget that gates on one while
        // reporting the other lets a day's money be spent twice.
        let tallinn = Window::Today.resolve(&now());
        let honolulu: Zoned = "2026-09-16T00:10:00-10:00[Pacific/Honolulu]"
            .parse()
            .expect("valid zoned timestamp");
        let pacific = Window::Today.resolve(&honolulu);

        assert_ne!(tallinn.start, pacific.start);
        assert_eq!(pacific.zone_name(), Some("Pacific/Honolulu"));
    }

    #[test]
    fn a_rolling_window_records_that_no_zone_was_involved() {
        // The absence is the point: nobody should have to wonder which zone "last 7 days" used.
        let span = Window::Last7Days.resolve(&now());

        assert!(span.reckoned_in.is_none());
        assert!(span.zone_name().is_none());
    }

    #[test]
    fn windows_are_half_open_so_consecutive_periods_do_not_double_count() {
        let span = Window::Last7Days.resolve(&now());
        let start = span.start.expect("rolling window has a start");

        assert!(span.contains(start), "the start is inside");
        assert!(!span.contains(span.end), "the end is not");
        assert!(!span.contains(start - 1.second()));
    }

    #[test]
    fn nothing_reaches_past_retention_except_the_window_that_says_so() {
        let at = now();

        assert!(!Window::Last30Days.resolve(&at).truncated_by_retention());
        assert!(
            !Window::Last90Days.resolve(&at).truncated_by_retention(),
            "ninety days is exactly what is kept, so it is complete"
        );
        assert!(
            !Window::AllTime.resolve(&at).truncated_by_retention(),
            "all time means all that is kept, and claiming otherwise would warn on every page load"
        );
    }

    #[test]
    fn a_month_to_date_window_is_truncated_only_once_it_outruns_retention() {
        // Month to date is the one window whose length is not fixed, so it is the only one that
        // can quietly become a lower bound. It cannot in practice — no month is 90 days — but the
        // check exists so that changing RETENTION_DAYS cannot silently produce a wrong total.
        let span = Window::MonthToDate.resolve(&now());
        assert!(!span.truncated_by_retention());

        let stretched = Span {
            start: Some(span.horizon - 1.second()),
            ..span
        };
        assert!(stretched.truncated_by_retention());
    }

    #[test]
    fn a_rolling_week_stays_168_hours_across_a_daylight_saving_change() {
        // Europe/Tallinn puts its clocks back on the last Sunday of October, making that local
        // day 25 hours long. A rolling week must not stretch with it: two operators comparing
        // "last 7 days" a day apart would otherwise be totalling different amounts of time and
        // calling the result by the same name.
        let across_dst: Zoned = "2026-10-26T12:00:00+02:00[Europe/Tallinn]"
            .parse()
            .expect("valid zoned timestamp");

        assert_eq!(
            Window::Last7Days.resolve(&across_dst).duration_secs(),
            Some(7 * 24 * 3_600)
        );
    }

    #[test]
    fn a_calendar_day_does_stretch_across_a_daylight_saving_change() {
        // And this is the contrast that justifies two kinds of window. At the same reading on the
        // same clock, more time has passed since midnight on the day the clocks went back — the
        // hour between 03:00 and 04:00 happened twice. Forcing that to 24 would be answering a
        // different question than the one asked.
        let ordinary: Zoned = "2026-10-20T23:00:00+03:00[Europe/Tallinn]"
            .parse()
            .expect("valid zoned timestamp");
        let transition: Zoned = "2026-10-25T23:00:00+02:00[Europe/Tallinn]"
            .parse()
            .expect("valid zoned timestamp");

        assert_eq!(
            Window::Today.resolve(&ordinary).duration_secs(),
            Some(23 * 3_600)
        );
        assert_eq!(
            Window::Today.resolve(&transition).duration_secs(),
            Some(24 * 3_600),
            "the clocks went back, so an extra hour has passed by the same reading"
        );
    }

    #[test]
    fn slugs_round_trip_and_are_unique() {
        let mut seen = Vec::new();
        for window in Window::ALL {
            assert_eq!(Window::from_slug(window.slug()), Some(window));
            assert!(
                !seen.contains(&window.slug()),
                "duplicate {}",
                window.slug()
            );
            seen.push(window.slug());
        }

        assert_eq!(Window::from_slug("fortnight"), None);
    }

    #[test]
    fn only_calendar_windows_need_a_zone() {
        for window in Window::ALL {
            let span = window.resolve(&now());
            assert_eq!(
                span.reckoned_in.is_some(),
                window.is_calendar(),
                "{window} disagrees about whether it used a zone"
            );
        }
    }

    #[test]
    fn an_open_window_contains_everything_up_to_now() {
        let span = Window::AllTime.resolve(&now());

        assert!(span.start.is_none());
        assert!(span.duration_secs().is_none());
        assert!(span.contains(Timestamp::MIN));
    }
}
