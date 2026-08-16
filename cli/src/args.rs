use crate::application::{
    CheckOperation, ContractOperation, EnvLockOperation, EnvRunOperation, EnvShellOperation,
    EnvShowOperation, ExecutionMode, ImpactOperation, Operation, OutputFormat, RepositoryOperation,
    ResultsCompareOperation, VerifyOperation,
};
use ayni_core::{Language, SignalKind};
use clap::{Args, Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

const DEFAULT_CONFIG: &str = "./.ayni.toml";

#[derive(Parser, Debug)]
#[command(name = "ayni")]
#[command(
    version,
    about = "Correct environments, focused feedback, one definitive quality gate"
)]
pub(crate) struct Cli {
    #[command(subcommand)]
    command: Commands,
}

impl Cli {
    pub(crate) fn into_operation(self) -> Operation {
        self.command.into_operation()
    }
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Inspect and manage the repository code environment.
    Env {
        #[command(subcommand)]
        command: EnvCommands,
    },
    /// Inspect and validate the repository quality contract.
    Contract {
        #[command(subcommand)]
        command: ContractCommands,
    },
    /// Run one quality signal with optional adapter-owned selectors.
    Verify {
        #[command(subcommand)]
        command: VerifyCommands,
    },
    /// Explain or run the checks affected by an explicit change.
    Impact {
        #[command(subcommand)]
        command: ImpactCommands,
    },
    /// Run the complete repository quality contract.
    Check(CheckOptions),
    /// Manage Ayni's agent instructions.
    Agents {
        #[command(subcommand)]
        command: AgentsCommands,
    },
    /// Inspect and compare explicit local result files.
    Results {
        #[command(subcommand)]
        command: ResultsCommands,
    },
    #[command(hide = true)]
    GenerateDocs,
}

impl Commands {
    fn into_operation(self) -> Operation {
        match self {
            Self::Env { command } => command.into_operation(),
            Self::Contract { command } => command.into_operation(),
            Self::Verify { command } => Operation::Verify(command.into_operation()),
            Self::Impact { command } => command.into_operation(),
            Self::Check(options) => Operation::Check(options.into_operation()),
            Self::Agents {
                command: AgentsCommands::Sync(options),
            } => Operation::AgentsSync(options.into()),
            Self::Results { command } => command.into_operation(),
            Self::GenerateDocs => Operation::GenerateDocs,
        }
    }
}

#[derive(Subcommand, Debug)]
enum EnvCommands {
    /// Explain the resolved environment plan without modifying state.
    Show(EnvShowOptions),
    /// Diagnose missing, conflicting, unsupported, or stale environment state.
    Doctor(RepositoryOptions),
    /// Resolve exact environment requirements into the committed lock.
    Lock(EnvLockOptions),
    /// Build the repository code-environment image from a current lock.
    Build(RepositoryOptions),
    /// Enter the managed environment with the checkout mounted.
    Shell(EnvShellOptions),
    /// Run an arbitrary command inside the managed environment.
    Run(EnvRunOptions),
}

impl EnvCommands {
    fn into_operation(self) -> Operation {
        match self {
            Self::Show(options) => Operation::EnvShow(options.into_operation()),
            Self::Doctor(options) => Operation::EnvDoctor(options.into()),
            Self::Lock(options) => Operation::EnvLock(options.into_operation()),
            Self::Build(options) => Operation::EnvBuild(options.into()),
            Self::Shell(options) => Operation::EnvShell(EnvShellOperation {
                repo_root: options.repo_root,
                language: options.target.language.map(LanguageArg::into_language),
                root: options.target.root,
            }),
            Self::Run(options) => Operation::EnvRun(EnvRunOperation {
                repo_root: options.repo_root,
                language: options.target.language.map(LanguageArg::into_language),
                root: options.target.root,
                command: options.command,
            }),
        }
    }
}

#[derive(Subcommand, Debug)]
enum ContractCommands {
    /// Render the effective quality contract.
    Show(ContractOptions),
    /// Validate the contract without discovery or tool execution.
    Validate(ContractOptions),
}

impl ContractCommands {
    fn into_operation(self) -> Operation {
        match self {
            Self::Show(options) => Operation::ContractShow(options.into_operation()),
            Self::Validate(options) => Operation::ContractValidate(options.into_operation()),
        }
    }
}

