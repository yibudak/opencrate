# Security policy

## Report a vulnerability

Use [GitHub private vulnerability reporting](https://github.com/yibudak/opencrate/security/advisories/new)
for security issues. Avoid public issues or pull requests that disclose an
unfixed vulnerability. Include the affected version, a minimal reproduction,
expected and observed behavior, and the potential impact. Remove credentials,
personal paths, device serial numbers and other identifying information.

Reports can cover unsafe hardware writes, privilege or service-boundary issues,
untrusted input handling, installer behavior and release integrity. Describe
hardware impact without disabling thermal protection or putting a device at risk.
Maintainers will assess the report and coordinate a fix and disclosure with you.

## Supported versions

Security fixes target the latest release and the current `main` branch. Update
to the latest release when a fix is available. Older versions do not have a
separate maintenance branch.

## Project boundaries

OpenCrate does not bundle ASUS services or drivers. Issues in those components
should also be reported to their vendor. Fan safety checks and restoration do
not replace firmware protection. Releases currently have SHA-256 checksums but
are unsigned; a checksum is not a publisher identity certificate.

Dependency advisories, CodeQL and source privacy checks run in CI. Weekly checks
can surface newly published advisories even when the source has not changed.
Passing automated checks does not prove that a program has no vulnerabilities.
