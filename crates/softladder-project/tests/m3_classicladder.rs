//! M3 acceptance tests: ClassicLadder import and export.
//!
//! The golden corpus lives in `testdata/classicladder-corpus/projects_examples`
//! (fetched by `scripts/fetch_corpus.sh`, git-ignored). When it is absent the
//! corpus group skips itself with a printed note, so the rest of the suite runs
//! in a bare checkout.

use std::path::PathBuf;

use softladder_core::model::WireMode;
use softladder_core::{
    ElementKind, PlacedElement, Project, Rung, ScanConfig, Section, SectionLanguage, Severity,
    SimSwitch, Symbol, TimerMode, VarKind, VarRef,
};
use softladder_project::classicladder::{self, Document};

const CORPUS_RUNGS: usize = 272;
const CORPUS_ELEMENTS: usize = 10257;

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

/// The cells of one exported row, trimmed.
fn row_cells(document: &Document, part: &str, row: usize) -> Vec<String> {
    let contents = document.part(part).expect("the part exists");
    let line = contents
        .lines()
        .filter(|line| !line.starts_with('#') && !line.is_empty())
        .nth(row)
        .expect("the row exists");
    line.split(',').map(|cell| cell.trim().to_owned()).collect()
}

/// A short project used by the geometry expectations.
fn geometry_project() -> Project {
    let mut project = Project::new("geometry");
    let mut rung = Rung::new(0);
    rung.label = "LAMP".to_owned();
    rung.comment = "A gap, a coil and a timer".to_owned();
    rung.elements.push(PlacedElement::with_var(
        ElementKind::ContactNo,
        VarRef::new(VarKind::PhysIn, 0),
        0,
        0,
    ));
    rung.elements.push(PlacedElement::with_var(
        ElementKind::CoilOut,
        VarRef::new(VarKind::PhysOut, 0),
        3,
        0,
    ));
    let mut timer = PlacedElement::with_var(
        ElementKind::Timer {
            mode: TimerMode::On,
        },
        VarRef::new(VarKind::TimerIec, 0),
        6,
        0,
    );
    timer.params = vec!["2500".to_owned()];
    rung.elements.push(timer);
    project.rungs.push(rung);
    let mut section = Section::new(0, "Main");
    section.rungs.push(0);
    project.sections.push(section);
    project
}

#[test]
fn document_api_round_trips_parts() {
    let source = "_FILES_CLASSICLADDER\n_FILE-general.txt\n#VER=3.0\n_/FILE-general.txt\n_/FILES_CLASSICLADDER\n";
    let document = Document::parse(source.as_bytes()).expect("the container parses");
    assert_eq!(document.parts().len(), 1);
    assert_eq!(document.serialize(), source);
    let mut edited = document.clone();
    edited.set_part("extra.txt", "hello\n".to_owned());
    assert!(edited.part("extra.txt").is_some());
    edited.remove_part("extra.txt");
    assert_eq!(edited, document);
}

// ---------------------------------------------------------------------------
// Ground truth for example.clprj
// ---------------------------------------------------------------------------

