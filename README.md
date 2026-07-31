<h1 align="center">Graduate</h1>

**Inspect Jira Cloud from the terminal.**

Graduate is a Jira Cloud CLI and terminal interface. It keeps domain behavior
independent from terminal and network I/O so commands remain predictable and
testable. The `graduate` core owns validated Jira sites, credentials,
identities, and Jira authentication transitions; `graduate-cli` owns external I/O.

## Install

Install the npm bootstrap package:

```bash
npm install --global @treramey/graduate
```

Other supported distribution paths:

```bash
nix run github:treramey/graduate
brew install treramey/tap/graduate
```

Or build directly from this repository:

```bash
cargo install --path crates/graduate-cli
```

## Run

```bash
gd auth setup jira
```

Like Drag, Graduate requires an explicit command. Run `gd --help` to list the
available commands.

## Jira configuration

Run the interactive Jira authentication wizard:

```bash
gd auth setup jira
```

Graduate uses the same Jira Cloud authentication method as Drag: Jira site,
Atlassian account email, and Atlassian API token. The token is masked, verified
with the read-only Jira `/rest/api/3/myself` endpoint, and saved only after the
review screen is confirmed. Existing tokens can be retained without being
loaded into an editable field.

The interactive wizard follows Drag's guided layout: boxed, focusable inputs;
explicit Continue, Connect, and Save actions; Tab and Shift-Tab navigation; and
a final connection manifest. Graduate omits Drag's Tempo authentication step. Set
`GRADUATE_REDUCED_MOTION=1` to replace moving setup effects with short fades.
The setup interface supports terminal panes at least 76 columns by 28 rows.

For unattended setup:

```bash
export ATLASSIAN_HOST=https://yourcompany.atlassian.net
export ATLASSIAN_EMAIL=you@example.com
export ATLASSIAN_TOKEN=...

gd auth setup jira --from-env
```

Preview local validation without networking or saving:

```bash
gd auth setup jira --from-env --dry-run
```

Add `--verify` to the dry-run for an explicit read-only Jira check. The default
configuration path is `~/.graduate/config.json`; override it with `--config` or
`GRADUATE_CONFIG`. Graduate stores connections in a versioned, provider-tagged
configuration so other ticket systems can be added without mixing credential
formats. Graduate reads the original flat Jira configuration and writes the new
schema after the next successful setup.

## Workspace

```text
crates/
├── graduate/      # I/O-independent Jira services and state transitions
└── graduate-cli/  # Clap, Crossterm, Ratatui, process errors, and terminal lifecycle
```

See [`docs/architecture.md`](docs/architecture.md) for the design rules.

## AI agent skills

Graduate includes repository-controlled Agent Skills generated from its CLI
contract:

```bash
cargo run -- generate-skills --force
npx skills add https://github.com/treramey/graduate
```

See the [skills index](docs/skills.md).

## Development

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --locked
```

## License

[MIT](LICENSE)
