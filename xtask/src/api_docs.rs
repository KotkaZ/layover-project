//! `book/src/http-api.md` against `api/openapi.yaml`.
//!
//! The server is generated from the specification, so the part of the HTTP surface that can drift
//! is the page people read. Its endpoint table is held to the specification both ways: every
//! operation is a row with its query parameters in the row's Query column, and every row is an
//! operation — an endpoint removed but still documented misleads as much as one added and left out.
//!
//! Only the table is checked. What a row says an endpoint does is prose, and request and response
//! bodies are described by the specification itself.

use std::collections::BTreeSet;
use std::path::Path;

use crate::openapi::{Document, SPEC_PATH};

/// The reference page, relative to the repository root.
const PAGE: &str = "book/src/http-api.md";

/// One endpoint: its method in capitals, its path template and its query parameters.
#[derive(Debug, PartialEq, Eq)]
struct Endpoint {
    method: String,
    path: String,
    query: BTreeSet<String>,
}

impl Endpoint {
    fn is(&self, other: &Self) -> bool {
        self.method == other.method && self.path == other.path
    }

    fn named(&self) -> String {
        format!("`{} {}`", self.method, self.path)
    }
}

/// Every way the page's endpoint table and the specification disagree.
pub fn problems(root: &Path) -> Vec<String> {
    let document = match crate::openapi::load(root) {
        Ok(document) => document,
        Err(error) => return vec![error.to_string()],
    };
    match std::fs::read_to_string(root.join(PAGE)) {
        Ok(page) => compare(&specified(&document), &documented(&page)),
        Err(error) => vec![format!("could not read {PAGE}: {error}")],
    }
}

/// Every operation the specification declares.
fn specified(document: &Document) -> Vec<Endpoint> {
    document
        .paths
        .iter()
        .flat_map(|(path, item)| {
            item.operations()
                .into_iter()
                .map(move |(method, operation)| Endpoint {
                    method: method.to_ascii_uppercase(),
                    path: path.clone(),
                    query: operation
                        .parameters
                        .iter()
                        .filter(|parameter| parameter.location == "query")
                        .map(|parameter| parameter.name.clone())
                        .collect(),
                })
        })
        .collect()
}

/// The rows of the page's endpoint table: `` | `GET` | `/runs` | `window`, `limit` | … ``. A row
/// of any other table starts with something that is not a method in capitals, and is passed over.
fn documented(page: &str) -> Vec<Endpoint> {
    page.lines()
        .filter_map(|line| {
            let mut cells = line.strip_prefix('|')?.split('|').map(str::trim);
            let method = code(cells.next()?)?;
            if method.is_empty() || !method.chars().all(|c| c.is_ascii_uppercase()) {
                return None;
            }
            let path = code(cells.next()?)?;
            let query = cells
                .next()
                .unwrap_or_default()
                .split('`')
                .skip(1)
                .step_by(2)
                .map(str::to_owned)
                .collect();
            Some(Endpoint {
                method: method.to_owned(),
                path: path.to_owned(),
                query,
            })
        })
        .collect()
}

/// The text of a cell that is one code span.
fn code(cell: &str) -> Option<&str> {
    cell.strip_prefix('`')?.strip_suffix('`')
}

fn compare(specified: &[Endpoint], documented: &[Endpoint]) -> Vec<String> {
    let mut problems = Vec::new();
    for endpoint in specified {
        let Some(row) = documented.iter().find(|row| row.is(endpoint)) else {
            problems.push(format!(
                "{PAGE} has no row for {}, which {SPEC_PATH} declares",
                endpoint.named()
            ));
            continue;
        };
        for name in endpoint.query.difference(&row.query) {
            problems.push(format!(
                "{PAGE} does not list `{name}` among the query parameters of {}",
                endpoint.named()
            ));
        }
        for name in row.query.difference(&endpoint.query) {
            problems.push(format!(
                "{PAGE} lists `{name}` as a query parameter of {}, which {SPEC_PATH} does not \
                 declare",
                endpoint.named()
            ));
        }
    }
    for row in documented {
        if !specified.iter().any(|endpoint| endpoint.is(row)) {
            problems.push(format!(
                "{PAGE} has a row for {}, which {SPEC_PATH} does not declare",
                row.named()
            ));
        }
    }
    problems
}

#[cfg(test)]
mod tests {
    use super::*;

    fn endpoint(method: &str, path: &str, query: &[&str]) -> Endpoint {
        Endpoint {
            method: method.to_owned(),
            path: path.to_owned(),
            query: query.iter().map(|name| (*name).to_owned()).collect(),
        }
    }

    #[test]
    fn only_rows_that_start_with_a_method_are_endpoints() {
        let page = "\
| Method | Path | Query | Purpose |
|---|---|---|---|
| `GET` | `/runs` | `window`, `limit` | Runs, with a | in the prose. |
| `DELETE` | `/flights/{flight_id}` | | Cancel queued work. |

| Field | What it is |
|---|---|
| `itinerary` | The chain. |
| `400` | A bad request. |
";

        assert_eq!(
            documented(page),
            [
                endpoint("GET", "/runs", &["window", "limit"]),
                endpoint("DELETE", "/flights/{flight_id}", &[]),
            ]
        );
    }

    #[test]
    fn every_operation_is_a_row_with_its_query_and_every_row_an_operation() {
        let specified = [
            endpoint("GET", "/runs", &["window", "limit"]),
            endpoint("POST", "/flights", &[]),
        ];
        let documented = [
            endpoint("GET", "/runs", &["window", "status"]),
            endpoint("GET", "/flights", &[]),
        ];

        let problems = compare(&specified, &documented);

        assert_eq!(problems.len(), 4, "{problems:#?}");
        assert!(problems[0].contains("`limit` among the query parameters of `GET /runs`"));
        assert!(problems[1].contains("lists `status` as a query parameter of `GET /runs`"));
        assert!(problems[2].contains("no row for `POST /flights`"));
        assert!(problems[3].contains("a row for `GET /flights`"));
    }

    #[test]
    fn the_page_matches_the_specification() {
        let problems = problems(&crate::repository_root());

        assert!(problems.is_empty(), "{problems:#?}");
    }
}
