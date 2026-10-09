//! M4 acceptance tests: the sequential (SFC / Grafcet) part of the
//! ClassicLadder format.
//!
//! The golden corpus lives in `testdata/classicladder-corpus/projects_examples`
//! (fetched by `scripts/fetch_corpus.sh`, git-ignored). When it is absent the
//! corpus group skips itself, so the rest of the suite runs in a bare checkout.

use std::path::PathBuf;

use softladder_core::{
    Diagnostic, Expr, Project, Section, SectionLanguage, SequentialPage, Severity, Step,
    Transition, VarKind, VarRef,
};
use softladder_project::classicladder::{self, Document};

/// Steps imported across the whole corpus.
const CORPUS_STEPS: usize = 73;
/// Transitions imported across the whole corpus.
const CORPUS_TRANSITIONS: usize = 100;
/// Pages imported across the whole corpus.
const CORPUS_PAGES: usize = 8;

/// Directory holding the fetched ClassicLadder example projects.
fn corpus_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../testdata/classicladder-corpus/projects_examples")
}

/// Corpus files in a stable order, or an empty list when the corpus is absent.
fn corpus_files() -> Vec<PathBuf> {
    let dir = corpus_dir();
    if !dir.is_dir() {
        println!(
            "note: the ClassicLadder corpus is absent from {}; \
             run scripts/fetch_corpus.sh to enable the corpus tests",
            dir.display()
        );
        return Vec::new();
    }
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("the corpus directory is readable")
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.is_file())
        .collect();
    files.sort();
    files
}

/// Builds a container from `(name, contents)` pairs.
fn container(parts: &[(&str, &str)]) -> Document {
    Document::from_parts(
        parts
            .iter()
            .map(|(name, contents)| ((*name).to_owned(), (*contents).to_owned()))
            .collect(),
    )
}

/// The pages a project owns, in section order.
fn pages(project: &Project) -> Vec<&SequentialPage> {
    project
        .sections
        .iter()
        .filter_map(|section| section.sequential_page.as_ref())
        .collect()
}

/// A condition expression over `%M<index>`.
fn mem_bit(index: u32) -> Expr {
    Expr::Var(VarRef::new(VarKind::MemBit, index))
}

/// Pads one direction of a `T` record to the reference's ten slots.
fn ten(list: &[i64]) -> Vec<i64> {
    let mut fields: Vec<i64> = list.to_vec();
    fields.resize(10, -1);
    fields.truncate(10);
    fields
}

/// Renders a `T` record the way the reference does.
fn t_record(
    slot: u32,
    activate: &[i64],
    deactivate: &[i64],
    links: &[i64],
    page: i64,
    x: i64,
    y: i64,
) -> String {
    let mut fields = ten(activate);
    fields.extend(ten(deactivate));
    fields.extend(ten(links));
    fields.extend(ten(&[]));
    let text = fields
        .iter()
        .map(i64::to_string)
        .collect::<Vec<_>>()
        .join(",");
    format!("T{slot},{text},{page},{x},{y}")
}

// ---------------------------------------------------------------------------
// Ground truth for example_sequential.clprj
// ---------------------------------------------------------------------------

#[test]
fn example_sequential_ground_truth() {
    let path = corpus_dir().join("example_sequential.clprj");
    if !path.is_file() {
        println!("note: skipping, the corpus is absent");
        return;
    }
    let bytes = std::fs::read(&path).expect("the corpus file is readable");
    let report = classicladder::import_bytes(&bytes).expect("the project imports");

    // `sections.csv` marks section 1 as the sequential one and points it at
    // page 0.
    let section = report
        .project
        .sections
        .iter()
        .find(|section| section.language == SectionLanguage::Sfc)
        .expect("the project has a sequential section");
    assert_eq!(section.id, 1);
    assert_eq!(section.name, "Grafcet");

    let mut expected = SequentialPage::new(0, "sequential demo !");
    expected.steps = vec![
        Step {
            number: 0,
            is_initial: true,
            x: 1,
            y: 1,
            page: 0,
        },
        Step {
            number: 1,
            is_initial: false,
            x: 1,
            y: 5,
            page: 0,
        },
        Step {
            number: 2,
            is_initial: false,
            x: 1,
            y: 7,
            page: 0,
        },
        Step {
            number: 3,
            is_initial: false,
            x: 3,
            y: 5,
            page: 0,
        },
        Step {
            number: 4,
            is_initial: false,
            x: 3,
            y: 7,
            page: 0,
        },
        Step {
            number: 5,
            is_initial: false,
            x: 1,
            y: 9,
            page: 0,
        },
        Step {
            number: 6,
            is_initial: false,
            x: 3,
            y: 9,
            page: 0,
        },
        Step {
            number: 7,
            is_initial: false,
            x: 1,
            y: 11,
            page: 0,
        },
        Step {
            number: 8,
            is_initial: false,
            x: 1,
            y: 3,
            page: 0,
        },
        Step {
            number: 9,
            is_initial: false,
            x: 3,
            y: 3,
            page: 0,
        },
    ];
    let transition =
        |number: u32, condition: u32, from: &[u32], to: &[u32], x: i32, y: i32| Transition {
            number,
            condition: Some(mem_bit(condition)),
            from: from.to_vec(),
            to: to.to_vec(),
            page: 0,
            x,
            y,
        };
    // T0 is the AND divergence: it activates steps 8 and 9 together. T1 and T2
    // are the AND convergence/divergence pair: both need steps 1 and 3 active,
    // and each activates its own branch.
    expected.transitions = vec![
        transition(0, 1, &[0], &[8, 9], 1, 2),
        transition(1, 2, &[1, 3], &[2], 1, 6),
        transition(2, 3, &[1, 3], &[4], 3, 6),
        transition(3, 4, &[2], &[5], 1, 8),
        transition(4, 5, &[4], &[6], 3, 8),
        transition(5, 6, &[5], &[7], 1, 10),
        transition(6, 7, &[6], &[7], 3, 10),
        transition(7, 8, &[7], &[0], 1, 12),
        transition(8, 9, &[8], &[1], 1, 4),
        transition(9, 10, &[9], &[3], 3, 4),
    ];

    assert_eq!(section.sequential_page.as_ref(), Some(&expected));
}

