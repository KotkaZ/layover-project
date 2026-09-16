//! Pipelines: named, triggerable entry points that carry flags.
//!
//! A route map says which agents *may* talk to each other. A pipeline says how work *enters* the
//! mesh: which agent receives it, whether a human starts it or a clock does, and which boolean
//! flags the run is parameterised by.
//!
//! Pipelines are deliberately thin. They do not describe a sequence of steps — agents still decide
//! where work goes next — so adding one does not turn the permission mesh into a pipeline engine.
//!
//! # Safety
//!
//! A schedule is the one part of Layover that starts work with no human present, so the floor on
//! how often it may fire is enforced here rather than left to the author. See
//! [`Schedule::MIN_INTERVAL_SECS`].

use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::agent::AgentName;

/// The name of a pipeline, as written in `layover.toml`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize, Serialize)]
#[serde(transparent)]
pub struct PipelineName(String);

impl PipelineName {
    /// Creates a pipeline name.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    /// Returns the name as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PipelineName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<&str> for PipelineName {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

/// When a scheduled pipeline fires.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Schedule {
    /// A fixed interval between runs.
    Every(Duration),
    /// A five-field cron expression, evaluated in the Tower's local time zone.
    Cron(String),
}

impl Schedule {
    /// The shortest interval a schedule may declare.
    ///
    /// Every firing is a real, paid CLI invocation, and a schedule runs with nobody watching. A
    /// minute is the floor because it is the finest granularity a five-field cron expression can
    /// express, so allowing anything shorter would make `every` and `cron` disagree about what is
    /// possible.
    pub const MIN_INTERVAL_SECS: u64 = 60;

    /// Returns the fixed interval, if this schedule is one.
    #[must_use]
    pub fn interval(&self) -> Option<Duration> {
        match self {
            Self::Every(duration) => Some(*duration),
            Self::Cron(_) => None,
        }
    }

    /// Returns a lower bound on the gap between firings, in seconds, when one can be established.
    ///
    /// For a fixed interval this is exact. For a cron expression it is read off the minute field,
    /// and only when that field states the answer unambiguously: `*` fires every minute and
    /// `*/n` every `n` minutes. Lists and ranges are left alone rather than guessed at, because a
    /// wrong lower bound here means a warning that is not true, and a validator that cries wolf is
    /// one people stop reading.
    #[must_use]
    pub fn min_gap_secs(&self) -> Option<u64> {
        match self {
            Self::Every(duration) => Some(duration.as_secs()),
            Self::Cron(expression) => {
                let minutes = expression.split_whitespace().next()?;
                if minutes == "*" {
                    return Some(60);
                }
                let step: u64 = minutes.strip_prefix("*/")?.parse().ok()?;
                step.checked_mul(60)
            }
        }
    }
}

impl fmt::Display for Schedule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Every(duration) => write!(f, "every {}s", duration.as_secs()),
            Self::Cron(expression) => write!(f, "cron `{expression}`"),
        }
    }
}

/// What starts a pipeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Trigger {
    /// A human sends the first flight.
    Manual,
    /// The Tower sends the first flight on a clock.
    Scheduled(Schedule),
}

impl fmt::Display for Trigger {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Manual => f.write_str("manual"),
            Self::Scheduled(schedule) => write!(f, "{schedule}"),
        }
    }
}

impl Trigger {
    /// Returns the schedule, if this trigger has one.
    #[must_use]
    pub fn schedule(&self) -> Option<&Schedule> {
        match self {
            Self::Manual => None,
            Self::Scheduled(schedule) => Some(schedule),
        }
    }

    /// Returns `true` when only a human can start this pipeline.
    #[must_use]
    pub fn is_manual(&self) -> bool {
        matches!(self, Self::Manual)
    }
}

/// A boolean parameter a pipeline accepts at trigger time.
///
/// Flags are how one factory definition serves several situations without duplicating prompts:
/// a tester that also runs a remote end-to-end suite is the same agent with one extra paragraph
/// of instructions. See [`crate::prompt`].
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FlagSpec {
    /// Value used when the trigger does not set the flag.
    #[serde(default)]
    pub default: bool,
    /// What turning this flag on actually does.
    #[serde(default)]
    pub description: Option<String>,
}

