use ayni_core::RunArtifact;
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::process::ExitCode;

pub(crate) fn run(artifact_path: &Path) -> ExitCode {
    let artifact = match load_artifact(artifact_path) {
        Ok(artifact) => artifact,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::from(2);
        }
    };
    let commands = verification_commands(&artifact);
    if commands.is_empty() {
        println!("no verification commands in {}", artifact_path.display());
    } else {
        println!("verification commands");
        for command in commands {
            println!("  {command}");
        }
    }
    ExitCode::SUCCESS
}

fn load_artifact(path: &Path) -> Result<RunArtifact, String> {
    let content = fs::read_to_string(path)
        .map_err(|error| format!("could not read result artifact {}: {error}", path.display()))?;
    serde_json::from_str(&content).map_err(|error| {
        format!(
            "could not parse result artifact {}: {error}",
            path.display()
        )
    })
}

fn verification_commands(artifact: &RunArtifact) -> Vec<&str> {
    deduplicate_commands(
        artifact
            .findings
            .iter()
            .flat_map(ayni_core::Findings::commands),
    )
}

fn deduplicate_commands<'a>(commands: impl IntoIterator<Item = &'a str>) -> Vec<&'a str> {
    let mut seen = BTreeSet::new();
    commands
        .into_iter()
        .filter(|command| seen.insert(*command))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::deduplicate_commands;

    #[test]
    fn command_listing_preserves_first_seen_order_and_removes_duplicates() {
        assert_eq!(
            deduplicate_commands(["ayni verify size", "ayni verify test", "ayni verify size"]),
            ["ayni verify size", "ayni verify test"]
        );
    }
}
