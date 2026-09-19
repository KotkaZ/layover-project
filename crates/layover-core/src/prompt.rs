//! Dynamic prompt composition.
//!
//! An agent's standing instructions may live in a file rather than inline in `layover.toml`, and
//! that file may pull in others conditionally:
//!
//! ```text
//! You are the tester. Run the project's verification command.
//!
//! @include(run_e2e) tester-e2e.md
//! @include(!run_e2e) tester-local-only.md
//! ```
//!
//! The conditions are the boolean flags a [`crate::pipeline::Pipeline`] declares and a trigger
//! supplies, so one factory definition serves several situations without duplicated prompts.
//!
//! # What this does not do
//!
//! Resolution produces the agent's *standing instructions* and nothing else. How those combine
//! with the flight body, the agent's memory, its learnings and any handover is
//! [`crate::payload`]'s job, and the order is settled there. Do not assemble a payload here.

use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};

use crate::pipeline::Flags;

/// How deep `@include` may nest.
///
/// Deep nesting makes a prompt impossible to reason about, and the limit doubles as a backstop for
/// any cycle the explicit detection somehow misses.
pub const MAX_INCLUDE_DEPTH: usize = 8;

/// How many files one prompt may expand to in total.
///
/// Cycle detection tracks ancestors, so a diamond — two files that both include a third — is legal
/// and expands more than once. That is fine in moderation and pathological in bulk, and validation
/// walks *both* branches of every condition, so it sees the worst case even when a run would not.
pub const MAX_INCLUDE_EXPANSIONS: usize = 1_000;

/// Somewhere prompt files can be read from.
///
/// The indirection exists so prompt composition can be tested without touching a filesystem, and
/// so the Tower can later serve prompts from somewhere other than a directory.
pub trait PromptSource {
    /// Reads the prompt file at `path`, which is always relative to the source's root.
    ///
    /// # Errors
    ///
    /// Returns [`PromptError::Missing`] when there is no such file, or [`PromptError::Unreadable`]
    /// when it exists but cannot be read.
    fn read(&self, path: &Path) -> Result<String, PromptError>;
}

/// A prompt source backed by a directory on disk.
///
/// Paths are confined to the root: an absolute path, or one that climbs out with `..`, is refused
/// before the filesystem is touched. Prompt files are ordinary repository content and a factory
/// definition should not be able to read arbitrary files by asking nicely.
#[derive(Debug, Clone)]
pub struct PromptDir {
    root: PathBuf,
}

impl PromptDir {
    /// Creates a source rooted at `root`.
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Returns the root directory.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }
}

impl PromptSource for PromptDir {
    fn read(&self, path: &Path) -> Result<String, PromptError> {
        let relative = confine(path)?;
        let full = self.root.join(&relative);

        // Lexical confinement handles `..`, absolute paths and UNC. It does not handle a symlink
        // *inside* the prompt directory pointing anywhere at all — the path is clean, the target
        // is not. So the resolved path is compared against the resolved root, which is the only
        // check that can see through a link.
        //
        // Today prompt files are reviewed repository content and anyone who can plant a symlink
        // can also set `runners.*.command`, so this is not yet a boundary. It becomes one the
        // moment agents write their own prompts, and doing it then means doing it under pressure.
        if let Some(escape) = self.escapes(&full) {
            return Err(PromptError::Escapes { path: escape });
        }

        match std::fs::read_to_string(&full) {
            Ok(text) => Ok(text),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Err(PromptError::Missing { path: relative })
            }
            Err(error) => Err(PromptError::Unreadable {
                path: relative,
                reason: error.to_string(),
            }),
        }
    }
}

impl PromptDir {
    /// Whether `full` resolves to somewhere outside the prompt root.
    ///
    /// Returns `None` when it is inside, or when the question cannot be answered — a file that
    /// does not exist cannot be canonicalised, and refusing it here would turn every missing
    /// prompt into a security error rather than the "no such file" it actually is. The read that
    /// follows reports it properly.
    fn escapes(&self, full: &Path) -> Option<PathBuf> {
        let resolved = std::fs::canonicalize(full).ok()?;
        let root = std::fs::canonicalize(&self.root).ok()?;

        (!resolved.starts_with(&root)).then(|| {
            // Reported as the path that was *asked for* rather than where it led. Printing the
            // resolved target would helpfully tell an attacker what exists outside the sandbox.
            full.strip_prefix(&self.root).unwrap_or(full).to_path_buf()
        })
    }
}

/// A prompt source held in memory, keyed by relative path.
#[derive(Debug, Clone, Default)]
pub struct PromptMap {
    files: std::collections::BTreeMap<PathBuf, String>,
}

impl PromptMap {
    /// Creates an empty source.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a file.
    #[must_use]
    pub fn with(mut self, path: impl Into<PathBuf>, text: impl Into<String>) -> Self {
        self.files.insert(path.into(), text.into());
        self
    }
}

