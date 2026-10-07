//! What will start on its own: the ticks of every schedule, the layovers waiting to be picked up,
//! and the ticks that came due and were skipped.
//!
//! # Why the times come from the Tower
//!
//! A cron expression says when it fires, but an `every` schedule is counted from when the Tower
//! started, and a tick held back by a Ground Stop fires when it is released. Only the Tower keeping
//! the clock knows either, so it hands this dashboard a [`Timetable`] to ask. A dashboard without
//! one — `--watch-only`, or a Tower in another process — lists no ticks and says it has no clock.
//! Working the times out here instead would produce a schedule that looks exact and is wrong by
//! however long ago the Tower started.
//!
//! # When a layover is picked up
//!
//! A layover is collected by the first tick of a resuming pipeline at or after it comes due — not
//! when it comes due. Showing only its due time would promise a follow-up at 09:00 that actually
//! happens at 09:44, when the resuming schedule next looks.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::Arc;

use axum::http::StatusCode;
use jiff::{SignedDuration, Timestamp};
use layover_core::config::Config;
use layover_core::cost::Window;
use layover_core::layover::Layover;
use layover_core::pipeline::{Pipeline, PipelineName, Schedule};
use layover_core::skip::{Skip, SkipReason as CoreSkipReason};
use layover_http::{
    GetUpcomingQuery, Problem, ScheduledFire, ScheduledWorkflow, SkipReason, SkippedTick, Upcoming,
    WaitingLayover,
};

use crate::api::Dashboard;

/// What the Tower that keeps the clock can say about it.
///
/// Implemented by `layover serve` over its own clock and dispatcher, so the answers are the ones
/// the Tower acts on rather than a second opinion worked out from the same files.
pub trait Timetable: Send + Sync + fmt::Debug {
    /// When `pipeline`'s next tick is due, or `None` when it has no schedule.
    fn next_due(&self, pipeline: &PipelineName) -> Option<Timestamp>;

    /// The pipelines whose previous wave is still queued or running: their next tick is skipped
    /// unless it finishes first.
    fn working(&self) -> BTreeSet<PipelineName>;
}

/// A clock, and what keeps it.
#[derive(Debug, Clone)]
pub(crate) struct Clocked {
    pub(crate) keeper: Arc<str>,
    pub(crate) timetable: Arc<dyn Timetable>,
}

/// How far ahead to look when the request does not say.
const DEFAULT_HOURS: i32 = 24;
/// The furthest ahead a request may look: a week.
const MAX_HOURS: i32 = 168;
/// Ticks listed per pipeline. A one-minute schedule has 1,440 a day, and listing them all would
/// bury every other schedule under it.
const LISTED_PER_WORKFLOW: usize = 24;
/// Ticks counted per pipeline before counting stops.
const COUNTED_PER_WORKFLOW: usize = 10_000;
/// Skipped ticks listed.
const SKIPS_LISTED: usize = 100;

impl Dashboard {
    /// Builds `GET /upcoming`.
    pub(crate) fn upcoming(&self, query: &GetUpcomingQuery) -> Result<Upcoming, Problem> {
        let config = self.config()?;
        let now = Timestamp::now();
        let hours = query.hours.unwrap_or(DEFAULT_HOURS).clamp(1, MAX_HOURS);
        let until = now
            .checked_add(SignedDuration::from_hours(i64::from(hours)))
            .unwrap_or(Timestamp::MAX);

        let clock = self.clock();
        let schedules = Schedules::of(&config, clock.map(|c| &*c.timetable), now);

        let journal = &self.state().journal;
        let week = self.state().history.resolve(Window::Last7Days);
        let skips = journal.skips(&week).map_err(unreadable("skipped ticks"))?;
        let mut waiting: Vec<Layover> = journal
            .layovers()
            .map_err(unreadable("layovers"))?
            .into_iter()
            .filter(|layover| layover.standing.is_pending())
            .collect();
        waiting.sort_by_key(|layover| layover.due_at);

        let layovers: Vec<WaitingLayover> = waiting
            .iter()
            .map(|layover| schedules.waiting(layover))
            .collect();
        let (workflows, fires) = schedules.ticks(until, &layovers, &skips);

        Ok(Upcoming {
            now: now.to_string(),
            until: until.to_string(),
            clock: clock.map(|c| c.keeper.to_string()),
            ground_stop: self.ground_stop_engaged(),
            workflows,
            fires,
            layovers,
            skips: skips
                .iter()
                .take(SKIPS_LISTED)
                .map(|skip| SkippedTick {
                    pipeline: skip.pipeline.to_string(),
                    at: skip.at.to_string(),
                    reason: match skip.reason {
                        CoreSkipReason::StillWorking => SkipReason::StillWorking,
                    },
                })
                .collect(),
        })
    }
}

