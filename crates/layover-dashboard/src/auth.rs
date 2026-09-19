//! Who may talk to the dashboard and the API behind it.
//!
//! # Why loopback is not enough on its own
//!
//! Binding to `127.0.0.1` keeps the network out, and that was a sufficient boundary while this
//! surface only read history. It is not sufficient now that the thing behind it spends money:
//! anything already on the machine can reach it, and so can a web page in a browser that knows
//! the port. A page cannot *read* a cross-origin response, but it can very happily POST one —
//! which here means queueing work that a real agent CLI will then run.
//!
//! # Why a token in the URL
//!
//! Minting a token at startup and printing it as part of the address is the pattern Jupyter and
//! others settled on for exactly this situation. It costs the operator one copy-paste, needs no
//! account, no password store and no configuration, and it is gone when the process is.
//!
//! The token arrives one of three ways, in this order: an `Authorization: Bearer` header, a
//! `?token=` query, or a cookie. The query is how a person first arrives; the response to that
//! sets the cookie, so the page's own later requests carry it without the token staying in the
//! address bar of every subsequent navigation.
//!
//! `--no-auth` exists because this is a tool one person often runs on their own laptop, and
//! friction on the most-used command of such a tool is a real cost. It is opt-out rather than
//! opt-in because the failure it prevents is silent and expensive, and the people most at risk
//! are the ones least likely to go looking for a flag.

use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::{StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

/// The cookie the dashboard keeps its token in.
pub const COOKIE: &str = "layover_token";

/// The query parameter a person arrives with.
pub const QUERY: &str = "token";

/// What the server will accept as proof.
#[derive(Debug, Clone)]
pub enum Guard {
    /// A token must be presented.
    Token(Arc<String>),
    /// Anything may connect.
    ///
    /// Chosen deliberately with `--no-auth`, and said out loud at startup: a surface that is open
    /// without anybody having decided it should be is the situation this type exists to prevent.
    Open,
}

impl Guard {
    /// Mints a guard with a fresh token.
    #[must_use]
    pub fn minted() -> Self {
        // Two ULIDs rather than one: a ULID's leading bits are a timestamp, so half of a single
        // one is guessable by anybody who knows roughly when the process started.
        let token = format!("{}{}", ulid::Ulid::new(), ulid::Ulid::new());
        Self::Token(Arc::new(token))
    }

    /// The token, when there is one.
    #[must_use]
    pub fn token(&self) -> Option<&str> {
        match self {
            Self::Token(token) => Some(token),
            Self::Open => None,
        }
    }

    /// Whether `presented` is the right token.
    ///
    /// Compared in constant time. The comparison is over a local socket where timing is noisy and
    /// an attacker who can measure it can usually read the token from the process anyway — but a
    /// short-circuiting `==` on a secret is the kind of thing that gets copied into somewhere it
    /// matters.
    #[must_use]
    pub fn accepts(&self, presented: &str) -> bool {
        let Self::Token(expected) = self else {
            return true;
        };

        if presented.len() != expected.len() {
            return false;
        }

        expected
            .bytes()
            .zip(presented.bytes())
            .fold(0_u8, |difference, (left, right)| {
                difference | (left ^ right)
            })
            == 0
    }

    /// The address to print, carrying the token when there is one.
    #[must_use]
    pub fn address(&self, bound: &std::net::SocketAddr) -> String {
        match self.token() {
            Some(token) => format!("http://{bound}/?{QUERY}={token}"),
            None => format!("http://{bound}"),
        }
    }
}

/// Reads whatever token the request carries.
#[must_use]
pub fn presented(request: &Request) -> Option<String> {
    if let Some(bearer) = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.trim_start_matches("Bearer ").trim())
        .filter(|value| !value.is_empty())
    {
        return Some(bearer.to_owned());
    }

    if let Some(from_query) = request.uri().query().and_then(|query| {
        query
            .split('&')
            .filter_map(|pair| pair.split_once('='))
            .find(|(name, _)| *name == QUERY)
            .map(|(_, value)| value.to_owned())
    }) {
        return Some(from_query);
    }

    request
        .headers()
        .get(header::COOKIE)
        .and_then(|value| value.to_str().ok())
        .and_then(|cookies| {
            cookies
                .split(';')
                .filter_map(|pair| pair.trim().split_once('='))
                .find(|(name, _)| *name == COOKIE)
                .map(|(_, value)| value.to_owned())
        })
}