// ---------------------------------------------------------------------------
// The corpus
// ---------------------------------------------------------------------------

#[test]
fn the_sequential_corpus_imports_executes_and_round_trips() {
    let files = corpus_files();
    if files.is_empty() {
        return;
    }
    let mut pages_found = 0usize;
    let mut steps_found = 0usize;
    let mut transitions_found = 0usize;
    for path in files {
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let bytes = std::fs::read(&path).expect("the corpus file is readable");
        let document = Document::parse(&bytes).expect("the container parses");
        let Some(_) = document.part("sequential.csv") else {
            continue;
        };
        let report = match classicladder::import_bytes(&bytes) {
            Ok(report) => report,
            Err(error) => panic!("{name} must import, got {error}"),
        };
        for diagnostic in &report.diagnostics {
            assert_ne!(
                diagnostic.severity,
                Severity::Error,
                "{name}: importing the sequential part must not error: {diagnostic}"
            );
        }

        for page in pages(&report.project) {
            pages_found += 1;
            steps_found += page.steps.len();
            transitions_found += page.transitions.len();
        }

        // import -> export -> import is a fixed point.
        let first =
            classicladder::export(&report.project, &report.extras).expect("the export succeeds");
        let first_bytes = first
            .document
            .to_bytes(false)
            .expect("the document renders");
        let second_import = match Document::parse(&first_bytes) {
            Ok(document) => classicladder::import(&document).expect("the export re-imports"),
            Err(error) => panic!("{name}: the export is not a container: {error}"),
        };
        assert_eq!(
            second_import.project, report.project,
            "{name}: import -> export -> import is not a fixed point"
        );
        let second = classicladder::export(&second_import.project, &second_import.extras)
            .expect("the second export succeeds");
        assert_eq!(
            first_bytes,
            second
                .document
                .to_bytes(false)
                .expect("the document renders"),
            "{name}: the second export is not byte-identical to the first"
        );

        // The imported chart runs without an error diagnostic. A host of
        // warnings (deprecated families, unmodelled parts) is expected, and
        // some corpus projects still raise errors in their *ladder* sections
        // (untranslated `=`-assignments, a known M3 gap), so the assertion is
        // scoped to the diagnostics a sequential section raises.
        let mut engine = softladder_core::ScanEngine::new(report.project.clone());
        for now in [0u64, 10, 20] {
            let scan = engine.scan_once(now);
            for diagnostic in &scan.diagnostics {
                if diagnostic.severity != Severity::Error {
                    continue;
                }
                let sequential = diagnostic
                    .section
                    .and_then(|index| report.project.sections.get(index))
                    .is_some_and(|section| section.language == SectionLanguage::Sfc);
                assert!(
                    !sequential,
                    "{name}: a sequential section must scan without errors: {diagnostic}"
                );
            }
        }
    }
    assert_eq!(pages_found, CORPUS_PAGES, "pages imported from the corpus");
    assert_eq!(steps_found, CORPUS_STEPS, "steps imported from the corpus");
    assert_eq!(
        transitions_found, CORPUS_TRANSITIONS,
        "transitions imported from the corpus"
    );
}

// ---------------------------------------------------------------------------
// A page no section references
// ---------------------------------------------------------------------------

