use super::CoverageResult;

#[test]
fn headline_percent_prefers_percent_then_line_then_branch() {
    assert_eq!(
        CoverageResult {
            percent: Some(90.0),
            line_percent: Some(70.0),
            branch_percent: Some(60.0),
            engine: String::new(),
            status: String::new(),
            failure: None,
        }
        .headline_percent(),
        Some(90.0)
    );
    assert_eq!(
        CoverageResult {
            percent: None,
            line_percent: Some(71.5),
            branch_percent: Some(60.0),
            engine: String::new(),
            status: String::new(),
            failure: None,
        }
        .headline_percent(),
        Some(71.5)
    );
    assert_eq!(
        CoverageResult {
            percent: None,
            line_percent: None,
            branch_percent: Some(55.0),
            engine: String::new(),
            status: String::new(),
            failure: None,
        }
        .headline_percent(),
        Some(55.0)
    );
}
