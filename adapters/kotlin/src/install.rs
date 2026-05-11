use std::fs;
use std::path::{Path, PathBuf};

const PLUGINS: &[(&str, &str)] = &[
    ("org.jetbrains.kotlinx.kover", "0.9.8"),
    ("io.gitlab.arturbosch.detekt", "1.23.8"),
    ("info.solidsoft.pitest", "1.19.0"),
];

pub fn ensure_gradle_plugins(root: &Path) -> Result<(), String> {
    let path = build_file(root).ok_or_else(|| {
        format!(
            "kotlin install failed: build.gradle.kts or build.gradle not found at {}",
            root.display()
        )
    })?;
    let content = fs::read_to_string(&path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    let updated = if path.file_name().and_then(|value| value.to_str()) == Some("build.gradle.kts") {
        ensure_plugins(&content, Dsl::Kotlin)?
    } else {
        ensure_plugins(&content, Dsl::Groovy)?
    };
    if updated != content {
        fs::write(&path, updated)
            .map_err(|error| format!("failed to write {}: {error}", path.display()))?;
    }
    Ok(())
}

fn build_file(root: &Path) -> Option<PathBuf> {
    let kotlin = root.join("build.gradle.kts");
    if kotlin.is_file() {
        return Some(kotlin);
    }
    let groovy = root.join("build.gradle");
    groovy.is_file().then_some(groovy)
}

#[derive(Clone, Copy)]
enum Dsl {
    Kotlin,
    Groovy,
}

fn ensure_plugins(content: &str, dsl: Dsl) -> Result<String, String> {
    let Some(insert_after) = content
        .lines()
        .position(|line| line.trim_start().starts_with("plugins {"))
    else {
        return Err(String::from(
            "kotlin install supports only Gradle builds with a direct plugins { } block",
        ));
    };

    let mut missing = Vec::new();
    for (id, version) in PLUGINS {
        if !content.contains(id) {
            missing.push(plugin_line(dsl, id, version));
        }
    }
    if missing.is_empty() {
        return Ok(content.to_string());
    }

    let mut lines = content.lines().map(str::to_string).collect::<Vec<_>>();
    for (offset, line) in missing.into_iter().enumerate() {
        lines.insert(insert_after + 1 + offset, line);
    }
    let mut out = lines.join("\n");
    if content.ends_with('\n') {
        out.push('\n');
    }
    Ok(out)
}

fn plugin_line(dsl: Dsl, id: &str, version: &str) -> String {
    match dsl {
        Dsl::Kotlin => format!("    id(\"{id}\") version \"{version}\""),
        Dsl::Groovy => format!("    id '{id}' version '{version}'"),
    }
}

#[cfg(test)]
mod tests {
    use super::ensure_gradle_plugins;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn inserts_kotlin_dsl_plugins() {
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join("build.gradle.kts");
        fs::write(
            &path,
            "plugins {\n    kotlin(\"jvm\") version \"2.0.0\"\n}\n",
        )
        .expect("build");

        ensure_gradle_plugins(dir.path()).expect("plugins");
        let updated = fs::read_to_string(path).expect("updated");

        assert!(updated.contains("id(\"org.jetbrains.kotlinx.kover\") version \"0.9.8\""));
        assert!(updated.contains("id(\"io.gitlab.arturbosch.detekt\") version \"1.23.8\""));
        assert!(updated.contains("id(\"info.solidsoft.pitest\") version \"1.19.0\""));
    }

    #[test]
    fn inserts_groovy_dsl_plugins() {
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join("build.gradle");
        fs::write(
            &path,
            "plugins {\n    id 'org.jetbrains.kotlin.jvm' version '2.0.0'\n}\n",
        )
        .expect("build");

        ensure_gradle_plugins(dir.path()).expect("plugins");
        let updated = fs::read_to_string(path).expect("updated");

        assert!(updated.contains("id 'org.jetbrains.kotlinx.kover' version '0.9.8'"));
    }

    #[test]
    fn rejects_unsupported_build_shape() {
        let dir = TempDir::new().expect("tempdir");
        fs::write(
            dir.path().join("build.gradle.kts"),
            "apply(plugin = \"x\")\n",
        )
        .expect("build");

        let error = ensure_gradle_plugins(dir.path()).expect_err("unsupported");
        assert!(error.contains("direct plugins"));
    }
}
