use super::*;

fn digest(character: char) -> String {
    format!("sha256:{}", character.to_string().repeat(64))
}

fn source(path: &str) -> RequirementSource {
    RequirementSource::new(
        "manifest",
        path,
        None::<String>,
        RequirementConfidence::Declared,
    )
    .expect("source")
}

fn platform() -> TargetPlatform {
    TargetPlatform {
        os: OperatingSystem::Linux,
        architecture: Architecture::Amd64,
        libc: Libc::Glibc,
    }
}

fn target(root: &str, runtime_version: VersionRequirement) -> TargetEnvironment {
    TargetEnvironment {
        target: TargetIdentity::new(Language::Node, root).expect("target"),
        workspace: Some(root.to_string()),
        package: Some(String::from("@example/web")),
        runtimes: vec![RuntimeRequirement {
            runtime: String::from("node"),
            version: runtime_version,
            components: vec![String::from("corepack"), String::from("corepack")],
            targets: Vec::new(),
            source: source(&format!("{root}/package.json")),
        }],
        package_manager: Some(PackageManagerRequirement {
            family: String::from("pnpm"),
            version: VersionRequirement::exact("10.14.0").expect("version"),
            ownership_root: root.to_string(),
            source: source(&format!("{root}/package.json")),
        }),
        signal_tools: vec![SignalToolRequirement {
            tool: String::from("vitest"),
            version: VersionRequirement::exact("3.2.4").expect("version"),
            provider: String::from("project_dependency"),
            scope: ToolInstallationScope::Project,
            signals: vec![SignalKind::Coverage, SignalKind::Test, SignalKind::Test],
            supported_platforms: vec![platform()],
            provisioning: ProvisioningSupport::LockedOffline,
            modifies_checkout: false,
            source: source(&format!("{root}/package.json")),
        }],
        system_requirements: vec![SystemRequirement {
            kind: SystemRequirementKind::Capability,
            name: String::from("native-build"),
            supported_platforms: vec![platform()],
            provisioning: ProvisioningSupport::LockedOffline,
            source: source(&format!("{root}/package.json")),
        }],
        dependency_locks: vec![DependencyLockRequirement {
            path: format!("{root}/pnpm-lock.yaml"),
            digest: digest('a'),
            owner_root: root.to_string(),
            source: source(&format!("{root}/pnpm-lock.yaml")),
        }],
    }
}

fn plan(targets: Vec<TargetEnvironment>) -> EnvironmentPlan {
    EnvironmentPlan::new(
        RepositoryIdentity {
            name: String::from("fixture"),
            contract_digest: digest('b'),
        },
        vec![platform(), platform()],
        targets,
        Vec::new(),
        Vec::new(),
    )
    .expect("plan")
}

