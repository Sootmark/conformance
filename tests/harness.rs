//! Proves the conformance checks catch each broken promise, using a tiny
//! text-log adapter with injectable faults.

use std::sync::atomic::{AtomicU64, Ordering};

use model::adapter::{Adapter, Confidence, Input, ParseError, Sink, Skipped};
use model::{Locator, Namespace, ParserInfo, Record, RecordTime, TimeKind, Ts};
use sootmark_conformance::{assert_conforms, violations, Violation};

const NAMESPACE: Namespace = Namespace::new("test.lines");
const PARSER: ParserInfo = ParserInfo {
    name: "lines",
    version: "1.0.0",
};

/// A log of `<unix seconds> <message>` lines.
const LOG: &[u8] = b"1700000000 service started\n1700000060 user alice logged on\nnot a timestamp\n1700000120 service stopped\n";

#[derive(Default)]
enum Fault {
    #[default]
    None,
    PanicsOnShortInput,
    Nondeterministic,
    SameLocatorForEveryLine,
    WrongParser,
    EmptySummary,
}

#[derive(Default)]
struct LinesAdapter {
    fault: Fault,
    calls: AtomicU64,
}

impl LinesAdapter {
    fn with(fault: Fault) -> Self {
        Self {
            fault,
            ..Self::default()
        }
    }

    fn record_for(
        &self,
        input: &Input<'_>,
        line_number: u64,
        seconds: i64,
        message: &str,
    ) -> Record {
        let line = match self.fault {
            Fault::SameLocatorForEveryLine => 1,
            _ => line_number,
        };
        let parser = match self.fault {
            Fault::WrongParser => ParserInfo {
                name: "lines",
                version: "0.0.1",
            },
            _ => PARSER,
        };
        let mut record = Record::new(input.evidence, NAMESPACE, Locator::Line(line), parser);
        record.times.push(RecordTime::new(
            TimeKind::Logged,
            "timestamp",
            Ts::from_unix_seconds(seconds),
        ));
        record.summary = match self.fault {
            Fault::EmptySummary => String::new(),
            Fault::Nondeterministic => format!("{message} #{}", self.calls.load(Ordering::Relaxed)),
            _ => message.to_owned(),
        };
        record
    }
}

impl Adapter for LinesAdapter {
    fn parser(&self) -> ParserInfo {
        PARSER
    }

    fn namespaces(&self) -> &'static [Namespace] {
        &[NAMESPACE]
    }

    fn probe(&self, name: &str, _head: &[u8]) -> Confidence {
        let is_log = std::path::Path::new(name)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("log"));
        if is_log {
            Confidence::Maybe
        } else {
            Confidence::No
        }
    }

    fn parse(&self, input: &Input<'_>, sink: &mut dyn Sink) -> Result<(), ParseError> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        if matches!(self.fault, Fault::PanicsOnShortInput) {
            assert!(input.data.len() > 10, "input too short");
        }
        let text = std::str::from_utf8(input.data)
            .map_err(|e| ParseError::at(e.valid_up_to() as u64, "not UTF-8"))?;
        for (index, line) in text.lines().enumerate() {
            let line_number = index as u64 + 1;
            let parsed = line
                .split_once(' ')
                .and_then(|(ts, msg)| Some((ts.parse().ok()?, msg)));
            match parsed {
                Some((seconds, message)) => {
                    sink.record(self.record_for(input, line_number, seconds, message));
                }
                None => sink.skipped(Skipped {
                    locator: Locator::Line(line_number),
                    reason: "no timestamp".into(),
                }),
            }
        }
        Ok(())
    }
}

#[test]
fn a_correct_adapter_conforms() {
    assert_conforms(&LinesAdapter::default(), "app.log", LOG);
}

#[test]
fn catches_panics_on_damaged_input() {
    let found = violations(
        &LinesAdapter::with(Fault::PanicsOnShortInput),
        "app.log",
        LOG,
    );
    assert!(
        found
            .iter()
            .any(|v| matches!(v, Violation::Panicked { .. })),
        "{found:?}"
    );
}

#[test]
fn catches_nondeterminism() {
    let found = violations(&LinesAdapter::with(Fault::Nondeterministic), "app.log", LOG);
    assert!(found.contains(&Violation::NotDeterministic), "{found:?}");
}

#[test]
fn catches_duplicate_ids() {
    let found = violations(
        &LinesAdapter::with(Fault::SameLocatorForEveryLine),
        "app.log",
        LOG,
    );
    assert!(
        found.iter().any(|v| matches!(v, Violation::DuplicateId(_))),
        "{found:?}"
    );
}

#[test]
fn catches_wrong_parser_info() {
    let found = violations(&LinesAdapter::with(Fault::WrongParser), "app.log", LOG);
    assert!(
        found.iter().any(|v| matches!(v, Violation::WrongParser(_))),
        "{found:?}"
    );
}

#[test]
fn catches_empty_summaries() {
    let found = violations(&LinesAdapter::with(Fault::EmptySummary), "app.log", LOG);
    assert!(
        found
            .iter()
            .any(|v| matches!(v, Violation::EmptySummary(_))),
        "{found:?}"
    );
}

#[test]
fn unparseable_lines_are_reported_not_dropped() {
    let mut sink = model::adapter::Collected::default();
    let input = Input {
        evidence: model::EvidenceId::of_content(LOG),
        name: "app.log",
        data: LOG,
    };
    LinesAdapter::default().parse(&input, &mut sink).unwrap();
    assert_eq!(sink.records.len(), 3);
    assert_eq!(
        sink.skipped,
        vec![Skipped {
            locator: Locator::Line(3),
            reason: "no timestamp".into()
        }]
    );
}
