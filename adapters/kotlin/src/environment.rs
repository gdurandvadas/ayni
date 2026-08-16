//! Read-only Kotlin/Gradle environment discovery.
//!
//! Gradle builds are intentionally treated conservatively: wrapper metadata can
//! pin Gradle, but dependency graphs need build scripts and plugin resolution.
//! This capability records only repository-contained evidence and never claims
//! an offline dependency install is possible without that graph.

use ayni_adapters_common::repository::{
    read_optional_contained_bytes, read_optional_contained_string, repository_relative,
};
use ayni_core::{
    AdapterError, DependencyLockRequirement, EnvironmentCapability, EnvironmentConflict,
    EnvironmentContribution, EnvironmentDiscoveryRequest, EnvironmentWarning, Language,
    PackageManagerRequirement, ProvisioningSupport, RequirementConfidence, RequirementSource,
    RuntimeRequirement, SignalKind, SignalToolRequirement, TargetEnvironment,
    ToolInstallationScope, VersionRequirement,
};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

#[derive(Debug, Default)]
pub(crate) struct KotlinEnvironmentCapability;

type GradleDiscovery = (
    PackageManagerRequirement,
    Vec<DependencyLockRequirement>,
    Vec<EnvironmentConflict>,
    Vec<EnvironmentWarning>,
);

impl EnvironmentCapability for KotlinEnvironmentCapability {
    fn language(&self) -> Language {
        Language::Kotlin
    }
    fn discover(
        &self,
        request: &EnvironmentDiscoveryRequest,
    ) -> Result<EnvironmentContribution, AdapterError> {
        discover(request)
    }
}

fn discover(
    request: &EnvironmentDiscoveryRequest,
) -> Result<EnvironmentContribution, AdapterError> {
    let target_root = request.target_root();
    let owner = gradle_owner(request.repo_root(), &target_root)?;
    let owner_root = relative(request.repo_root(), &owner)?;
    let (java, mut conflicts, mut warnings) = java_requirement(request, &owner)?;
    let (gradle, wrapper_locks, wrapper_conflicts, wrapper_warnings) =
        gradle_requirement(request, &owner, &owner_root)?;
    conflicts.extend(wrapper_conflicts);
    warnings.extend(wrapper_warnings);
    warnings.sort();
    warnings.dedup();
    conflicts.sort();
    conflicts.dedup();

    EnvironmentContribution::new(
        TargetEnvironment {
            target: request.target().clone(),
            workspace: (owner != target_root).then_some(owner_root.clone()),
            package: None,
            runtimes: vec![java],
            package_manager: Some(gradle),
            signal_tools: signal_tools(request, &wrapper_locks)?,
            system_requirements: Vec::new(),
            dependency_locks: wrapper_locks,
        },
        warnings,
        conflicts,
    )
    .map_err(error)
}

fn gradle_owner(repo_root: &Path, target_root: &Path) -> Result<PathBuf, AdapterError> {
    let mut current = target_root;
    loop {
        if settings_path(current).is_some() {
            return Ok(current.to_path_buf());
        }
        if current == repo_root {
            return Ok(target_root.to_path_buf());
        }
        current = current
            .parent()
            .filter(|path| path.starts_with(repo_root))
            .ok_or_else(|| error("Gradle target has no repository-contained ancestor"))?;
    }
}

fn settings_path(root: &Path) -> Option<PathBuf> {
    ["settings.gradle.kts", "settings.gradle"]
        .into_iter()
        .map(|name| root.join(name))
        .find(|path| path.is_file())
}

fn java_requirement(
    request: &EnvironmentDiscoveryRequest,
    owner: &Path,
) -> Result<
    (
        RuntimeRequirement,
        Vec<EnvironmentConflict>,
        Vec<EnvironmentWarning>,
    ),
    AdapterError,
