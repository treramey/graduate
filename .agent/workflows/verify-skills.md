---
description: Verify all generated skills against Graduate's actual CLI output
---

# Verify Skills

Ensure every `skills/*/SKILL.md` file is accurate, reproducible, and optimized
for AI agent consumption.

## Steps

1. **Read the repository contract**

Read `AGENTS.md`, `CONTEXT.md`, and `docs/architecture.md` before changing the
generator or generated output.

2. **List all generated skill files**

```bash
find skills -name SKILL.md | sort
```

3. **Build Graduate**

// turbo
```bash
cargo build --workspace --locked
```

4. **Capture the public command contract**

// turbo
```bash
./target/debug/gd --help 2>&1
./target/debug/gd auth --help 2>&1
./target/debug/gd auth setup --help 2>&1
./target/debug/gd auth setup jira --help 2>&1
./target/debug/gd diff --help 2>&1
./target/debug/gd restack --help 2>&1
./target/debug/gd generate-skills --help 2>&1
```

5. **For each `SKILL.md`, verify the following against `--help` output and the
   implementation:**

   - [ ] Command and subcommand names match exactly
   - [ ] Positional arguments and flags use valid syntax
   - [ ] Defaults and allowed values are accurate
   - [ ] Environment variable names are accurate
   - [ ] Examples are safe and executable
   - [ ] Interactive-only commands are identified
   - [ ] Secrets are never printed, copied, or placed on command lines
   - [ ] Guidance prefers structured output for agent automation
   - [ ] The generated command-contract block matches `gd --help`

6. **Cross-check `docs/skills.md`**

   - [ ] Every generated skill is indexed
   - [ ] Links point to existing `SKILL.md` files
   - [ ] Descriptions match the generated skills
   - [ ] The generated-file warning is present

7. **Fix the generator, not generated files**

Update `crates/graduate-cli/src/generate_skills.rs` and its tests. Do not edit
files under `skills/` or `docs/skills.md` directly.

Public CLI behavior changes also require tests, README updates, a
`CHANGELOG.md` entry, and a changeset as described in `AGENTS.md`.

8. **Regenerate repository-controlled skills**

// turbo
```bash
cargo run --locked -- generate-skills --force
```

9. **Validate reproducibility and skill format**

// turbo
```bash
git diff --exit-code -- skills/graduate docs/skills.md
```

```bash
failed=0
for skill_dir in skills/*/; do
  if ! uvx --from skills-ref@0.1.1 agentskills validate "$skill_dir"; then
    failed=1
  fi
done
exit "$failed"
```

10. **Run the relevant test suite**

// turbo
```bash
cargo test --workspace --locked
```