/// Every scheduled pipeline, with its next tick as the Tower has it.
struct Schedules<'a> {
    now: Timestamp,
    working: BTreeSet<PipelineName>,
    each: Vec<Scheduled<'a>>,
}

struct Scheduled<'a> {
    name: &'a PipelineName,
    pipeline: &'a Pipeline,
    schedule: &'a Schedule,
    next: Option<Timestamp>,
}

impl<'a> Schedules<'a> {
    fn of(config: &'a Config, timetable: Option<&dyn Timetable>, now: Timestamp) -> Self {
        let each = config
            .pipelines
            .iter()
            .filter_map(|(name, pipeline)| {
                let schedule = pipeline.trigger.schedule()?;
                Some(Scheduled {
                    name,
                    pipeline,
                    schedule,
                    next: timetable.and_then(|clock| clock.next_due(name)),
                })
            })
            .collect();

        Self {
            now,
            working: timetable.map(Timetable::working).unwrap_or_default(),
            each,
        }
    }

    /// A layover, and the resuming tick that will pick it up.
    fn waiting(&self, layover: &Layover) -> WaitingLayover {
        // Soonest first; on a tie, the first by name, which is the order the Tower ticks them in.
        let (collected_at, collected_by) = self
            .each
            .iter()
            .filter(|scheduled| scheduled.pipeline.resumes)
            .filter_map(|scheduled| {
                let at = collecting_tick(
                    scheduled.schedule,
                    scheduled.next?,
                    self.now,
                    layover.due_at,
                )?;
                Some((at, scheduled.name))
            })
            .min()
            .unzip();

        WaitingLayover {
            layover_id: layover.id.as_str().to_owned(),
            agent: layover.agent.to_string(),
            waiting_for: layover.waiting_for.clone(),
            booked_by: layover.booked_by.as_str().to_owned(),
            pipeline: layover
                .scope
                .as_ref()
                .and_then(|scope| scope.pipeline.as_ref())
                .map(ToString::to_string),
            booked_at: layover.booked_at.to_string(),
            due_at: layover.due_at.to_string(),
            collected_at: collected_at.map(|at| at.to_string()),
            collected_by: collected_by.map(ToString::to_string),
        }
    }

    /// Each schedule, and its ticks before `until`, soonest first.
    fn ticks(
        &self,
        until: Timestamp,
        layovers: &[WaitingLayover],
        skips: &[Skip],
    ) -> (Vec<ScheduledWorkflow>, Vec<ScheduledFire>) {
        let mut collects: BTreeMap<(&str, &str), i32> = BTreeMap::new();
        for layover in layovers {
            if let (Some(by), Some(at)) = (&layover.collected_by, &layover.collected_at) {
                *collects.entry((by.as_str(), at.as_str())).or_default() += 1;
            }
        }

        let mut workflows = Vec::new();
        let mut fires = Vec::new();

        for scheduled in &self.each {
            let working = self.working.contains(scheduled.name);
            let overlaps = scheduled.pipeline.allows_overlap();
            let ticks = Ticks {
                schedule: scheduled.schedule,
                next: scheduled.next,
                now: self.now,
            }
            .take_while(|at| *at <= until)
            .take(COUNTED_PER_WORKFLOW);

            let mut counted = 0_usize;
            for at in ticks {
                if counted < LISTED_PER_WORKFLOW {
                    let stamp = at.to_string();
                    fires.push(ScheduledFire {
                        pipeline: scheduled.name.to_string(),
                        overdue: at < self.now,
                        resumes: scheduled.pipeline.resumes,
                        may_skip: counted == 0 && working && !overlaps,
                        collects: collects
                            .get(&(scheduled.name.as_str(), stamp.as_str()))
                            .copied()
                            .unwrap_or(0),
                        at: stamp,
                    });
                }
                counted += 1;
            }

            let skipped: Vec<&Skip> = skips
                .iter()
                .filter(|skip| skip.pipeline == *scheduled.name)
                .collect();

            workflows.push(ScheduledWorkflow {
                pipeline: scheduled.name.to_string(),
                next_at: scheduled.next.map(|at| at.to_string()),
                resumes: scheduled.pipeline.resumes,
                overlaps,
                working,
                fires_in_window: i32::try_from(counted).unwrap_or(i32::MAX),
                skipped_7d: i32::try_from(skipped.len()).unwrap_or(i32::MAX),
                // Most recent first, as the journal reads them.
                last_skipped_at: skipped.first().map(|skip| skip.at.to_string()),
            });
        }

        // Ordered as they will happen. Two at the same moment go in the order the Tower ticks
        // them, which is by name.
        fires.sort_by(|a, b| a.at.cmp(&b.at).then_with(|| a.pipeline.cmp(&b.pipeline)));
        (workflows, fires)
    }
}