> {
    let mut evidence = java_evidence(request.repo_root(), owner)?;
    evidence.sort_by(|left, right| left.source.cmp(&right.source));
    let values = evidence
        .iter()
        .map(|item| item.value.as_str())
        .collect::<BTreeSet<_>>();
    let (version, source, conflicts) = match evidence.first() {
        None => {
            let path = relative(request.repo_root(), &owner.join("gradle.properties"))?;
            (
                VersionRequirement::unresolved(
                    "no repository-contained JDK selector or Gradle JVM toolchain declaration",
                )
                .map_err(error)?,
                source(
                    "kotlin_java_unresolved",
                    &path,
                    None,
                    RequirementConfidence::Assumed,
                )?,
                Vec::new(),
            )
        }
        Some(first) if values.len() == 1 => (
            java_version(&first.value)?,
            first.source.clone(),
            Vec::new(),
        ),
        Some(first) => (
            VersionRequirement::unresolved("conflicting repository-contained JDK requirements")
                .map_err(error)?,
            first.source.clone(),
            vec![EnvironmentConflict {
                code: "kotlin_java_requirement_conflict".into(),
                message: format!(
                    "JDK requirements disagree: {}",
                    values.into_iter().collect::<Vec<_>>().join(", ")
                ),
                target: Some(request.target().clone()),
                sources: evidence.iter().map(|item| item.source.clone()).collect(),
            }],
        ),
    };
    let warnings = if version.is_exact() || !conflicts.is_empty() {
        Vec::new()
    } else {
        vec![EnvironmentWarning {
        code: "kotlin_java_unresolved".into(),
        message: "No exact JDK version was found; add .java-version, .tool-versions java, or a Gradle JVM toolchain declaration.".into(),
        target: Some(request.target().clone()),
    }]
    };
    Ok((
        RuntimeRequirement {
            runtime: "java".into(),
            version,
            components: Vec::new(),
            targets: Vec::new(),
            source,
        },
        conflicts,
        warnings,
    ))
}

#[derive(Debug, Clone)]
struct Evidence {
    value: String,
    source: RequirementSource,
}

fn java_evidence(repo_root: &Path, owner: &Path) -> Result<Vec<Evidence>, AdapterError> {
    let mut result = java_selector_evidence(repo_root, owner)?;
    result.extend(gradle_toolchain_evidence(repo_root, owner)?);
    Ok(result)
}

fn java_selector_evidence(repo_root: &Path, owner: &Path) -> Result<Vec<Evidence>, AdapterError> {
    let mut result = Vec::new();
    for (file, kind) in [
        (".java-version", "kotlin_java_version"),
        (".tool-versions", "kotlin_tool_versions_java"),
    ] {
        let path = owner.join(file);
        let Some(content) = read_optional_contained_string(repo_root, &path).map_err(error)? else {
            continue;
        };
        let value = java_selector_value(file, &content, &path)?;
        if let Some(value) = value {
            result.push(Evidence {
                value,
                source: source(
                    kind,
                    &relative(repo_root, &path)?,
                    None,
                    RequirementConfidence::Declared,
                )?,
            });
        }
    }
    Ok(result)
}

fn java_selector_value(
    file: &str,
    content: &str,
    path: &Path,
) -> Result<Option<String>, AdapterError> {
    let value = if file == ".tool-versions" {
        content.lines().find_map(|line| {
            line.split_once(char::is_whitespace)
                .filter(|(name, value)| *name == "java" && !value.trim().is_empty())
                .and_then(|(_, value)| value.split_whitespace().next().map(str::to_owned))
        })
    } else {
        single_selector(content, path)?
    };
    if value.as_ref().is_some_and(|value| safe_selector(value)) {
        return Ok(value);
    }
    let declares_java = file == ".java-version"
        || content
            .lines()
            .any(|line| line.trim_start().starts_with("java"));
    if declares_java {
        Err(error(format!(
            "{} contains an invalid JDK selector",
            path.display()
        )))
    } else {
        Ok(None)
    }
}

fn gradle_toolchain_evidence(
    repo_root: &Path,
    owner: &Path,
) -> Result<Vec<Evidence>, AdapterError> {
    let mut result = Vec::new();
    for file in [
        "settings.gradle.kts",
        "settings.gradle",
        "build.gradle.kts",
        "build.gradle",
    ] {
        let path = owner.join(file);
        let Some(content) = read_optional_contained_string(repo_root, &path).map_err(error)? else {
            continue;
        };
        for value in jvm_toolchains(&content) {
            result.push(Evidence {
                value,
                source: source(
                    "kotlin_gradle_jvm_toolchain",
                    &relative(repo_root, &path)?,
                    None,
                    RequirementConfidence::Declared,
                )?,
            });
        }
    }
    Ok(result)
}

