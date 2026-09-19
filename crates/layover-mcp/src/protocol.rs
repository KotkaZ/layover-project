//! JSON-RPC 2.0, and dispatching a tool call to the runtime.
//!
//! # Why this layer never panics and never trusts a field
//!
//! Requests arrive from an agent CLI, and the content that shaped them arrived from a work item, a
//! pull request comment or another agent. Everything here is untrusted input in the ordinary
//! security sense, so a missing field, a wrong type or an unknown method is answered rather than
//! unwrapped.
//!
//! # Why a failed tool call is a successful response
//!
//! MCP distinguishes *the call failed* from *the protocol failed*. "You may not send to that
//! agent" is a perfectly well-formed answer to a well-formed question, and returning it as a
//! JSON-RPC error would tell the CLI its connection was broken rather than telling the agent it
//! asked for something it is not allowed.
//!
//! So a refusal comes back as a result with `isError: true` and text the agent can act on. The
//! agent reads it, and can try something else — which is the entire point of telling it.

use layover_core::tools::Tool;
use std::fmt::Write as _;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::runtime::{Runtime, Session, ToolError};

/// The protocol version this speaks.
pub const PROTOCOL_VERSION: &str = "2024-11-05";

/// A JSON-RPC request.
#[derive(Debug, Clone, Deserialize)]
pub struct Request {
    /// Always `"2.0"`; not enforced, because refusing on it helps nobody.
    #[serde(default)]
    pub jsonrpc: String,
    /// The call's identifier. Absent for a notification, which expects no reply.
    #[serde(default)]
    pub id: Option<Value>,
    /// What is being asked.
    pub method: String,
    /// Its arguments.
    #[serde(default)]
    pub params: Value,
}

/// A JSON-RPC response.
#[derive(Debug, Clone, Serialize)]
pub struct Response {
    /// Always `"2.0"`.
    pub jsonrpc: &'static str,
    /// Echoes the request's identifier.
    pub id: Value,
    /// The answer, when there was one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    /// The protocol-level failure, when there was one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<Value>,
}

impl Response {
    fn ok(id: Value, result: Value) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: Some(result),
            error: None,
        }
    }

    fn failed(id: Value, code: i32, message: &str) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: None,
            error: Some(json!({ "code": code, "message": message })),
        }
    }
}

/// Answers one request.
///
/// Returns `None` for a notification, which by the protocol expects no reply.
#[must_use]
pub fn handle(request: &Request, session: &Session, runtime: &dyn Runtime) -> Option<Response> {
    let id = request.id.clone()?;

    Some(match request.method.as_str() {
        "initialize" => Response::ok(id, initialize()),
        "tools/list" => Response::ok(id, tools_list()),
        "tools/call" => Response::ok(id, call(&request.params, session, runtime)),
        // Answered rather than refused: a client that pings before every call should not have
        // every ping look like a fault.
        "ping" => Response::ok(id, json!({})),
        other => Response::failed(id, -32601, &format!("no method `{other}`")),
    })
}

/// A reply to a body that did not parse as a request.
///
/// The identifier is null because there is nothing to echo: the id lived in the body that failed
/// to parse. This is a genuine JSON-RPC error rather than a refusal, because the client's framing
/// is wrong and no tool was reached.
#[must_use]
pub fn malformed(detail: &str) -> Response {
    Response::failed(
        Value::Null,
        -32700,
        &format!("could not parse the request: {detail}"),
    )
}

/// What Layover says it is and what it can do.
fn initialize() -> Value {
    json!({
        "protocolVersion": PROTOCOL_VERSION,
        "capabilities": { "tools": {} },
        "serverInfo": {
            "name": "layover",
            "version": env!("CARGO_PKG_VERSION"),
        },
    })
}

/// Every tool, with the schema a CLI needs to offer it.
fn tools_list() -> Value {
    let tools: Vec<Value> = Tool::ALL
        .into_iter()
        .map(|tool| {
            json!({
                "name": tool.name(),
                "description": tool.description(),
                "inputSchema": schema_for(tool),
            })
        })
        .collect();

    json!({ "tools": tools })
}

