use ayni_core::{AyniPolicy, Language, SignalKind, ThresholdFloat, ToolCommandOverride};
use std::collections::BTreeSet;
use std::fmt::Write;
use std::path::Path;

const SIGNALS: [(SignalKind, &str); 6] = [
    (SignalKind::Test, "test"),
    (SignalKind::Coverage, "coverage"),
    (SignalKind::Size, "size"),
    (SignalKind::Complexity, "complexity"),
    (SignalKind::Deps, "deps"),
    (SignalKind::Mutation, "mutation"),
];

pub(crate) fn display(config_path: &Path) -> Result<String, String> {
    let policy = AyniPolicy::load_from_path(config_path)?;
    render(&policy)
}

fn render(policy: &AyniPolicy) -> Result<String, String> {
    let languages = policy
        .enabled_languages()?
        .into_iter()
        .collect::<BTreeSet<_>>();
    let mut output = String::from("Configured signal contract\n");

    for language in languages {
        writeln!(output, "\nlanguage: {language}").expect("writing to String cannot fail");
        writeln!(output, "  roots:").expect("writing to String cannot fail");
        for root in policy.roots_for(language) {
            writeln!(output, "    - {root}").expect("writing to String cannot fail");
        }
        writeln!(output, "  signals:").expect("writing to String cannot fail");
        for (kind, name) in SIGNALS {
            let state = if signal_enabled(policy, kind) {
                "enabled"
            } else {
                "disabled"
            };
            writeln!(output, "    {name}: {state}").expect("writing to String cannot fail");
            render_signal_policy(&mut output, policy, language, kind);
        }
    }

    Ok(output)
}

fn signal_enabled(policy: &AyniPolicy, kind: SignalKind) -> bool {
    match kind {
        SignalKind::Test => policy.checks.test,
        SignalKind::Coverage => policy.checks.coverage,
        SignalKind::Size => policy.checks.size,
        SignalKind::Complexity => policy.checks.complexity,
        SignalKind::Deps => policy.checks.deps,
        SignalKind::Mutation => policy.checks.mutation,
    }
}

fn render_signal_policy(
    output: &mut String,
    policy: &AyniPolicy,
    language: Language,
    kind: SignalKind,
) {
    let tooling = policy.language_tooling(language);
    match kind {
        SignalKind::Test | SignalKind::Mutation => {
            render_tool_override(output, policy.tool_override_for(language, kind));
        }
        SignalKind::Coverage => {
            writeln!(output, "      thresholds:").expect("writing to String cannot fail");
            render_threshold(
                output,
                "line_percent (minimum)",
                tooling
                    .coverage
                    .as_ref()
                    .and_then(|value| value.line_percent),
            );
            render_threshold(
                output,
                "branch_percent (minimum)",
                tooling
                    .coverage
                    .as_ref()
                    .and_then(|value| value.branch_percent),
            );
            render_tool_override(output, policy.tool_override_for(language, kind));
        }
        SignalKind::Size => {
            if tooling.size.is_empty() {
                writeln!(output, "      rules: not configured")
                    .expect("writing to String cannot fail");
            } else {
                writeln!(output, "      rules:").expect("writing to String cannot fail");
                for (pattern, rule) in &tooling.size {
                    writeln!(
                        output,
                        "        - pattern: {} | warn: {} | fail: {}",
                        quoted(pattern),
                        rule.warn,
                        rule.fail
                    )
                    .expect("writing to String cannot fail");
                    if rule.exclude.is_empty() {
                        writeln!(output, "          exclusions: none")
                            .expect("writing to String cannot fail");
                    } else {
                        writeln!(
                            output,
                            "          exclusions: {}",
                            quoted_list(&rule.exclude)
                        )
                        .expect("writing to String cannot fail");
                    }
                }
            }
        }
        SignalKind::Complexity => {
            writeln!(output, "      thresholds:").expect("writing to String cannot fail");
            render_threshold(
                output,
                "fn_cyclomatic (maximum)",
                tooling
                    .complexity
                    .as_ref()
                    .and_then(|value| value.fn_cyclomatic),
            );
            render_threshold(
                output,
                "fn_cognitive (maximum)",
                tooling
                    .complexity
                    .as_ref()
                    .and_then(|value| value.fn_cognitive),
            );
        }
        SignalKind::Deps => {
            let restrictions = tooling.deps.as_ref().map(|value| &value.forbidden);
            if restrictions.is_none_or(|value| value.is_empty()) {
                writeln!(output, "      restrictions: not configured")
                    .expect("writing to String cannot fail");
            } else if let Some(restrictions) = restrictions {
                writeln!(output, "      restrictions:").expect("writing to String cannot fail");
                for (from, targets) in restrictions {
                    if targets.is_empty() {
                        writeln!(output, "        - {} -> none", quoted(from))
                            .expect("writing to String cannot fail");
                    } else {
                        for target in targets {
                            writeln!(output, "        - {} -> {}", quoted(from), quoted(target))
                                .expect("writing to String cannot fail");
                        }
                    }
                }
            }
        }
    }
}

fn render_threshold(output: &mut String, name: &str, threshold: Option<ThresholdFloat>) {
    if let Some(threshold) = threshold {
        writeln!(
            output,
            "        {name}: warn {} | fail {}",
            threshold.warn, threshold.fail
        )
        .expect("writing to String cannot fail");
    } else {
        writeln!(output, "        {name}: not configured").expect("writing to String cannot fail");
    }
}

fn render_tool_override(output: &mut String, value: Option<&ToolCommandOverride>) {
    if let Some(value) = value {
        writeln!(
            output,
            "      tool override: command {} | args {}",
            quoted(&value.command),
            quoted_list(&value.args)
        )
        .expect("writing to String cannot fail");
    } else {
        writeln!(output, "      tool override: not configured")
            .expect("writing to String cannot fail");
    }
}

fn quoted_list(values: &[String]) -> String {
    let values = values
        .iter()
        .map(|value| quoted(value))
        .collect::<Vec<_>>()
        .join(", ");
    format!("[{values}]")
}

fn quoted(value: &str) -> String {
    format!("{value:?}")
}
