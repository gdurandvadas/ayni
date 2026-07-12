use crate::SizeThreshold;
use crate::signal::{Level, SizeOffender, SizeResult};
use glob::Pattern;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use walkdir::WalkDir;

struct CompiledRule<'a> {
    threshold: &'a SizeThreshold,
    include: Pattern,
    excludes: Vec<Pattern>,
}

#[derive(Debug)]
pub struct SizeCollection {
    pub result: SizeResult,
    pub offenders: Vec<SizeOffender>,
    pub budget: Value,
}

pub fn collect_size(
    repo_root: &Path,
    workdir: &Path,
    size_map: &BTreeMap<String, SizeThreshold>,
    excluded_dir_names: &[&str],
) -> Result<SizeCollection, String> {
    let compiled = compile_rules(size_map)?;
    let mut offenders = Vec::new();
    let mut max_lines = 0_u64;
    let mut warn_count = 0_u64;
    let mut fail_count = 0_u64;
    let mut total_files = 0_u64;

    for entry in WalkDir::new(workdir)
        .into_iter()
        .filter_entry(|entry| !is_excluded_dir(entry.path(), excluded_dir_names))
    {
        let entry = match entry {
            Ok(value) => value,
            Err(_) => continue,
        };
        if !entry.file_type().is_file() {
            continue;
        }

        let rel_for_match = to_repo_relative_path(workdir, entry.path());
        let Some(threshold) = first_matching(&compiled, &rel_for_match) else {
            continue;
        };

        let rel = to_repo_relative_path(repo_root, entry.path());
        total_files += 1;

        let content = fs::read_to_string(entry.path())
            .map_err(|error| format!("failed to read {}: {error}", entry.path().display()))?;
        let line_count = content.lines().count() as u64;
        max_lines = max_lines.max(line_count);

        if line_count > threshold.warn {
            let level = if line_count > threshold.fail {
                fail_count += 1;
                Level::Fail
            } else {
                warn_count += 1;
                Level::Warn
            };
            offenders.push(SizeOffender {
                file: rel,
                value: line_count,
                warn: threshold.warn,
                fail: threshold.fail,
                level,
            });
        }
    }

    let budget_rules: Vec<_> = size_map
        .iter()
        .map(|(glob, t)| json!({ "glob": glob, "warn": t.warn, "fail": t.fail }))
        .collect();

    Ok(SizeCollection {
        result: SizeResult {
            max_lines,
            total_files,
            warn_count,
            fail_count,
            failure: None,
        },
        offenders,
        budget: json!({ "rules": budget_rules }),
    })
}

fn compile_rules(map: &BTreeMap<String, SizeThreshold>) -> Result<Vec<CompiledRule<'_>>, String> {
    map.iter()
        .map(|(glob, threshold)| {
            let include = Pattern::new(glob)
                .map_err(|error| format!("invalid size glob '{glob}': {error}"))?;
            let excludes = threshold
                .exclude
                .iter()
                .map(|exclude| {
                    Pattern::new(exclude)
                        .map_err(|error| format!("invalid exclude glob '{exclude}': {error}"))
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok(CompiledRule {
                threshold,
                include,
                excludes,
            })
        })
        .collect()
}

fn first_matching<'a>(compiled: &[CompiledRule<'a>], rel: &str) -> Option<&'a SizeThreshold> {
    compiled
        .iter()
        .find(|rule| rule.include.matches(rel) && !rule.excludes.iter().any(|ex| ex.matches(rel)))
        .map(|rule| rule.threshold)
}

fn is_excluded_dir(path: &Path, excluded_dir_names: &[&str]) -> bool {
    matches!(
        path.file_name().and_then(|value| value.to_str()),
        Some(name) if excluded_dir_names.contains(&name)
    )
}

