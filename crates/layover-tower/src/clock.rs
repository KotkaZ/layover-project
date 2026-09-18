//! When a scheduled pipeline is next due, and whether it should actually fire.
//!
//! # Why a tick can be skipped
//!
//! A pipeline whose previous wave has not finished is still working. Starting a second copy means
//! paying twice for one result and, where agents share a workspace, two of them writing to the
//! same files. Skipping means being one interval late. For unattended spending those are not
//! comparable, so the default is to skip — and `overlap = "allow"` opts back in for pipelines
//! where a second copy is harmless.
//!
//! A skip is reported rather than swallowed. A schedule that quietly skips every tick because its
//! work always overruns looks exactly like a schedule that is running fine, and the difference is
//! that nothing is happening.
//!
//! # Why the next fire is computed from the schedule, not from the last run
//!
//! Adding an interval to when the last run *finished* makes the period drift by however long the
//! work took, so an hourly job slowly becomes a ninety-minute job. Both forms here are computed
//! from the clock: `cron` by its own definition, and `every` by stepping forward from the previous
//! due time until it is in the future. Stepping — rather than adding one interval to now — means a
//! Tower that was asleep for six hours does not fire six times in a row when it wakes.

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::Duration;

use jiff::Timestamp;
use layover_core::config::Config;
use layover_core::pipeline::{PipelineName, Schedule};

/// Why a pipeline that was due did not start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Skipped {
    /// Its previous wave is still going.
    StillWorking {
        /// Which pipeline.
        pipeline: PipelineName,
    },
}

impl std::fmt::Display for Skipped {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::StillWorking { pipeline } => write!(
                f,
                "`{pipeline}` was due but its previous run has not finished; \
                 set `overlap = \"allow\"` if two at once is safe"
            ),
        }
    }
}

/// What a tick found.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Due {
    /// Pipelines that should start now.
    pub fire: Vec<PipelineName>,
    /// Pipelines that were due and did not start.
    pub skipped: Vec<Skipped>,
}

/// The next due time for every scheduled pipeline.
#[derive(Debug)]
pub struct Clock {
    next: Mutex<BTreeMap<PipelineName, Timestamp>>,
}

impl Clock {
    /// Works out when each scheduled pipeline is first due, starting from `now`.
    ///
    /// Nothing fires at startup. A Tower restarting is not a reason to run every hourly job
    /// immediately, and a factory that fired everything on launch would make restarts expensive
    /// enough to avoid — which is the opposite of what a supervisor wants.
    #[must_use]
    pub fn new(config: &Config, now: Timestamp) -> Self {
        let mut next = BTreeMap::new();

        for (name, pipeline) in &config.pipelines {
            if let Some(schedule) = pipeline.trigger.schedule()
                && let Some(at) = next_after(schedule, now)
            {
                next.insert(name.clone(), at);
            }
        }

        Self {
            next: Mutex::new(next),
        }
    }

    /// Pipelines due at `now`, advancing each past this firing.
    ///
    /// `working` answers whether a pipeline's previous wave is still going. It is asked only for
    /// pipelines that are actually due, so the caller does not pay to look up the rest.
    pub fn tick(
        &self,
        config: &Config,
        now: Timestamp,
        working: impl Fn(&PipelineName) -> bool,
    ) -> Due {
        let Ok(mut next) = self.next.lock() else {
            return Due::default();
        };

        let mut due = Due::default();

        for (name, at) in next.iter_mut() {
            if *at > now {
                continue;
            }

            // Advanced before deciding whether to fire. A skipped tick is still a tick that
            // happened; leaving the due time in the past would make the next pass fire
            // immediately, turning one skip into a busy loop.
            if let Some(schedule) = config
                .pipelines
                .get(name)
                .and_then(|p| p.trigger.schedule())
                && let Some(following) = next_after(schedule, now)
            {
                *at = following;
            }

            let overlaps = config
                .pipelines
                .get(name)
                .is_some_and(layover_core::pipeline::Pipeline::allows_overlap);

            if overlaps || !working(name) {
                due.fire.push(name.clone());
            } else {
                due.skipped.push(Skipped::StillWorking {
                    pipeline: name.clone(),
                });
            }
        }

        due
    }

    /// When a pipeline is next due, for reporting.
    #[must_use]
    pub fn next_due(&self, pipeline: &PipelineName) -> Option<Timestamp> {
        self.next.lock().ok()?.get(pipeline).copied()
    }

    /// How long to wait before the soonest pipeline is due.
    ///
    /// `None` when nothing is scheduled, which is the difference between a Tower that should sleep
    /// until a clock says otherwise and one that has no clock to wait for.
    #[must_use]
    pub fn until_next(&self, now: Timestamp) -> Option<Duration> {
        let next = self.next.lock().ok()?;
        let soonest = next.values().min()?;

        Some(
            soonest
                .duration_since(now)
                .try_into()
                .unwrap_or(Duration::ZERO),
        )
    }
}