fn single_selector(content: &str, path: &Path) -> Result<Option<String>, AdapterError> {
    let value = content.trim();
    if value.is_empty() {
        return Err(error(format!(
            "{} must contain one non-empty JDK selector",
            path.display()
        )));
    }
    if value.lines().count() != 1 {
        return Err(error(format!(
            "{} must contain one JDK selector",
            path.display()
        )));
    }
    Ok(Some(value.to_owned()))
}

fn jvm_toolchains(content: &str) -> Vec<String> {
    // These narrow forms cover Gradle Kotlin and Groovy DSL without treating
    // arbitrary source text as environment metadata.
    let mut values = Vec::new();
    for marker in ["JavaLanguageVersion.of(", "jvmToolchain("] {
        let mut remaining = content;
        while let Some((_, suffix)) = remaining.split_once(marker) {
            let number = suffix
                .trim_start()
                .split(|ch: char| !ch.is_ascii_digit())
                .next()
                .unwrap_or_default();
            if !number.is_empty() {
                values.push(number.to_owned());
            }
            remaining = suffix;
        }
    }
    values.sort();
    values.dedup();
    values
}

fn gradle_requirement(
    request: &EnvironmentDiscoveryRequest,
    owner: &Path,
    owner_root: &str,
) -> Result<GradleDiscovery, AdapterError> {
    let properties = owner.join("gradle/wrapper/gradle-wrapper.properties");
    let Some(bytes) =
        read_optional_contained_bytes(request.repo_root(), &properties).map_err(error)?
    else {
        let wrapper_source = source(
            "kotlin_gradle_unresolved",
            &relative(request.repo_root(), &owner.join("build.gradle.kts"))?,
            None,
            RequirementConfidence::Assumed,
        )?;
        return Ok((PackageManagerRequirement { family: "gradle".into(), version: VersionRequirement::unresolved("no Gradle wrapper metadata").map_err(error)?, ownership_root: owner_root.into(), source: wrapper_source }, Vec::new(), Vec::new(), vec![EnvironmentWarning { code: "kotlin_gradle_wrapper_missing".into(), message: "No gradle/wrapper/gradle-wrapper.properties was found; Gradle cannot be pinned from repository metadata.".into(), target: Some(request.target().clone()) }]));
    };
    let content = String::from_utf8(bytes.clone())
        .map_err(|cause| error(format!("{} is not UTF-8: {cause}", properties.display())))?;
    let parsed = parse_wrapper_properties(&content).map_err(error)?;
    let path = relative(request.repo_root(), &properties)?;
    let wrapper_source = source(
        "kotlin_gradle_wrapper",
        &path,
        Some("distributionUrl"),
        RequirementConfidence::Exact,
    )?;
    let locks = gradle_inputs(request.repo_root(), owner, owner_root)?;
    let mut warnings = Vec::new();
    if parsed.checksum.is_none() {
        warnings.push(EnvironmentWarning { code: "kotlin_gradle_wrapper_checksum_missing".into(), message: "Gradle wrapper distributionSha256Sum is absent; wrapper distribution integrity is not fully pinned.".into(), target: Some(request.target().clone()) });
    }
    Ok((
        PackageManagerRequirement {
            family: "gradle".into(),
            version: VersionRequirement::exact(parsed.version).map_err(error)?,
            ownership_root: owner_root.into(),
            source: wrapper_source,
        },
        locks,
        Vec::new(),
        warnings,
    ))
}

fn gradle_inputs(
    repo_root: &Path,
    owner: &Path,
    owner_root: &str,
) -> Result<Vec<DependencyLockRequirement>, AdapterError> {
    let required = required_gradle_inputs(repo_root, owner)?;
    let paths = collect_gradle_inputs(owner, required)?;
    validate_gradle_inputs(&paths)?;
    paths
        .into_iter()
        .map(|path| gradle_input(repo_root, owner_root, path))
        .collect()
}

fn required_gradle_inputs(repo_root: &Path, owner: &Path) -> Result<[PathBuf; 3], AdapterError> {
    let required = [
        owner.join("gradlew"),
        owner.join("gradle/wrapper/gradle-wrapper.jar"),
        owner.join("gradle/wrapper/gradle-wrapper.properties"),
    ];
    for path in &required {
        if read_optional_contained_bytes(repo_root, path)
            .map_err(error)?
            .is_none()
        {
            return Err(error(format!(
                "managed Gradle requires repository-contained {}",
                path.display()
            )));
        }
    }
    Ok(required)
}