#[test]
fn normalized_target_identity_is_checkout_independent() {
    let target = TargetIdentity::new(Language::Rust, r".\crates\core\").expect("identity");
    assert_eq!(target.root, "crates/core");
}

#[test]
fn equal_semantic_inputs_produce_byte_stable_ordered_plans() {
    let first = plan(vec![
        target(
            "apps/zeta",
            VersionRequirement::exact("22.1.0").expect("version"),
        ),
        target(
            "apps/alpha",
            VersionRequirement::exact("20.2.0").expect("version"),
        ),
    ]);
    let second = plan(vec![
        target(
            "apps/alpha",
            VersionRequirement::exact("20.2.0").expect("version"),
        ),
        target(
            "apps/zeta",
            VersionRequirement::exact("22.1.0").expect("version"),
        ),
    ]);
    assert_eq!(first, second);
    assert_eq!(
        serde_json::to_string(&first).expect("serialize"),
        serde_json::to_string(&second).expect("serialize")
    );
    assert_eq!(first.targets()[0].target.root, "apps/alpha");
    assert_eq!(first.targets()[0].runtimes[0].components, ["corepack"]);
    assert_eq!(
        first.targets()[0].signal_tools[0].signals,
        [SignalKind::Test, SignalKind::Coverage]
    );
}

#[test]
fn version_evidence_preserves_ecosystem_semantics() {
    let values = [
        VersionRequirement::selector("stable").expect("selector"),
        VersionRequirement::compatibility(">=20 <23").expect("compatibility"),
        VersionRequirement::minimum("1.80").expect("minimum"),
    ];
    let json = serde_json::to_value(values).expect("serialize");
    assert_eq!(json[0]["state"], "selector");
    assert_eq!(json[1]["state"], "compatibility");
    assert_eq!(json[2]["state"], "minimum");
}

#[test]
fn workspace_and_package_do_not_change_target_identity() {
    let mut first = target(
        "apps/web",
        VersionRequirement::exact("22.1.0").expect("version"),
    );
    let mut second = first.clone();
    first.workspace = Some(String::from("."));
    second.workspace = Some(String::from("apps"));
    second.package = Some(String::from("renamed-package"));
    assert_eq!(first.target, second.target);
    assert!(matches!(
        EnvironmentPlan::new(
            RepositoryIdentity {
                name: String::from("fixture"),
                contract_digest: digest('b'),
            },
            vec![platform()],
            vec![first, second],
            Vec::new(),
            Vec::new(),
        ),
        Err(EnvironmentPlanError::DuplicateTarget(_))
    ));
}

#[test]
fn conflicts_and_unresolved_requirements_cannot_be_resolved() {
    let conflict = EnvironmentConflict {
        code: String::from("runtime_conflict"),
        message: String::from("runtime sources disagree"),
        target: None,
        sources: vec![source("rust-toolchain.toml")],
    };
    let conflicting = EnvironmentPlan::new(
        RepositoryIdentity {
            name: String::from("fixture"),
            contract_digest: digest('b'),
        },
        vec![platform()],
        vec![target(
            "apps/web",
            VersionRequirement::exact("22.1.0").expect("version"),
        )],
        Vec::new(),
        vec![conflict],
    )
    .expect("plan");
    assert_eq!(
        conflicting.resolve(),
        Err(EnvironmentPlanError::BlockingConflicts(1))
    );

    let unresolved = plan(vec![target(
        "apps/web",
        VersionRequirement::compatibility(">=20 <23").expect("compatibility"),
    )]);
    assert_eq!(
        unresolved.resolve(),
        Err(EnvironmentPlanError::UnresolvedRequirements(1))
    );
}

#[test]
fn resolved_plan_requires_exact_non_floating_versions() {
    assert_eq!(
        VersionRequirement::exact("latest"),
        Err(EnvironmentPlanError::FloatingExactVersion(String::from(
            "latest"
        )))
    );
    let resolved = plan(vec![target(
        "apps/web",
        VersionRequirement::exact("22.1.0").expect("version"),
    )])
    .resolve()
    .expect("resolved plan");
    assert!(resolved.plan().targets()[0].runtimes[0].version.is_exact());
}

#[test]
fn absolute_parent_and_windows_paths_are_rejected() {
    for path in ["/tmp/repo", "../outside", r"C:\\repo", "apps/../../outside"] {
        let error = TargetIdentity::new(Language::Rust, path).expect_err("path must fail");
        assert!(matches!(
            error,
            EnvironmentPlanError::NonPortablePath { .. }
        ));
    }
}

#[test]
fn duplicate_targets_and_invalid_digests_fail_validation() {
    let duplicate = target(
        "apps/web",
        VersionRequirement::exact("22.1.0").expect("version"),
    );
    assert!(matches!(
        EnvironmentPlan::new(
            RepositoryIdentity {
                name: String::from("fixture"),
                contract_digest: digest('b'),
            },
            vec![platform()],
            vec![duplicate.clone(), duplicate],
            Vec::new(),
            Vec::new(),
        ),
        Err(EnvironmentPlanError::DuplicateTarget(_))
    ));
    assert!(matches!(
        EnvironmentPlan::new(
            RepositoryIdentity {
                name: String::from("fixture"),
                contract_digest: String::from("not-a-digest"),
            },
            vec![platform()],
            Vec::new(),
            Vec::new(),
            Vec::new(),
        ),
        Err(EnvironmentPlanError::InvalidDigest { .. })
    ));
}

#[test]
fn diagnostic_paths_are_normalized_and_must_reference_plan_targets() {
    let identity = TargetIdentity::new(Language::Node, "apps/web").expect("target");
    let warning = EnvironmentWarning {
        code: String::from("missing_pin"),
        message: String::from("runtime is not pinned"),
        target: Some(TargetIdentity {
            language: Language::Node,
            root: String::from("apps\\web"),
        }),
    };
    let normalized = EnvironmentPlan::new(
        RepositoryIdentity {
            name: String::from("fixture"),
            contract_digest: digest('b'),
        },
        vec![platform()],
        vec![target(
            "apps/web",
            VersionRequirement::exact("22.1.0").expect("version"),
        )],
        vec![warning],
        Vec::new(),
    )
    .expect("plan");
    assert_eq!(normalized.warnings()[0].target.as_ref(), Some(&identity));

    let unknown = EnvironmentWarning {
        code: String::from("missing_pin"),
        message: String::from("runtime is not pinned"),
        target: Some(TargetIdentity {
            language: Language::Node,
            root: String::from("apps/other"),
        }),
    };
    assert!(matches!(
        EnvironmentPlan::new(
            RepositoryIdentity {
                name: String::from("fixture"),
                contract_digest: digest('b'),
            },
            vec![platform()],
            vec![target(
                "apps/web",
                VersionRequirement::exact("22.1.0").expect("version"),
            )],
            vec![unknown],
            Vec::new(),
        ),
        Err(EnvironmentPlanError::UnknownDiagnosticTarget(_))
    ));
}

#[test]
fn conflict_sources_cannot_contain_host_paths() {
    let conflict = EnvironmentConflict {
        code: String::from("runtime_conflict"),
        message: String::from("runtime sources disagree"),
        target: None,
        sources: vec![RequirementSource {
            kind: String::from("manifest"),
            path: String::from("/tmp/Cargo.toml"),
            detail: None,
            confidence: RequirementConfidence::Declared,
        }],
    };
    assert!(matches!(
        EnvironmentPlan::new(
            RepositoryIdentity {
                name: String::from("fixture"),
                contract_digest: digest('b'),
            },
            vec![platform()],
            vec![target(
                "apps/web",
                VersionRequirement::exact("22.1.0").expect("version"),
            )],
            Vec::new(),
            vec![conflict],
        ),
        Err(EnvironmentPlanError::NonPortablePath { .. })
    ));
}

#[test]
fn normalization_rejects_forged_floating_versions() {
    let mut target = target(
        "apps/web",
        VersionRequirement::exact("22.1.0").expect("version"),
    );
    target.runtimes[0].version = VersionRequirement::Exact {
        version: String::from("latest"),
    };
    assert_eq!(
        EnvironmentPlan::new(
            RepositoryIdentity {
                name: String::from("fixture"),
                contract_digest: digest('b'),
            },
            vec![platform()],
            vec![target],
            Vec::new(),
            Vec::new(),
        ),
        Err(EnvironmentPlanError::FloatingExactVersion(String::from(
            "latest"
        )))
    );
}

#[test]
fn contribution_normalizes_forged_version_values() {
    let variants = [
        VersionRequirement::Exact {
            version: String::from(" 22.1.0 "),
        },
        VersionRequirement::Selector {
            expression: String::from(" stable "),
        },
        VersionRequirement::Compatibility {
            expression: String::from(" >=20 <23 "),
        },
        VersionRequirement::Minimum {
            version: String::from(" 1.80 "),
        },
        VersionRequirement::Unresolved {
            reason: String::from(" ambiguous sources "),
        },
    ];
    let expected = ["22.1.0", "stable", ">=20 <23", "1.80", "ambiguous sources"];

    for (version, expected) in variants.into_iter().zip(expected) {
        let contribution =
            EnvironmentContribution::new(target("apps/web", version), Vec::new(), Vec::new())
                .expect("contribution");
        let serialized = serde_json::to_value(contribution).expect("serialize");
        let runtime_version = &serialized["target"]["runtimes"][0]["version"];
        let value = runtime_version
            .get("version")
            .or_else(|| runtime_version.get("expression"))
            .or_else(|| runtime_version.get("reason"))
            .and_then(serde_json::Value::as_str)
            .expect("version value");
        assert_eq!(value, expected);
    }
}

#[test]
fn ownership_uses_path_components_not_string_prefixes() {
    let mut target = target(
        "apps/api-v2",
        VersionRequirement::exact("22.1.0").expect("version"),
    );
    target.workspace = Some(String::from("apps/api"));
    assert!(matches!(
        EnvironmentPlan::new(
            RepositoryIdentity {
                name: String::from("fixture"),
                contract_digest: digest('b'),
            },
            vec![platform()],
            vec![target],
            Vec::new(),
            Vec::new(),
        ),
        Err(EnvironmentPlanError::PathOutsideOwner { .. })
    ));
}

#[test]
fn dependency_locks_must_be_below_their_owner() {
    let mut target = target(
        "apps/web",
        VersionRequirement::exact("22.1.0").expect("version"),
    );
    target.dependency_locks[0].path = String::from("shared/pnpm-lock.yaml");
    assert!(matches!(
        EnvironmentPlan::new(
            RepositoryIdentity {
                name: String::from("fixture"),
                contract_digest: digest('b'),
            },
            vec![platform()],
            vec![target],
            Vec::new(),
            Vec::new(),
        ),
        Err(EnvironmentPlanError::PathOutsideOwner { .. })
    ));
}

#[test]
fn deserialization_rejects_invalid_schema_paths_and_digests() {
    let valid = serde_json::to_value(plan(vec![target(
        "apps/web",
        VersionRequirement::exact("22.1.0").expect("version"),
    )]))
    .expect("serialize");
    for (pointer, invalid) in [
        ("/schema_version", serde_json::json!("9.9.9")),
        ("/targets/0/target/root", serde_json::json!("/tmp/repo")),
        ("/repository/contract_digest", serde_json::json!("bad")),
    ] {
        let mut value = valid.clone();
        *value.pointer_mut(pointer).expect("pointer") = invalid;
        assert!(
            serde_json::from_value::<EnvironmentPlan>(value).is_err(),
            "{pointer}"
        );
    }
}

#[test]
fn zero_target_plan_fails_closed() {
    assert_eq!(
        EnvironmentPlan::new(
            RepositoryIdentity {
                name: String::from("fixture"),
                contract_digest: digest('b'),
            },
            vec![platform()],
            Vec::new(),
            Vec::new(),
            Vec::new(),
        ),
        Err(EnvironmentPlanError::MissingTargets)
    );
}

#[test]
fn provisioning_readiness_rejects_unsupported_or_mutating_requirements() {
    let mut unsupported = target(
        "apps/web",
        VersionRequirement::exact("22.1.0").expect("version"),
    );
    unsupported.system_requirements[0].provisioning = ProvisioningSupport::Unsupported;
    assert!(matches!(
        plan(vec![unsupported]).resolve(),
        Err(EnvironmentPlanError::UnsupportedProvisioning { .. })
    ));

    let mut mutating = target(
        "apps/web",
        VersionRequirement::exact("22.1.0").expect("version"),
    );
    mutating.signal_tools[0].modifies_checkout = true;
    assert!(matches!(
        plan(vec![mutating]).resolve(),
        Err(EnvironmentPlanError::CheckoutMutation { .. })
    ));
}

#[test]
fn provisioning_readiness_rejects_unsupported_requested_platform() {
    let mut target = target(
        "apps/web",
        VersionRequirement::exact("22.1.0").expect("version"),
    );
    target.signal_tools[0].supported_platforms = vec![TargetPlatform {
        os: OperatingSystem::Linux,
        architecture: Architecture::Arm64,
        libc: Libc::Glibc,
    }];
    assert!(matches!(
        plan(vec![target]).resolve(),
        Err(EnvironmentPlanError::UnsupportedPlatform { .. })
    ));
}

#[test]
fn targets_require_runtime_evidence() {
    let mut target = target(
        "apps/web",
        VersionRequirement::exact("22.1.0").expect("version"),
    );
    target.runtimes.clear();
    assert!(matches!(
        EnvironmentPlan::new(
            RepositoryIdentity {
                name: String::from("fixture"),
                contract_digest: digest('b'),
            },
            vec![platform()],
            vec![target],
            Vec::new(),
            Vec::new(),
        ),
        Err(EnvironmentPlanError::MissingRuntime(_))
    ));
}

#[test]
fn serialized_contract_contains_no_provider_commands_or_host_paths() {
    let json = serde_json::to_string_pretty(&plan(vec![target(
        "apps/web",
        VersionRequirement::exact("22.1.0").expect("version"),
    )]))
    .expect("serialize");
    assert!(!json.contains("mise"));
    assert!(!json.contains("Dockerfile"));
    assert!(!json.contains("/Users/"));
    assert!(!json.contains("command"));
}