/// Refuses a request that does not carry the token.
///
/// # Errors
///
/// Answers `401` when the token is missing or wrong.
pub async fn require(State(guard): State<Guard>, request: Request, next: Next) -> Response {
    // Checked before anything is read from the request. Going through the token path with an open
    // guard refuses a request that presents *nothing*, because there is no token to check — which
    // would make `--no-auth` refuse everything.
    if matches!(guard, Guard::Open) {
        return next.run(request).await;
    }

    let offered = presented(&request);

    let Some(token) = offered.filter(|token| guard.accepts(token)) else {
        return (
            StatusCode::UNAUTHORIZED,
            "This factory's dashboard needs the token printed when it started. Look for the \
             address in the terminal running `layover serve`, or pass `--no-auth` if you want it \
             open.",
        )
            .into_response();
    };

    let mut response = next.run(request).await;

    // Set on the way out so the page's own later requests carry it: a person arrives once with
    // `?token=`, and everything after that is the cookie. `SameSite=Strict` is the part that
    // matters — it is what stops another page in the same browser posting work to this API.
    if let Ok(cookie) =
        format!("{COOKIE}={token}; Path=/; SameSite=Strict; HttpOnly; Max-Age=86400").parse()
    {
        response.headers_mut().insert(header::SET_COOKIE, cookie);
    }

    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;

    fn guard() -> Guard {
        Guard::Token(Arc::new("secret-token".to_owned()))
    }

    fn with(header_name: &str, value: &str) -> Request {
        Request::builder()
            .uri("/runs")
            .header(header_name, value)
            .body(Body::empty())
            .expect("a request")
    }

    #[test]
    fn a_minted_token_is_not_guessable_from_the_clock() {
        // A ULID's leading bits are a timestamp, so half of a single one is guessable by anybody
        // who knows roughly when the process started.
        let first = Guard::minted();
        let second = Guard::minted();

        let (Some(a), Some(b)) = (first.token(), second.token()) else {
            panic!("a minted guard has a token");
        };

        assert_ne!(a, b);
        assert!(a.len() >= 52, "two ULIDs' worth of entropy: {}", a.len());
    }

    #[test]
    fn the_right_token_is_accepted_and_a_wrong_one_is_not() {
        assert!(guard().accepts("secret-token"));
        assert!(!guard().accepts("secret-tokeN"));
        assert!(!guard().accepts("secret-toke"));
        assert!(!guard().accepts(""));
    }

    #[test]
    fn an_open_guard_accepts_anything_including_nothing() {
        // `--no-auth` means what it says. Half-enforcing it would be worse than either choice.
        assert!(Guard::Open.accepts(""));
        assert!(Guard::Open.accepts("whatever"));
        assert_eq!(Guard::Open.token(), None);
    }

    #[test]
    fn a_bearer_header_is_read() {
        assert_eq!(
            presented(&with("authorization", "Bearer secret-token")).as_deref(),
            Some("secret-token")
        );
    }

    #[test]
    fn a_query_parameter_is_read_because_it_is_how_a_person_arrives() {
        let request = Request::builder()
            .uri("/?token=secret-token")
            .body(Body::empty())
            .expect("a request");

        assert_eq!(presented(&request).as_deref(), Some("secret-token"));
    }

    #[test]
    fn a_cookie_is_read_even_among_others() {
        assert_eq!(
            presented(&with(
                "cookie",
                "other=1; layover_token=secret-token; more=2"
            ))
            .as_deref(),
            Some("secret-token")
        );
    }

    #[test]
    fn a_request_carrying_nothing_presents_nothing() {
        let request = Request::builder()
            .uri("/runs")
            .body(Body::empty())
            .expect("a request");

        assert_eq!(presented(&request), None);
    }

    #[test]
    fn the_printed_address_carries_the_token_so_it_can_be_copied_once() {
        let bound = "127.0.0.1:7878".parse().expect("an address");

        assert_eq!(
            guard().address(&bound),
            "http://127.0.0.1:7878/?token=secret-token"
        );
        assert_eq!(Guard::Open.address(&bound), "http://127.0.0.1:7878");
    }
}