#[derive(Subcommand, Debug)]
enum VerifyCommands {
    /// Run only the test signal.
    Test {
        #[command(flatten)]
        options: VerifyFilePackageOptions,
        #[arg(long)]
        name: Option<String>,
    },
    /// Run only the coverage signal.
    Coverage(VerifyCommonOptions),
    /// Run only the size signal.
    Size(VerifyFileOptions),
    /// Run only the complexity signal.
    Complexity(VerifyFilePackageOptions),
    /// Run only the dependency signal.
    Deps(VerifyFilePackageOptions),
    /// Run only the mutation signal.
    Mutation(VerifyCommonOptions),
}

impl VerifyCommands {
    fn into_operation(self) -> VerifyOperation {
        let (signal, options, file, package, name) = match self {
            Self::Test { options, name } => (
                SignalKind::Test,
                options.common,
                options.file,
                options.package,
                name,
            ),
            Self::Coverage(options) => (SignalKind::Coverage, options, None, None, None),
            Self::Size(options) => (SignalKind::Size, options.common, options.file, None, None),
            Self::Complexity(options) => (
                SignalKind::Complexity,
                options.common,
                options.file,
                options.package,
                None,
            ),
            Self::Deps(options) => (
                SignalKind::Deps,
                options.common,
                options.file,
                options.package,
                None,
            ),
            Self::Mutation(options) => (SignalKind::Mutation, options, None, None, None),
        };
        VerifyOperation {
            signal,
            config: options.config,
            language: options.language.map(LanguageArg::into_language),
            root: options.root,
            file,
            package,
            name,
            output: options.output.into(),
            execution_mode: execution_mode(options.host),
            debug: options.debug,
        }
    }
}

#[derive(Subcommand, Debug)]
enum ImpactCommands {
    /// Explain the quality work affected by a change without running it.
    Show(ImpactShowOptions),
    /// Execute the quality work affected by a change.
    Run(ImpactRunOptions),
}

impl ImpactCommands {
    fn into_operation(self) -> Operation {
        match self {
            Self::Show(options) => Operation::ImpactShow(options.into_operation()),
            Self::Run(options) => Operation::ImpactRun(options.into_operation()),
        }
    }
}

#[derive(Subcommand, Debug)]
enum AgentsCommands {
    /// Create or refresh only Ayni's managed AGENTS.md block.
    Sync(RepositoryOptions),
}

#[derive(Subcommand, Debug)]
enum ResultsCommands {
    /// Compare two explicit compatible result files.
    Compare(ResultsCompareOptions),
}

impl ResultsCommands {
    fn into_operation(self) -> Operation {
        match self {
            Self::Compare(options) => Operation::ResultsCompare(ResultsCompareOperation {
                baseline: options.baseline,
                candidate: options.candidate,
                output: options.output.into(),
            }),
        }
    }
}

#[derive(Args, Debug)]
struct RepositoryOptions {
    #[arg(long, default_value = ".")]
    repo_root: PathBuf,
}

impl From<RepositoryOptions> for RepositoryOperation {
    fn from(options: RepositoryOptions) -> Self {
        Self {
            repo_root: options.repo_root,
        }
    }
}

#[derive(Args, Debug)]
struct EnvShowOptions {
    /// Policy configuration file, resolved under the repository root.
    #[arg(long, default_value = DEFAULT_CONFIG)]
    config: PathBuf,
    /// Repository root that contains the policy and configured targets.
    #[arg(long, default_value = ".")]
    repo_root: PathBuf,
    /// Render a human-readable plan or one JSON document.
    #[arg(long, value_enum, default_value_t)]
    output: DataOutputArg,
}

impl EnvShowOptions {
    fn into_operation(self) -> EnvShowOperation {
        EnvShowOperation {
            config: self.config,
            repo_root: self.repo_root,
            output: self.output.into(),
        }
    }
}

#[derive(Args, Debug)]
struct EnvLockOptions {
    /// Policy configuration file, resolved under the repository root.
    #[arg(long, default_value = DEFAULT_CONFIG)]
    config: PathBuf,
    /// Repository root where `.ayni.lock` will be written.
    #[arg(long, default_value = ".")]
    repo_root: PathBuf,
    /// Exact environment base as `<reference>@sha256:<digest>`; otherwise resolve the release base with Docker Buildx.
    #[arg(long)]
    base: Option<String>,
}