#[test]
fn example_project_rung_zero_ground_truth() {
    if corpus_files().is_empty() {
        return;
    }
    let path = corpus_dir().join("example.clprj");
    let report = classicladder::import_file(&path).expect("example.clprj imports");
    let project = &report.project;

    assert_eq!(project.name, "Example project");
    assert_eq!(project.author, "Marc Le Douarain");
    assert_eq!(
        project.comment,
        "Comment for the example project\nbla bla bla...\nand another third line !"
    );
    assert_eq!(
        project.scan,
        ScanConfig {
            period_ms: 50,
            input_period_ms: 10
        }
    );

    let rung = project.rung(0).expect("rung 0 is imported");
    assert_eq!(rung.label, "START");
    assert_eq!(rung.comment, "Big one");
    assert_eq!(rung.wire_mode, WireMode::Explicit);

    // The reference file's rows 0 and 1 are:
    //
    //   `1-0-50/1 , 2-0-50/2 , 9-0-0/0 , 9-0-0/0 , 9-0-0/0 , 99-0-0/0 ,
    //    13-0-0/0 , 9-0-0/0 , 9-0-0/0 , 9-0-0/0 , 9-0-0/0 , 50-0-0/1`
    //   `1-0-0/1 , 0-0-0/0 , 0-0-0/0 , 0-0-0/0 , 0-0-0/0 , 99-0-0/0 ,
    //    99-0-0/0 , 0-0-0/0 , 0-0-0/0 , 0-0-0/1 , 0-0-0/0 , 0-0-0/0`
    //
    // The `ELE_UNUSABLE` body cells of the timer produce nothing, and cell
    // (9,1) is a free cell whose *VarNum* is 1 (`0-0-0/1` is
    // `Type-ConnectedWithTop-VarType/VarNum`), so it carries no vertical link
    // and is not materialised either.
    let expected: Vec<(ElementKind, Option<VarRef>, u8, u8, bool)> = vec![
        (ElementKind::ContactNo, Some(v("%I1")), 0, 0, false),
        (ElementKind::ContactNc, Some(v("%I2")), 1, 0, false),
        (ElementKind::Connection, None, 2, 0, false),
        (ElementKind::Connection, None, 3, 0, false),
        (ElementKind::Connection, None, 4, 0, false),
        // The timer is placed on the column the reference taps for its inputs
        // (the body column, whose `99-` cell the file holds at column 5), and a
        // wire in the reference's "alive" column carries its output onwards.
        (
            ElementKind::Timer {
                mode: TimerMode::Off,
            },
            Some(VarRef::new(VarKind::TimerIec, 0)),
            5,
            0,
            false,
        ),
        (ElementKind::Connection, None, 6, 0, false),
        (ElementKind::Connection, None, 7, 0, false),
        (ElementKind::Connection, None, 8, 0, false),
        (ElementKind::Connection, None, 9, 0, false),
        (ElementKind::Connection, None, 10, 0, false),
        (ElementKind::CoilOut, Some(v("%M1")), 11, 0, false),
        (ElementKind::ContactNo, Some(v("%M1")), 0, 1, false),
    ];
    // Rows 0 and 1 are exactly the cells the reference file holds; the lower
    // rows of the same rung are checked by the corpus tests.
    let first_rows: Vec<&PlacedElement> = rung
        .elements
        .iter()
        .filter(|element| element.row < 2)
        .collect();
    assert_eq!(first_rows.len(), expected.len());
    for (element, (kind, var, col, row, linked)) in first_rows.iter().zip(&expected) {
        assert_eq!(element.kind, *kind, "cell ({col},{row})");
        assert_eq!(element.var, *var, "cell ({col},{row})");
        assert_eq!((element.col, element.row), (*col, *row));
        assert_eq!(element.connected_with_top, *linked, "cell ({col},{row})");
    }
    // The timer keeps the preset and the time base of `timers_iec.csv`
    // (`TM0,0,2,1` is two 60-minute units on an off-delay timer). It is placed on
    // the body column so that its enable reads the same power the reference taps.
    let timer = rung
        .elements
        .iter()
        .find(|element| matches!(element.kind, ElementKind::Timer { .. }))
        .expect("the timer is there");
    assert_eq!(timer.params, vec!["2m".to_owned()]);

    let section = &project.sections[0];
    assert_eq!(section.id, 0);
    assert_eq!(section.name, "Prog1");
    assert_eq!(section.language, SectionLanguage::Ladder);
    assert_eq!(section.subroutine, None);
    assert_eq!(section.rungs, vec![0, 1, 2, 3, 7, 4, 5, 6, 8]);

    let input = project
        .symbols
        .iter()
        .find(|symbol| symbol.name == "input1")
        .expect("the `input1` symbol is imported");
    assert_eq!(input.var, Some(v("%I1")));
    let register = project
        .symbols
        .iter()
        .find(|symbol| symbol.name == "WordA")
        .expect("the `WordA` symbol is imported");
    assert_eq!(register.var, Some(v("%MW1")));
    let timer_symbol = project
        .symbols
        .iter()
        .find(|symbol| symbol.name == "tim")
        .expect("the `tim` symbol is imported");
    assert_eq!(timer_symbol.var, Some(v("%TM2.Q")));
    assert_eq!(timer_symbol.comment, "PARTIAL SYMBOL: tim.X use");
}

/// Parses a canonical variable reference.
fn v(text: &str) -> VarRef {
    text.parse().expect("the test variable parses")
}

// ---------------------------------------------------------------------------
// Corpus
// ---------------------------------------------------------------------------