fn to_repo_relative_path(repo_root: &Path, candidate: &Path) -> String {
    if let Ok(relative) = candidate.strip_prefix(repo_root) {
        return relative.to_string_lossy().replace('\\', "/");
    }
    if let Ok(canonical_repo_root) = repo_root.canonicalize()
        && let Ok(canonical_candidate) = candidate.canonicalize()
        && let Ok(relative) = canonical_candidate.strip_prefix(canonical_repo_root)
    {
        return relative.to_string_lossy().replace('\\', "/");
    }
    candidate.to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::collect_size;
    use crate::SizeThreshold;
    use crate::signal::Level;
    use std::collections::BTreeMap;
    use std::fs;
    use tempfile::TempDir;

    fn lines(count: usize) -> String {
        "line\n".repeat(count)
    }

    fn size_map(
        glob: &str,
        warn: u64,
        fail: u64,
        exclude: Vec<String>,
    ) -> BTreeMap<String, SizeThreshold> {
        BTreeMap::from([(
            glob.to_string(),
            SizeThreshold {
                warn,
                fail,
                exclude,
            },
        )])
    }

    #[test]
    fn classifies_warn_and_fail_offenders() {
        let dir = TempDir::new().expect("tempdir");
        fs::write(dir.path().join("small.rs"), lines(3)).expect("small");
        fs::write(dir.path().join("warn.rs"), lines(12)).expect("warn");
        fs::write(dir.path().join("fail.rs"), lines(30)).expect("fail");

        let collection = collect_size(
            dir.path(),
            dir.path(),
            &size_map("*.rs", 10, 20, Vec::new()),
            &[],
        )
        .expect("collect");

        assert_eq!(collection.result.total_files, 3);
        assert_eq!(collection.result.max_lines, 30);
        assert_eq!(collection.result.warn_count, 1);
        assert_eq!(collection.result.fail_count, 1);
        assert_eq!(collection.offenders.len(), 2);
        let warn = collection
            .offenders
            .iter()
            .find(|offender| offender.file == "warn.rs")
            .expect("warn offender");
        assert_eq!(warn.level, Level::Warn);
        let fail = collection
            .offenders
            .iter()
            .find(|offender| offender.file == "fail.rs")
            .expect("fail offender");
        assert_eq!(fail.level, Level::Fail);
    }

    #[test]
    fn exclude_globs_and_excluded_dirs_skip_files() {
        let dir = TempDir::new().expect("tempdir");
        fs::create_dir_all(dir.path().join("generated")).expect("generated dir");
        fs::create_dir_all(dir.path().join("target/debug")).expect("target dir");
        fs::write(dir.path().join("ok.rs"), lines(2)).expect("ok");
        fs::write(dir.path().join("generated/huge.rs"), lines(100)).expect("generated");
        fs::write(dir.path().join("target/debug/huge.rs"), lines(100)).expect("built");

        let collection = collect_size(
            dir.path(),
            dir.path(),
            &size_map("**/*.rs", 10, 20, vec![String::from("generated/**")]),
            &["target"],
        )
        .expect("collect");

        assert_eq!(collection.result.total_files, 1);
        assert!(collection.offenders.is_empty());
    }

    #[test]
    fn invalid_glob_is_an_error() {
        let dir = TempDir::new().expect("tempdir");
        let error = collect_size(
            dir.path(),
            dir.path(),
            &size_map("[", 10, 20, Vec::new()),
            &[],
        )
        .expect_err("invalid glob");
        assert!(error.contains("invalid size glob"));
    }

    #[test]
    fn budget_lists_rules() {
        let dir = TempDir::new().expect("tempdir");
        let collection = collect_size(
            dir.path(),
            dir.path(),
            &size_map("*.rs", 5, 9, Vec::new()),
            &[],
        )
        .expect("collect");
        let rules = collection.budget["rules"].as_array().expect("rules array");
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0]["glob"], "*.rs");
        assert_eq!(rules[0]["warn"], 5);
        assert_eq!(rules[0]["fail"], 9);
    }
}