impl PromptSource for PromptMap {
    fn read(&self, path: &Path) -> Result<String, PromptError> {
        let relative = confine(path)?;
        self.files
            .get(&relative)
            .cloned()
            .ok_or(PromptError::Missing { path: relative })
    }
}

/// Resolves `entry` into finished prompt text, following `@include` directives.
///
/// # Errors
///
/// Returns a [`PromptError`] when a directive is malformed, names a flag that `flags` does not
/// declare, points at a missing file, escapes the prompt root, or forms a cycle.
pub fn resolve(
    source: &dyn PromptSource,
    entry: impl AsRef<Path>,
    flags: &Flags,
) -> Result<String, PromptError> {
    let mut stack = Vec::new();
    let mut output = String::new();
    expand(
        source,
        &confine(entry.as_ref())?,
        flags,
        &mut stack,
        &mut output,
    )?;
    Ok(output)
}

/// Returns every flag name `entry` and its includes mention, ignoring their values.
///
/// This is what lets load-time validation catch a prompt referring to a flag no pipeline declares,
/// before a run discovers it. Conditions are not evaluated, so every branch is walked — a flag
/// behind a condition that is false today still has to be declared.
///
/// It applies exactly the rules [`resolve`] does: the same cycle detection, the same
/// [`MAX_INCLUDE_DEPTH`], and the same confinement. Anything this accepts, `resolve` can compose;
/// anything it rejects would have failed at run time instead, which is the whole point of
/// checking.
///
/// # Errors
///
/// Returns a [`PromptError`] when a directive is malformed, points at a missing file, escapes the
/// prompt root, forms a cycle, nests too deeply, or expands past [`MAX_INCLUDE_EXPANSIONS`].
pub fn referenced_flags(
    source: &dyn PromptSource,
    entry: impl AsRef<Path>,
) -> Result<BTreeSet<String>, PromptError> {
    let mut found = BTreeSet::new();
    let mut stack = Vec::new();
    let mut budget = MAX_INCLUDE_EXPANSIONS;

    walk(
        source,
        &confine(entry.as_ref())?,
        &mut stack,
        &mut budget,
        &mut found,
    )?;

    Ok(found)
}

/// Walks every branch of an include graph, collecting flag names.
fn walk(
    source: &dyn PromptSource,
    path: &Path,
    stack: &mut Vec<PathBuf>,
    budget: &mut usize,
    found: &mut BTreeSet<String>,
) -> Result<(), PromptError> {
    if stack.len() >= MAX_INCLUDE_DEPTH {
        return Err(PromptError::TooDeep {
            path: path.to_path_buf(),
            limit: MAX_INCLUDE_DEPTH,
        });
    }
    if stack.iter().any(|seen| seen == path) {
        return Err(PromptError::Cycle {
            path: path.to_path_buf(),
        });
    }
    // Both branches of every condition are walked, so a wide include graph can expand far more
    // than it would at run time. Bound the work rather than letting validation hang.
    *budget = budget.checked_sub(1).ok_or_else(|| PromptError::TooWide {
        path: path.to_path_buf(),
        limit: MAX_INCLUDE_EXPANSIONS,
    })?;

    let text = source.read(path)?;
    stack.push(path.to_path_buf());

    for (number, line) in text.lines().enumerate() {
        let Some(directive) = Directive::parse(line, path, number + 1)? else {
            continue;
        };
        if let Some(condition) = &directive.condition {
            found.insert(condition.flag.clone());
        }
        let target = confine(&resolve_relative(path, &directive.path))?;
        walk(source, &target, stack, budget, found)?;
    }

    stack.pop();
    Ok(())
}

fn expand(
    source: &dyn PromptSource,
    path: &Path,
    flags: &Flags,
    stack: &mut Vec<PathBuf>,
    output: &mut String,
) -> Result<(), PromptError> {
    if stack.len() >= MAX_INCLUDE_DEPTH {
        return Err(PromptError::TooDeep {
            path: path.to_path_buf(),
            limit: MAX_INCLUDE_DEPTH,
        });
    }
    if stack.iter().any(|seen| seen == path) {
        return Err(PromptError::Cycle {
            path: path.to_path_buf(),
        });
    }

    let text = source.read(path)?;
    stack.push(path.to_path_buf());

    for (number, line) in text.lines().enumerate() {
        match Directive::parse(line, path, number + 1)? {
            None => {
                output.push_str(line);
                output.push('\n');
            }
            Some(directive) => {
                let include = match &directive.condition {
                    None => true,
                    Some(condition) => {
                        let value =
                            flags
                                .get(&condition.flag)
                                .ok_or_else(|| PromptError::UnknownFlag {
                                    flag: condition.flag.clone(),
                                    path: path.to_path_buf(),
                                    line: number + 1,
                                })?;
                        value != condition.negated
                    }
                };

                if include {
                    let target = confine(&resolve_relative(path, &directive.path))?;
                    expand(source, &target, flags, stack, output)?;
                }
            }
        }
    }

    stack.pop();
    Ok(())
}

/// A parsed `@include` line.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Directive {
    condition: Option<Condition>,
    path: PathBuf,
}