impl EnvLockOptions {
    fn into_operation(self) -> EnvLockOperation {
        EnvLockOperation {
            config: self.config,
            repo_root: self.repo_root,
            base: self.base,
        }
    }
}

#[derive(Args, Debug)]
struct EnvironmentTargetOptions {
    /// Select a locked language target; required with --root and when otherwise ambiguous.
    #[arg(long, value_enum)]
    language: Option<LanguageArg>,
    /// Select one normalized locked root.
    #[arg(long)]
    root: Option<String>,
}

#[derive(Args, Debug)]
struct EnvShellOptions {
    #[arg(long, default_value = ".")]
    repo_root: PathBuf,
    #[command(flatten)]
    target: EnvironmentTargetOptions,
}

#[derive(Args, Debug)]
struct EnvRunOptions {
    #[arg(long, default_value = ".")]
    repo_root: PathBuf,
    #[command(flatten)]
    target: EnvironmentTargetOptions,
    #[arg(required = true, last = true, allow_hyphen_values = true)]
    command: Vec<String>,
}

#[derive(Args, Debug)]
struct ContractOptions {
    #[arg(long, default_value = DEFAULT_CONFIG)]
    config: PathBuf,
    #[arg(long, value_enum, default_value_t)]
    output: DataOutputArg,
}

impl ContractOptions {
    fn into_operation(self) -> ContractOperation {
        ContractOperation {
            config: self.config,
            output: self.output.into(),
        }
    }
}

#[derive(Args, Debug)]
struct CheckOptions {
    #[arg(long, default_value = DEFAULT_CONFIG)]
    config: PathBuf,
    #[arg(long, value_enum, default_value_t)]
    output: OutputArg,
    /// Run on the host instead of in the managed environment.
    #[arg(long)]
    host: bool,
    /// Print raw command diagnostics.
    #[arg(long)]
    debug: bool,
}

impl CheckOptions {
    fn into_operation(self) -> CheckOperation {
        CheckOperation {
            config: self.config,
            output: self.output.into(),
            execution_mode: execution_mode(self.host),
            debug: self.debug,
        }
    }
}

#[derive(Args, Debug)]
struct VerifyCommonOptions {
    #[arg(long, default_value = DEFAULT_CONFIG)]
    config: PathBuf,
    #[arg(long, value_enum)]
    language: Option<LanguageArg>,
    /// Select exactly one normalized root configured for the selected language.
    #[arg(long)]
    root: Option<String>,
    #[arg(long, value_enum, default_value_t)]
    output: OutputArg,
    /// Run on the host instead of in the managed environment.
    #[arg(long)]
    host: bool,
    /// Print raw command diagnostics.
    #[arg(long)]
    debug: bool,
}

#[derive(Args, Debug)]
struct VerifyFileOptions {
    #[command(flatten)]
    common: VerifyCommonOptions,
    #[arg(long)]
    file: Option<String>,
}

#[derive(Args, Debug)]
struct VerifyFilePackageOptions {
    #[command(flatten)]
    common: VerifyCommonOptions,
    #[arg(long)]
    file: Option<String>,
    #[arg(long)]
    package: Option<String>,
}

#[derive(Args, Debug)]
struct ImpactShowOptions {
    #[arg(long, default_value = DEFAULT_CONFIG)]
    config: PathBuf,
    /// Explicit base revision used to calculate the change.
    #[arg(long)]
    base: String,
    #[arg(long, value_enum, default_value_t)]
    output: OutputArg,
}

impl ImpactShowOptions {
    fn into_operation(self) -> ImpactOperation {
        ImpactOperation {
            config: self.config,
            base: self.base,
            output: self.output.into(),
            execution_mode: ExecutionMode::Managed,
            debug: false,
        }
    }
}

#[derive(Args, Debug)]
struct ImpactRunOptions {
    #[command(flatten)]
    common: ImpactShowOptions,
    /// Run on the host instead of in the managed environment.
    #[arg(long)]
    host: bool,
    /// Print raw command diagnostics.
    #[arg(long)]
    debug: bool,
}