/// How an itinerary's workspace relates to other itineraries'.
///
/// A pipeline that can have several instances in flight — one per pull request, say — needs each
/// to work somewhere of its own, or two `read-write` agents in two unrelated itineraries will
/// clobber each other in the shared directory.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Workspace {
    /// Every itinerary works in the one shared `work_dir`.
    ///
    /// The default, because it is what a single-instance pipeline wants and because a worktree
    /// per itinerary costs disk and setup time.
    #[default]
    Shared,
    /// Each itinerary gets its own git worktree, named after the itinerary.
    ///
    /// This is what makes several instances of one pipeline safe to run at once.
    PerItinerary,
}

impl Workspace {
    /// Returns `true` when each itinerary is isolated from the others.
    #[must_use]
    pub fn is_isolated(&self) -> bool {
        matches!(self, Self::PerItinerary)
    }
}

/// A named entry point into the mesh.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pipeline {
    /// One line saying what this pipeline is for.
    #[serde(default)]
    pub description: Option<String>,
    /// The agent that receives the first flight.
    pub entry: AgentName,
    /// What starts it.
    #[serde(default = "manual_trigger")]
    pub trigger: Trigger,
    /// Whether instances of this pipeline share a workspace or get one each.
    #[serde(default)]
    pub workspace: Workspace,
    /// Whether this pipeline picks up booked layovers rather than starting fresh work.
    ///
    /// A resuming pipeline does not open an itinerary on every tick. It looks for work that was
    /// set down and is now due, and opens one seeded with what the earlier chain knew. A tick
    /// that finds nothing due costs nothing, which is what makes checking every twenty minutes
    /// affordable.
    #[serde(default)]
    pub resumes: bool,
    /// Boolean parameters this pipeline accepts, keyed by flag name.
    #[serde(default)]
    pub flags: BTreeMap<String, FlagSpec>,
}

impl Pipeline {
    /// Resolves the flag values for one run, filling in declared defaults.
    ///
    /// # Errors
    ///
    /// Returns [`FlagError::Undeclared`] if `overrides` names a flag this pipeline does not
    /// declare. Silently ignoring it would let a typo at trigger time change nothing while
    /// appearing to work.
    pub fn flags_for_run(&self, overrides: &BTreeMap<String, bool>) -> Result<Flags, FlagError> {
        if let Some(unknown) = overrides.keys().find(|key| !self.flags.contains_key(*key)) {
            return Err(FlagError::Undeclared {
                flag: unknown.clone(),
            });
        }

        let values = self
            .flags
            .iter()
            .map(|(name, spec)| {
                let value = overrides.get(name).copied().unwrap_or(spec.default);
                (name.clone(), value)
            })
            .collect();

        Ok(Flags(values))
    }
}

/// The resolved boolean parameters of a single run.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Flags(BTreeMap<String, bool>);

impl Flags {
    /// Builds a flag set directly, for tests and for triggers that declare no pipeline.
    #[must_use]
    pub fn new(values: BTreeMap<String, bool>) -> Self {
        Self(values)
    }

    /// Returns the value of `name`, or `None` when the flag was never declared.
    ///
    /// An undeclared flag is deliberately distinct from one that is declared and false: a prompt
    /// referring to a flag nobody declared is a mistake, not a false condition.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<bool> {
        self.0.get(name).copied()
    }

    /// Returns every declared flag and its value.
    pub fn iter(&self) -> impl Iterator<Item = (&str, bool)> {
        self.0.iter().map(|(name, value)| (name.as_str(), *value))
    }

    /// Returns `true` when no flags are declared.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// Why a flag could not be resolved.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FlagError {
    /// The trigger set a flag the pipeline does not declare.
    #[error("flag `{flag}` is not declared by this pipeline")]
    Undeclared {
        /// The offending flag name.
        flag: String,
    },
}

