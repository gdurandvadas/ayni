use super::*;
use ayni_adapters_common::environment::assert_environment_capability_conformance;
use ayni_core::{Architecture, Libc, OperatingSystem, SignalKind, TargetIdentity, TargetPlatform};
use std::fs;
use tempfile::TempDir;

#[test]
fn corepack_integrity_suffix_is_not_part_of_the_locked_manager_version() {
    let (_, version) = parse_package_manager("pnpm@9.15.4+sha512.aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
        .expect("Corepack package manager");
    assert_eq!(version, VersionRequirement::exact("9.15.4").expect("exact"));
    assert!(parse_package_manager("pnpm@9.15.4+metadata").is_ok());
}

fn request(root: &Path, target: &str, signals: Vec<SignalKind>) -> EnvironmentDiscoveryRequest {
    ayni_adapters_common::environment::environment_discovery_request(
        root.to_path_buf(),
        TargetIdentity::new(Language::Node, target).expect("target"),
        signals,
        vec![TargetPlatform {
            os: OperatingSystem::Linux,
            architecture: Architecture::Amd64,
            libc: Libc::Glibc,
        }],
    )
    .expect("request")
}

#[test]
fn runtime_manager_locks_tools_and_conformance() {
    let fixture = TempDir::new().expect("fixture");
    fs::write(fixture.path().join(".node-version"), "22.14.0\n").expect("selector");
    fs::write(
        fixture.path().join("pnpm-lock.yaml"),
        "lockfileVersion: '9.0'\n",
    )
    .expect("lock");
    fs::write(
            fixture.path().join("package.json"),
            r#"{"packageManager":"pnpm@9.15.4","devDependencies":{"vitest":"^3.2.4","@vitest/coverage-v8":"3.2.4"}}"#,
        )
        .expect("manifest");
    let contribution = assert_environment_capability_conformance(
        &NodeEnvironmentCapability,
        &request(
            fixture.path(),
            ".",
            vec![SignalKind::Test, SignalKind::Coverage],
        ),
    )
    .expect("conformance");
    assert_eq!(
        contribution.target().runtimes[0].version,
        VersionRequirement::exact("22.14.0").expect("version")
    );
    assert_eq!(
        contribution
            .target()
            .package_manager
            .as_ref()
            .expect("manager")
            .family,
        "pnpm"
    );
    assert_eq!(contribution.target().signal_tools.len(), 2);
    assert!(contribution.target().signal_tools.iter().all(|tool| {
        tool.provisioning == ProvisioningSupport::OnlineOnly && !tool.modifies_checkout
    }));
}

#[test]
fn direct_member_evidence_precedes_workspace_defaults() {
    let fixture = TempDir::new().expect("fixture");
    fs::create_dir_all(fixture.path().join("packages/app")).expect("package");
    fs::write(
        fixture.path().join("package.json"),
        r#"{"workspaces":["packages/*"],"packageManager":"pnpm@9.0.0","engines":{"node":">=20"}}"#,
    )
    .expect("workspace manifest");
    fs::write(
        fixture.path().join("pnpm-lock.yaml"),
        "lockfileVersion: '9.0'\n",
    )
    .expect("workspace lock");
    fs::write(
            fixture.path().join("packages/app/package.json"),
            r#"{"name":"app","packageManager":"yarn@4.1.0","engines":{"node":"^22"},"devDependencies":{"vitest":"3.2.4"}}"#,
        )
        .expect("package manifest");
    fs::write(
        fixture.path().join("packages/app/yarn.lock"),
        "__metadata:\n  version: 8\n",
    )
    .expect("package lock");
    let contribution = NodeEnvironmentCapability
        .discover(&request(
            fixture.path(),
            "packages/app",
            vec![SignalKind::Test],
        ))
        .expect("discovery");
    assert_eq!(contribution.target().workspace.as_deref(), Some("."));
    assert_eq!(
        contribution.target().runtimes[0].version,
        VersionRequirement::compatibility("^22").expect("compatibility")
    );
    let manager = contribution
        .target()
        .package_manager
        .as_ref()
        .expect("manager");
    assert_eq!(manager.family, "yarn");
    assert_eq!(manager.ownership_root, "packages/app");
    assert_eq!(
        contribution.target().dependency_locks[0].path,
        "packages/app/yarn.lock"
    );
    assert!(contribution.target().dependency_locks.iter().any(|input| {
        input.path == "packages/app/package.json" && input.source.kind == "node_manifest"
    }));
}

#[test]
fn hashes_only_declared_node_workspace_manifests() {
    let fixture = TempDir::new().expect("fixture");
    for path in ["packages/app", "packages/lib", "examples/unrelated"] {
        fs::create_dir_all(fixture.path().join(path)).expect("package");
        fs::write(
            fixture.path().join(path).join("package.json"),
            format!(r#"{{"name":"{}"}}"#, path.replace('/', "-")),
        )
        .expect("manifest");
    }
    fs::write(
        fixture.path().join("package.json"),
        r#"{"workspaces":["packages/*"],"packageManager":"npm@10.9.0","engines":{"node":"22.x"}}"#,
    )
    .expect("workspace manifest");
    fs::write(
        fixture.path().join("package-lock.json"),
        r#"{"lockfileVersion":3,"packages":{}}"#,
    )
    .expect("lock");
    let contribution = NodeEnvironmentCapability
        .discover(&request(fixture.path(), "packages/app", Vec::new()))
        .expect("discovery");
    let paths = contribution
        .target()
        .dependency_locks
        .iter()
        .map(|input| input.path.as_str())
        .collect::<Vec<_>>();
    assert!(paths.contains(&"package.json"));
    assert!(paths.contains(&"packages/app/package.json"));
    assert!(paths.contains(&"packages/lib/package.json"));
    assert!(!paths.contains(&"examples/unrelated/package.json"));
}

#[test]
fn non_member_descendant_is_not_claimed_by_workspace() {
    let fixture = TempDir::new().expect("fixture");
    fs::create_dir_all(fixture.path().join("other/app")).expect("package");
    fs::write(
        fixture.path().join("package.json"),
        r#"{"workspaces":["packages/*"],"engines":{"node":">=20"}}"#,
    )
    .expect("workspace manifest");
    fs::write(
        fixture.path().join("other/app/package.json"),
        r#"{"name":"app","engines":{"node":"22.x"}}"#,
    )
    .expect("package manifest");
    let contribution = NodeEnvironmentCapability
        .discover(&request(fixture.path(), "other/app", Vec::new()))
        .expect("discovery");
    assert_eq!(contribution.target().workspace, None);
    assert_eq!(
        contribution
            .target()
            .package_manager
            .as_ref()
            .expect("manager")
            .family,
        "npm"
    );
    assert!(
        contribution
            .conflicts()
            .iter()
            .any(|conflict| conflict.code == "node_dependency_lock_missing")
    );
}

#[test]
fn conflicts_include_values_and_absent_tools_require_checkout_change() {
    let fixture = TempDir::new().expect("fixture");
    fs::write(fixture.path().join(".node-version"), "20\n").expect("node version");
    fs::write(fixture.path().join(".nvmrc"), "22\n").expect("nvmrc");
    fs::write(
        fixture.path().join("yarn.lock"),
        "__metadata:\n  version: 8\n",
    )
    .expect("lock");
    fs::write(
        fixture.path().join("package.json"),
        r#"{"packageManager":"pnpm@9"}"#,
    )
    .expect("manifest");
    let contribution = NodeEnvironmentCapability
        .discover(&request(fixture.path(), ".", vec![SignalKind::Mutation]))
        .expect("discovery");
    assert_eq!(contribution.conflicts().len(), 2);
    assert!(
        contribution.conflicts()[0]
            .sources
            .iter()
            .all(|source| source.detail.is_some())
    );
    assert!(contribution.target().signal_tools[0].modifies_checkout);
}

#[test]
fn malformed_manifest_package_manager_and_empty_lock_fail_closed() {
    for (manifest, lock) in [
        ("{", None),
        (r#"{"packageManager":"pnpm"}"#, None),
        (r#"{"engines":{"node":">=20"}}"#, Some("")),
    ] {
        let fixture = TempDir::new().expect("fixture");
        fs::write(fixture.path().join("package.json"), manifest).expect("manifest");
        if let Some(lock) = lock {
            fs::write(fixture.path().join("package-lock.json"), lock).expect("lock");
        }
        assert!(
            NodeEnvironmentCapability
                .discover(&request(fixture.path(), ".", Vec::new()))
                .is_err()
        );
    }
}

#[test]
fn malformed_runtime_and_tool_declarations_fail_closed() {
    for (manifest, signals) in [
        (r#"{"engines":{"node":22}}"#, Vec::new()),
        (r#"{"engines":">=20"}"#, Vec::new()),
        (
            r#"{"devDependencies":{"vitest":true}}"#,
            vec![SignalKind::Test],
        ),
        (r#"{"devDependencies":[]}"#, vec![SignalKind::Test]),
    ] {
        let fixture = TempDir::new().expect("fixture");
        fs::write(fixture.path().join("package.json"), manifest).expect("manifest");
        fs::write(fixture.path().join("package-lock.json"), "{}").expect("lock");
        assert!(
            NodeEnvironmentCapability
                .discover(&request(fixture.path(), ".", signals))
                .is_err()
        );
    }
}

#[test]
fn target_selector_does_not_hide_malformed_workspace_selector() {
    let fixture = TempDir::new().expect("fixture");
    fs::create_dir_all(fixture.path().join("packages/app")).expect("package");
    fs::write(
        fixture.path().join("package.json"),
        r#"{"workspaces":["packages/*"],"packageManager":"npm@10.0.0"}"#,
    )
    .expect("workspace manifest");
    fs::write(fixture.path().join("package-lock.json"), "{}").expect("lock");
    fs::write(fixture.path().join(".nvmrc"), "\n").expect("workspace selector");
    fs::write(
        fixture.path().join("packages/app/package.json"),
        r#"{"name":"app"}"#,
    )
    .expect("package manifest");
    fs::write(
        fixture.path().join("packages/app/.node-version"),
        "22.14.0\n",
    )
    .expect("target selector");

    assert!(
        NodeEnvironmentCapability
            .discover(&request(fixture.path(), "packages/app", Vec::new()))
            .is_err()
    );
}

#[cfg(unix)]
#[test]
fn source_file_symlink_escape_fails_closed() {
    use std::os::unix::fs::symlink;
    let fixture = TempDir::new().expect("fixture");
    let repository = fixture.path().join("repository");
    fs::create_dir(&repository).expect("repository");
    let outside = fixture.path().join("outside.json");
    fs::write(&outside, r#"{"engines":{"node":"22"}}"#).expect("outside");
    symlink(&outside, repository.join("package.json")).expect("link");
    assert!(
        ayni_adapters_common::environment::environment_discovery_request(
            repository.clone(),
            TargetIdentity::new(Language::Node, ".").expect("target"),
            Vec::new(),
            vec![TargetPlatform {
                os: OperatingSystem::Linux,
                architecture: Architecture::Amd64,
                libc: Libc::Glibc
            }],
        )
        .is_err()
            || NodeEnvironmentCapability
                .discover(&request(&repository, ".", Vec::new()))
                .is_err()
    );
}
