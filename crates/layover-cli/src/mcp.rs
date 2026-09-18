//! The MCP endpoint a `layover run` serves for the duration of its drain.
//!
//! # Why it lives for exactly as long as the drain
//!
//! The endpoint exists so a running agent can call back — send a flight, write a note, ask for
//! help. Outside a drain there is nothing to call back into: no run holds a token, so every
//! request would be refused anyway. Binding it for the drain and dropping it afterwards keeps the
//! window in which a port is open as small as the work requires.
//!
//! # Why port zero
//!
//! The address is chosen by the operating system and read back after binding. A port picked in
//! advance can be taken between picking it and binding it, and a child told an address nothing is
//! listening on fails in a way that reads as the agent misbehaving rather than as a port clash.

use std::sync::Arc;

use layover_core::config::Config;
use layover_core::graph::RouteGraph;
use layover_mcp::{Served, Session, Sessions};
use layover_store::Journal;
use layover_tower::{FactoryRuntime, Tokens};

use crate::commands::Failure;

/// A bound MCP endpoint, serving until it is dropped.
pub struct ServedMcp {
    /// The address to tell children about.
    pub endpoint: String,
    tokens: Arc<TokenGate>,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

/// Resolves tokens against whichever registry the factory ended up with.
///
/// Indirection rather than a direct `Arc<Tokens>` because the endpoint has to bind *before* the
/// factory is built — the factory needs the bound address — and the registry only exists once the
/// factory does. Until it is filled in, every token is unknown, which is the correct answer: no
/// run has started, so no token has been minted.
#[derive(Default)]
struct TokenGate(std::sync::OnceLock<Arc<Tokens>>);

impl Sessions for TokenGate {
    fn resolve(&self, token: &str) -> Option<Session> {
        self.0.get()?.resolve(token)
    }
}

impl ServedMcp {
    /// Binds an endpoint on loopback and serves it on its own thread.
    ///
    /// # Errors
    ///
    /// Returns a failure when no port can be bound or the runtime cannot be built.
    pub fn start(
        config: &Config,
        root: &std::path::Path,
        journal: Arc<Journal>,
    ) -> Result<Self, Failure> {
        let gate = Arc::new(TokenGate::default());
        let hangars = root.join(".layover").join("hangars");

        let runtime = FactoryRuntime::new(
            Arc::new(config.clone()),
            Arc::new(RouteGraph::from_config(config)),
            hangars,
            Arc::new(move |queued| {
                journal
                    .queue(queued)
                    .map_err(|error| format!("the queue would not take it: {error}"))
            }),
        );

        let served = Served {
            sessions: Arc::clone(&gate) as Arc<dyn Sessions>,
            runtime: Arc::new(runtime),
        };

        let tokio = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(|error| error.to_string())?;

        // Bound here, on this thread, so a failure to bind is reported to the operator rather than
        // disappearing into a thread nobody is watching.
        let listener = tokio
            .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
            .map_err(|error| format!("could not bind an MCP endpoint: {error}"))?;
        let bound = listener.local_addr().map_err(|error| error.to_string())?;

        let (shutdown, stop) = tokio::sync::oneshot::channel();
        let router = layover_mcp::router(served);

        let thread = std::thread::Builder::new()
            .name("layover-mcp".to_owned())
            .spawn(move || {
                tokio.block_on(async move {
                    let _ = axum::serve(listener, router)
                        .with_graceful_shutdown(async move {
                            let _ = stop.await;
                        })
                        .await;
                });
            })
            .map_err(|error| error.to_string())?;

        Ok(Self {
            endpoint: format!("http://{bound}{}", layover_mcp::ENDPOINT_PATH),
            tokens: gate,
            shutdown: Some(shutdown),
            thread: Some(thread),
        })
    }

    /// Points the endpoint at the registry the factory mints into.
    ///
    /// Until this is called every token is unknown, which is correct: no run has started, so none
    /// has been minted.
    pub fn resolve_against(&self, tokens: Arc<Tokens>) {
        let _ = self.tokens.0.set(tokens);
    }
}

impl Drop for ServedMcp {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }

        // Joined rather than detached: the endpoint holds a port, and a command that returns while
        // something is still listening makes the next run fail for a reason the operator cannot
        // see.
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn factory() -> Config {
        toml::from_str(
            r#"
[layover]
work_dir = "work"

[defaults]
runner = "shell"

[runners.shell]
command = ["echo"]

[agents.analyst]
prompt = "analyse"
entry = true

[pipelines.build]
entry = "analyst"
"#,
        )
        .expect("the fixture factory parses")
    }

