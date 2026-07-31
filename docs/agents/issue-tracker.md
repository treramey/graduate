# Issue tracker: GitHub

Issues and product requirements for this repository live as GitHub issues. Use
the `gh` CLI for operations and infer the repository from `git remote -v`.

## Conventions

- Create: `gh issue create --title "..." --body "..."`
- Read: `gh issue view <number> --comments`
- List: `gh issue list --state open --json number,title,body,labels,comments`
- Comment: `gh issue comment <number> --body "..."`
- Label: `gh issue edit <number> --add-label "..."`
- Close: `gh issue close <number> --comment "..."`

GitHub shares one number space across issues and pull requests. Resolve an
ambiguous number with `gh pr view <number>` and fall back to
`gh issue view <number>`.

Pull requests are not a feature-request surface.
