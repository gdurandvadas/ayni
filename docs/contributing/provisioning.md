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