fn collect_gradle_inputs(
    owner: &Path,
    required: [PathBuf; 3],
) -> Result<BTreeSet<PathBuf>, AdapterError> {
    let mut paths = BTreeSet::from(required);
    for entry in walkdir::WalkDir::new(owner)
        .follow_links(false)
        .into_iter()
        .filter_entry(included_gradle_directory)
    {
        let entry =
            entry.map_err(|cause| error(format!("failed to inspect Gradle metadata: {cause}")))?;
        if entry.file_type().is_file() && is_gradle_input(owner, entry.path())? {
            paths.insert(entry.into_path());
        }
    }
    Ok(paths)
}

fn included_gradle_directory(entry: &walkdir::DirEntry) -> bool {
    !matches!(
        entry.file_name().to_str(),
        Some(".git" | ".ayni" | ".gradle" | "build" | "out")
    )
}

fn is_gradle_input(owner: &Path, path: &Path) -> Result<bool, AdapterError> {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    let portable = path
        .strip_prefix(owner)
        .map_err(|_| error("Gradle metadata escapes its owner"))?
        .to_string_lossy()
        .replace('\\', "/");
    Ok(matches!(
        name,
        "settings.gradle"
            | "settings.gradle.kts"
            | "build.gradle"
            | "build.gradle.kts"
            | "gradle.properties"
            | "gradle.lockfile"
    ) || portable.starts_with("gradle/dependency-locks/") && name.ends_with(".lockfile")
        || portable.starts_with("gradle/") && name.ends_with(".versions.toml")
        || portable == "gradle/verification-metadata.xml")
}

fn validate_gradle_inputs(paths: &BTreeSet<PathBuf>) -> Result<(), AdapterError> {
    if !paths.iter().any(|path| {
        path.file_name()
            .is_some_and(|name| name == "gradle.lockfile")
            || path
                .to_string_lossy()
                .replace('\\', "/")
                .contains("/gradle/dependency-locks/")
    }) {
        return Err(error(
            "managed Gradle dependency preparation requires committed dependency lock files",
        ));
    }
    if !paths.iter().any(|path| {
        matches!(
            path.file_name().and_then(|name| name.to_str()),
            Some("build.gradle" | "build.gradle.kts")
        )
    }) {
        return Err(error(
            "managed Gradle requires a build.gradle or build.gradle.kts",
        ));
    }
    Ok(())
}

fn gradle_input(
    repo_root: &Path,
    owner_root: &str,
    path: PathBuf,
) -> Result<DependencyLockRequirement, AdapterError> {
    let bytes = read_optional_contained_bytes(repo_root, &path)
        .map_err(error)?
        .ok_or_else(|| error(format!("missing Gradle input {}", path.display())))?;
    if bytes.is_empty()
        && path
            .file_name()
            .is_some_and(|name| name == "gradle.lockfile")
    {
        return Err(error(format!(
            "Gradle dependency lock must not be empty: {}",
            path.display()
        )));
    }
    let relative_path = relative(repo_root, &path)?;
    let kind = gradle_input_kind(&relative_path);
    lock(
        &relative_path,
        format!("sha256:{:x}", Sha256::digest(bytes)),
        owner_root,
        source(kind, &relative_path, None, RequirementConfidence::Exact)?,
    )
}

fn gradle_input_kind(path: &str) -> &'static str {
    if path.ends_with(".lockfile") {
        "kotlin_gradle_dependency_lock"
    } else if path.ends_with("gradle-wrapper.properties") {
        "kotlin_gradle_wrapper"
    } else if path.ends_with("gradle-wrapper.jar") {
        "kotlin_gradle_wrapper_jar"
    } else if path.ends_with("gradlew") {
        "kotlin_gradle_wrapper_script"
    } else {
        "kotlin_gradle_build_input"
    }
}