impl ImpactRunOptions {
    fn into_operation(self) -> ImpactOperation {
        let mut operation = self.common.into_operation();
        operation.execution_mode = execution_mode(self.host);
        operation.debug = self.debug;
        operation
    }
}

#[derive(Args, Debug)]
struct ResultsCompareOptions {
    /// Earlier result file.
    #[arg(long)]
    baseline: PathBuf,
    /// Later result file.
    #[arg(long)]
    candidate: PathBuf,
    #[arg(long, value_enum, default_value_t)]
    output: DataOutputArg,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, ValueEnum)]
enum DataOutputArg {
    /// Human-readable terminal output.
    #[default]
    Human,
    /// One deterministic JSON document on stdout.
    Json,
}

impl From<DataOutputArg> for OutputFormat {
    fn from(output: DataOutputArg) -> Self {
        match output {
            DataOutputArg::Human => Self::Human,
            DataOutputArg::Json => Self::Json,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, ValueEnum)]
enum OutputArg {
    /// Human-readable terminal output.
    #[default]
    Human,
    /// One deterministic JSON document on stdout.
    Json,
    /// Deterministic Markdown output.
    Markdown,
}

impl From<OutputArg> for OutputFormat {
    fn from(output: OutputArg) -> Self {
        match output {
            OutputArg::Human => Self::Human,
            OutputArg::Json => Self::Json,
            OutputArg::Markdown => Self::Markdown,
        }
    }
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum LanguageArg {
    Rust,
    Go,
    Node,
    Python,
    Kotlin,
}

impl LanguageArg {
    fn into_language(self) -> Language {
        match self {
            Self::Rust => Language::Rust,
            Self::Go => Language::Go,
            Self::Node => Language::Node,
            Self::Python => Language::Python,
            Self::Kotlin => Language::Kotlin,
        }
    }
}

fn execution_mode(host: bool) -> ExecutionMode {
    if host {
        ExecutionMode::Host
    } else {
        ExecutionMode::Managed
    }
}

#[cfg(test)]
mod tests {
    use super::Cli;
    use crate::application::{ExecutionMode, Operation, OutputFormat};
    use ayni_core::{Language, SignalKind};
    use clap::{CommandFactory, Parser};
    use std::path::PathBuf;

    #[test]
    fn complete_command_tree_is_exposed() {
        let command = Cli::command();
        let names = command
            .get_subcommands()
            .filter(|subcommand| !subcommand.is_hide_set())
            .map(|subcommand| subcommand.get_name())
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            [
                "env", "contract", "verify", "impact", "check", "agents", "results"
            ]
        );
    }

    #[test]
    fn every_public_command_maps_to_one_typed_operation() {
        let cases = [
            (vec!["ayni", "env", "show"], "EnvShow"),
            (vec!["ayni", "env", "doctor"], "EnvDoctor"),
            (vec!["ayni", "env", "lock"], "EnvLock"),
            (vec!["ayni", "env", "build"], "EnvBuild"),
            (vec!["ayni", "env", "shell"], "EnvShell"),
            (vec!["ayni", "env", "run", "--", "cargo", "test"], "EnvRun"),
            (vec!["ayni", "contract", "show"], "ContractShow"),
            (vec!["ayni", "contract", "validate"], "ContractValidate"),
            (vec!["ayni", "verify", "test"], "Verify"),
            (
                vec!["ayni", "impact", "show", "--base", "main"],
                "ImpactShow",
            ),
            (vec!["ayni", "impact", "run", "--base", "main"], "ImpactRun"),
            (vec!["ayni", "check"], "Check"),
            (vec!["ayni", "agents", "sync"], "AgentsSync"),
            (
                vec![
                    "ayni",
                    "results",
                    "compare",
                    "--baseline",
                    "before.json",
                    "--candidate",
                    "after.json",
                ],
                "ResultsCompare",
            ),
        ];

        for (arguments, expected) in cases {
            let operation = Cli::try_parse_from(arguments)
                .expect("arguments parse")
                .into_operation();
            assert!(
                format!("{operation:?}").starts_with(expected),
                "{operation:?}"
            );
        }
    }