/// The arguments a tool takes.
fn schema_for(tool: Tool) -> Value {
    match tool {
        Tool::Send => json!({
            "type": "object",
            "required": ["to", "body"],
            "properties": {
                "to": { "type": "string", "description": "The agent to send to. Must be one layover_peers lists." },
                "body": { "type": "string", "description": "The work itself. Everything the receiving agent needs; it cannot see your run." },
            },
        }),
        Tool::Report => json!({
            "type": "object",
            "required": ["headline"],
            "properties": {
                "headline": { "type": "string", "description": "One line: what happened, not what you attempted." },
                "body": { "type": "string", "description": "The detail, for somebody who reads past the headline." },
            },
        }),
        Tool::Help => json!({
            "type": "object",
            "required": ["summary"],
            "properties": {
                "summary": { "type": "string", "description": "One line naming what is in the way." },
                "detail": { "type": "string", "description": "What you tried, what happened, and what you need." },
                "fatal": { "type": "boolean", "description": "True if this stopped the work; false if it merely limited it." },
            },
        }),
        Tool::MemoryWrite | Tool::Learn | Tool::LogbookAppend => json!({
            "type": "object",
            "required": ["text"],
            "properties": { "text": { "type": "string" } },
        }),
        Tool::Wait => json!({
            "type": "object",
            "required": ["until", "because"],
            "properties": {
                "until": { "type": "string", "description": "When to pick this up again." },
                "because": { "type": "string", "description": "What you are waiting for." },
            },
        }),
        Tool::Peers | Tool::MemoryRead | Tool::Status => {
            json!({ "type": "object", "properties": {} })
        }
    }
}

/// Runs one tool.
fn call(params: &Value, session: &Session, runtime: &dyn Runtime) -> Value {
    let Some(name) = params.get("name").and_then(Value::as_str) else {
        return failure("a tool call needs a `name`");
    };

    let Some(tool) = Tool::from_name(name) else {
        // Named rather than generic: an agent that misremembered a tool can correct itself, and
        // this is also how a factory learns its prompts are out of date.
        return failure(&format!(
            "`{name}` is not a Layover tool. Call tools/list to see what is available."
        ));
    };

    let arguments = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));

    match run_tool(tool, &arguments, session, runtime) {
        Ok(text) => success(&text),
        Err(error) => failure(&error.to_string()),
    }
}

/// Dispatches to the runtime, having checked the arguments.
fn run_tool(
    tool: Tool,
    arguments: &Value,
    session: &Session,
    runtime: &dyn Runtime,
) -> Result<String, ToolError> {
    match tool {
        Tool::Peers => Ok(describe_peers(&runtime.peers(session))),

        Tool::Send => {
            let to = required(arguments, "to")?;
            let body = required(arguments, "body")?;
            let flight = runtime.send(session, &layover_core::agent::AgentName::new(to), body)?;
            Ok(format!("Sent to `{to}` as {flight}."))
        }

        Tool::Report => {
            let headline = required(arguments, "headline")?;
            let body = arguments
                .get("body")
                .and_then(Value::as_str)
                .unwrap_or_default();
            runtime.report(session, headline, body)?;
            Ok("Recorded.".to_owned())
        }

        Tool::Help => {
            let summary = required(arguments, "summary")?;
            let detail = arguments
                .get("detail")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let fatal = arguments
                .get("fatal")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            runtime.help(session, summary, detail, fatal)?;
            Ok("Recorded. A human will see this.".to_owned())
        }

        Tool::MemoryRead => runtime.memory_read(session),

        Tool::MemoryWrite => {
            let text = required(arguments, "text")?;
            runtime.memory_write(session, text)?;
            Ok("Written.".to_owned())
        }

        Tool::Status => Ok(format!(
            "You are `{}`. This chain may send {} more message(s).",
            session.agent, session.hops_remaining
        )),

        Tool::Wait => {
            let until = required(arguments, "until")?;
            let because = required(arguments, "because")?;
            runtime.wait(session, until, because)
        }

        // Declared so agents can see them, and honest about not being connected yet. Better than
        // omitting them: a tool that appears and disappears between releases is harder to write a
        // prompt against than one that says what it is waiting for.
        Tool::Learn | Tool::LogbookAppend => Err(ToolError::Unavailable {
            detail: format!("`{tool}` is not connected yet in this release."),
        }),
    }
}