#[test]
fn corpus_imports_and_reports_its_diagnostics() {
    let files = corpus_files();
    if files.is_empty() {
        return;
    }
    let mut rungs = 0usize;
    let mut elements = 0usize;
    for path in files {
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let bytes = std::fs::read(&path).expect("the corpus file is readable");
        let report = match classicladder::import_bytes(&bytes) {
            Ok(report) => report,
            Err(error) => panic!("{name} must import, got {error}"),
        };
        for diagnostic in &report.diagnostics {
            assert!(
                diagnostic.code.starts_with("SL-"),
                "{name}: unstable diagnostic code {}",
                diagnostic.code
            );
            assert!(
                diagnostic.message.contains(".csv") || diagnostic.message.contains(".txt"),
                "{name}: diagnostic without a location: {diagnostic}"
            );
            assert_ne!(
                diagnostic.severity,
                Severity::Error,
                "{name}: unexpected error diagnostic: {diagnostic}"
            );
        }
        rungs += report.project.rungs.len();
        elements += report
            .project
            .rungs
            .iter()
            .map(|rung| rung.elements.len())
            .sum::<usize>();
    }
    assert_eq!(
        rungs, CORPUS_RUNGS,
        "total rungs imported across the corpus"
    );
    assert_eq!(
        elements, CORPUS_ELEMENTS,
        "total elements imported across the corpus"
    );
}

#[test]
fn corpus_round_trip_is_a_fixed_point() {
    let files = corpus_files();
    if files.is_empty() {
        return;
    }
    for path in files {
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let bytes = std::fs::read(&path).expect("the corpus file is readable");
        let report = classicladder::import_bytes(&bytes).expect("the corpus file imports");
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
            "{name}: import → export → import is not a fixed point"
        );

        let second = classicladder::export(&second_import.project, &second_import.extras)
            .expect("the second export succeeds");
        let second_bytes = second
            .document
            .to_bytes(false)
            .expect("the document renders");
        assert_eq!(
            first_bytes, second_bytes,
            "{name}: the second export is not byte-identical to the first"
        );
    }
}

#[test]
fn corpus_passthrough_parts_survive_byte_for_byte() {
    let files = corpus_files();
    if files.is_empty() {
        return;
    }
    for path in files {
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let bytes = std::fs::read(&path).expect("the corpus file is readable");
        let report = classicladder::import_bytes(&bytes).expect("the corpus file imports");
        let exported =
            classicladder::export(&report.project, &report.extras).expect("the export succeeds");
        for (part, contents) in report.extras.parts() {
            // `general.txt` and `project_infos.txt` are partly modelled: their
            // keys are rewritten in place and every other line is kept, which
            // the fixed-point test above covers byte for byte.
            if part == "general.txt" || part == "project_infos.txt" {
                continue;
            }
            assert_eq!(
                exported.document.part(part),
                Some(contents.as_str()),
                "{name}: passthrough part `{part}` changed"
            );
        }
        // Every passthrough part of the document is reported.
        for diagnostic in report
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "SL-W032")
        {
            assert!(
                report.extras.parts().iter().any(|(part, _)| {
                    part != "general.txt"
                        && part != "project_infos.txt"
                        && diagnostic.message.contains(part.as_str())
                }),
                "{name}: SL-W032 without a passthrough part: {diagnostic}"
            );
        }
    }
}

#[test]
fn corpus_files_survive_truncation_and_byte_flips() {
    let files = corpus_files();
    if files.is_empty() {
        return;
    }
    for path in files {
        let bytes = std::fs::read(&path).expect("the corpus file is readable");
        for cut in [
            0,
            1,
            bytes.len() / 3,
            bytes.len() / 2,
            bytes.len().saturating_sub(1),
        ] {
            let _ = Document::parse(&bytes[..cut.min(bytes.len())]);
            let _ = classicladder::import_bytes(&bytes[..cut.min(bytes.len())]);
        }
        for position in [
            0,
            bytes.len() / 4,
            bytes.len() / 2,
            bytes.len().saturating_sub(1),
        ] {
            let mut mutated = bytes.clone();
            if let Some(byte) = mutated.get_mut(position) {
                *byte = byte.wrapping_add(1);
            }
            if let Ok(document) = Document::parse(&mutated) {
                if let Ok(report) = classicladder::import(&document) {
                    let _ = classicladder::export(&report.project, &report.extras);
                }
            }
        }
    }
}

#[test]
fn a_deprecated_timer_keeps_its_preset_and_base() {
    // `timers.csv` in the legacy positional spelling: `<base>,<preset>` per
    // row, so row 0 is a 5-second preset with `ELE_TIMER` instance 0.
    let document = container(&[
        (
            "rung_0.csv",
            "#VER=2.0\n#LABEL=\n#COMMENT=\n#PREVRUNG=-1\n#NEXTRUNG=-1\n#NBRLINES=8\n\
             1-0-50/0 , 9-0-0/0 , 10-0-0/0 , 9-0-0/0 , 0-0-0/0 , 0-0-0/0 , 0-0-0/0 , 0-0-0/0 , \
             0-0-0/0 , 0-0-0/0 , 0-0-0/0 , 50-0-0/0\n",
        ),
        ("timers.csv", "1,5\n"),
        ("sections.csv", "#VER=1.0\n000,0,-1,0,0,0\n"),
    ]);
    let report = classicladder::import(&document).expect("the rung imports");
    let timer = report.project.rungs[0]
        .elements
        .iter()
        .find(|element| matches!(element.kind, ElementKind::Timer { .. }))
        .expect("the deprecated timer is imported");
    assert_eq!(
        timer.kind,
        ElementKind::Timer {
            mode: TimerMode::On
        }
    );
    assert_eq!(timer.var, Some(VarRef::new(VarKind::TimerIec, 0)));
    assert_eq!(timer.params, vec!["5000".to_owned()]);
    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "SL-W031"
                && diagnostic.rung == Some(0)
                && diagnostic.message.contains("cell (2,0)")),
        "the deprecated timer names its rung and cell: {:?}",
        report.diagnostics
    );
}