struct Wrapper {
    version: String,
    checksum: Option<String>,
}
fn parse_wrapper_properties(content: &str) -> Result<Wrapper, String> {
    let mut url = None;
    let mut checksum = None;
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with('!') {
            continue;
        }
        let Some((key, value)) = line.split_once(['=', ':']) else {
            continue;
        };
        match key.trim() {
            "distributionUrl" => url = Some(value.trim().replace("\\:", ":")),
            "distributionSha256Sum" => checksum = Some(value.trim().to_owned()),
            _ => {}
        }
    }
    let url = url.ok_or("gradle-wrapper.properties must declare distributionUrl")?;
    let prefix = "https://services.gradle.org/distributions/gradle-";
    let value = url.strip_prefix(prefix).and_then(|value| value.strip_suffix("-bin.zip").or_else(|| value.strip_suffix("-all.zip")))
        .ok_or("gradle wrapper distributionUrl must use an HTTPS services.gradle.org Gradle distribution")?;
    if !exact_version(value) {
        return Err("gradle wrapper distributionUrl must contain an exact Gradle version".into());
    }
    if let Some(value) = &checksum
        && (value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()))
    {
        return Err("gradle wrapper distributionSha256Sum must be a SHA-256 digest".into());
    }
    Ok(Wrapper {
        version: value.to_owned(),
        checksum,
    })
}

fn plugin_declaration_inputs(
    request: &EnvironmentDiscoveryRequest,
    inputs: &[DependencyLockRequirement],
) -> Result<Vec<(String, String)>, AdapterError> {
    // Plugin versions can be owned by a project build, settings/pluginManagement,
    // or Gradle version catalog. All are staged dependency inputs, so resolve
    // declarations only from that bounded evidence set.
    inputs
        .iter()
        .filter(|input| {
            input.path.ends_with("build.gradle")
                || input.path.ends_with("build.gradle.kts")
                || input.path.ends_with("settings.gradle")
                || input.path.ends_with("settings.gradle.kts")
                || input.path.ends_with(".versions.toml")
        })
        .map(|input| {
            let path = request.repo_root().join(&input.path);
            let content = read_optional_contained_string(request.repo_root(), &path)
                .map_err(error)?
                .ok_or_else(|| error(format!("missing Gradle build input {}", input.path)))?;
            Ok((input.path.clone(), content))
        })
        .collect()
}

fn signal_tools(
    request: &EnvironmentDiscoveryRequest,
    inputs: &[DependencyLockRequirement],
) -> Result<Vec<SignalToolRequirement>, AdapterError> {
    let scripts = plugin_declaration_inputs(request, inputs)?;
    Ok([
        coverage_signal_tool(request, &scripts)?,
        complexity_signal_tool(request, &scripts)?,
        mutation_signal_tool(request, &scripts)?,
    ]
    .into_iter()
    .flatten()
    .collect())
}

fn coverage_signal_tool(
    request: &EnvironmentDiscoveryRequest,
    scripts: &[(String, String)],
) -> Result<Option<SignalToolRequirement>, AdapterError> {
    if !request.requires_any(&[SignalKind::Coverage]) {
        return Ok(None);
    }
    let kover = find_plugin(
        scripts,
        &[
            "org.jetbrains.kotlinx.kover",
            "org.jetbrains.kotlinx.kover.gradle.plugin",
        ],
    )?
    .map(|(path, version)| ("kover", path, version));
    let jacoco = find_jacoco(scripts)?.map(|(path, version)| ("jacoco", path, version));
    let found = kover.or(jacoco).ok_or_else(|| {
        error("Kotlin coverage requires an exact Kover plugin or JaCoCo toolVersion declaration")
    })?;
    gradle_plugin_tool(request, found.0, &found.1, &found.2, SignalKind::Coverage).map(Some)
}

fn complexity_signal_tool(
    request: &EnvironmentDiscoveryRequest,
    scripts: &[(String, String)],
) -> Result<Option<SignalToolRequirement>, AdapterError> {
    if !request.requires_any(&[SignalKind::Complexity]) {
        return Ok(None);
    }
    let (path, version) = find_plugin(scripts, &["dev.detekt", "io.gitlab.arturbosch.detekt"])?
        .ok_or_else(|| error("Kotlin complexity requires an exact Detekt plugin declaration"))?;
    gradle_plugin_tool(request, "detekt", &path, &version, SignalKind::Complexity).map(Some)
}

fn mutation_signal_tool(
    request: &EnvironmentDiscoveryRequest,
    scripts: &[(String, String)],
) -> Result<Option<SignalToolRequirement>, AdapterError> {
    if !request.requires_any(&[SignalKind::Mutation]) {
        return Ok(None);
    }
    let (path, version) = find_plugin(scripts, &["info.solidsoft.pitest"])?
        .ok_or_else(|| error("Kotlin mutation requires an exact PIT plugin declaration"))?;
    gradle_plugin_tool(request, "pitest", &path, &version, SignalKind::Mutation).map(Some)
}

