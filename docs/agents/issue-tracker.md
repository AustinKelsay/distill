# Issue Tracker: GitHub

Issues and specs for this repository live in GitHub Issues. Use the `gh` CLI for all operations and infer `AustinKelsay/distill` from the repository remote.

## Conventions

- Create: `gh issue create --title "..." --body-file <file>`.
- Read: `gh issue view <number> --comments` and fetch labels.
- List: `gh issue list --state open --json number,title,body,labels,comments` with appropriate filters.
- Comment: `gh issue comment <number> --body-file <file>`.
- Label: `gh issue edit <number> --add-label "..."` or `--remove-label "..."`.
- Close: `gh issue close <number> --comment "..."`.

GitHub shares one number space across issues and pull requests. Resolve an ambiguous `#42` with `gh pr view 42`, falling back to `gh issue view 42`.

## Pull Requests As A Triage Surface

**PRs as a request surface: no.** External pull requests do not automatically enter the issue-triage queue.

## Skill Publishing

When a skill says to publish a spec, ticket, or other issue-tracker artifact, create a GitHub issue in this repository.

## Blocking Relationships

Use GitHub native issue dependencies when available. The dependency endpoint requires the blocker's numeric database ID, not its issue number or node ID:

```text
POST repos/AustinKelsay/distill/issues/<child>/dependencies/blocked_by
issue_id=<blocker database id>
```

If native dependencies are unavailable, place `Blocked by: #<issue>` at the top of the dependent issue body.