#[test]
fn corpus_deprecated_families_are_warned_about() {
    let files = corpus_files();
    if files.is_empty() {
        return;
    }
    let mut warnings = 0usize;
    for path in files {
        let bytes = std::fs::read(&path).expect("the corpus file is readable");
        let report = classicladder::import_bytes(&bytes).expect("the corpus file imports");
        warnings += report
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "SL-W031")
            .count();
    }
    assert!(
        warnings > 0,
        "the corpus uses deprecated `%T`/`%M` families, which must be reported"
    );
}

// ---------------------------------------------------------------------------
// Hostile input
// ---------------------------------------------------------------------------

#[test]
fn a_free_cell_with_a_vertical_link_becomes_a_connection() {
    // `0-1-0/0` is `ELE_FREE` with `ConnectedWithTop = 1`: the reference stores
    // the vertical link on the cell, not on an element, so importing it has to
    // materialise a linked `Connection` or the branch would change shape.
    let document = container(&[(
        "rung_0.csv",
        "#VER=3.0\n#LABEL=\n#COMMENT=\n#PREVRUNG=-1\n#NEXTRUNG=-1\n#NBRLINES=8\n\
         1-0-50/0 , 9-0-0/0 , 0-1-0/0 , 0-0-0/0 , 0-0-0/0 , 0-0-0/0 , 0-0-0/0 , 0-0-0/0 , \
         0-0-0/0 , 0-0-0/0 , 0-0-0/0 , 50-0-0/0\n\
         0-0-0/0 , 0-0-0/0 , 1-0-50/1 , 0-0-0/0 , 0-0-0/0 , 0-0-0/0 , 0-0-0/0 , 0-0-0/0 , \
         0-0-0/0 , 0-0-0/0 , 0-0-0/0 , 0-0-0/0\n",
    )]);
    let report = classicladder::import(&document).expect("the rung imports");
    let rung = &report.project.rungs[0];
    let linked = rung
        .elements
        .iter()
        .find(|element| element.connected_with_top)
        .expect("the vertical link is materialised");
    assert_eq!(linked.kind, ElementKind::Connection);
    assert_eq!((linked.col, linked.row), (2, 0));
    // contact, connection, linked connection and coil on row 0, the branch
    // contact on row 1.
    assert_eq!(rung.elements.len(), 5);
}

#[test]
fn malformed_containers_are_rejected() {
    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("empty input", Vec::new()),
        (
            "missing header",
            b"_FILE-general.txt\nx\n_/FILE-general.txt\n_/FILES_CLASSICLADDER\n".to_vec(),
        ),
        (
            "truncated container",
            b"_FILES_CLASSICLADDER\n_FILE-general.txt\n#VER=3.0\n".to_vec(),
        ),
        (
            "missing end marker",
            b"_FILES_CLASSICLADDER\n_FILE-general.txt\nx\n_/FILE-general.txt\n".to_vec(),
        ),
        (
            "part never closed",
            b"_FILES_CLASSICLADDER\n_FILE-general.txt\nx\n_/FILES_CLASSICLADDER\n".to_vec(),
        ),
        (
            "mismatched close",
            b"_FILES_CLASSICLADDER\n_FILE-a.txt\n_FILE-b.txt\n_/FILES_CLASSICLADDER\n".to_vec(),
        ),
        ("non-UTF-8", vec![0xff, 0xfe, 0x00, 0x01]),
    ];
    for (name, bytes) in cases {
        assert!(
            Document::parse(&bytes).is_err(),
            "`{name}` must not parse as a container"
        );
        assert!(
            classicladder::import_bytes(&bytes).is_err(),
            "`{name}` must not import"
        );
    }

    // A gzip stream that is not a container: valid compression, wrong payload.
    let plain = b"not a container at all\n";
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    std::io::Write::write_all(&mut encoder, plain).expect("compresses");
    let compressed = encoder.finish().expect("finishes");
    assert!(
        classicladder::import_bytes(&compressed).is_err(),
        "a gzip stream that is not a container must be rejected"
    );
}