#[test]
fn a_page_no_section_references_is_kept_in_a_synthesized_section() {
    let body = format!(
        "#VER=1.0\nS0,1,0,0,1,1\nS1,0,1,0,1,3\n{}\nC0,0,0/0\n",
        t_record(0, &[1], &[0], &[], 0, 1, 2)
    );
    let document = container(&[
        (
            "sections.csv",
            "#VER=1.0\n#NAME000=Ladder\n000,0,-1,0,0,0\n",
        ),
        ("sequential.csv", &body),
    ]);
    let report = classicladder::import(&document).expect("the document imports");
    let chart = report
        .project
        .sections
        .iter()
        .find(|section| section.sequential_page.is_some())
        .expect("the unreferenced page got a home");
    assert_eq!(chart.id, 1);
    assert_eq!(chart.name, "Sequential0");
    let page = chart
        .sequential_page
        .as_ref()
        .expect("the chart has a page");
    assert_eq!(page.steps.len(), 2);
    assert_eq!(page.transitions.len(), 1);
    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "SL-W030"
                && diagnostic.message.contains("sequential.csv")),
        "the synthesized section is reported"
    );

    // And the synthesis survives the round trip.
    let exported = classicladder::export(&report.project, &report.extras).expect("export");
    let reimported = classicladder::import(&exported.document).expect("re-import");
    assert_eq!(reimported.project, report.project);
}

// ---------------------------------------------------------------------------
// Export, cell by cell
// ---------------------------------------------------------------------------

#[test]
fn a_hand_built_chart_exports_cell_by_cell() {
    let mut page = SequentialPage::new(0, "start");
    page.steps = vec![
        Step {
            number: 0,
            is_initial: true,
            x: 1,
            y: 1,
            page: 0,
        },
        Step {
            number: 5,
            is_initial: false,
            x: 2,
            y: 3,
            page: 0,
        },
    ];
    page.transitions = vec![Transition {
        number: 0,
        condition: Some(mem_bit(3)),
        from: vec![0],
        to: vec![5],
        page: 0,
        x: 1,
        y: 2,
    }];
    let project = Project {
        sections: vec![Section::sfc(0, "Chart", page)],
        ..Project::new("chart")
    };

    let report = classicladder::export(&project, &Document::empty()).expect("the export succeeds");
    let expected = format!(
        "#VER=1.0\nP0,start\nS0,1,0,0,1,1\nS1,0,5,0,2,3\n{}\nC0,0,0/3\n",
        t_record(0, &[1], &[0], &[], 0, 1, 2)
    );
    assert_eq!(
        report.document.part("sequential.csv"),
        Some(expected.as_str()),
        "the step slots are renumbered densely and the transition names them"
    );
    assert_eq!(
        report.document.part("sections.csv"),
        Some("#VER=1.0\n#NAME000=Chart\n000,1,-1,0,0,0\n"),
        "an SFC section points at its page"
    );

    let reimported = classicladder::import(&report.document).expect("the chart re-imports");
    assert_eq!(reimported.project, project);
}

#[test]
fn a_transition_without_a_condition_is_reported_on_export() {
    let mut page = SequentialPage::new(0, "");
    page.steps = vec![Step::default()];
    page.transitions = vec![Transition {
        number: 0,
        condition: None,
        from: vec![0],
        to: vec![0],
        page: 0,
        ..Transition::default()
    }];
    let project = Project {
        sections: vec![Section::sfc(0, "Chart", page)],
        ..Project::new("chart")
    };
    let report = classicladder::export(&project, &Document::empty()).expect("the export succeeds");
    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "SL-W033"
                && diagnostic.message.contains("no condition")),
        "the impossible condition is reported: {:?}",
        report
            .diagnostics
            .iter()
            .map(|diagnostic| diagnostic.message.as_str())
            .collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// Hostile input
// ---------------------------------------------------------------------------

/// Imports a `sequential.csv` body together with a one-chart `sections.csv`.
fn import_sequential(body: &str) -> Vec<Diagnostic> {
    let document = container(&[
        ("sections.csv", "#VER=1.0\n#NAME000=Chart\n000,1,-1,0,0,0\n"),
        ("sequential.csv", body),
    ]);
    let report = classicladder::import(&document).expect("hostile input still imports");
    // Nothing may panic on the way out either.
    let exported = classicladder::export(&report.project, &report.extras).expect("export");
    let _ = classicladder::import(&exported.document);
    report.diagnostics
}