fn find_plugin(
    scripts: &[(String, String)],
    plugin_ids: &[&str],
) -> Result<Option<(String, String)>, AdapterError> {
    let mut matches = std::collections::BTreeMap::new();
    for (path, content) in scripts {
        for plugin_id in plugin_ids {
            let escaped = regex::escape(plugin_id);
            let expressions = [
                format!(r#"id\(\s*["']{escaped}["']\s*\)\s*version\s*["']([^"']+)["']"#),
                format!(r#"id\s+["']{escaped}["']\s+version\s+["']([^"']+)["']"#),
            ];
            for expression in expressions {
                let pattern = regex::Regex::new(&expression)
                    .map_err(|cause| error(format!("invalid Gradle plugin parser: {cause}")))?;
                for captures in pattern.captures_iter(content) {
                    let version = captures.get(1).expect("version capture").as_str();
                    if !safe_plugin_version(version) {
                        return Err(error(format!(
                            "Gradle plugin {plugin_id} has a dynamic or unsafe version {version}"
                        )));
                    }
                    // Identical declarations are compatible regardless of
                    // ownership file; only distinct resolved versions conflict.
                    matches
                        .entry(version.to_owned())
                        .or_insert_with(|| path.clone());
                }
            }
        }
    }
    if matches.len() > 1 {
        Err(error(format!(
            "Gradle plugin declarations resolve multiple versions: {}",
            matches
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>()
                .join(", ")
        )))
    } else {
        Ok(matches
            .into_iter()
            .next()
            .map(|(version, path)| (path, version)))
    }
}

fn find_jacoco(scripts: &[(String, String)]) -> Result<Option<(String, String)>, AdapterError> {
    let plugin = regex::Regex::new(r#"(?:id\(\s*["']jacoco["']\s*\)|id\s+["']jacoco["'])"#)
        .expect("static JaCoCo plugin pattern");
    let version = regex::Regex::new(r#"toolVersion\s*=\s*["']([^"']+)["']"#)
        .expect("static JaCoCo version pattern");
    let matches = scripts
        .iter()
        .filter(|(_, content)| plugin.is_match(content))
        .flat_map(|(path, content)| {
            version
                .captures_iter(content)
                .map(move |captures| (path.clone(), captures[1].to_owned()))
        })
        .collect::<Vec<_>>();
    let versions = matches
        .iter()
        .map(|(_, version)| version.as_str())
        .collect::<BTreeSet<_>>();
    for value in &versions {
        if !safe_plugin_version(value) {
            return Err(error(format!(
                "JaCoCo toolVersion has a dynamic or unsafe version {value}"
            )));
        }
    }
    if versions.len() > 1 {
        return Err(error(format!(
            "JaCoCo declarations resolve multiple versions: {}",
            versions.into_iter().collect::<Vec<_>>().join(", ")
        )));
    }
    Ok(matches.into_iter().next())
}

fn safe_plugin_version(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'+' | b'_'))
        && !value.ends_with('+')
        && !value.contains(".+")
        && !value.to_ascii_lowercase().contains("latest")
        && !value.to_ascii_lowercase().contains("release")
}

fn gradle_plugin_tool(
    request: &EnvironmentDiscoveryRequest,
    tool: &str,
    path: &str,
    version: &str,
    signal: SignalKind,
) -> Result<SignalToolRequirement, AdapterError> {
    if !safe_plugin_version(version) {
        return Err(error(format!(
            "Gradle plugin {tool} has a dynamic or unsafe version {version}"
        )));
    }
    Ok(SignalToolRequirement {
        tool: tool.into(),
        version: VersionRequirement::exact(version).map_err(error)?,
        provider: "gradle-plugin".into(),
        scope: ToolInstallationScope::Project,
        signals: vec![signal],
        supported_platforms: request.requested_platforms().to_vec(),
        provisioning: ProvisioningSupport::LockedOffline,
        modifies_checkout: false,
        source: source(
            "kotlin_gradle_plugin",
            path,
            Some(tool),
            RequirementConfidence::Exact,
        )?,
    })
}

fn java_version(value: &str) -> Result<VersionRequirement, AdapterError> {
    if exact_version(value) {
        VersionRequirement::exact(value).map_err(error)
    } else {
        VersionRequirement::selector(value).map_err(error)
    }
}
fn exact_version(value: &str) -> bool {
    value.split('.').count() >= 2
        && value
            .split('.')
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
}
fn safe_selector(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'+' | b'@')
        })
}
fn lock(
    path: &str,
    digest: String,
    owner: &str,
    source: RequirementSource,
) -> Result<DependencyLockRequirement, AdapterError> {
    Ok(DependencyLockRequirement {
        path: path.into(),
        digest,
        owner_root: owner.into(),
        source,
    })
}
fn source(
    kind: &str,
    path: &str,
    detail: Option<&str>,
    confidence: RequirementConfidence,
) -> Result<RequirementSource, AdapterError> {
    RequirementSource::new(kind, path, detail, confidence).map_err(error)
}
fn relative(root: &Path, path: &Path) -> Result<String, AdapterError> {
    repository_relative(root, path).map_err(error)
}
fn error(message: impl std::fmt::Display) -> AdapterError {
    AdapterError::new(Language::Kotlin, message.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ayni_adapters_common::environment::assert_environment_capability_conformance;
    use ayni_core::{Architecture, Libc, OperatingSystem, TargetIdentity, TargetPlatform};
    use std::fs;
    use tempfile::TempDir;

    fn request(repo: &Path, target: &str, signals: Vec<SignalKind>) -> EnvironmentDiscoveryRequest {
        ayni_adapters_common::environment::environment_discovery_request(
            repo.to_path_buf(),
            TargetIdentity::new(Language::Kotlin, target).expect("target"),
            signals,
            vec![TargetPlatform {
                os: OperatingSystem::Linux,
                architecture: Architecture::Amd64,
                libc: Libc::Glibc,
            }],
        )
        .expect("request")
    }

    fn wrapper(repo: &Path, version: &str) {
        fs::create_dir_all(repo.join("gradle/wrapper")).expect("wrapper directory");
        fs::write(repo.join("gradlew"), "#!/bin/sh\nexit 0\n").expect("wrapper script");
        fs::write(repo.join("gradle/wrapper/gradle-wrapper.jar"), "wrapper").expect("wrapper jar");
        fs::write(
            repo.join("gradle.lockfile"),
            "example:dependency:1.0=runtimeClasspath\n",
        )
        .expect("dependency lock");
        fs::write(
            repo.join("build.gradle.kts"),
            "plugins { id(\"org.jetbrains.kotlinx.kover\") version \"0.9.1\" }\n",
        )
        .expect("build script");
        fs::write(repo.join("gradle/wrapper/gradle-wrapper.properties"), format!("distributionUrl=https\\://services.gradle.org/distributions/gradle-{version}-bin.zip\ndistributionSha256Sum={}\n", "a".repeat(64))).expect("wrapper properties");
    }

    #[test]
    fn plugin_versions_reject_dynamic_values_and_allow_identical_declarations() {
        let duplicate = vec![
            (
                "build.gradle.kts".into(),
                "plugins { id(\"dev.detekt\") version \"1.23.8\" }".into(),
            ),
            (
                "settings.gradle.kts".into(),
                "pluginManagement { plugins { id(\"dev.detekt\") version \"1.23.8\" } }".into(),
            ),
        ];
        assert_eq!(
            find_plugin(&duplicate, &["dev.detekt"])
                .expect("duplicate")
                .expect("plugin")
                .1,
            "1.23.8"
        );
        for version in ["1.+", "latest.release"] {
            let scripts = vec![(
                "build.gradle.kts".into(),
                format!("plugins {{ id(\"dev.detekt\") version \"{version}\" }}"),
            )];
            assert!(find_plugin(&scripts, &["dev.detekt"]).is_err(), "{version}");
        }
    }

    #[test]
    fn jacoco_versions_detect_conflicts_and_dynamic_values() {
        let scripts = vec![
            (
                "a.gradle.kts".into(),
                "plugins { id(\"jacoco\") }; jacoco { toolVersion = \"0.8.12\" }".into(),
            ),
            (
                "b.gradle.kts".into(),
                "plugins { id(\"jacoco\") }; jacoco { toolVersion = \"0.8.13\" }".into(),
            ),
        ];
        assert!(find_jacoco(&scripts).is_err());
        let dynamic = vec![(
            "a.gradle.kts".into(),
            "plugins { id(\"jacoco\") }; jacoco { toolVersion = \"latest.release\" }".into(),
        )];
        assert!(find_jacoco(&dynamic).is_err());
    }

    #[test]
    fn parses_only_safe_exact_gradle_wrapper_urls() {
        assert_eq!(parse_wrapper_properties("distributionUrl=https\\://services.gradle.org/distributions/gradle-8.10.2-all.zip\n").expect("wrapper").version, "8.10.2");
        for value in [
            "distributionUrl=http\\://services.gradle.org/distributions/gradle-8.10.2-bin.zip",
            "distributionUrl=https\\://example.invalid/gradle-8.10.2-bin.zip",
            "distributionUrl=https\\://services.gradle.org/distributions/gradle-latest-bin.zip",
            "distributionUrl=https\\://services.gradle.org/distributions/gradle-8.10.2-bin.zip\ndistributionSha256Sum=nope",
        ] {
            assert!(parse_wrapper_properties(value).is_err(), "{value}");
        }
    }

    #[test]
    fn discovers_owner_jdk_wrapper_integrity_and_configured_tasks() {
        let fixture = TempDir::new().expect("fixture");
        fs::create_dir_all(fixture.path().join("apps/demo")).expect("nested target");
        fs::write(
            fixture.path().join("settings.gradle.kts"),
            "rootProject.name = \"fixture\"",
        )
        .expect("settings");
        fs::write(fixture.path().join(".java-version"), "21.0.6\n").expect("java");
        wrapper(fixture.path(), "8.10.2");
        fs::write(
            fixture.path().join("gradle/wrapper/gradle-wrapper.jar"),
            "wrapper",
        )
        .expect("jar");
        let contribution = assert_environment_capability_conformance(
            &KotlinEnvironmentCapability,
            &request(
                fixture.path(),
                "apps/demo",
                vec![SignalKind::Test, SignalKind::Coverage],
            ),
        )
        .expect("conformance");
        let target = contribution.target();
        assert_eq!(target.workspace.as_deref(), Some("."));
        assert_eq!(
            target.runtimes[0].version,
            VersionRequirement::exact("21.0.6").expect("exact")
        );
        assert_eq!(
            target.package_manager.as_ref().expect("gradle").version,
            VersionRequirement::exact("8.10.2").expect("exact")
        );
        assert_eq!(target.dependency_locks.len(), 6);
        assert!(
            target
                .signal_tools
                .iter()
                .all(|tool| tool.provisioning == ProvisioningSupport::LockedOffline)
        );
        assert_eq!(
            target
                .signal_tools
                .iter()
                .map(|tool| tool.tool.as_str())
                .collect::<Vec<_>>(),
            ["kover"]
        );
    }

    #[test]
    fn jvm_sources_conflict_deterministically_and_toolchains_are_discovered() {
        let fixture = TempDir::new().expect("fixture");
        fs::write(
            fixture.path().join("settings.gradle.kts"),
            "java { toolchain { languageVersion.set(JavaLanguageVersion.of(21)) } }",
        )
        .expect("settings");
        fs::write(fixture.path().join(".tool-versions"), "java 17\n").expect("tool versions");
        wrapper(fixture.path(), "8.10.2");
        let first = KotlinEnvironmentCapability
            .discover(&request(fixture.path(), ".", vec![]))
            .expect("discovery");
        let second = KotlinEnvironmentCapability
            .discover(&request(fixture.path(), ".", vec![]))
            .expect("discovery");
        assert_eq!(first, second);
        assert_eq!(
            first.conflicts()[0].code,
            "kotlin_java_requirement_conflict"
        );
        assert_eq!(
            first.conflicts()[0]
                .sources
                .iter()
                .map(|source| source.path.as_str())
                .collect::<Vec<_>>(),
            ["settings.gradle.kts", ".tool-versions"]
        );
    }

    #[test]
    fn malformed_or_unsafe_metadata_fails_closed() {
        let fixture = TempDir::new().expect("fixture");
        fs::write(fixture.path().join("settings.gradle.kts"), "").expect("settings");
        wrapper(fixture.path(), "latest");
        assert!(
            KotlinEnvironmentCapability
                .discover(&request(fixture.path(), ".", vec![]))
                .is_err()
        );
        wrapper(fixture.path(), "8.10.2");
        fs::write(fixture.path().join(".java-version"), "21\n22\n").expect("java");
        assert!(
            KotlinEnvironmentCapability
                .discover(&request(fixture.path(), ".", vec![]))
                .is_err()
        );
    }
}