/// Why a trigger could not be understood.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TriggerError {
    /// The trigger keyword was not one Layover knows.
    #[error(
        "unknown trigger `{found}`; expected `manual`, `{{ every = \"1h\" }}` or `{{ cron = \"0 * * * *\" }}`"
    )]
    UnknownKeyword {
        /// What was written.
        found: String,
    },
    /// An `every` value was not a duration.
    #[error("could not read `{found}` as a duration; expected a number followed by s, m, h or d")]
    BadDuration {
        /// What was written.
        found: String,
    },
    /// An `every` value was shorter than the floor.
    #[error(
        "`every = \"{found}\"` fires more often than once every {minimum}s, which is not allowed \
         for unattended work"
    )]
    TooFrequent {
        /// What was written.
        found: String,
        /// The floor that was breached.
        minimum: u64,
    },
    /// A cron expression did not parse.
    #[error("could not read `{found}` as a cron expression: {reason}")]
    BadCron {
        /// What was written.
        found: String,
        /// Why the parser rejected it.
        reason: String,
    },
    /// A cron expression carried a seconds field.
    #[error(
        "cron expression `{found}` has {fields} fields; Layover accepts five-field expressions \
         only, because a seconds field can schedule work faster than a run can finish"
    )]
    SubMinuteCron {
        /// What was written.
        found: String,
        /// How many fields it had.
        fields: usize,
    },
    /// Both `every` and `cron` were given.
    #[error("a trigger sets both `every` and `cron`; give exactly one")]
    AmbiguousSchedule,
    /// A trigger table was empty.
    #[error("a trigger table sets neither `every` nor `cron`")]
    EmptySchedule,
}

const fn manual_trigger() -> Trigger {
    Trigger::Manual
}