/// The `(flag)` or `(!flag)` part of a directive.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Condition {
    flag: String,
    negated: bool,
}

impl Directive {
    /// Parses one line, returning `None` when it is ordinary prompt text.
    fn parse(line: &str, path: &Path, number: usize) -> Result<Option<Self>, PromptError> {
        let trimmed = line.trim();
        let Some(rest) = trimmed.strip_prefix("@include") else {
            return Ok(None);
        };

        let malformed = || PromptError::MalformedDirective {
            path: path.to_path_buf(),
            line: number,
            text: trimmed.to_owned(),
        };

        let (condition, remainder) = if let Some(after) = rest.strip_prefix('(') {
            let (inside, after) = after.split_once(')').ok_or_else(malformed)?;
            (Some(parse_condition(inside).ok_or_else(malformed)?), after)
        } else if rest.starts_with(char::is_whitespace) {
            (None, rest)
        } else {
            // `@includes foo` or `@include(` — close enough to a directive to be a typo rather
            // than prose, so refuse instead of silently treating it as text.
            return Err(malformed());
        };

        let target = remainder.trim().trim_matches('"').trim();
        if target.is_empty() {
            return Err(malformed());
        }

        Ok(Some(Self {
            condition,
            path: PathBuf::from(target),
        }))
    }
}

fn parse_condition(inside: &str) -> Option<Condition> {
    let trimmed = inside.trim();
    let (negated, name) = match trimmed.strip_prefix('!') {
        Some(rest) => (true, rest.trim()),
        None => (false, trimmed),
    };

    let mut chars = name.chars();
    let first = chars.next()?;
    if !(first.is_ascii_alphabetic() || first == '_') {
        return None;
    }
    if !chars.all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return None;
    }

    Some(Condition {
        flag: name.to_owned(),
        negated,
    })
}

/// Resolves an included path against the directory of the file that included it.
fn resolve_relative(including: &Path, target: &Path) -> PathBuf {
    match including.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.join(target),
        _ => target.to_path_buf(),
    }
}

/// Rejects a path that would leave the prompt root, and normalises it.
///
/// `..` is resolved lexically rather than refused outright, so `shared/../common.md` is fine while
/// `../../etc/passwd` is not. This is deliberately a lexical check: it does not follow symlinks,
/// so a symlink inside the prompt root still points wherever it points. Prompt files are
/// repository content under the same review as the rest of the factory definition.
fn confine(path: &Path) -> Result<PathBuf, PromptError> {
    let escapes = || PromptError::Escapes {
        path: path.to_path_buf(),
    };
    let mut clean = PathBuf::new();

    for component in path.components() {
        match component {
            Component::Normal(part) => clean.push(part),
            Component::CurDir => {}
            Component::ParentDir => {
                if !clean.pop() {
                    return Err(escapes());
                }
            }
            Component::RootDir | Component::Prefix(_) => return Err(escapes()),
        }
    }

    if clean.as_os_str().is_empty() {
        return Err(escapes());
    }

    Ok(clean)
}

/// Why a prompt could not be assembled.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PromptError {
    /// A file referenced by a directive does not exist.
    #[error("prompt file `{path}` does not exist")]
    Missing {
        /// The path that was looked up.
        path: PathBuf,
    },
    /// A file exists but could not be read.
    #[error("prompt file `{path}` could not be read: {reason}")]
    Unreadable {
        /// The path that was looked up.
        path: PathBuf,
        /// Why the read failed.
        reason: String,
    },
    /// A path pointed outside the prompt root.
    #[error("prompt path `{path}` leaves the prompt directory")]
    Escapes {
        /// The offending path.
        path: PathBuf,
    },
    /// A line began with `@include` but was not a usable directive.
    #[error("`{path}` line {line}: could not read `{text}` as an @include directive")]
    MalformedDirective {
        /// The file the line was in.
        path: PathBuf,
        /// One-based line number.
        line: usize,
        /// The offending line.
        text: String,
    },
    /// A directive named a flag no pipeline declares.
    #[error("`{path}` line {line}: flag `{flag}` is not declared by any pipeline")]
    UnknownFlag {
        /// The flag that was referenced.
        flag: String,
        /// The file the line was in.
        path: PathBuf,
        /// One-based line number.
        line: usize,
    },
    /// A file includes itself, directly or through others.
    #[error("prompt file `{path}` includes itself")]
    Cycle {
        /// The file that closed the loop.
        path: PathBuf,
    },
    /// Includes nested further than [`MAX_INCLUDE_DEPTH`].
    #[error("prompt file `{path}` nests includes more than {limit} deep")]
    TooDeep {
        /// The file that breached the limit.
        path: PathBuf,
        /// The limit.
        limit: usize,
    },
    /// An include graph that expands to more than [`MAX_INCLUDE_EXPANSIONS`] files.
    #[error("prompt composition reached `{path}` after more than {limit} expansions")]
    TooWide {
        /// The file that breached the limit.
        path: PathBuf,
        /// The limit.
        limit: usize,
    },
}