/// One schedule's ticks from its next due time onwards, as the Tower will fire them.
///
/// A tick already past is held, not lost: it fires as soon as the Tower can — when a Ground Stop
/// is released, usually — and the one after it is counted from then, because the Tower works out
/// each tick from the moment it fired the one before.
struct Ticks<'a> {
    schedule: &'a Schedule,
    next: Option<Timestamp>,
    now: Timestamp,
}

impl Iterator for Ticks<'_> {
    type Item = Timestamp;

    fn next(&mut self) -> Option<Timestamp> {
        let at = self.next?;
        self.next = self.schedule.next_after(at.max(self.now));
        Some(at)
    }
}

/// The first of a resuming schedule's [`Ticks`] at which a layover due at `due` is picked up.
///
/// The same answer as walking the ticks until one is at or after `due`, without the walk: a layover
/// set down for three days behind a one-minute schedule is four thousand steps away.
fn collecting_tick(
    schedule: &Schedule,
    next: Timestamp,
    now: Timestamp,
    due: Timestamp,
) -> Option<Timestamp> {
    // The next tick fires no sooner than now, so it collects anything due by then.
    let fires = next.max(now);
    if due <= fires {
        return Some(next);
    }

    let after = schedule.next_after(fires)?;
    if due <= after {
        return Some(after);
    }

    match schedule {
        Schedule::Every(interval) => {
            let step = i128::from(interval.as_secs().max(1)) * 1_000_000_000;
            let gap = due.as_nanosecond() - after.as_nanosecond();
            let steps = (gap + step - 1) / step;
            Timestamp::from_nanosecond(after.as_nanosecond() + steps * step).ok()
        }
        Schedule::Cron(_) => {
            schedule.next_after(due.checked_sub(SignedDuration::from_nanos(1)).ok()?)
        }
    }
}

/// Turns a journal that cannot be read into a problem naming what could not be.
fn unreadable(what: &'static str) -> impl Fn(layover_store::StoreError) -> Problem {
    move |error| {
        Problem::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("the {what} could not be read"),
        )
        .with_detail(error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str) -> Timestamp {
        text.parse().expect("valid")
    }

    fn every(minutes: u64) -> Schedule {
        Schedule::Every(std::time::Duration::from_secs(minutes * 60))
    }

    /// The tick a walk finds, for checking the shortcut against it.
    fn walked(schedule: &Schedule, next: Timestamp, now: Timestamp, due: Timestamp) -> Timestamp {
        Ticks {
            schedule,
            next: Some(next),
            now,
        }
        .find(|tick| *tick >= due || (*tick < now && due <= now))
        .expect("a tick comes")
    }

    #[test]
    fn a_layover_is_collected_by_the_first_tick_at_or_after_it_is_due() {
        let now = at("2026-10-07T09:00:00Z");
        let next = at("2026-10-07T09:05:00Z");
        let schedule = every(45);

        for due in [
            "2026-10-07T08:00:00Z",
            "2026-10-07T09:05:00Z",
            "2026-10-07T09:05:01Z",
            "2026-10-07T10:00:00Z",
            "2026-10-07T10:35:00Z",
            "2026-10-10T17:12:00Z",
        ] {
            let due = at(due);
            assert_eq!(
                collecting_tick(&schedule, next, now, due),
                Some(walked(&schedule, next, now, due)),
                "due {due}"
            );
        }
        assert_eq!(
            collecting_tick(&schedule, next, now, at("2026-10-07T10:00:00Z")),
            Some(at("2026-10-07T10:35:00Z")),
            "09:05, 09:50, then 10:35"
        );
    }

    #[test]
    fn a_held_tick_collects_everything_due_by_the_time_it_can_fire() {
        // Due at 08:00 and held by a Ground Stop: when it is released it fires at once, and finds
        // the layover that came due at 08:30 waiting.
        let now = at("2026-10-07T09:00:00Z");
        let held = at("2026-10-07T08:00:00Z");
        let schedule = every(60);

        assert_eq!(
            collecting_tick(&schedule, held, now, at("2026-10-07T08:30:00Z")),
            Some(held)
        );
        assert_eq!(
            collecting_tick(&schedule, held, now, at("2026-10-07T09:30:00Z")),
            Some(at("2026-10-07T10:00:00Z")),
            "the tick after a held one counts from now"
        );
    }

    #[test]
    fn a_cron_schedule_collects_at_its_first_matching_minute_after_the_due_time() {
        let schedule = Schedule::Cron("*/30 * * * *".to_owned());
        let now = at("2026-10-07T09:00:00Z");
        let next = schedule.next_after(now).expect("matches");
        let due = at("2026-10-08T13:10:00Z");

        let collected = collecting_tick(&schedule, next, now, due).expect("collects");

        assert_eq!(collected, walked(&schedule, next, now, due));
        assert!(collected >= due, "{collected}");
        assert!(
            collected.duration_since(due) < SignedDuration::from_mins(30),
            "{collected}"
        );
    }
}