/// Parses a duration such as `30s`, `15m`, `1h` or `2d`.
fn parse_duration(text: &str) -> Result<Duration, TriggerError> {
    let trimmed = text.trim();
    let bad = || TriggerError::BadDuration {
        found: text.to_owned(),
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

/// Validates a cron expression, rejecting anything finer than a minute.
fn parse_cron(expression: &str) -> Result<String, TriggerError> {
    let fields = expression.split_whitespace().count();
    if fields > 5 {
        return Err(TriggerError::SubMinuteCron {
            found: expression.to_owned(),
            fields,
        });
    }

    croner::Cron::from_str(expression).map_err(|error| TriggerError::BadCron {
        found: expression.to_owned(),
        reason: error.to_string(),
    })?;

    Ok(expression.to_owned())
}

/// The table form of a trigger, once its fields are known.
fn trigger_from_parts(
    every: Option<String>,
    cron: Option<String>,
) -> Result<Trigger, TriggerError> {
    match (every, cron) {
        (Some(every), None) => {
            let duration = parse_duration(&every)?;
            if duration.as_secs() < Schedule::MIN_INTERVAL_SECS {
                return Err(TriggerError::TooFrequent {
                    found: every,
                    minimum: Schedule::MIN_INTERVAL_SECS,
                });
            }
            Ok(Trigger::Scheduled(Schedule::Every(duration)))
        }
        (None, Some(cron)) => Ok(Trigger::Scheduled(Schedule::Cron(parse_cron(&cron)?))),
        (Some(_), Some(_)) => Err(TriggerError::AmbiguousSchedule),
        (None, None) => Err(TriggerError::EmptySchedule),
    }
}

impl<'de> Deserialize<'de> for Trigger {
    /// Accepts `"manual"` or a table with exactly one of `every` and `cron`.
    ///
    /// Hand-written rather than an untagged enum: untagged loses the inner error and reports only
    /// "data did not match any variant", which turns a one-character typo into a puzzle. A
    /// factory definition should say what is wrong with it while a human is still watching.
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct TriggerVisitor;

        impl<'de> serde::de::Visitor<'de> for TriggerVisitor {
            type Value = Trigger;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(r#""manual", { every = "1h" } or { cron = "0 * * * *" }"#)
            }

            fn visit_str<E>(self, value: &str) -> Result<Trigger, E>
            where
                E: serde::de::Error,
            {
                if value == "manual" {
                    return Ok(Trigger::Manual);
                }
                Err(E::custom(TriggerError::UnknownKeyword {
                    found: value.to_owned(),
                }))
            }

            fn visit_map<M>(self, mut map: M) -> Result<Trigger, M::Error>
            where
                M: serde::de::MapAccess<'de>,
            {
                const FIELDS: &[&str] = &["every", "cron"];

                let mut every: Option<String> = None;
                let mut cron: Option<String> = None;

                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "every" if every.is_some() => {
                            return Err(serde::de::Error::duplicate_field("every"));
                        }
                        "every" => every = Some(map.next_value()?),
                        "cron" if cron.is_some() => {
                            return Err(serde::de::Error::duplicate_field("cron"));
                        }
                        "cron" => cron = Some(map.next_value()?),
                        other => return Err(serde::de::Error::unknown_field(other, FIELDS)),
                    }
                }

                trigger_from_parts(every, cron).map_err(serde::de::Error::custom)
            }
        }

        deserializer.deserialize_any(TriggerVisitor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pipeline(body: &str) -> Pipeline {
        toml::from_str(body).expect("pipeline parses")
    }

    fn trigger_error(body: &str) -> String {
        toml::from_str::<Pipeline>(body)
            .expect_err("trigger must be rejected")
            .to_string()
    }

    #[test]
    fn a_pipeline_defaults_to_manual() {
        let pipeline = pipeline(r#"entry = "analyst""#);

        assert_eq!(pipeline.trigger, Trigger::Manual);
        assert!(pipeline.trigger.is_manual());
        assert!(pipeline.trigger.schedule().is_none());
    }

    #[test]
    fn manual_may_be_stated_explicitly() {
        let pipeline = pipeline(
            r#"
            entry = "analyst"
            trigger = "manual"
            "#,
        );

        assert_eq!(pipeline.trigger, Trigger::Manual);
    }

    #[test]
    fn an_interval_schedule_is_parsed() {
        let pipeline = pipeline(
            r#"
            entry = "pr_scanner"
            trigger = { every = "1h" }
            "#,
        );

        assert_eq!(
            pipeline.trigger,
            Trigger::Scheduled(Schedule::Every(Duration::from_secs(3_600)))
        );
        assert_eq!(
            pipeline.trigger.schedule().and_then(Schedule::interval),
            Some(Duration::from_secs(3_600))
        );
    }

    #[test]
    fn every_unit_is_understood() {
        assert_eq!(parse_duration("90s"), Ok(Duration::from_secs(90)));
        assert_eq!(parse_duration("15m"), Ok(Duration::from_secs(900)));
        assert_eq!(parse_duration("2h"), Ok(Duration::from_secs(7_200)));
        assert_eq!(parse_duration("1d"), Ok(Duration::from_secs(86_400)));
    }

    #[test]
    fn a_malformed_duration_is_rejected() {
        assert!(parse_duration("soon").is_err());
        assert!(parse_duration("10").is_err());
        assert!(parse_duration("").is_err());
        assert!(parse_duration("1w").is_err());
        assert!(parse_duration("-5m").is_err());
        assert!(
            parse_duration("1é").is_err(),
            "must not panic on a multi-byte tail"
        );
        assert!(parse_duration("99999999999999999999d").is_err());
    }

    #[test]
    fn a_trigger_setting_both_forms_is_refused() {
        let message = trigger_error(
            r#"
            entry = "pr_scanner"
            trigger = { every = "1h", cron = "0 * * * *" }
            "#,
        );

        assert!(
            message.contains("both"),
            "a trigger must not silently pick one of two schedules, got: {message}"
        );
    }

    #[test]
    fn an_empty_trigger_table_is_refused() {
        let message = trigger_error(
            r#"
            entry = "pr_scanner"
            trigger = {}
            "#,
        );

        assert!(message.contains("neither"), "got: {message}");
    }

    #[test]
    fn an_unknown_trigger_field_is_refused_by_name() {
        // A one-character typo must say which character. An untagged enum would report only
        // "data did not match any variant", which is why this is deserialised by hand.
        let message = trigger_error(
            r#"
            entry = "pr_scanner"
            trigger = { evry = "1h" }
            "#,
        );

        assert!(
            message.contains("evry"),
            "the error must name the offending field, got: {message}"
        );
        assert!(
            message.contains("every") && message.contains("cron"),
            "the error must list what was expected, got: {message}"
        );
    }

    #[test]
    fn a_duplicate_trigger_field_is_refused() {
        let message = trigger_error(
            r#"
            entry = "pr_scanner"
            trigger = { every = "1h", every = "2h" }
            "#,
        );

        assert!(!message.is_empty(), "got: {message}");
    }

    #[test]
    fn a_schedule_faster_than_the_floor_is_refused() {
        let message = trigger_error(
            r#"
            entry = "pr_scanner"
            trigger = { every = "30s" }
            "#,
        );

        assert!(
            message.contains("fires more often"),
            "expected a frequency refusal, got: {message}"
        );
    }

    #[test]
    fn a_cron_schedule_is_validated_at_load_time() {
        let pipeline = pipeline(
            r#"
            entry = "pr_scanner"
            trigger = { cron = "0 * * * *" }
            "#,
        );

        assert_eq!(
            pipeline.trigger,
            Trigger::Scheduled(Schedule::Cron("0 * * * *".to_owned()))
        );
    }

    #[test]
    fn a_nonsense_cron_expression_is_refused() {
        let message = trigger_error(
            r#"
            entry = "pr_scanner"
            trigger = { cron = "every hour please" }
            "#,
        );

        assert!(
            message.contains("cron expression"),
            "expected a cron refusal, got: {message}"
        );
    }

    #[test]
    fn a_six_field_cron_expression_is_refused() {
        // A seconds field can schedule work faster than a run can finish, which is a fork bomb
        // with a clock attached.
        let message = trigger_error(
            r#"
            entry = "pr_scanner"
            trigger = { cron = "*/5 * * * * *" }
            "#,
        );

        assert!(
            message.contains("five-field"),
            "expected a granularity refusal, got: {message}"
        );
    }

    #[test]
    fn an_unknown_trigger_keyword_is_refused() {
        let message = trigger_error(
            r#"
            entry = "pr_scanner"
            trigger = "whenever"
            "#,
        );

        assert!(message.contains("unknown trigger"));
    }

    #[test]
    fn flags_fall_back_to_their_declared_defaults() {
        let pipeline = pipeline(
            r#"
            entry = "analyst"

            [flags]
            run_e2e = { default = false, description = "Run the remote suite" }
            verbose = { default = true }
            "#,
        );

        let flags = pipeline
            .flags_for_run(&BTreeMap::new())
            .expect("no overrides is always valid");

        assert_eq!(flags.get("run_e2e"), Some(false));
        assert_eq!(flags.get("verbose"), Some(true));
        assert_eq!(flags.get("undeclared"), None);
        assert!(!flags.is_empty());
    }

    #[test]
    fn a_trigger_override_wins_over_the_default() {
        let pipeline = pipeline(
            r#"
            entry = "analyst"

            [flags]
            run_e2e = { default = false }
            "#,
        );

        let overrides = BTreeMap::from([("run_e2e".to_owned(), true)]);
        let flags = pipeline.flags_for_run(&overrides).expect("declared flag");

        assert_eq!(flags.get("run_e2e"), Some(true));
    }

    #[test]
    fn setting_an_undeclared_flag_is_an_error() {
        // A typo at trigger time would otherwise change nothing while appearing to work.
        let pipeline = pipeline(r#"entry = "analyst""#);
        let overrides = BTreeMap::from([("run_e2ee".to_owned(), true)]);

        assert_eq!(
            pipeline.flags_for_run(&overrides),
            Err(FlagError::Undeclared {
                flag: "run_e2ee".to_owned()
            })
        );
    }

    #[test]
    fn flags_iterate_in_a_stable_order() {
        let flags = Flags::new(BTreeMap::from([
            ("b".to_owned(), true),
            ("a".to_owned(), false),
        ]));

        assert_eq!(
            flags.iter().collect::<Vec<_>>(),
            [("a", false), ("b", true)]
        );
    }
}