#[test]
fn a_part_header_without_a_body_is_an_empty_part() {
    let document = container(&[("general.txt", ""), ("sections.csv", "")]);
    let report = classicladder::import(&document).expect("an empty part imports");
    assert!(report.project.rungs.is_empty());
    assert_eq!(report.project.sections.len(), 1);
}

#[test]
fn hostile_parts_produce_diagnostics_instead_of_panicking() {
    let document = container(&[
        (
            "rung_0.csv",
            "#VER=3.0\n#LABEL=x\nnot-a-cell , 1-0-50/0 , 1-0-abc/0 , 9-0-0/0 , 77-0-0/0 , \
             1-0-999/0 , 20-0-0/4242 , 60-0-0/-1\n",
        ),
        (
            "sections.csv",
            "#VER=1.0\nnot,numbers\n000,0,-1,99999999999999,0,0\n",
        ),
        ("counters.csv", "C0,not-a-number\n"),
        ("timers_iec.csv", "TM0,99,1,1\n"),
        ("symbols.csv", "%ZZZ,bad,\n%I0,good,\n"),
        ("arithmetic_expressions.csv", "#VER=2.0\n0000,\n"),
        (
            "general.txt",
            "PERIODIC_REFRESH=abc\nSIZE_REGISTER_LIST=-1\n",
        ),
    ]);
    let report = classicladder::import(&document).expect("hostile parts still import");
    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "SL-E030"),
        "a malformed part must be reported as SL-E030"
    );
    // The valid cell of the rung survives.
    let rung = &report.project.rungs[0];
    assert!(rung
        .elements
        .iter()
        .any(|element| element.kind == ElementKind::ContactNo));
    // The export still produces a container.
    classicladder::export(&report.project, &report.extras).expect("the export survives");
}

#[test]
fn absurd_counts_and_unknown_types_do_not_panic() {
    let document = container(&[
        (
            "rung_4294967295.csv",
            "#VER=3.0\n#NBRLINES=99999999999\n1-0-50/4294967295\n",
        ),
        (
            "rung_0.csv",
            "#VER=9.9\n#PREVRUNG=-99999\n#NEXTRUNG=99999\n0-0-0/0\n",
        ),
        ("sections.csv", "#VER=1.0\n000,7,-88,0,-5,0\n"),
        ("general.txt", "SIZE_NBR_RUNGS=999999999999999999999999\n"),
    ]);
    let report = classicladder::import(&document).expect("absurd counts still import");
    assert!(report
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.severity == Severity::Error));
    classicladder::export(&report.project, &report.extras).expect("the export survives");
}

// ---------------------------------------------------------------------------
// Export geometry
// ---------------------------------------------------------------------------

#[test]
fn export_rebuilds_the_reference_matrix_cell_by_cell() {
    let project = geometry_project();
    let report = classicladder::export(&project, &Document::empty()).expect("the export succeeds");
    let document = report.document;

    let expected_row_0 = vec![
        "1-0-50/0",
        "9-0-0/0",
        "9-0-0/0",
        "50-0-60/0",
        "9-0-0/0",
        "9-0-0/0",
        "99-0-0/0",
        "13-0-0/0",
        "0-0-0/0",
        "0-0-0/0",
        "0-0-0/0",
        "0-0-0/0",
    ];
    assert_eq!(row_cells(&document, "rung_0.csv", 0), expected_row_0);
    // The timer's two body cells on the second row, and nothing else.
    let expected_row_1 = vec![
        "0-0-0/0", "0-0-0/0", "0-0-0/0", "0-0-0/0", "0-0-0/0", "0-0-0/0", "99-0-0/0", "99-0-0/0",
        "0-0-0/0", "0-0-0/0", "0-0-0/0", "0-0-0/0",
    ];
    assert_eq!(row_cells(&document, "rung_0.csv", 1), expected_row_1);
    for row in 2..8 {
        assert_eq!(
            row_cells(&document, "rung_0.csv", row),
            vec!["0-0-0/0"; 12],
            "row {row} must be empty"
        );
    }

    let rung = document.part("rung_0.csv").expect("the rung is written");
    assert!(rung.starts_with("#VER=3.0\n#LABEL=LAMP\n"));
    assert!(rung.contains("#COMMENT=A gap, a coil and a timer\n"));
    assert!(rung.contains("#NBRLINES=8\n"));

    assert_eq!(
        document.part("timers_iec.csv"),
        Some("#VER=2.0\nTM0,2,25,0\n")
    );
    assert_eq!(document.part("counters.csv"), Some("#VER=2.0\n"));
    assert_eq!(document.part("registers.csv"), Some("#VER=1.0\n"));
    assert_eq!(
        document.part("sections.csv"),
        Some("#VER=1.0\n#NAME000=Main\n000,0,-1,0,0,0\n")
    );

    let general = document
        .part("general.txt")
        .expect("general.txt is written");
    for line in [
        "PERIODIC_REFRESH=10",
        "PERIODIC_INPUTS_REFRESH=10",
        "SIZE_NBR_RUNGS=300",
        "SIZE_NBR_BITS=500",
        "SIZE_NBR_WORDS=200",
        "SIZE_NBR_COUNTERS=50",
        "SIZE_NBR_TIMERS_IEC=50",
        "SIZE_NBR_REGISTERS=10",
        "SIZE_REGISTER_LIST=500",
        "SIZE_NBR_PHYS_INPUTS=50",
        "SIZE_NBR_PHYS_OUTPUTS=50",
        "SIZE_NBR_ARITHM_EXPR=200",
        "SIZE_NBR_SECTIONS=10",
        "SIZE_NBR_SYMBOLS=300",
    ] {
        assert!(
            general.lines().any(|candidate| candidate == line),
            "general.txt must contain `{line}`, got:\n{general}"
        );
    }
}

