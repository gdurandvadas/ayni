use crate::policy::load_from_path;
use ayni_core::{
    AyniPolicy, DockerAccess, EnvironmentResourceLimits, Language, NetworkAccess,
    PolicyEffectivenessFacts, PolicyEffectivenessWarning, SignalKind, ThresholdFloat,
    ToolCommandOverride,
};
use serde::Serialize;
use std::collections::BTreeSet;
use std::fmt::Write;
use std::path::Path;

const CONTRACT_PROJECTION_VERSION: &str = "0.4.0";
const SIGNALS: [SignalKind; 6] = [
    SignalKind::Test,
    SignalKind::Coverage,
    SignalKind::Size,
    SignalKind::Complexity,
    SignalKind::Deps,
    SignalKind::Mutation,
];

#[derive(Debug, Serialize)]
struct ContractProjection {
    projection_version: &'static str,
    environment: EnvironmentProjection,
    languages: Vec<LanguageProjection>,
    warnings: Vec<PolicyEffectivenessWarning>,
}

#[derive(Debug, Serialize)]
struct EnvironmentProjection {
    tools: Vec<EnvironmentToolProjection>,
    debian_packages: Vec<String>,
    docker: DockerAccess,
    network: NetworkAccess,
    resources: EnvironmentResourceLimits,
}

#[derive(Debug, Serialize)]
struct EnvironmentToolProjection {
    tool: String,
    version: String,
}

#[derive(Debug, Serialize)]
struct LanguageProjection {
    language: Language,
    roots: Vec<String>,
    signals: Vec<SignalProjection>,
}