#[test]
fn hostile_sequential_records_produce_diagnostics_not_panics() {
    let no_step = format!("#VER=1.0\n{}\n", t_record(0, &[0], &[7], &[], 0, 1, 2));
    let other_page = format!(
        "#VER=1.0\nS0,1,0,0,1,1\n{}\n",
        t_record(0, &[], &[0], &[], 9, 1, 2)
    );
    let cases: Vec<(&str, String)> = vec![
        ("truncated step", "#VER=1.0\nS0,1,0\n".to_owned()),
        ("unusable number", "#VER=1.0\nS0,1,zero,0,1,1\n".to_owned()),
        ("truncated transition", "#VER=1.0\nT0,1,2\n".to_owned()),
        ("unknown record", "#VER=1.0\nZ0,whatever\n".to_owned()),
        (
            "colliding step numbers",
            "#VER=1.0\nS0,1,0,0,1,1\nS1,0,0,0,1,3\n".to_owned(),
        ),
        ("transition without a step record", no_step),
        ("transition on a page nothing else uses", other_page),
        ("newer version", "#VER=9.0\nS0,1,0,0,1,1\n".to_owned()),
    ];
    for (label, body) in &cases {
        let diagnostics = import_sequential(body);
        assert!(
            diagnostics
                .iter()
                .all(|diagnostic| diagnostic.code.starts_with("SL-")),
            "{label}: unstable diagnostic code"
        );
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("sequential.csv")),
            "{label}: the diagnostic must name the part: {:?}",
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.message.as_str())
                .collect::<Vec<_>>()
        );
    }

    // The malformed records themselves are errors, and they are numbered.
    let codes: Vec<&str> = import_sequential("#VER=1.0\nS0,1,0\n")
        .iter()
        .map(|diagnostic| diagnostic.code)
        .collect();
    assert!(codes.contains(&"SL-E030"), "{codes:?}");
    let codes: Vec<&str> = import_sequential("#VER=1.0\nS0,1,0,0,1,1\nS1,0,0,0,1,3\n")
        .iter()
        .map(|diagnostic| diagnostic.code)
        .collect();
    assert!(
        codes.contains(&"SL-E030"),
        "colliding step numbers: {codes:?}"
    );
}

#[test]
fn a_page_with_nothing_but_an_empty_comment_is_not_a_page() {
    let document = container(&[
        ("sections.csv", "#VER=1.0\n#NAME000=Chart\n000,1,-1,0,0,0\n"),
        ("sequential.csv", "#VER=1.0\nP0,\n"),
    ]);
    let report = classicladder::import(&document).expect("the document imports");
    assert!(
        pages(&report.project).is_empty(),
        "an empty comment is not a chart"
    );
    let exported = classicladder::export(&report.project, &report.extras).expect("export");
    let reimported = classicladder::import(&exported.document).expect("reimport");
    assert_eq!(reimported.project, report.project);
    assert_eq!(
        reimported
            .project
            .sections
            .first()
            .and_then(|section| section.sequential_page.as_ref()),
        None
    );
}

#[test]
fn a_skipped_transition_keeps_the_round_trip_a_fixed_point() {
    // `VarType 290` is `%SW<n>`, which SoftLadder does not model: the
    // transition is skipped, and the numbering of the transitions that survive
    // must still round-trip.
    let body = format!(
        "#VER=1.0\nS0,1,0,0,1,1\nS1,0,1,0,1,3\n{}\nC0,0,290/0\n",
        t_record(0, &[1], &[0], &[], 0, 1, 2)
    );
    let document = container(&[
        ("sections.csv", "#VER=1.0\n#NAME000=Chart\n000,1,-1,0,0,0\n"),
        ("sequential.csv", &body),
    ]);
    let report = classicladder::import(&document).expect("the document imports");
    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "SL-W031"),
        "the unmodelled variable family is reported"
    );
    assert!(
        pages(&report.project)
            .iter()
            .all(|page| page.transitions.is_empty()),
        "the transition with the unmodelled condition is skipped"
    );

    let first =
        classicladder::export(&report.project, &report.extras).expect("the export succeeds");
    let first_bytes = first
        .document
        .to_bytes(false)
        .expect("the document renders");
    let second = classicladder::import(&Document::parse(&first_bytes).expect("container"))
        .expect("reimport");
    assert_eq!(second.project, report.project);
    let second_bytes =
        classicladder::export(&second.project, &second.extras).expect("second export");
    assert_eq!(
        first_bytes,
        second_bytes.document.to_bytes(false).expect("renders")
    );
}

#[test]
fn sequential_comments_and_or_links_are_reported() {
    let body = format!(
        "#VER=1.0\n\
         S0,1,0,0,1,1\nS1,0,1,0,1,3\n\
         {}\n\
         C0,0,0/0\n\
         N0,0,1,1,The first step\n",
        t_record(0, &[1], &[0], &[1], 0, 1, 2)
    );
    let diagnostics = import_sequential(&body);
    let messages: Vec<&str> = diagnostics
        .iter()
        .map(|diagnostic| diagnostic.message.as_str())
        .collect();
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "SL-W030"
                && diagnostic.message.contains("sequential comment")),
        "an N record is reported: {messages:?}"
    );
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "SL-W030"
                && diagnostic.message.contains("OR-branch")),
        "the editor's OR links are reported: {messages:?}"
    );
}
