//! Answering a help request from the dashboard, and continuing the work it stopped.
//!
//! What the answer becomes is decided in `layover_core::help::reply`; what it is, as one step with
//! queueing it, in the store. This turns both into the API's answer, and finds the workflow of a
//! request filed before requests recorded their own.

use axum::http::StatusCode;
use jiff::Timestamp;
use layover_core::RunId;
use layover_core::cost::Window;
use layover_core::flight::ItineraryId;
use layover_core::help::Reply;
use layover_core::help::reply::{Refused, continuation};
use layover_core::pipeline::PipelineName;
use layover_http::{HelpReplied, HelpReplyRequest, Problem};
use layover_store::{Answered, RunFilter};

use crate::api::Dashboard;

impl Dashboard {
    /// Answers `request.run_id`'s open help requests and queues the work that continues them.
    pub(crate) fn reply(&self, request: &HelpReplyRequest) -> Result<HelpReplied, Problem> {
        let config = self.config()?;

        // Like a trigger: a kill switch that halts running work while letting more be booked is
        // not a kill switch.
        if self.ground_stop_engaged() {
            return Err(
                Problem::new(StatusCode::CONFLICT, "a Ground Stop is engaged")
                    .with_detail("Release it before continuing any work. The request stays open."),
            );
        }
        if request.body.trim().is_empty() {
            return Err(Problem::new(
                StatusCode::BAD_REQUEST,
                "a reply needs something to say",
            ));
        }

        let run = RunId::from(request.run_id.as_str());
        let by = request
            .by
            .as_deref()
            .map(str::trim)
            .filter(|by| !by.is_empty())
            .map_or_else(operator, str::to_owned);
        let at = Timestamp::now();

        let answered = self
            .state()
            .journal
            .answer(&run, |asked| {
                let pipeline = asked
                    .first()
                    .filter(|first| !first.records_its_chain())
                    .and_then(|first| self.pipeline_on_record(&first.itinerary));
                let queued = continuation(
                    &config,
                    asked,
                    &request.body,
                    request.flags.as_ref(),
                    pipeline,
                )?;
                let reply = Reply {
                    by: by.clone(),
                    body: request.body.clone(),
                    at,
                    itinerary: queued.flight.itinerary.clone(),
                };
                Ok::<_, Refused>((queued, reply))
            })
            .map_err(|error| {
                Problem::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "the journal is unwritable",
                )
                .with_detail(error.to_string())
            })?;

        match answered {
            Answered::Queued { flight, answered } => Ok(HelpReplied {
                flight_id: flight.flight.id.as_str().to_owned(),
                itinerary_id: flight.flight.itinerary.as_str().to_owned(),
                to: flight.flight.to.to_string(),
                pipeline: flight.pipeline.as_ref().map(ToString::to_string),
                flags: Some(flight.flags.clone()),
                answered: i32::try_from(answered.len()).unwrap_or(i32::MAX),
            }),
            Answered::Unknown => Err(Problem::new(StatusCode::NOT_FOUND, "no such help request")
                .with_detail(format!("`{}` raised no help request that is still kept", request.run_id))),
            Answered::Closed => Err(Problem::new(
                StatusCode::CONFLICT,
                "already dealt with",
            )
            .with_detail(format!(
                "Every request `{}` raised has been resolved or answered, so this would start the \
                 work a second time. To start it again on purpose, continue the chain instead.",
                request.run_id
            ))),
            Answered::Refused(why @ Refused::FlagsNotRecorded { .. }) => Err(Problem::new(
                StatusCode::CONFLICT,
                "say which flags the chain had",
            )
            .with_detail(why.to_string())),
            Answered::Refused(why) => {
                Err(Problem::new(StatusCode::BAD_REQUEST, "the reply cannot continue the work")
                    .with_detail(why.to_string()))
            }
        }
    }

    /// The workflow a chain belonged to, from its runs in history or its work still queued.
    fn pipeline_on_record(&self, chain: &ItineraryId) -> Option<PipelineName> {
        let span = self.state().history.resolve(Window::AllTime);
        self.runs(&span, &RunFilter::default())
            .ok()?
            .into_iter()
            .filter(|record| record.itinerary == *chain)
            .find_map(|record| record.pipeline)
            .or_else(|| {
                self.state()
                    .journal
                    .pending()
                    .ok()?
                    .into_iter()
                    .filter(|queued| queued.flight.itinerary == *chain)
                    .find_map(|queued| queued.pipeline)
            })
    }
}

/// Who is replying when nobody said: the account the dashboard runs as, which on a machine one
/// person runs a factory from is that person.
fn operator() -> String {
    std::env::var("USERNAME")
        .or_else(|_| std::env::var("USER"))
        .ok()
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| "the dashboard".to_owned())
}
