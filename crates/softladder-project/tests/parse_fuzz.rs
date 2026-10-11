//! Input-format fuzzing (M8): native projects and ClassicLadder containers.
//!
//! Same contract as the core fuzz targets: arbitrary bytes become `Ok` or
//! `Err`, never a panic — through the whole ClassicLadder pipeline
//! (`import_bytes`: container, gzip, document, mapping) and the native
//! `.slprj` loader. Inputs stay small so the suite is fast; deep coverage of
//! valid files belongs to the golden-corpus tests, not here.

use proptest::prelude::*;
use softladder_project::{classicladder, native};

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn native_projects_never_panic_on_arbitrary_text(text in "\\PC{0,300}") {
        let _ = native::from_json(&text);
    }

    #[test]
    fn classicladder_containers_never_panic_on_arbitrary_bytes(
        bytes in prop::collection::vec(any::<u8>(), 0..512),
    ) {
        let _ = classicladder::import_bytes(&bytes);
    }

    #[test]
    fn classicladder_containers_never_panic_on_hostile_fragments(
        fragment in prop::sample::select(vec![
            b"_FILES_CLASSICLADDER".to_vec(),
            b"_FILES_CLASSICLADDER\n_FILE-a\n".to_vec(),
            b"_FILE-a\nline\n_/FILE-a\n_/FILES_CLASSICLADDER".to_vec(),
            b"\x1f\x8b\x08\x00truncated gzip".to_vec(),
            b"#VER=9.9\n".to_vec(),
            b"_FILES_CLASSICLADDER\r\n_FILE-rungs.txt\r\n#VER=3.0\r\n".to_vec(),
        ]),
    ) {
        let _ = classicladder::import_bytes(&fragment);
    }
}
