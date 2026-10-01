# Issue tracker: GitHub

Issues and specs live in GitHub Issues for `miran248/terra`.
Use the `gh` CLI from the repository root; outside the clone, pass
`--repo miran248/terra` to issue commands.

## Operations

- Create: `gh issue create --title "Title" --body-file <path>`
- Read: `gh issue view <number> --comments`; use `--json title,body,labels,comments` for structured data.
- List: `gh issue list --state open`; filter with `--label <label>`.
- Edit: `gh issue edit <number> --body-file <path>`
- Comment: `gh issue comment <number> --body-file <path>`
- Assign: `gh issue edit <number> --add-assignee @me`
- Close: `gh issue close <number>`
- Reopen: `gh issue reopen <number>`
- Add a label: `gh issue edit <number> --add-label <label>`
- Remove a label: `gh issue edit <number> --remove-label <label>`

Write multiline bodies to a temporary file and pass it with `--body-file`.
Use issue numbers or GitHub issue URLs in references.

## Triage labels

Use the mapping in `docs/agents/triage-labels.md`.
Preserve unrelated labels when changing a triage label.

## Pull requests as a triage surface

**PRs as a request surface: no.**

## Skill terminology

“Publish to the issue tracker” means create a GitHub issue.
“Fetch the relevant ticket” means run `gh issue view <number> --comments`.

## Wayfinding operations

- A map is a GitHub issue labelled `wayfinder:map`. Decision tickets are native
  sub-issues labelled `wayfinder:grilling`, `wayfinder:prototype`,
  `wayfinder:research`, or `wayfinder:task`.
- Fetch a ticket's database ID with
  `gh api repos/miran248/terra/issues/<number> --jq .id`.
- Attach a child with
  `gh api --method POST repos/miran248/terra/issues/<map-number>/sub_issues -F sub_issue_id=<child-database-id>`.
- Add a blocker with
  `gh api --method POST repos/miran248/terra/issues/<ticket-number>/dependencies/blocked_by -F issue_id=<blocker-database-id>`.
  These IDs are database IDs, not issue numbers.
- List map children through
  `gh api --paginate repos/miran248/terra/issues/<map-number>/sub_issues`.
  The frontier consists of open, unassigned children whose
  `issue_dependencies_summary.blocked_by` is zero, in map order.
- Claim the selected ticket before working it:
  `gh issue edit <number> --add-assignee @me`.
- Resolve by posting the answer as a comment, closing the ticket, and adding a
  linked-title summary to the map's Decisions so far. Keep the full answer in
  the ticket rather than copying it into the map.
- Use native relationships. If the repository does not support them, record
  children as a map task list and blockers as linked titles in ticket bodies.

The domain-model planning map is [Plan Terra domain-model alignment](https://github.com/miran248/terra/issues/1).
