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
    collect_size_inner(repo_root, workdir, None, size_map, excluded_dir_names)
}

/// Collects size for exactly one repository file while applying the same
/// include rules, rule exclusions, and adapter-owned directory exclusions as
/// repository collection.
pub fn collect_size_file(
    repo_root: &Path,
    workdir: &Path,
    file: &str,
    size_map: &BTreeMap<String, SizeThreshold>,
    excluded_dir_names: &[&str],
) -> Result<SizeCollection, String> {
    let candidate = Path::new(file);
    let candidate = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        repo_root.join(candidate)
    };
    let canonical_repo_root = repo_root
        .canonicalize()
        .map_err(|error| format!("failed to resolve repository root: {error}"))?;
    let candidate = candidate.canonicalize().map_err(|error| {
        format!(
            "selected size file {} could not be resolved: {error}",
            candidate.display()
        )
    })?;
    if !candidate.is_file() {
        return Err(format!(
            "selected size path {} is not a file",
            candidate.display()
        ));
    }
    candidate.strip_prefix(&canonical_repo_root).map_err(|_| {
        format!(
            "selected size file {} is outside repository root {}",
            candidate.display(),
            canonical_repo_root.display()
        )
    })?;
    collect_size_inner(
        repo_root,
        workdir,
        Some(candidate.as_path()),
        size_map,
        excluded_dir_names,
    )
}

fn collect_size_inner(
    repo_root: &Path,
    workdir: &Path,
    selected_file: Option<&Path>,
    size_map: &BTreeMap<String, SizeThreshold>,
    excluded_dir_names: &[&str],
) -> Result<SizeCollection, String> {
    let compiled = compile_rules(size_map)?;
    let mut offenders = Vec::new();
    let mut max_lines = 0_u64;
    let mut warn_count = 0_u64;
    let mut fail_count = 0_u64;
    let mut total_files = 0_u64;

    let walk_root = selected_file.unwrap_or(workdir);
    for entry in WalkDir::new(walk_root)
        .into_iter()
        .filter_entry(|entry| !is_excluded_path(workdir, entry.path(), excluded_dir_names))
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

fn is_excluded_path(workdir: &Path, path: &Path, excluded_dir_names: &[&str]) -> bool {
    path.strip_prefix(workdir)
        .unwrap_or(path)
        .components()
        .any(|component| {
            component
                .as_os_str()
                .to_str()
                .is_some_and(|name| excluded_dir_names.contains(&name))
        })
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
    use super::{collect_size, collect_size_file};
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

    #[test]
    fn exact_file_collection_measures_only_the_selected_file() {
        let dir = TempDir::new().expect("tempdir");
        fs::create_dir_all(dir.path().join("src")).expect("src dir");
        fs::write(dir.path().join("src/selected.rs"), lines(12)).expect("selected");
        fs::write(dir.path().join("src/other.rs"), lines(30)).expect("other");

        let collection = collect_size_file(
            dir.path(),
            dir.path(),
            "src/selected.rs",
            &size_map("**/*.rs", 10, 20, Vec::new()),
            &[],
        )
        .expect("collect selected file");

        assert_eq!(collection.result.total_files, 1);
        assert_eq!(collection.result.max_lines, 12);
        assert_eq!(collection.result.warn_count, 1);
        assert_eq!(collection.result.fail_count, 0);
        assert_eq!(collection.offenders[0].file, "src/selected.rs");
    }

    #[test]
    fn exact_file_collection_honors_rules_and_exclusions() {
        let dir = TempDir::new().expect("tempdir");
        fs::create_dir_all(dir.path().join("generated")).expect("generated dir");
        fs::write(dir.path().join("generated/selected.rs"), lines(30)).expect("selected");

        let rule_excluded = collect_size_file(
            dir.path(),
            dir.path(),
            "generated/selected.rs",
            &size_map("**/*.rs", 10, 20, vec![String::from("generated/**")]),
            &[],
        )
        .expect("rule-excluded file");
        assert_eq!(rule_excluded.result.total_files, 0);

        let directory_excluded = collect_size_file(
            dir.path(),
            dir.path(),
            "generated/selected.rs",
            &size_map("**/*.rs", 10, 20, Vec::new()),
            &["generated"],
        )
        .expect("directory-excluded file");
        assert_eq!(directory_excluded.result.total_files, 0);
    }
}