    fn temp(name: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("layover-mcp-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("a temporary directory");
        path
    }

    #[test]
    fn the_endpoint_reports_the_address_it_actually_bound() {
        // A port chosen in advance can be taken between choosing it and binding it, and a child
        // told the wrong address fails in a way that reads as the agent misbehaving.
        let root = temp("bound");
        let journal = Arc::new(Journal::open(root.join("journal")).expect("opens"));

        let served = ServedMcp::start(&factory(), &root, journal).expect("binds");

        assert!(
            served.endpoint.starts_with("http://127.0.0.1:"),
            "{}",
            served.endpoint
        );
        assert!(served.endpoint.ends_with("/mcp"), "{}", served.endpoint);
        assert!(
            !served.endpoint.contains(":0/"),
            "port zero must be resolved, not reported: {}",
            served.endpoint
        );

        drop(served);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_token_is_unknown_until_a_factory_has_minted_one() {
        // The endpoint binds before the factory exists, because the factory needs its address.
        let root = temp("ungated");
        let journal = Arc::new(Journal::open(root.join("journal")).expect("opens"));

        let served = ServedMcp::start(&factory(), &root, journal).expect("binds");
        assert!(served.tokens.resolve("lvt_anything").is_none());

        drop(served);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn dropping_the_endpoint_frees_the_port() {
        // A command that returns while something is still listening makes the next run fail for a
        // reason the operator cannot see.
        let root = temp("freed");
        let journal = Arc::new(Journal::open(root.join("journal")).expect("opens"));

        let served = ServedMcp::start(&factory(), &root, journal).expect("binds");
        let address = served
            .endpoint
            .trim_start_matches("http://")
            .trim_end_matches("/mcp")
            .to_owned();
        drop(served);

        std::net::TcpListener::bind(&address)
            .expect("the port must be free once the endpoint is dropped");

        let _ = std::fs::remove_dir_all(&root);
    }

    /// Posts a JSON-RPC body to the live endpoint over a real socket.
    ///
    /// Written by hand rather than with an HTTP client so this test needs no dependency that the
    /// shipped binary does not already have — and so it exercises the same path a child CLI does:
    /// a TCP connection, a bearer token, a JSON body.
    fn post(endpoint: &str, token: &str, body: &str) -> String {
        use std::io::{Read as _, Write as _};

        let rest = endpoint.trim_start_matches("http://");
        let (address, path) = rest.split_once('/').expect("an endpoint with a path");

        let request = format!(
            "POST /{path} HTTP/1.1\r\nHost: {address}\r\nAuthorization: Bearer {token}\r\n\
             Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );

        let mut stream = std::net::TcpStream::connect(address).expect("the endpoint is listening");
        stream.write_all(request.as_bytes()).expect("writes");

        let mut response = String::new();
        stream.read_to_string(&mut response).expect("reads");
        response
    }

    #[test]
    fn a_send_over_the_wire_queues_a_flight_that_continues_the_chain() {
        // The whole point of serving MCP: an agent calls `layover_send`, a real flight lands in
        // the real queue, and it belongs to the chain that sent it rather than to a fresh one.
        let root = temp("wire");
        let journal = Arc::new(Journal::open(root.join("journal")).expect("opens"));

        let config: Config = toml::from_str(
            r#"
[layover]
work_dir = "work"

[defaults]
runner = "shell"

[runners.shell]
command = ["echo"]

[agents.analyst]
prompt = "analyse"
entry = true

[agents.developer]
prompt = "develop"

[pipelines.build]
entry = "analyst"

[[routes]]
from = "analyst"
to = "developer"
"#,
        )
        .expect("parses");

        let served = ServedMcp::start(&config, &root, Arc::clone(&journal)).expect("binds");

        // Stands in for the Tower minting a token as it starts a run of `analyst`.
        let tokens = Arc::new(layover_tower::Tokens::new());
        let chain = layover_core::flight::ItineraryId::generate();
        let token = tokens.mint(
            layover_core::RunId::generate(),
            layover_core::agent::AgentName::new("analyst"),
            chain.clone(),
            3,
        );
        served.resolve_against(Arc::clone(&tokens));

        let response = post(
            &served.endpoint,
            &token,
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call",
                "params":{"name":"layover_send",
                          "arguments":{"to":"developer","body":"please build it"}}}"#,
        );

        assert!(response.contains("200 OK"), "{response}");

        let pending = journal.pending().expect("readable");
        assert_eq!(pending.len(), 1, "one flight should be waiting");
        assert_eq!(pending[0].flight.to.to_string(), "developer");
        assert_eq!(
            pending[0].flight.itinerary, chain,
            "the flight must continue the caller's chain, not start a new one"
        );

        drop(served);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_send_the_route_map_forbids_queues_nothing() {
        let root = temp("forbidden");
        let journal = Arc::new(Journal::open(root.join("journal")).expect("opens"));

        let served = ServedMcp::start(&factory(), &root, Arc::clone(&journal)).expect("binds");

        let tokens = Arc::new(layover_tower::Tokens::new());
        let token = tokens.mint(
            layover_core::RunId::generate(),
            layover_core::agent::AgentName::new("analyst"),
            layover_core::flight::ItineraryId::generate(),
            3,
        );
        served.resolve_against(Arc::clone(&tokens));

        let response = post(
            &served.endpoint,
            &token,
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call",
                "params":{"name":"layover_send",
                          "arguments":{"to":"nobody","body":"go"}}}"#,
        );

        // A refusal is a successful response the agent can read, not a broken connection.
        assert!(response.contains("200 OK"), "{response}");
        assert!(response.contains("isError"), "{response}");
        assert!(
            journal.pending().expect("readable").is_empty(),
            "a refused send must queue nothing"
        );

        drop(served);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_token_from_a_finished_run_cannot_send_anything() {
        // A token that outlives its run is a finished process that can still queue work.
        let root = temp("revoked");
        let journal = Arc::new(Journal::open(root.join("journal")).expect("opens"));

        let served = ServedMcp::start(&factory(), &root, Arc::clone(&journal)).expect("binds");

        let tokens = Arc::new(layover_tower::Tokens::new());
        let token = tokens.mint(
            layover_core::RunId::generate(),
            layover_core::agent::AgentName::new("analyst"),
            layover_core::flight::ItineraryId::generate(),
            3,
        );
        served.resolve_against(Arc::clone(&tokens));
        tokens.revoke(&token);

        let response = post(
            &served.endpoint,
            &token,
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
        );

        assert!(response.contains("401"), "{response}");

        drop(served);
        let _ = std::fs::remove_dir_all(&root);
    }
}
