# Security policy

Only the latest release receives security fixes while Graduate is pre-1.0.

Report vulnerabilities with GitHub private vulnerability reporting under
**Security → Report a vulnerability**. Do not open a public issue containing
private data, terminal escape payloads, or exploit details.

Include the affected version, platform, reproduction, impact, and suggested
fix if known. Maintainers will acknowledge reports within seven days and
coordinate disclosure after a patch is available.

## Authentication credential handling

Interactive `gd auth setup jira` masks typed and pasted Atlassian API tokens. Stored
tokens can be retained without loading them into editable terminal fields, and
the review screen contains only non-secret identity and connection state.
Setup verifies Jira with a read-only request before writing configuration.
Cancellation, validation failures, authentication rejection, network errors,
and terminal errors leave existing configuration unchanged.

Graduate creates configuration through an exclusive, same-directory temporary file
and atomically replaces the destination. On Unix, both the temporary and final
files are restricted to the current user. Avoid exposing
`ATLASSIAN_TOKEN` through shell history, process diagnostics, or CI logs. Use
`gd auth setup jira --from-env --dry-run` for local validation without network access
or writes; add `--verify` only when a read-only remote check is intended.