/// Reads a required string argument.
fn required<'a>(arguments: &'a Value, field: &str) -> Result<&'a str, ToolError> {
    arguments
        .get(field)
        .and_then(Value::as_str)
        .filter(|text| !text.trim().is_empty())
        .ok_or_else(|| ToolError::BadArguments {
            detail: format!("`{field}` is required and must be a non-empty string"),
        })
}

/// Renders the peer list for an agent to read.
fn describe_peers(peers: &[Peer]) -> String {
    if peers.is_empty() {
        return "You cannot send to anyone. Finish your work and report.".to_owned();
    }

    let mut out = String::from("You may send to:\n");
    for peer in peers {
        let _ = write!(out, "\n- `{}`", peer.name);
        if let Some(description) = &peer.description {
            let _ = write!(out, " — {description}");
        }
        if peer.spawns {
            out.push_str(" (opens a new chain)");
        }
    }
    out
}

use crate::runtime::Peer;

/// A tool call that did what was asked.
fn success(text: &str) -> Value {
    json!({ "content": [{ "type": "text", "text": text }], "isError": false })
}

/// A tool call that did not, reported so the agent can act on it.
fn failure(text: &str) -> Value {
    json!({ "content": [{ "type": "text", "text": text }], "isError": true })
}

#[cfg(test)]
mod tests {
    use super::*;
    use layover_core::agent::AgentName;
    use layover_core::flight::{ItineraryId, RunId};
    use std::cell::RefCell;

    #[derive(Default)]
    struct Spy {
        sent: RefCell<Vec<(String, String)>>,
        reported: RefCell<Vec<String>>,
        helped: RefCell<Vec<(String, bool)>>,
        written: RefCell<Vec<String>>,
        booked: RefCell<Vec<String>>,
        refuse_send: bool,
    }

    impl Runtime for Spy {
        fn peers(&self, _: &Session) -> Vec<Peer> {
            vec![Peer {
                name: AgentName::new("developer"),
                description: Some("Writes the code".to_owned()),
                spawns: false,
            }]
        }

        fn send(&self, session: &Session, to: &AgentName, body: &str) -> Result<String, ToolError> {
            if self.refuse_send {
                return Err(ToolError::NotPermitted {
                    from: session.agent.clone(),
                    to: to.clone(),
                });
            }
            self.sent
                .borrow_mut()
                .push((to.to_string(), body.to_owned()));
            Ok("flt_1".to_owned())
        }

        fn report(&self, _: &Session, headline: &str, _: &str) -> Result<(), ToolError> {
            self.reported.borrow_mut().push(headline.to_owned());
            Ok(())
        }

        fn help(&self, _: &Session, summary: &str, _: &str, fatal: bool) -> Result<(), ToolError> {
            self.helped.borrow_mut().push((summary.to_owned(), fatal));
            Ok(())
        }

        fn memory_read(&self, _: &Session) -> Result<String, ToolError> {
            Ok("what I remember".to_owned())
        }

        fn memory_write(&self, _: &Session, text: &str) -> Result<(), ToolError> {
            self.written.borrow_mut().push(text.to_owned());
            Ok(())
        }

        fn wait(&self, _: &Session, until: &str, because: &str) -> Result<String, ToolError> {
            self.booked.borrow_mut().push(format!("{until}:{because}"));
            Ok("Set down.".to_owned())
        }
    }

    fn session() -> Session {
        Session {
            run: RunId::generate(),
            agent: AgentName::new("analyst"),
            itinerary: ItineraryId::generate(),
            hops_remaining: 3,
        }
    }

    fn request(method: &str, params: Value) -> Request {
        Request {
            jsonrpc: "2.0".to_owned(),
            id: Some(json!(1)),
            method: method.to_owned(),
            params,
        }
    }

    fn call_tool(name: &str, arguments: &Value, runtime: &dyn Runtime) -> Value {
        let request = request(
            "tools/call",
            json!({ "name": name, "arguments": arguments }),
        );
        handle(&request, &session(), runtime)
            .expect("a call expects a reply")
            .result
            .expect("a tool call always returns a result")
    }

