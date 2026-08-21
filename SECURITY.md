# Security Policy

## Supported versions

Ayni is currently pre-1.0. Security fixes are made on `main` and included in
the next supported release. Only the latest published release line receives
security fixes; older releases and development snapshots do not receive
guaranteed backports.

## Reporting a vulnerability

Do not include exploit details, secrets, or sensitive repository data in a
public issue or discussion. If GitHub shows **Report a vulnerability** on the
repository's Security page, use that private form. Otherwise, open a minimal
public issue that asks the maintainers to establish a private contact channel;
do not describe the vulnerability in that issue. The project must not treat
the direct advisory URL as private-reporting support unless that repository
setting is enabled.

Include, where possible:

- the affected Ayni version, operating system, and container engine;
- the commands and configuration needed to reproduce the issue;
- the expected and observed security boundary;
- the practical impact and any known prerequisites;
- a minimal reproduction that contains no credentials or third-party data; and
- whether the issue is already public or subject to a disclosure deadline.

Maintainers will acknowledge the report, validate its scope, coordinate a fix
and release where appropriate, and agree on disclosure timing with the
reporter. Response and remediation time depend on severity and maintainer
availability; this project does not currently promise a fixed service-level
agreement.

## Scope

Reports about escaping Ayni's documented execution boundaries, bypassing lock
validation, leaking host or repository secrets, unsafe artifact handling, or
compromising the published release path are in scope. The normative execution
and trust assumptions are documented in the
[security and trust model](docs/product/security.md).

Behavior explicitly documented as an intentional trust transition—such as
running directly on the host, mounting the checkout read-write for development,
or granting access to a container daemon—is not by itself a vulnerability.
Undocumented escalation beyond the selected mode remains in scope.

## Good-faith research

Keep testing limited to systems and data you are authorized to use. Avoid
privacy violations, destructive actions, persistence, denial of service, and
access to other users' data. Stop when sensitive data is encountered, preserve
only the minimum evidence needed for the report, and allow a reasonable period
for coordinated remediation before disclosure.