    #[test]
    fn verify_maps_signal_selectors_and_global_execution_options() {
        let operation = Cli::try_parse_from([
            "ayni",
            "verify",
            "test",
            "--config",
            "policy.toml",
            "--language",
            "node",
            "--root",
            "frontend",
            "--package",
            "@example/web",
            "--file",
            "frontend/cart.test.ts",
            "--name",
            "formats money",
            "--host",
            "--output",
            "json",
        ])
        .expect("arguments parse")
        .into_operation();
        let Operation::Verify(operation) = operation else {
            panic!("verify operation");
        };
        assert_eq!(operation.signal, SignalKind::Test);
        assert_eq!(operation.config, PathBuf::from("policy.toml"));
        assert_eq!(operation.language, Some(Language::Node));
        assert_eq!(operation.root.as_deref(), Some("frontend"));
        assert_eq!(operation.package.as_deref(), Some("@example/web"));
        assert_eq!(operation.file.as_deref(), Some("frontend/cart.test.ts"));
        assert_eq!(operation.name.as_deref(), Some("formats money"));
        assert_eq!(operation.execution_mode, ExecutionMode::Host);
        assert_eq!(operation.output, OutputFormat::Json);
    }

    #[test]
    fn signal_invalid_selectors_are_rejected_before_execution() {
        for (signal, selector) in [
            ("coverage", "--file"),
            ("size", "--package"),
            ("mutation", "--name"),
        ] {
            let error = Cli::try_parse_from(["ayni", "verify", signal, selector, "target"])
                .expect_err("invalid selector must fail");
            assert!(error.to_string().contains("unexpected argument"));
        }
    }

    #[test]
    fn env_run_preserves_the_forwarded_command() {
        let operation = Cli::try_parse_from([
            "ayni",
            "env",
            "run",
            "--repo-root",
            "fixture",
            "--language",
            "rust",
            "--root",
            "crates/app",
            "--",
            "cargo",
            "test",
            "--workspace",
        ])
        .expect("arguments parse")
        .into_operation();
        let Operation::EnvRun(operation) = operation else {
            panic!("env run operation");
        };
        assert_eq!(operation.repo_root, PathBuf::from("fixture"));
        assert_eq!(operation.language, Some(Language::Rust));
        assert_eq!(operation.root.as_deref(), Some("crates/app"));
        assert_eq!(operation.command, ["cargo", "test", "--workspace"]);
    }

    #[test]
    fn managed_execution_is_default_and_host_is_explicit() {
        let Operation::Check(default) = Cli::try_parse_from(["ayni", "check"])
            .expect("check parses")
            .into_operation()
        else {
            panic!("check operation");
        };
        let Operation::Check(host) = Cli::try_parse_from(["ayni", "check", "--host"])
            .expect("host check parses")
            .into_operation()
        else {
            panic!("check operation");
        };
        assert_eq!(default.execution_mode, ExecutionMode::Managed);
        assert_eq!(host.execution_mode, ExecutionMode::Host);
    }

    #[test]
    fn output_support_is_explicit_per_command() {
        assert!(Cli::try_parse_from(["ayni", "contract", "show", "--output", "markdown"]).is_err());
        assert!(
            Cli::try_parse_from([
                "ayni",
                "results",
                "compare",
                "--baseline",
                "before.json",
                "--candidate",
                "after.json",
                "--output",
                "markdown",
            ])
            .is_err()
        );
        assert!(Cli::try_parse_from(["ayni", "check", "--output", "markdown"]).is_ok());
    }

    #[test]
    fn impact_show_rejects_execution_only_options() {
        for option in ["--host", "--debug"] {
            assert!(
                Cli::try_parse_from(["ayni", "impact", "show", "--base", "main", option]).is_err(),
                "{option} must remain exclusive to impact run"
            );
        }
        assert!(
            Cli::try_parse_from([
                "ayni", "impact", "run", "--base", "main", "--host", "--debug"
            ])
            .is_ok()
        );
    }

    #[test]
    fn superseded_commands_do_not_parse() {
        for command in ["analyze", "install", "artifact"] {
            let error = Cli::try_parse_from(["ayni", command]).expect_err("old command must fail");
            assert!(error.to_string().contains("unrecognized subcommand"));
        }
        assert!(Cli::try_parse_from(["ayni", "contract", "display"]).is_err());
    }
}