    fn text_of(result: &Value) -> String {
        result["content"][0]["text"]
            .as_str()
            .unwrap_or("")
            .to_owned()
    }

    #[test]
    fn initialize_announces_the_protocol_and_the_server() {
        let reply = handle(
            &request("initialize", json!({})),
            &session(),
            &Spy::default(),
        )
        .expect("replies");
        let result = reply.result.expect("a result");

        assert_eq!(result["protocolVersion"], PROTOCOL_VERSION);
        assert_eq!(result["serverInfo"]["name"], "layover");
    }

    #[test]
    fn every_tool_is_offered_with_a_schema() {
        let reply = handle(
            &request("tools/list", json!({})),
            &session(),
            &Spy::default(),
        )
        .expect("replies");
        let tools = reply.result.expect("a result")["tools"]
            .as_array()
            .expect("a list")
            .clone();

        assert_eq!(tools.len(), Tool::ALL.len());
        for tool in &tools {
            assert!(tool["name"].is_string());
            assert!(
                tool["description"].as_str().is_some_and(|d| d.len() > 60),
                "{tool}"
            );
            assert_eq!(tool["inputSchema"]["type"], "object");
        }
    }

    #[test]
    fn a_notification_gets_no_reply() {
        let notification = Request {
            jsonrpc: "2.0".to_owned(),
            id: None,
            method: "notifications/initialized".to_owned(),
            params: json!({}),
        };

        assert!(handle(&notification, &session(), &Spy::default()).is_none());
    }

    #[test]
    fn an_unknown_method_is_a_protocol_error() {
        let reply =
            handle(&request("nonsense", json!({})), &session(), &Spy::default()).expect("replies");

        assert!(reply.error.is_some());
        assert!(reply.result.is_none());
    }

    #[test]
    fn sending_reaches_the_runtime_with_what_the_agent_asked() {
        let spy = Spy::default();
        let result = call_tool(
            "layover_send",
            &json!({ "to": "developer", "body": "fix the retry policy" }),
            &spy,
        );

        assert_eq!(result["isError"], json!(false), "{result}");
        assert_eq!(
            spy.sent.borrow().as_slice(),
            [("developer".to_owned(), "fix the retry policy".to_owned())]
        );
    }

    #[test]
    fn a_refused_send_is_a_successful_response_the_agent_can_act_on() {
        // MCP separates "the call failed" from "the protocol failed". Returning a refusal as a
        // JSON-RPC error would tell the CLI its connection broke rather than telling the agent it
        // asked for something it is not allowed to do.
        let spy = Spy {
            refuse_send: true,
            ..Spy::default()
        };
        let result = call_tool(
            "layover_send",
            &json!({ "to": "publisher", "body": "go" }),
            &spy,
        );

        assert_eq!(result["isError"], json!(true), "{result}");
        assert!(
            text_of(&result).contains("layover_peers"),
            "a refusal should say how to find out what is allowed: {result}"
        );
    }

    #[test]
    fn a_missing_argument_is_explained_rather_than_swallowed() {
        let result = call_tool(
            "layover_send",
            &json!({ "to": "developer" }),
            &Spy::default(),
        );

        assert_eq!(result["isError"], json!(true));
        assert!(text_of(&result).contains("`body`"), "{result}");
    }

    #[test]
    fn an_empty_argument_is_treated_as_missing() {
        // An agent sending an empty body has not sent work, and the receiving agent would start
        // with nothing to do and no way to say so.
        let result = call_tool(
            "layover_send",
            &json!({ "to": "developer", "body": "   " }),
            &Spy::default(),
        );

        assert_eq!(result["isError"], json!(true), "{result}");
    }

    #[test]
    fn a_tool_that_does_not_exist_is_named_in_the_refusal() {
        let result = call_tool("layover_publish", &json!({}), &Spy::default());

        assert_eq!(result["isError"], json!(true));
        assert!(text_of(&result).contains("layover_publish"), "{result}");
        assert!(text_of(&result).contains("tools/list"), "{result}");
    }