#[derive(Debug, Serialize)]
struct SignalProjection {
    kind: SignalKind,
    enabled: bool,
    detail: SignalDetail,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum SignalDetail {
    Tool {
        tool_override: Option<ToolOverrideProjection>,
    },
    Coverage {
        line_percent: Option<ThresholdProjection>,
        branch_percent: Option<ThresholdProjection>,
        coverage_satisfies_test: bool,
        tool_override: Option<ToolOverrideProjection>,
    },
    Size {
        rules: Vec<SizeRuleProjection>,
    },
    Complexity {
        fn_cyclomatic: Option<ThresholdProjection>,
        fn_cognitive: Option<ThresholdProjection>,
    },
    Deps {
        forbidden: Vec<ForbiddenDependencyProjection>,
    },
}

#[derive(Debug, Serialize)]
struct ThresholdProjection {
    warn: f64,
    fail: f64,
}

#[derive(Debug, Serialize)]
struct ToolOverrideProjection {
    command: String,
    args: Vec<String>,
}

#[derive(Debug, Serialize)]
struct SizeRuleProjection {
    pattern: String,
    warn: u64,
    fail: u64,
    exclude: Vec<String>,
}

#[derive(Debug, Serialize)]
struct ForbiddenDependencyProjection {
    from: String,
    targets: Vec<String>,
}

pub(crate) fn display(
    config_path: &Path,
    adapter_facts: &[PolicyEffectivenessFacts],
    json: bool,
) -> Result<String, String> {
    let policy = load_from_path(config_path)?;
    let projection = project(&policy, adapter_facts)?;
    if json {
        serde_json::to_string_pretty(&projection)
            .map(|output| format!("{output}\n"))
            .map_err(|error| format!("failed to serialize contract projection: {error}"))
    } else {
        Ok(render_human(&projection))
    }
}

fn project(
    policy: &AyniPolicy,
    adapter_facts: &[PolicyEffectivenessFacts],
) -> Result<ContractProjection, String> {
    let languages = policy
        .enabled_languages()?
        .into_iter()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(|language| LanguageProjection {
            language,
            roots: policy.roots_for(language).to_vec(),
            signals: SIGNALS
                .into_iter()
                .map(|kind| SignalProjection {
                    kind,
                    enabled: signal_enabled(policy, kind),
                    detail: project_signal(policy, language, kind),
                })
                .collect(),
        })
        .collect();
    let capabilities = policy.environment_capabilities();
    Ok(ContractProjection {
        projection_version: CONTRACT_PROJECTION_VERSION,
        environment: EnvironmentProjection {
            tools: policy
                .environment_tools()
                .iter()
                .map(|(tool, version)| EnvironmentToolProjection {
                    tool: tool.clone(),
                    version: version.clone(),
                })
                .collect(),
            debian_packages: policy.environment_debian_packages().to_vec(),
            docker: capabilities.docker,
            network: capabilities.network,
            resources: policy.environment_resource_limits(),
        },
        languages,
        warnings: policy.effectiveness_warnings(adapter_facts),
    })
}

fn project_signal(policy: &AyniPolicy, language: Language, kind: SignalKind) -> SignalDetail {
    let tooling = policy.language_tooling(language);
    match kind {
        SignalKind::Test | SignalKind::Mutation => SignalDetail::Tool {
            tool_override: project_tool_override(policy.tool_override_for(language, kind)),
        },
        SignalKind::Coverage => SignalDetail::Coverage {
            line_percent: tooling
                .coverage
                .as_ref()
                .and_then(|value| value.line_percent)
                .map(project_threshold),
            branch_percent: tooling
                .coverage
                .as_ref()
                .and_then(|value| value.branch_percent)
                .map(project_threshold),
            coverage_satisfies_test: tooling.tooling.coverage_satisfies_test,
            tool_override: project_tool_override(policy.tool_override_for(language, kind)),
        },
        SignalKind::Size => SignalDetail::Size {
            rules: tooling
                .size
                .iter()
                .map(|(pattern, rule)| SizeRuleProjection {
                    pattern: pattern.clone(),
                    warn: rule.warn,
                    fail: rule.fail,
                    exclude: rule.exclude.clone(),
                })
                .collect(),
        },
        SignalKind::Complexity => SignalDetail::Complexity {
            fn_cyclomatic: tooling
                .complexity
                .as_ref()
                .and_then(|value| value.fn_cyclomatic)
                .map(project_threshold),
            fn_cognitive: tooling
                .complexity
                .as_ref()
                .and_then(|value| value.fn_cognitive)
                .map(project_threshold),
        },
        SignalKind::Deps => SignalDetail::Deps {
            forbidden: tooling
                .deps
                .as_ref()
                .map(|deps| {
                    deps.forbidden
                        .iter()
                        .map(|(from, targets)| ForbiddenDependencyProjection {
                            from: from.clone(),
                            targets: targets.clone(),
                        })
                        .collect()
                })
                .unwrap_or_default(),
        },
    }
}

fn project_threshold(threshold: ThresholdFloat) -> ThresholdProjection {
    ThresholdProjection {
        warn: threshold.warn,
        fail: threshold.fail,
    }
}

fn project_tool_override(value: Option<&ToolCommandOverride>) -> Option<ToolOverrideProjection> {
    value.map(|value| ToolOverrideProjection {
        command: value.command.clone(),
        args: value.args.clone(),
    })
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

fn render_human(projection: &ContractProjection) -> String {
    let mut output = format!(
        "Configured signal contract (projection version {})\n",
        projection.projection_version
    );
    writeln!(output, "\nenvironment:").expect("writing to String cannot fail");
    writeln!(
        output,
        "  docker: {:?} | network: {:?}",
        projection.environment.docker, projection.environment.network
    )
    .expect("writing to String cannot fail");
    writeln!(
        output,
        "  resources: cpus={} memory={}MiB memory+swap={}MiB pids={} nofile={}",
        projection.environment.resources.cpus,
        projection.environment.resources.memory_mib,
        projection.environment.resources.memory_swap_mib,
        projection.environment.resources.pids,
        projection.environment.resources.nofile,
    )
    .expect("writing to String cannot fail");
    writeln!(output, "  tools:").expect("writing to String cannot fail");
    if projection.environment.tools.is_empty() {
        writeln!(output, "    none").expect("writing to String cannot fail");
    } else {
        for tool in &projection.environment.tools {
            writeln!(output, "    - {}@{}", tool.tool, tool.version)
                .expect("writing to String cannot fail");
        }
    }
    writeln!(output, "  Debian packages:").expect("writing to String cannot fail");
    if projection.environment.debian_packages.is_empty() {
        writeln!(output, "    none").expect("writing to String cannot fail");
    } else {
        for package in &projection.environment.debian_packages {
            writeln!(output, "    - {package}").expect("writing to String cannot fail");
        }
    }
    for language in &projection.languages {
        writeln!(output, "\nlanguage: {}", language.language)
            .expect("writing to String cannot fail");
        writeln!(output, "  roots:").expect("writing to String cannot fail");
        for root in &language.roots {
            writeln!(output, "    - {root}").expect("writing to String cannot fail");
        }
        writeln!(output, "  signals:").expect("writing to String cannot fail");
        for signal in &language.signals {
            writeln!(
                output,
                "    {}: {}",
                signal_name(signal.kind),
                if signal.enabled {
                    "enabled"
                } else {
                    "disabled"
                }
            )
            .expect("writing to String cannot fail");
            render_signal_detail(&mut output, &signal.detail);
        }
    }
    writeln!(output, "\nwarnings:").expect("writing to String cannot fail");
    if projection.warnings.is_empty() {
        writeln!(output, "  none").expect("writing to String cannot fail");
    } else {
        for warning in &projection.warnings {
            writeln!(
                output,
                "  - [{}] {}: {}",
                warning.code, warning.policy_path, warning.message
            )
            .expect("writing to String cannot fail");
        }
    }
    output
}

fn render_signal_detail(output: &mut String, detail: &SignalDetail) {
    match detail {
        SignalDetail::Tool { tool_override } => {
            render_tool_override(output, tool_override.as_ref())
        }
        SignalDetail::Coverage {
            line_percent,
            branch_percent,
            coverage_satisfies_test,
            tool_override,
        } => {
            writeln!(output, "      thresholds:").expect("writing to String cannot fail");
            render_threshold(output, "line_percent (minimum)", line_percent.as_ref());
            render_threshold(output, "branch_percent (minimum)", branch_percent.as_ref());
            writeln!(
                output,
                "      coverage satisfies test: {}",
                if *coverage_satisfies_test {
                    "yes"
                } else {
                    "no"
                }
            )
            .expect("writing to String cannot fail");
            render_tool_override(output, tool_override.as_ref());
        }
        SignalDetail::Size { rules } => {
            if rules.is_empty() {
                writeln!(output, "      rules: not configured")
                    .expect("writing to String cannot fail");
            } else {
                writeln!(output, "      rules:").expect("writing to String cannot fail");
                for rule in rules {
                    writeln!(
                        output,
                        "        - pattern: {} | warn: {} | fail: {}",
                        quoted(&rule.pattern),
                        rule.warn,
                        rule.fail
                    )
                    .expect("writing to String cannot fail");
                    writeln!(
                        output,
                        "          exclusions: {}",
                        if rule.exclude.is_empty() {
                            String::from("none")
                        } else {
                            quoted_list(&rule.exclude)
                        }
                    )
                    .expect("writing to String cannot fail");
                }
            }
        }
        SignalDetail::Complexity {
            fn_cyclomatic,
            fn_cognitive,
        } => {
            writeln!(output, "      thresholds:").expect("writing to String cannot fail");
            render_threshold(output, "fn_cyclomatic (maximum)", fn_cyclomatic.as_ref());
            render_threshold(output, "fn_cognitive (maximum)", fn_cognitive.as_ref());
        }
        SignalDetail::Deps { forbidden } => {
            if forbidden.is_empty() {
                writeln!(output, "      restrictions: not configured")
                    .expect("writing to String cannot fail");
            } else {
                writeln!(output, "      restrictions:").expect("writing to String cannot fail");
                for rule in forbidden {
                    if rule.targets.is_empty() {
                        writeln!(output, "        - {} -> none", quoted(&rule.from))
                            .expect("writing to String cannot fail");
                    } else {
                        for target in &rule.targets {
                            writeln!(
                                output,
                                "        - {} -> {}",
                                quoted(&rule.from),
                                quoted(target)
                            )
                            .expect("writing to String cannot fail");
                        }
                    }
                }
            }
        }
    }
}

fn signal_name(kind: SignalKind) -> &'static str {
    match kind {
        SignalKind::Test => "test",
        SignalKind::Coverage => "coverage",
        SignalKind::Size => "size",
        SignalKind::Complexity => "complexity",
        SignalKind::Deps => "deps",
        SignalKind::Mutation => "mutation",
    }
}

fn render_threshold(output: &mut String, name: &str, threshold: Option<&ThresholdProjection>) {
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

fn render_tool_override(output: &mut String, value: Option<&ToolOverrideProjection>) {
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
    format!(
        "[{}]",
        values
            .iter()
            .map(|value| quoted(value))
            .collect::<Vec<_>>()
            .join(", ")
    )
}
fn quoted(value: &str) -> String {
    format!("{value:?}")
}
