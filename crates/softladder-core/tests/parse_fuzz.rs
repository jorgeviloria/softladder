//! Input-format fuzzing (M8): expressions and variable references.
//!
//! Every parser reachable from user input must turn arbitrary text into
//! `Ok` or `Err` — never a panic. Beyond that, a parsed expression must
//! display, re-parse to the same tree and evaluate against an empty store
//! without panicking either (out-of-range indices are the interesting case).

use proptest::prelude::*;
use softladder_core::{eval, parse, VarRef, VarStore};

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn expressions_never_panic_on_arbitrary_text(text in "\\PC{0,160}") {
        if let Ok(expr) = parse(&text) {
            let printed = expr.to_string();
            let reparsed = parse(&printed).expect("a displayed expression re-parses");
            prop_assert_eq!(reparsed.clone(), expr);
            let store = VarStore::new();
            let _ = eval(&reparsed, &store);
        }
        let _ = text.parse::<VarRef>();
    }

    #[test]
    fn expressions_never_panic_on_hostile_fragments(
        fragment in prop::sample::select(vec![
            "%MW[%MW0]", "%MW0.99", "%TM999999.V", "%C-1.V", "%X[",
            "$FFFFFFFF", "1/0", "SHL(%MW0,99)", "((((((((((1))))))))))",
            "%QLED99999999999", "\u{0}%I0", "%I0\u{7f}%Q1", "MINI(",
        ]),
    ) {
        if let Ok(expr) = parse(fragment) {
            let store = VarStore::new();
            let _ = eval(&expr, &store);
        }
        let _ = fragment.parse::<VarRef>();
    }
}