/// The first firing of `schedule` strictly after `now`.
fn next_after(schedule: &Schedule, now: Timestamp) -> Option<Timestamp> {
    schedule.next_after(now)
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::ToSpan as _;

    fn factory(pipelines: &str) -> Config {
        let text = format!(
            r#"
[layover]
work_dir = "work"

[defaults]
runner = "shell"

[runners.shell]
command = ["echo"]

[agents.worker]
prompt = "work"
entry = true

{pipelines}
"#
        );

        toml::from_str(&text).expect("the fixture factory parses")
    }

    fn name(text: &str) -> PipelineName {
        PipelineName::new(text)
    }

    fn never_working(_: &PipelineName) -> bool {
        false
    }

    #[test]
    fn nothing_fires_at_startup() {
        // A Tower restarting is not a reason to run every hourly job at once. If it were,
        // restarting would be expensive enough to avoid.
        let config = factory(
            r#"
[pipelines.sweep]
entry = "worker"
trigger = { every = "1h" }
"#,
        );

        let now = Timestamp::now();
        let clock = Clock::new(&config, now);
        let due = clock.tick(&config, now, never_working);

        assert!(due.fire.is_empty(), "{:?}", due.fire);
        assert!(due.skipped.is_empty());
    }

    #[test]
    fn an_interval_pipeline_fires_once_its_interval_has_passed() {
        let config = factory(
            r#"
[pipelines.sweep]
entry = "worker"
trigger = { every = "1h" }
"#,
        );

        let now = Timestamp::now();
        let clock = Clock::new(&config, now);

        let later = now.checked_add(61_i32.minutes()).expect("in range");
        let due = clock.tick(&config, later, never_working);

        assert_eq!(due.fire, [name("sweep")]);
    }

    #[test]
    fn a_tower_asleep_for_hours_fires_once_on_waking_not_once_per_missed_tick() {
        // Six hours of catch-up runs is six times the bill for one result nobody was waiting for.
        let config = factory(
            r#"
[pipelines.sweep]
entry = "worker"
trigger = { every = "1h" }
"#,
        );

        let now = Timestamp::now();
        let clock = Clock::new(&config, now);

        let much_later = now.checked_add(6_i32.hours()).expect("in range");
        let due = clock.tick(&config, much_later, never_working);

        assert_eq!(due.fire, [name("sweep")], "one firing, not six");
    }

    #[test]
    fn a_pipeline_that_is_still_working_is_skipped_and_says_why() {
        // Paying twice for one result, and possibly two agents writing to one workspace.
        let config = factory(
            r#"
[pipelines.sweep]
entry = "worker"
trigger = { every = "1h" }
"#,
        );

        let now = Timestamp::now();
        let clock = Clock::new(&config, now);
        let later = now.checked_add(61_i32.minutes()).expect("in range");

        let due = clock.tick(&config, later, |_| true);

        assert!(due.fire.is_empty());
        assert_eq!(
            due.skipped,
            [Skipped::StillWorking {
                pipeline: name("sweep")
            }]
        );
        assert!(
            due.skipped[0].to_string().contains("overlap"),
            "a skip should say how to opt out of it: {}",
            due.skipped[0]
        );
    }

    #[test]
    fn overlap_allow_starts_a_second_copy_deliberately() {
        let config = factory(
            r#"
[pipelines.sweep]
entry = "worker"
trigger = { every = "1h" }
overlap = "allow"
"#,
        );

        let now = Timestamp::now();
        let clock = Clock::new(&config, now);
        let later = now.checked_add(61_i32.minutes()).expect("in range");

        let due = clock.tick(&config, later, |_| true);

        assert_eq!(due.fire, [name("sweep")]);
        assert!(due.skipped.is_empty());
    }

    #[test]
    fn a_skipped_tick_still_advances_the_clock() {
        // Otherwise the due time stays in the past and every following pass fires immediately:
        // one skip becomes a busy loop.
        let config = factory(
            r#"
[pipelines.sweep]
entry = "worker"
trigger = { every = "1h" }
"#,
        );

        let now = Timestamp::now();
        let clock = Clock::new(&config, now);
        let later = now.checked_add(61_i32.minutes()).expect("in range");

        clock.tick(&config, later, |_| true);
        let again = clock.tick(&config, later, never_working);

        assert!(
            again.fire.is_empty(),
            "the same tick must not fire twice: {:?}",
            again.fire
        );
    }

    #[test]
    fn a_manual_pipeline_is_never_due() {
        let config = factory(
            r#"
[pipelines.onbehalf]
entry = "worker"
"#,
        );

        let now = Timestamp::now();
        let clock = Clock::new(&config, now);
        let far = now.checked_add(30_i32.hours()).expect("in range");

        assert!(clock.tick(&config, far, never_working).fire.is_empty());
        assert!(clock.until_next(now).is_none(), "nothing to wait for");
    }

    #[test]
    fn a_cron_pipeline_is_due_at_its_next_matching_minute() {
        let config = factory(
            r#"
[pipelines.digest]
entry = "worker"
trigger = { cron = "* * * * *" }
"#,
        );

        let now = Timestamp::now();
        let clock = Clock::new(&config, now);
        let next = clock.next_due(&name("digest")).expect("scheduled");

        assert!(next > now, "the next firing is in the future");
        assert!(
            next.duration_since(now).as_secs() <= 60,
            "an every-minute cron should be due within the minute"
        );
    }

    #[test]
    fn the_wait_is_until_the_soonest_pipeline_not_the_first_one_declared() {
        let config = factory(
            r#"
[pipelines.slow]
entry = "worker"
trigger = { every = "6h" }

[pipelines.quick]
entry = "worker"
trigger = { every = "5m" }
"#,
        );

        let now = Timestamp::now();
        let clock = Clock::new(&config, now);

        let wait = clock.until_next(now).expect("something is scheduled");

        assert!(
            wait <= Duration::from_mins(5),
            "should wait for `quick`, not `slow`: {wait:?}"
        );
    }
}
