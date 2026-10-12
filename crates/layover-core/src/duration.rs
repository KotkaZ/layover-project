//! Durations written the way schedules are: `30s`, `15m`, `4h`, `2d`.
//!
//! Every key that holds a length of time used to take a bare number in a unit its name carried —
//! `timeout_sec`, `window_hours`, `keep_lapsed_days` — while a schedule took `every = "45m"`. So a
//! four-hour timeout was `14400`, and reading it back meant doing the arithmetic. Each of those
//! keys now also answers to a name without the unit — `timeout`, `window`, `keep_lapsed` — and
//! either name takes either form: a duration such as `"4h"`, or a bare number, which is read in the
//! unit of the original key. A key whose unit is hours or days takes only a duration that is a
//! whole number of them, rather than rounding one somebody wrote on purpose.

use std::fmt;
use std::time::Duration;

use serde::Deserializer;
use serde::de::{self, Visitor};

/// Parses a duration such as `30s`, `15m`, `4h` or `2d`.
///
/// # Errors
///
/// Returns a sentence saying what was expected when `text` is not one.
pub fn parse(text: &str) -> Result<Duration, String> {
    let trimmed = text.trim();
    let bad = || {
        format!("could not read `{text}` as a duration; expected a number followed by s, m, h or d")
    };

    // Split on the last *character* rather than the last byte, so a stray multi-byte character
    // produces an error instead of a panic.
    let (digits, unit) = match trimmed.char_indices().next_back() {
        Some((index, unit)) => (&trimmed[..index], unit),
        None => return Err(bad()),
    };

    let multiplier = match unit {
        's' => 1_u64,
        'm' => 60,
        'h' => 60 * 60,
        'd' => 24 * 60 * 60,
        _ => return Err(bad()),
    };

    let amount: u64 = digits.parse().map_err(|_| bad())?;
    amount
        .checked_mul(multiplier)
        .map(Duration::from_secs)
        .ok_or_else(bad)
}

/// A whole number of `unit`-second units, from a bare number in that unit or a duration that is a
/// whole number of them.
struct InUnits {
    unit: u64,
    name: &'static str,
}

impl Visitor<'_> for InUnits {
    type Value = u64;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "a duration such as \"4h\", or a number of {}", self.name)
    }

    fn visit_u64<E: de::Error>(self, value: u64) -> Result<u64, E> {
        Ok(value)
    }

    fn visit_i64<E: de::Error>(self, value: i64) -> Result<u64, E> {
        u64::try_from(value).map_err(|_| E::custom(format!("{value} is negative")))
    }

    fn visit_str<E: de::Error>(self, text: &str) -> Result<u64, E> {
        let seconds = parse(text).map_err(E::custom)?.as_secs();
        if seconds % self.unit != 0 {
            return Err(E::custom(format!(
                "`{text}` is not a whole number of {}",
                self.name
            )));
        }
        Ok(seconds / self.unit)
    }
}

fn in_units<'de, D: Deserializer<'de>>(
    deserializer: D,
    unit: u64,
    name: &'static str,
) -> Result<u64, D::Error> {
    deserializer.deserialize_any(InUnits { unit, name })
}

/// Seconds, from `"4h"` or a bare number of seconds.
///
/// # Errors
///
/// Returns the deserializer's error for anything else.
pub fn seconds<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u64, D::Error> {
    in_units(deserializer, 1, "seconds")
}

/// Seconds, when the key is optional.
///
/// # Errors
///
/// Returns the deserializer's error for anything else.
pub fn optional_seconds<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<u64>, D::Error> {
    seconds(deserializer).map(Some)
}

/// Whole hours, from `"36h"`, `"2d"` or a bare number of hours.
///
/// # Errors
///
/// Returns the deserializer's error for anything else, including a duration that is not a whole
/// number of hours.
pub fn hours<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u64, D::Error> {
    in_units(deserializer, 3_600, "hours")
}

/// Whole days, from `"14d"` or a bare number of days.
///
/// # Errors
///
/// Returns the deserializer's error for anything else, including a duration that is not a whole
/// number of days, or more days than fit.
pub fn days<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u32, D::Error> {
    let days = in_units(deserializer, 86_400, "days")?;
    u32::try_from(days).map_err(|_| de::Error::custom(format!("{days} days is too many")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_duration_is_a_number_and_a_unit() {
        assert_eq!(parse("30s"), Ok(Duration::from_secs(30)));
        assert_eq!(parse("15m"), Ok(Duration::from_mins(15)));
        assert_eq!(parse(" 4h "), Ok(Duration::from_hours(4)));
        assert_eq!(parse("2d"), Ok(Duration::from_hours(48)));
        for bad in ["", "4", "h", "4w", "1h30m", "-4h", "4é"] {
            assert!(parse(bad).is_err(), "{bad:?} should not parse");
        }
    }
}