#[test]
fn an_explicit_rung_keeps_its_gaps_and_an_implicit_one_fills_them() {
    // Implicit wiring: the gap between the contact and the coil conducts.
    let mut implicit = Project::new("implicit");
    let mut rung = Rung::new(0);
    rung.elements.push(PlacedElement::with_var(
        ElementKind::ContactNo,
        VarRef::new(VarKind::MemBit, 0),
        0,
        0,
    ));
    rung.elements.push(PlacedElement::with_var(
        ElementKind::CoilOut,
        VarRef::new(VarKind::MemBit, 1),
        2,
        0,
    ));
    implicit.rungs.push(rung.clone());
    let mut section = Section::new(0, "Main");
    section.rungs.push(0);
    implicit.sections.push(section);

    let report = classicladder::export(&implicit, &Document::empty()).expect("the export succeeds");
    assert_eq!(
        &row_cells(&report.document, "rung_0.csv", 0)[..3],
        &["1-0-0/0", "9-0-0/0", "50-0-0/1"][..]
    );

    let reimported = classicladder::import(&report.document).expect("the export re-imports");
    let rung = reimported.project.rung(0).expect("rung 0");
    assert_eq!(rung.wire_mode, WireMode::Explicit);
    assert_eq!(rung.elements.len(), 3);
    assert_eq!(
        rung.elements[1].kind,
        ElementKind::Connection,
        "the gap is now an explicit connection"
    );

    // Explicit wiring: a gap stays a gap, so the circuit is unchanged.
    let mut explicit = implicit.clone();
    explicit.rungs[0].wire_mode = WireMode::Explicit;
    let report = classicladder::export(&explicit, &Document::empty()).expect("the export succeeds");
    assert_eq!(
        &row_cells(&report.document, "rung_0.csv", 0)[..3],
        &["1-0-0/0", "0-0-0/0", "50-0-0/1"][..]
    );
    let reimported = classicladder::import(&report.document).expect("the export re-imports");
    assert_eq!(reimported.project.rungs[0].elements.len(), 2);
}

#[test]
fn the_extras_template_is_passed_through_and_rewritten_parts_are_regenerated() {
    let document = container(&[
        (
            "project_infos.txt",
            "PROJECT_NAME=kept\nPROJECT_SITE=here\nPARAM_AUTHOR=me\nCUSTOM=x\n",
        ),
        (
            "general.txt",
            "PERIODIC_REFRESH=1\nMODBUS_MASTER_SERIAL_SPEED=38400\nCUSTOM_KEY=1\n",
        ),
        ("com_params.txt", "MODBUS_ELEMENT_OFFSET=1\n"),
        ("ioconf.csv", "#VER=1.0\n"),
        ("sections.csv", "#VER=1.0\n#NAME000=Old\n000,0,-1,0,0,0\n"),
    ]);
    let report = classicladder::import(&document).expect("the template imports");
    let exported =
        classicladder::export(&report.project, &report.extras).expect("the export succeeds");

    assert_eq!(
        exported.document.part("com_params.txt"),
        Some("MODBUS_ELEMENT_OFFSET=1\n")
    );
    assert_eq!(exported.document.part("ioconf.csv"), Some("#VER=1.0\n"));
    assert_eq!(
        exported.document.part("project_infos.txt"),
        Some(
            "PROJECT_NAME=kept\nPROJECT_SITE=here\nPARAM_AUTHOR=me\nCUSTOM=x\nPARAM_VERSION=\n\
             PARAM_COMPANY=\nCREA_DATE=\nMODIF_DATE=\nPARAM_COMMENT=\n"
        )
    );
    let general = exported.document.part("general.txt").expect("general.txt");
    assert!(general.contains("MODBUS_MASTER_SERIAL_SPEED=38400\n"));
    assert!(general.contains("CUSTOM_KEY=1\n"));
    // The scan periods are rewritten from the project, which read them from
    // this very template.
    assert!(general.contains("PERIODIC_REFRESH=1\n"));
    // The modelled part is regenerated, not passed through.
    let sections = exported
        .document
        .part("sections.csv")
        .expect("sections.csv");
    assert!(sections.contains("#NAME000=Old\n"), "{sections}");
}

