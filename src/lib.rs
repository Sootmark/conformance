//! Checks that an [`Adapter`] keeps the contract described in
//! [`model::adapter`].
//!
//! Adapter test suites call [`assert_conforms`] with each fixture file. It
//! parses the fixture, parses it again, then parses damaged variants of it
//! (truncated, bytes flipped), and fails with a list of every broken promise.

use std::collections::HashSet;
use std::fmt;
use std::panic::{self, AssertUnwindSafe};

use model::adapter::{Adapter, Collected, Input, ParseError};
use model::{EvidenceId, RecordId};

/// A broken promise.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Violation {
    /// Parsing panicked instead of returning records, skips or an error.
    Panicked {
        /// Which input variant triggered it.
        input: String,
    },
    /// Two parses of the same input produced different output.
    NotDeterministic,
    /// Two records share an id.
    DuplicateId(RecordId),
    /// A record carries another parser's name or version.
    WrongParser(RecordId),
    /// A record uses a namespace the adapter doesn't declare.
    UndeclaredNamespace(RecordId),
    /// A record claims to come from different evidence than the input.
    WrongEvidence(RecordId),
    /// A record has no summary line for the timeline.
    EmptySummary(RecordId),
    /// A record timestamp has no field name.
    UnnamedTime(RecordId),
}

impl fmt::Display for Violation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Panicked { input } => write!(f, "panicked on {input}"),
            Self::NotDeterministic => f.write_str("two parses of the same input differ"),
            Self::DuplicateId(id) => write!(f, "duplicate record id {id}"),
            Self::WrongParser(id) => write!(f, "record {id} carries the wrong parser info"),
            Self::UndeclaredNamespace(id) => write!(f, "record {id} uses an undeclared namespace"),
            Self::WrongEvidence(id) => write!(f, "record {id} names the wrong evidence"),
            Self::EmptySummary(id) => write!(f, "record {id} has an empty summary"),
            Self::UnnamedTime(id) => write!(f, "record {id} has a timestamp without a field name"),
        }
    }
}

/// Every contract violation found when parsing `data` (named `name`).
#[must_use]
pub fn violations(adapter: &dyn Adapter, name: &str, data: &[u8]) -> Vec<Violation> {
    let evidence = EvidenceId::of_content(data);
    let input = Input {
        evidence,
        name,
        data,
        modified: None,
    };

    let Some(first) = parse_catching(adapter, &input) else {
        return vec![Violation::Panicked {
            input: "the original input".into(),
        }];
    };
    let mut found = Vec::new();
    match parse_catching(adapter, &input) {
        Some(second) if same_output(&first, &second) => {}
        Some(_) => found.push(Violation::NotDeterministic),
        None => found.push(Violation::Panicked {
            input: "a second parse of the original input".into(),
        }),
    }
    found.extend(record_violations(adapter, evidence, &first.0));
    found.extend(robustness_violations(adapter, name, data));
    found
}

/// Panic with a readable list if `adapter` breaks the contract on `data`.
///
/// # Panics
/// When there is at least one [`Violation`].
pub fn assert_conforms(adapter: &dyn Adapter, name: &str, data: &[u8]) {
    let found = violations(adapter, name, data);
    assert!(
        found.is_empty(),
        "adapter {} broke the contract on {name}:\n{}",
        adapter.parser().name,
        found
            .iter()
            .map(|v| format!("  - {v}"))
            .collect::<Vec<_>>()
            .join("\n"),
    );
}

type Outcome = (Collected, Result<(), ParseError>);

fn parse_catching(adapter: &dyn Adapter, input: &Input<'_>) -> Option<Outcome> {
    panic::catch_unwind(AssertUnwindSafe(|| {
        let mut sink = Collected::default();
        let result = adapter.parse(input, &mut sink);
        (sink, result)
    }))
    .ok()
}

fn same_output(a: &Outcome, b: &Outcome) -> bool {
    a.0.records == b.0.records && a.0.skipped == b.0.skipped && a.1 == b.1
}

fn record_violations(
    adapter: &dyn Adapter,
    evidence: EvidenceId,
    output: &Collected,
) -> Vec<Violation> {
    let mut found = Vec::new();
    let mut seen = HashSet::new();
    for record in &output.records {
        let id = record.id();
        if !seen.insert(id) {
            found.push(Violation::DuplicateId(id));
        }
        if record.parser() != adapter.parser() {
            found.push(Violation::WrongParser(id));
        }
        if !adapter.namespaces().contains(&record.namespace()) {
            found.push(Violation::UndeclaredNamespace(id));
        }
        if record.evidence() != evidence {
            found.push(Violation::WrongEvidence(id));
        }
        if record.summary.trim().is_empty() {
            found.push(Violation::EmptySummary(id));
        }
        if record.times.iter().any(|t| t.field.is_empty()) {
            found.push(Violation::UnnamedTime(id));
        }
    }
    found
}

/// Parse damaged variants of the input; any panic is a violation.
fn robustness_violations(adapter: &dyn Adapter, name: &str, data: &[u8]) -> Vec<Violation> {
    damaged_variants(data)
        .into_iter()
        .filter_map(|(description, damaged)| {
            let input = Input {
                evidence: EvidenceId::of_content(&damaged),
                name,
                data: &damaged,
                modified: None,
            };
            parse_catching(adapter, &input)
                .is_none()
                .then_some(Violation::Panicked { input: description })
        })
        .collect()
}

/// Number of evenly spaced truncation points tried.
const TRUNCATION_POINTS: usize = 16;
/// Number of evenly spaced single-byte corruptions tried.
const FLIP_POINTS: usize = 32;

fn damaged_variants(data: &[u8]) -> Vec<(String, Vec<u8>)> {
    let len = data.len();
    let truncations = (0..TRUNCATION_POINTS).map(|i| {
        let cut = len * i / TRUNCATION_POINTS;
        (
            format!("the input truncated to {cut} bytes"),
            data[..cut].to_vec(),
        )
    });
    let flips = (0..FLIP_POINTS.min(len)).map(|i| {
        let at = len * i / FLIP_POINTS.min(len);
        let mut damaged = data.to_vec();
        damaged[at] = !damaged[at];
        (format!("the input with byte {at} inverted"), damaged)
    });
    truncations.chain(flips).collect()
}
