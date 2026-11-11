//! Self-tests for the shared lexer (`workspace_guards::scan`): the one rule
//! every guard family relies on, so the predicate that decides what is test
//! code is pinned here once.

use workspace_guards::scan::{non_test_source, test_only_attribute};

const PLAIN: &str = "#[cfg(test)]";
const ALL_TEST: &str = "#[cfg(all(test))]";
const ALL_TEST_UNIX: &str = "#[cfg(all(test, unix))]";
const ALL_NESTED: &str = "#[cfg(all(test, all(unix, test)))]";

#[test]
fn test_only_attributes_are_recognised() {
    assert!(test_only_attribute(PLAIN), "{PLAIN}");
    assert!(test_only_attribute(ALL_TEST), "{ALL_TEST}");
    assert!(test_only_attribute(ALL_TEST_UNIX), "{ALL_TEST_UNIX}");
    assert!(test_only_attribute(ALL_NESTED), "{ALL_NESTED}");
}

#[test]
fn a_condition_that_also_builds_outside_test_is_not_test_only() {
    assert!(!test_only_attribute("#[cfg(any(test, feature = \"x\"))]"));
    assert!(!test_only_attribute("#[cfg(any(test))]"));
    assert!(!test_only_attribute("#[cfg(all(test, feature = \"x\"))]"));
    assert!(!test_only_attribute("#[cfg(all(test, not(unix)))]"));
    assert!(!test_only_attribute("#[cfg(not(test))]"));
    assert!(!test_only_attribute("#[cfg(feature = \"x\")]"));
    assert!(!test_only_attribute("#[cfg(unix)]"));
    assert!(!test_only_attribute("mod tests {"));
}

#[test]
fn both_test_module_forms_are_stripped_and_the_code_after_them_kept() {
    let plain = "#[cfg(test)] mod t { fn f() { marker_in_tests(); } }\nfn production() { marker_production(); }\n";
    let out = non_test_source(plain);
    assert!(!out.contains("marker_in_tests"), "{out:?}");
    assert!(out.contains("marker_production"), "{out:?}");
    assert_eq!(out.lines().count(), plain.lines().count());

    let conditional = "#[cfg(all(test, unix))] mod t { fn f() { marker_conditional(); } }\nfn second() { marker_second(); }\n";
    let out = non_test_source(conditional);
    assert!(!out.contains("marker_conditional"), "{out:?}");
    assert!(out.contains("marker_second"), "{out:?}");
    assert_eq!(out.lines().count(), conditional.lines().count());
}

#[test]
fn a_conditional_module_that_is_not_test_only_is_kept() {
    let src = "#[cfg(any(test, feature = \"x\"))] mod t { fn f() { marker_kept(); } }\n";
    assert!(non_test_source(src).contains("marker_kept"));
}
