//! `[update] window`: when an unattended install may begin, in the machine's local time.
//!
//! Written `"Sat 02:00-06:00"`, `"Mon-Fri 22:00-06:00"`, `"Sat,Sun 01:00-05:00"`, or with no days —
//! `"02:00-04:00"` — for every day. A window that ends before it starts runs past midnight into the
//! next day, which belongs to the day it started on.

use std::fmt;

use jiff::civil::{Time, Weekday};
use jiff::{Timestamp, Zoned};
use serde::{Deserialize, Deserializer};

/// When unattended installs may begin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UpdateWindow {
    /// The days it opens on, Monday first.
    days: [bool; 7],
    /// When it opens, in minutes after midnight.
    start: u16,
    /// When it closes, in minutes after midnight; at or before `start` for one running past
    /// midnight.
    end: u16,
}

const NAMES: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

fn day_index(weekday: Weekday) -> usize {
    usize::try_from(weekday.to_monday_zero_offset()).unwrap_or(0)
}

fn minutes(time: Time) -> u16 {
    u16::try_from(i32::from(time.hour()) * 60 + i32::from(time.minute())).unwrap_or(0)
}

impl UpdateWindow {
    /// Reads `"Sat 02:00-06:00"` and the other forms the module describes.
    ///
    /// # Errors
    ///
    /// Says what could not be read.
    pub fn parse(text: &str) -> Result<Self, String> {
        let bad = |why: &str| {
            format!(
                "could not read `{text}` as a window ({why}); write it like \"Sat 02:00-06:00\", \
                 \"Mon-Fri 22:00-06:00\" or \"02:00-04:00\""
            )
        };
        let text = text.trim();
        let (days, times) = match text.rsplit_once(' ') {
            Some((days, times)) => (Some(days.trim()), times),
            None => (None, text),
        };
        let (from, to) = times
            .split_once('-')
            .ok_or_else(|| bad("no `-` between the times"))?;
        let clock = |part: &str| -> Result<u16, String> {
            let (hours, mins) = part
                .split_once(':')
                .ok_or_else(|| bad("a time is written HH:MM"))?;
            let hours: u16 = hours.parse().map_err(|_| bad("an hour is not a number"))?;
            let mins: u16 = mins.parse().map_err(|_| bad("a minute is not a number"))?;
            if hours > 24 || mins > 59 || (hours == 24 && mins > 0) {
                return Err(bad("a time is past midnight"));
            }
            Ok(hours * 60 + mins)
        };
        let start = clock(from)?;
        let end = clock(to)? % (24 * 60);
        if start == end {
            return Err(bad("it opens and closes at the same time"));
        }
        let mut chosen = [false; 7];
        match days {
            None => chosen = [true; 7],
            Some(days) if days.eq_ignore_ascii_case("daily") => chosen = [true; 7],
            Some(days) => {
                let index = |name: &str| {
                    NAMES
                        .iter()
                        .position(|day| day.eq_ignore_ascii_case(name.trim()))
                        .ok_or_else(|| bad("a day is written Mon, Tue … Sun"))
                };
                for part in days.split(',') {
                    match part.split_once('-') {
                        Some((first, last)) => {
                            let (first, last) = (index(first)?, index(last)?);
                            let mut day = first;
                            loop {
                                chosen[day] = true;
                                if day == last {
                                    break;
                                }
                                day = (day + 1) % 7;
                            }
                        }
                        None => chosen[index(part)?] = true,
                    }
                }
            }
        }
        Ok(Self {
            days: chosen,
            start,
            end,
        })
    }

    /// When the occurrence of the window that `now` falls in closes; `None` when `now` is outside
    /// it.
    #[must_use]
    pub fn open_until(&self, now: &Zoned) -> Option<Timestamp> {
        let at = minutes(now.time());
        let today = day_index(now.weekday());
        let yesterday = (today + 6) % 7;
        let midnight = now.start_of_day().ok()?;
        let close = |days_ahead: i64| {
            midnight
                .checked_add(
                    jiff::Span::new()
                        .days(days_ahead)
                        .minutes(i64::from(self.end)),
                )
                .ok()
                .map(|zoned| zoned.timestamp())
        };
        if self.start < self.end {
            (self.days[today] && self.start <= at && at < self.end).then(|| close(0))?
        } else if self.days[today] && at >= self.start {
            close(1)
        } else if self.days[yesterday] && at < self.end {
            close(0)
        } else {
            None
        }
    }

    /// How long the window is open each time, in minutes.
    #[must_use]
    pub fn length_minutes(&self) -> u16 {
        if self.start < self.end {
            self.end - self.start
        } else {
            24 * 60 - self.start + self.end
        }
    }
}

impl fmt::Display for UpdateWindow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.days != [true; 7] {
            let named: Vec<&str> = NAMES
                .iter()
                .zip(self.days)
                .filter_map(|(name, chosen)| chosen.then_some(*name))
                .collect();
            write!(f, "{} ", named.join(","))?;
        }
        write!(
            f,
            "{:02}:{:02}-{:02}:{:02}",
            self.start / 60,
            self.start % 60,
            self.end / 60,
            self.end % 60
        )
    }
}

impl<'de> Deserialize<'de> for UpdateWindow {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Self::parse(&text).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str) -> Zoned {
        // A fixed zone, so the test means the same everywhere.
        format!("{text}[UTC]").parse().expect("valid")
    }

    #[test]
    fn a_window_reads_as_it_is_written() {
        let window = UpdateWindow::parse("Sat 02:00-06:00").expect("reads");
        assert_eq!(window.to_string(), "Sat 02:00-06:00");
        assert_eq!(window.length_minutes(), 240);
        assert_eq!(
            UpdateWindow::parse("02:00-04:00")
                .expect("reads")
                .to_string(),
            "02:00-04:00"
        );
        assert_eq!(
            UpdateWindow::parse("Fri-Mon 22:00-06:00")
                .expect("reads")
                .to_string(),
            "Mon,Fri,Sat,Sun 22:00-06:00"
        );
        for bad in [
            "",
            "Sat",
            "Sat 2-6",
            "Someday 02:00-06:00",
            "25:00-26:00",
            "02:00-02:00",
        ] {
            assert!(UpdateWindow::parse(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn a_window_is_open_on_its_days_between_its_times() {
        let window = UpdateWindow::parse("Sat 02:00-06:00").expect("reads");
        // 17 October 2026 is a Saturday.
        assert_eq!(
            window.open_until(&at("2026-10-17T03:30:00Z")),
            Some("2026-10-17T06:00:00Z".parse().expect("valid"))
        );
        assert_eq!(window.open_until(&at("2026-10-17T06:00:00Z")), None);
        assert_eq!(window.open_until(&at("2026-10-17T01:59:00Z")), None);
        assert_eq!(
            window.open_until(&at("2026-10-18T03:00:00Z")),
            None,
            "Sunday"
        );
    }

    #[test]
    fn a_window_past_midnight_belongs_to_the_day_it_opened() {
        let window = UpdateWindow::parse("Fri 22:00-02:00").expect("reads");
        // Friday 16 October, late; then early Saturday.
        assert_eq!(
            window.open_until(&at("2026-10-16T23:00:00Z")),
            Some("2026-10-17T02:00:00Z".parse().expect("valid"))
        );
        assert_eq!(
            window.open_until(&at("2026-10-17T01:00:00Z")),
            Some("2026-10-17T02:00:00Z".parse().expect("valid"))
        );
        assert_eq!(
            window.open_until(&at("2026-10-17T23:00:00Z")),
            None,
            "Saturday night"
        );
        assert_eq!(window.length_minutes(), 240);
    }
}
