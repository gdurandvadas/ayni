use super::*;
use ayni_adapters_common::environment::assert_environment_capability_conformance;
use ayni_core::{Architecture, Libc, OperatingSystem, TargetIdentity, TargetPlatform};
use std::fs;
use tempfile::TempDir;
fn request(root: &Path, signals: Vec<SignalKind>) -> EnvironmentDiscoveryRequest {
    ayni_adapters_common::environment::environment_discovery_request(
        root.to_path_buf(),
        TargetIdentity::new(Language::Python, ".").expect("target"),
        signals,
        vec![TargetPlatform {
            os: OperatingSystem::Linux,
            architecture: Architecture::Amd64,
            libc: Libc::Glibc,
        }],
    )
    .expect("request")
}
fn uv(root: &Path) {
    fs::write(
        root.join("pyproject.toml"),
        "[project]\nname='fixture'\nrequires-python='>=3.12'\n[dependency-groups]\ndev=['pytest==8.3.5', 'pytest-json-report==1.5.0']\n[tool.uv]\nrequired-version='0.6.0'\n",
    )
    .expect("manifest");
    fs::write(root.join(".python-version"), "3.12.4\n").expect("selector");
    fs::write(root.join("uv.lock"),"[[package]]\nname='pytest'\nversion='8.3.5'\n[[package]]\nname='pytest-json-report'\nversion='1.5.0'\n").expect("lock");
}
#[test]
fn locked_uv_is_deterministic_and_conformant() {
    let d = TempDir::new().expect("dir");
    uv(d.path());
    let a = assert_environment_capability_conformance(
        &PythonEnvironmentCapability,
        &request(d.path(), vec![SignalKind::Test]),
    )
    .expect("plan");
    let b = PythonEnvironmentCapability
        .discover(&request(d.path(), vec![SignalKind::Test]))
        .expect("again");
    assert_eq!(a, b);
    assert!(a.conflicts().is_empty());
    assert_eq!(a.target().dependency_locks.len(), 2);
    assert!(a.target().signal_tools.iter().all(|x| x.version.is_exact()));
}
#[test]
fn conflict_and_malformed_inputs_fail_closed() {
    let d = TempDir::new().expect("dir");
    uv(d.path());
    fs::write(d.path().join(".python-version"), "3.11.0\n").expect("selector");
    let c = PythonEnvironmentCapability
        .discover(&request(d.path(), vec![]))
        .expect("conflict");
    assert!(
        c.conflicts()
            .iter()
            .any(|x| x.code == "python_runtime_source_conflict")
    );
    fs::write(d.path().join("uv.lock"), "not = [valid").expect("bad lock");
    assert!(
        PythonEnvironmentCapability
            .discover(&request(d.path(), vec![SignalKind::Test]))
            .is_err()
    );
}
#[test]
fn unsupported_host_manager_is_explicitly_blocked() {
    let d = TempDir::new().expect("dir");
    fs::write(d.path().join("pyproject.toml"), "[project]\nname='x'\n").expect("manifest");
    fs::write(d.path().join("poetry.lock"), "x").expect("lock");
    let c = PythonEnvironmentCapability
        .discover(&request(d.path(), vec![]))
        .expect("plan");
    assert!(
        c.conflicts()
            .iter()
            .any(|x| x.code == "python_managed_environment_unsupported")
    );
    assert!(matches!(
        c.target()
            .package_manager
            .as_ref()
            .expect("manager")
            .version,
        VersionRequirement::Unresolved { .. }
    ));
}

#[test]
fn uv_workspace_requires_membership_and_signal_tool_declaration() {
    let d = TempDir::new().expect("dir");
    fs::write(
        d.path().join("pyproject.toml"),
        "[tool.uv.workspace]\nmembers=['packages/*']\nexclude=['packages/ignored']\n[tool.uv]\nrequired-version='0.6.0'\n",
    )
    .expect("workspace manifest");
    fs::write(d.path().join("uv.lock"), "version=1\n").expect("lock");
    fs::create_dir_all(d.path().join("packages/ignored")).expect("target");
    fs::write(
        d.path().join("packages/ignored/pyproject.toml"),
        "[project]\nname='ignored'\nrequires-python='>=3.12'\n",
    )
    .expect("target manifest");
    let contribution = PythonEnvironmentCapability
        .discover(
            &ayni_adapters_common::environment::environment_discovery_request(
                d.path().to_path_buf(),
                TargetIdentity::new(Language::Python, "packages/ignored").expect("target"),
                [],
                vec![TargetPlatform {
                    os: OperatingSystem::Linux,
                    architecture: Architecture::Amd64,
                    libc: Libc::Glibc,
                }],
            )
            .expect("request"),
        )
        .expect("plan");
    assert!(contribution.target().workspace.is_none());
    assert!(
        contribution
            .conflicts()
            .iter()
            .any(|conflict| conflict.code == "python_managed_environment_unsupported")
    );

    let declared = TempDir::new().expect("declared");
    uv(declared.path());
    fs::write(
        declared.path().join("pyproject.toml"),
        "[project]\nname='fixture'\nrequires-python='>=3.12'\n[tool.uv]\nrequired-version='0.6.0'\n",
    )
    .expect("manifest without pytest declaration");
    assert!(
        PythonEnvironmentCapability
            .discover(&request(declared.path(), vec![SignalKind::Test]))
            .is_err()
    );
}
