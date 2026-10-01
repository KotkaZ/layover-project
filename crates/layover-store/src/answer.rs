//! Answering a help request: queueing the work that continues it and recording the answer, as one
//! step.
//!
//! Two people replying to one request at once, or one person pressing the button twice, must start
//! one chain and not two — so whether the request is still open, the queued flight and the
//! answer are decided under the queue's lock and the request's segment's, together.

use std::sync::{Arc, Mutex, PoisonError};

use jiff::{SignedDuration, Timestamp};
use layover_core::cost::Window;
use layover_core::flight::RunId;
use layover_core::help::{HelpRequest, Reply};
use layover_core::queue::Queued;

use crate::Journal;
use crate::history::StoreError;

/// What answering a run's help requests came to.
#[derive(Debug)]
pub enum Answered<E> {
    /// The continuation is queued, and these requests are marked answered.
    Queued {
        /// The flight that continues the work.
        flight: Box<Queued>,
        /// The requests it answered, as they now stand.
        answered: Vec<HelpRequest>,
    },
    /// The run raised no help request that is still kept.
    Unknown,
    /// Every request the run raised has already been dealt with.
    Closed,
    /// The answer could not continue the work, for the reason given.
    Refused(E),
}

impl Journal {
    /// Answers every open help request `run` raised: `compose` is shown them and says what to
    /// queue and what to record, and both happen or neither does.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Io`] if the requests or the queue cannot be read or written.
    pub fn answer<E>(
        &self,
        run: &RunId,
        compose: impl FnOnce(&[HelpRequest]) -> Result<(Queued, Reply), E>,
    ) -> Result<Answered<E>, StoreError> {
        let _queue = self.writes.lock().map_err(|_| crate::journal::poisoned())?;

        let everything = Window::AllTime.resolve(
            &(Timestamp::now() + SignedDuration::from_mins(1)).to_zoned(jiff::tz::TimeZone::UTC),
        );
        let mut segments = Vec::new();
        for path in crate::segment::segments_covering(self.root(), "help", &everything)? {
            let requests: Vec<HelpRequest> = crate::segment::read_segment(&path)?;
            if requests.iter().any(|request| request.run == *run) {
                segments.push(path);
            }
        }
        if segments.is_empty() {
            return Ok(Answered::Unknown);
        }

        // Each segment held while it is read and rewritten, so a request an agent files meanwhile
        // is not overwritten. In path order, so two answers cannot take them the other way round.
        segments.sort();
        let locks: Vec<Arc<Mutex<()>>> = segments
            .iter()
            .map(|path| crate::lock::for_path(path))
            .collect();
        let _held: Vec<_> = locks
            .iter()
            .map(|lock| lock.lock().unwrap_or_else(PoisonError::into_inner))
            .collect();

        let mut read = Vec::new();
        for path in &segments {
            read.push(crate::segment::read_segment::<HelpRequest>(path)?);
        }
        let open: Vec<HelpRequest> = read
            .iter()
            .flatten()
            .filter(|request| request.run == *run && request.is_open())
            .cloned()
            .collect();
        if open.is_empty() {
            return Ok(Answered::Closed);
        }

        let (flight, reply) = match compose(&open) {
            Ok(composed) => composed,
            Err(why) => return Ok(Answered::Refused(why)),
        };

        // Queued first: if marking the request fails, the work is taken back off, so a retry
        // cannot start the chain twice.
        let mut pending = self.pending()?;
        pending.push(flight.clone());
        crate::segment::write_document(&self.pending_path(), &pending)?;

        let mut answered = Vec::new();
        for (path, requests) in segments.iter().zip(read) {
            let changed: Vec<HelpRequest> = requests
                .into_iter()
                .map(|request| {
                    if request.run == *run && request.is_open() {
                        let replied = request.replied(reply.clone());
                        answered.push(replied.clone());
                        replied
                    } else {
                        request
                    }
                })
                .collect();
            if let Err(error) = crate::segment::rewrite_segment(path, &changed) {
                pending.retain(|queued| queued.flight.id != flight.flight.id);
                let _ = crate::segment::write_document(&self.pending_path(), &pending);
                return Err(error);
            }
        }

        Ok(Answered::Queued {
            flight: Box::new(flight),
            answered,
        })
    }
}
