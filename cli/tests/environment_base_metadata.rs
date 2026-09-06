use std::fs;
use std::path::Path;

#[test]
fn published_base_metadata_matches_the_backend_contract() {
    let repository = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let versions =
        fs::read_to_string(repository.join(".github/docker/provisioning.versions")).unwrap();
    assert!(versions.contains(&format!(
        "MISE_VERSION={}",
        ayni_environment::BASE_MISE_VERSION
    )));
    let dockerfile =
        fs::read_to_string(repository.join(".github/docker/ayni-env.Dockerfile")).unwrap();
    assert!(dockerfile.contains(&format!(
        "dev.ayni.executor.lock-schema=\"{}\"",
        ayni_core::ENVIRONMENT_LOCK_SCHEMA_VERSION
    )));
    assert!(dockerfile.contains(&format!(
        "dev.ayni.executor.recipe=\"{}\"",
        ayni_core::ENVIRONMENT_LOCK_RECIPE_VERSION
    )));
    assert!(dockerfile.contains("USER 10001:10001"));
    assert!(dockerfile.contains("ENTRYPOINT [\"ayni\"]"));
    assert!(dockerfile.contains("sha256sum --check --strict"));
    for runtime in [
        "RUN rustup",
        " nodejs ",
        " python3 ",
        " golang ",
        " openjdk",
    ] {
        assert!(
            !dockerfile.contains(runtime),
            "base unexpectedly installs {runtime}"
        );
    }
}

#[test]
fn candidate_executor_metadata_matches_the_backend_contract() {
    let repository = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let dockerfile =
        fs::read_to_string(repository.join(".github/docker/ayni-candidate.Dockerfile")).unwrap();
    assert!(dockerfile.contains(&format!(
        "dev.ayni.executor.lock-schema=\"{}\"",
        ayni_core::ENVIRONMENT_LOCK_SCHEMA_VERSION
    )));
    assert!(dockerfile.contains(&format!(
        "dev.ayni.executor.recipe=\"{}\"",
        ayni_core::ENVIRONMENT_LOCK_RECIPE_VERSION
    )));
    assert!(dockerfile.contains("COPY --chmod=0755 ayni /usr/local/bin/ayni"));
    assert!(dockerfile.contains("COPY LICENSE NOTICE /usr/share/doc/ayni/"));
    assert!(dockerfile.contains("ENTRYPOINT [\"ayni\"]"));
}
