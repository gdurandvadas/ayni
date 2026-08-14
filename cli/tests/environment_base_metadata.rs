use std::fs;
use std::path::Path;

#[test]
fn published_base_metadata_matches_the_backend_contract() {
    let repository = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let versions = fs::read_to_string(repository.join(".github/docker/ayni-env.versions")).unwrap();
    assert!(versions.contains(&format!(
        "MISE_VERSION={}",
        ayni_environment::BASE_MISE_VERSION
    )));
    let dockerfile =
        fs::read_to_string(repository.join(".github/docker/ayni-env.Dockerfile")).unwrap();
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