    #[test]
    fn peers_are_described_for_an_agent_to_choose_between() {
        let result = call_tool("layover_peers", &json!({}), &Spy::default());
        let text = text_of(&result);

        assert!(text.contains("developer"), "{text}");
        assert!(
            text.contains("Writes the code"),
            "a name alone is not enough to choose by: {text}"
        );
    }

    #[test]
    fn an_agent_with_nowhere_to_send_is_told_what_to_do_instead() {
        struct Alone;
        impl Runtime for Alone {
            fn peers(&self, _: &Session) -> Vec<Peer> {
                Vec::new()
            }
            fn send(&self, _: &Session, _: &AgentName, _: &str) -> Result<String, ToolError> {
                unreachable!()
            }
            fn report(&self, _: &Session, _: &str, _: &str) -> Result<(), ToolError> {
                Ok(())
            }
            fn help(&self, _: &Session, _: &str, _: &str, _: bool) -> Result<(), ToolError> {
                Ok(())
            }
            fn memory_read(&self, _: &Session) -> Result<String, ToolError> {
                Ok(String::new())
            }
            fn memory_write(&self, _: &Session, _: &str) -> Result<(), ToolError> {
                Ok(())
            }
            fn wait(&self, _: &Session, _: &str, _: &str) -> Result<String, ToolError> {
                Ok("Set down.".to_owned())
            }
        }

        let text = text_of(&call_tool("layover_peers", &json!({}), &Alone));
        assert!(text.contains("report"), "{text}");
    }

    #[test]
    fn help_carries_whether_the_work_stopped() {
        let spy = Spy::default();
        call_tool(
            "layover_help",
            &json!({ "summary": "the token expired", "detail": "401 on push", "fatal": true }),
            &spy,
        );

        assert_eq!(
            spy.helped.borrow().as_slice(),
            [("the token expired".to_owned(), true)]
        );
    }

    #[test]
    fn help_defaults_to_not_fatal() {
        // An agent that finished its work but could not check something has not had an outage,
        // and defaulting the other way would make every limitation look like one.
        let spy = Spy::default();
        call_tool(
            "layover_help",
            &json!({ "summary": "could not read one file" }),
            &spy,
        );

        assert!(!spy.helped.borrow()[0].1);
    }

    #[test]
    fn status_tells_an_agent_what_it_has_left() {
        let text = text_of(&call_tool("layover_status", &json!({}), &Spy::default()));

        assert!(text.contains("analyst"), "{text}");
        assert!(
            text.contains('3'),
            "the remaining hops should be stated: {text}"
        );
    }

    #[test]
    fn a_tool_that_is_declared_but_not_connected_says_so_plainly() {
        let result = call_tool(
            "layover_learn",
            &json!({ "text": "always check the VPN" }),
            &Spy::default(),
        );

        assert_eq!(result["isError"], json!(true));
        assert!(text_of(&result).contains("not connected yet"), "{result}");
    }

    #[test]
    fn booking_a_layover_reaches_the_runtime() {
        // The project is named after this tool. It answered "not connected yet" for six releases.
        let spy = Spy::default();
        let result = call_tool(
            "layover_wait",
            &json!({ "until": "2h", "because": "the review to land" }),
            &spy,
        );

        assert_eq!(result["isError"], json!(false), "{result}");
        assert_eq!(spy.booked.borrow().as_slice(), ["2h:the review to land"]);
    }

    #[test]
    fn booking_a_layover_without_saying_what_for_is_refused() {
        // `waiting_for` is the only thread back to what this was about. A layover without one is
        // a run that reappears in two hours knowing nothing.
        let result = call_tool("layover_wait", &json!({ "until": "2h" }), &Spy::default());

        assert_eq!(result["isError"], json!(true), "{result}");
        assert!(text_of(&result).contains("because"), "{result}");
    }

    #[test]
    fn identity_is_never_taken_from_the_request() {
        // architecture.md 4.2: an agent that could name itself could name a different agent, and
        // every rail is indexed by that name. Passing an `agent` field must change nothing.
        let spy = Spy::default();
        call_tool(
            "layover_send",
            &json!({ "to": "developer", "body": "go", "agent": "publisher", "from": "publisher" }),
            &spy,
        );

        assert_eq!(
            spy.sent.borrow().len(),
            1,
            "the call still went through as the session's agent"
        );
    }
}