#[test]
fn export_warns_about_features_the_reference_cannot_express() {
    let mut project = Project::new("warnings");
    let mut rung = Rung::new(0);
    // A bit accessor and a column outside the reference matrix.
    rung.elements.push(PlacedElement::with_var(
        ElementKind::ContactNo,
        VarRef::new(VarKind::MemWord, 0).with_bit(3),
        0,
        0,
    ));
    rung.elements.push(PlacedElement::with_var(
        ElementKind::CoilOut,
        VarRef::new(VarKind::MemBit, 0),
        14,
        0,
    ));
    project.rungs.push(rung);
    let mut sfc = Section::new(0, "Chart");
    sfc.language = SectionLanguage::Sfc;
    project.sections.push(sfc);
    project.simulation.switches.push(SimSwitch {
        var: VarRef::new(VarKind::PhysIn, 0),
        label: "start".to_owned(),
        momentary: false,
    });

    let report = classicladder::export(&project, &Document::empty()).expect("the export succeeds");
    let messages: Vec<&str> = report
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.message.as_str())
        .collect();
    assert!(
        report
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.code == "SL-W033"),
        "the export only warns with SL-W033: {messages:?}"
    );
    assert!(
        messages
            .iter()
            .any(|message| message.contains("bit access")),
        "a bit accessor must be reported: {messages:?}"
    );
    assert!(
        messages.iter().any(|message| message.contains("column 14")),
        "an out-of-range column must be reported: {messages:?}"
    );
    assert!(
        messages
            .iter()
            .any(|message| message.contains("sequential")),
        "an SFC section must be reported: {messages:?}"
    );
    assert!(
        messages
            .iter()
            .any(|message| message.contains("symbol") || message.contains("bench")),
        "the simulation panel must be reported: {messages:?}"
    );
}

#[test]
fn a_symbol_is_written_back_in_the_reference_spelling() {
    let mut project = Project::new("symbols");
    project.symbols.push(Symbol {
        name: "start".to_owned(),
        var: Some(VarRef::new(VarKind::MemBit, 1)),
        comment: "Start button".to_owned(),
        unit: None,
    });
    project.symbols.push(Symbol {
        name: "word".to_owned(),
        var: Some(VarRef::new(VarKind::MemWord, 2)),
        comment: String::new(),
        unit: None,
    });
    let report = classicladder::export(&project, &Document::empty()).expect("the export succeeds");
    assert_eq!(
        report.document.part("symbols.csv"),
        Some("#VER=1.0\n%B1,start,Start button\n%W2,word,\n")
    );
    let reimported = classicladder::import(&report.document).expect("the export re-imports");
    assert_eq!(reimported.project.symbols, project.symbols);
}

#[test]
fn the_scan_periods_and_project_properties_come_from_the_text_parts() {
    let document = container(&[
        (
            "general.txt",
            "PERIODIC_REFRESH=25\nPERIODIC_INPUTS_REFRESH=5\n",
        ),
        (
            "project_infos.txt",
            "PROJECT_NAME=Plant\nPARAM_AUTHOR=Ada\nPARAM_COMMENT=a\\nb\n",
        ),
    ]);
    let report = classicladder::import(&document).expect("the parts import");
    assert_eq!(report.project.name, "Plant");
    assert_eq!(report.project.author, "Ada");
    assert_eq!(report.project.comment, "a\nb");
    assert_eq!(
        report.project.scan,
        ScanConfig {
            period_ms: 25,
            input_period_ms: 5
        }
    );
}

