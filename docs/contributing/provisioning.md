# Provisioning publication and CI evidence

Ayni is migrating to one composed repository environment whose durable
provisioning substrate is independent of the executor being tested. The new
`ayni-provisioning` image contains Debian prerequisites, Mise, and the execution
user; it contains no Ayni executable or language-specific runtime.

## Publication and adoption

`.github/docker/provisioning.versions` is the shared source of immutable Debian
and exact Mise inputs. The existing release image sources these same inputs and
keeps its current archive/image contract during migration.

The provisioning workflow runs only from trusted `main`, on provisioning input
changes or explicit manual dispatch. It builds and verifies both native Linux
architectures, scans each image, assembles the two-platform manifest, signs its
immutable digest, and verifies the signature before publishing the recipe tag.
Missing architecture, scan, signature, or publication work fails completion.
Publication is serialized and is not cancelled midway. Dispatching again is the
recovery path; previously adopted immutable digests remain addressable even if
the recipe tag points at a recovered build.

The `provisioning-reference` artifact retains the verified immutable reference
and manifest for 14 days. `environment/provisioning.json` is the authoritative
adopted substrate definition embedded in Ayni. The initial digest was built,
scanned, signed and publicly consumed on both architectures by
[publication run 34032769817](https://github.com/gdurandvadas/ayni/actions/runs/34032769817).
Adopt updates only after the same gates succeed. Never commit a job-local
registry reference as the provisioning base.

Environment lock schema `0.7.0` separates that substrate from executor identity.
CI passes its checkout image to `env build --executor-image`, and compares the
complete committed lock without deleting or substituting any base fields.
Release-lock synchronization uses the checkout CLI and retains the committed
base, requiring two identical regenerations. Older public release images retain their original contracts for tagged-release
recovery. Migrated executors advertise their lock schema and recipe; only their
executable is copied into the final prepared environment.

## Required validation

The stable required `status` check accepts only successful required jobs.
Dependency PRs have no author-based exemption. Until conservative change
classification is introduced, they run the full suite, including Rust, native
managed examples, documentation build/audit, and workflow validation.

Ordinary merges must wait for the required status. The default-branch ruleset
has no standing bypass actors. An emergency requires an explicit temporary
administrator change and restoration of the original protection afterwards.
Never use bypass to hide failed or missing validation.

The Rust dependency audit runs independently from fast correctness checks. Its
exact executable is cached by tool version, platform and build toolchain;
advisory data is fetched on each invocation, not baked into the executable cache.

Managed jobs upload only declared locks, signals, summaries and stage timings
before cleanup, with 14-day retention. `scripts/ci/timed.py` records stage name,
start time, duration and exit code, not command arguments or environment values.
Image build currently combines provisioning and dependency preparation into one
stage; the subsequent build refactor will expose them independently. Measure
elapsed workflow duration and summed job durations separately: parallel setup
duplication consumes runner time without adding the same amount to wall time.

## Shared PR candidates and fixture execution

`scripts/ci/fixtures.json` declares the required native PR platforms and fixtures,
including roots and expected check exits. PRs currently require Linux amd64;
the same candidate and consumer contracts also accept native arm64 runners.
The coordinator publishes `required-work/plan.json` before building candidates.
No labels, authors or uncertain impact classifications reduce this inventory.

Each platform builds the CLI once with the pinned Bookworm Rust builder. The
candidate artifact contains that executable, a Docker archive of its minimal
executor image, checksums, exact commit, version, image identity and platform.
The Docker archive transports the OCI-compatible image without publishing it.
Consumers download only from the current unprivileged PR run, check the complete
inventory before import, verify the image labels and executable bytes, and push
to a loopback-only job-local registry. The immutable local manifest reference is
passed to `env build --executor-image`; it never enters the committed lock.
Candidate compilation caches have a PR-only prefix and publication never reads
them. Cold caches use the same pinned builder, locked dependencies and validation.

Lock consistency, repository self-validation and the five fixture jobs consume
the same candidate. Repository validation and the Rust fixture are independent;
no fixture job compiles its own orchestrator or executor. Compiling a fixture's
own tests inside its managed environment is still part of its quality contract.
Classic tests, MSRV and generated documentation remain separate contracts. PR
validation uses read-only permissions throughout; check output and retained logs
provide the summary instead of a privileged PR-comment step.

`scripts/ci/run_fixture.py` takes an explicit CLI path, executor identity, fixture
root, expected roots/outcome and artifact directory. Both candidate validation
and publicly installed release validation call it. Artifact interpretation stays
in the existing strict fixture validator; the runner adds lifecycle orchestration
and verifies the signals digest against execution provenance. Recovery of tags
predating this helper uses their original validation path from tagged source.
There are no privileged consumers of PR candidate artifacts.

Every managed consumer retains its effective lock, build record, signals,
execution provenance, stage logs, timings and a checksummed receipt for 14 days.
Final `status` requires every declared job to succeed and independently checks
all receipts, source/platform identities, signal/build/lock bindings and both
complete lock regenerations. A skipped/cancelled job or missing artifact fails
completion even if other jobs succeeded. Fixture contracts intentionally expect
some failing quality signals; their semantic validator must still succeed.

Measure first actionable failure, elapsed run time, summed job time, candidate
build count, transfer/import time and warm/cold cache behavior separately. The
8–9 minute full-PR target is a comparison goal, not a reason to omit coverage.