#[test]
fn gzip_documents_import_and_export_transparently() {
    let project = geometry_project();
    let bytes = classicladder::export_bytes(&project, &Document::empty(), true)
        .expect("the compressed export succeeds");
    assert!(bytes.starts_with(&[0x1f, 0x8b]));
    let report = classicladder::import_bytes(&bytes).expect("the compressed project imports");
    // The implicitly wired gaps came back as explicit connections.
    assert_eq!(report.project.rungs[0].elements.len(), 8);
    assert_eq!(report.project.rungs[0].wire_mode, WireMode::Explicit);
}

#[test]
fn files_are_written_and_read_back() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let path = directory.path().join("geometry.clprj");
    let project = geometry_project();
    let report = classicladder::export_file(&project, &Document::empty(), &path)
        .expect("the export writes a file");
    assert!(report.document.part("rung_0.csv").is_some());
    let imported = classicladder::import_file(&path).expect("the file imports");
    assert_eq!(imported.project.rungs[0].elements.len(), 8);
}

// ---------------------------------------------------------------------------
// Property test
// ---------------------------------------------------------------------------

mod properties {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]

        #[test]
        fn arbitrary_bytes_never_panic(bytes in proptest::collection::vec(any::<u8>(), 0..2048)) {
            let _ = Document::parse(&bytes);
            let _ = classicladder::import_bytes(&bytes);
        }

        #[test]
        fn arbitrary_containers_never_panic(parts in proptest::collection::vec(
            ("[a-z_]{1,12}\\.(csv|txt)", proptest::collection::vec(any::<char>(), 0..256)),
            0..6,
        )) {
            let document = Document::from_parts(
                parts
                    .into_iter()
                    .map(|(name, contents)| (name, contents.into_iter().collect::<String>()))
                    .collect(),
            );
            let text = document.serialize();
            if let Ok(parsed) = Document::parse(text.as_bytes()) {
                if let Ok(report) = classicladder::import(&parsed) {
                    let _ = classicladder::export(&report.project, &report.extras);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Behavioural parity: an imported block must be enabled by the power the
// reference taps for it.
// ---------------------------------------------------------------------------

#[test]
fn an_imported_timer_is_enabled_by_the_power_the_reference_taps() {
    let path = corpus_dir().join("example.clprj");
    if !path.is_file() {
        println!("note: skipping, the corpus is absent");
        return;
    }
    let document = Document::parse(&std::fs::read(&path).expect("the corpus file is readable"))
        .expect("the container parses");
    let report = classicladder::import(&document).expect("the project imports");

    let mut engine = softladder_core::ScanEngine::new(report.project.clone());
    let bit = |text: &str| text.parse::<VarRef>().expect("variable parses");
    let set = |engine: &mut softladder_core::ScanEngine, text: &str, value: bool| {
        engine
            .store_mut()
            .set(&bit(text), softladder_core::Value::Bit(value))
            .expect("bit is writable");
    };
    let get = |engine: &softladder_core::ScanEngine, text: &str| engine.store().get(&bit(text));

    // Rung 0 of the example wires `%I1` (normally open) in series with `%I2`
    // (normally closed) into `%TM0`, an off-delay timer preset to two 60-minute
    // units. The reference reads that enable as the power arriving at the
    // block's *body* column; the importer places the block there so the same
    // power arrives. Before that mapping was fixed the block sat one column to
    // the right, on a cell the reference leaves as an unused body cell, so the
    // timer never saw its enable.
    set(&mut engine, "%I1", true);
    set(&mut engine, "%I2", false);
    engine.scan_once(0);
    assert_eq!(
        get(&engine, "%TM0.Q"),
        Some(softladder_core::Value::Bit(true)),
        "a live enable must switch the off-delay timer's output on"
    );
    assert_eq!(
        get(&engine, "%TM0.P"),
        Some(softladder_core::Value::Word(2)),
        "the preset is preserved in the timer's own time-base units"
    );

    // Breaking the enable starts the off delay: two units of the one-minute base,
    // i.e. exactly 120 000 ms. Each scan contributes exactly 1000 ms (the engine
    // clamps a single scan's elapsed time), so the output must still be on after
    // 119 counting scans and drop on the 120th.
    set(&mut engine, "%I2", true);
    let mut now = 1000u64;
    engine.scan_once(now);
    for _ in 0..118 {
        now += 1000;
        engine.scan_once(now);
    }
    assert_eq!(
        get(&engine, "%TM0.Q"),
        Some(softladder_core::Value::Bit(true)),
        "119 scans are 119 000 ms, one second short of the off delay"
    );
    now += 1000;
    engine.scan_once(now);
    assert_eq!(
        get(&engine, "%TM0.Q"),
        Some(softladder_core::Value::Bit(false)),
        "the off delay expires after exactly 120 000 ms, so the preset and the \
         one-minute base survived the import"
    );
}
