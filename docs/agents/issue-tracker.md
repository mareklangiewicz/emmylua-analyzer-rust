# Issue tracker: GitHub

Issues and specs for this fork live as GitHub issues on **mareklangiewicz/emmylua-analyzer-rust**
(the permanent fork; branch `marek-main-fixes`). Use the `gh` CLI for all operations.

## This fork

- **Pass `--repo mareklangiewicz/emmylua-analyzer-rust` on every `gh` call.** This clone has two
  remotes (`marek` = fork, `origin` = upstream EmmyLuaLs) and no `gh` default, so a bare
  `gh issue …` can land on upstream. Every command below assumes `R=mareklangiewicz/emmylua-analyzer-rust`.
- Scope: fork engine work, flagged engine quirks, and the `gndx emmylua` lab, whose code lives in
  luagnd (`~/code/lua/luagnd`). A lab issue says so in its first line.
- Start at issue #1 ("Start here"), then pick `ready-for-agent`.
- Refer to upstream as plain text ("upstream #1370", "upstream commit 7bf34a75"), never as a link
  or `EmmyLuaLs/emmylua-analyzer-rust#N`: a link shows on their thread, which is interaction.
  Upstream issues/PRs are Marek's call and are opened as drafts; they are not filed from here.
- `gh` on myxps is 2.83: no `--parent` / `--add-sub-issue`. Use the `sub_issues` API form below.

## Conventions

- **Create an issue**: `gh issue create --repo $R --title "..." --body "..."`. Use a heredoc for multi-line bodies.
- **Read an issue**: `gh issue view <number> --repo $R --json number,title,body,labels,comments`.
- **List issues**: `gh issue list --repo $R --state open --json number,title,body,labels,comments --jq '[.[] | {number, title, body, labels: [.labels[].name], comments: [.comments[].body]}]'` with appropriate `--label` and `--state` filters.
- **Make an issue a sub-issue of a parent**: `gh api --method POST repos/$R/issues/<parent>/sub_issues -F sub_issue_id=<child-db-id>` (database id, as in **Blocking** below). With `gh` 2.94+, `gh issue create --parent <parent> ...` also works. Without sub-issues, put `Part of #<parent>` at the top of the child body.
- **Comment on an issue**: `gh issue comment <number> --repo $R --body "..."`
- **Apply / remove labels**: `gh issue edit <number> --repo $R --add-label "..."` / `--remove-label "..."`
- **Close**: `gh issue close <number> --repo $R --comment "..."`

## Pull requests as a triage surface

**PRs as a request surface: no.** _(Set to `yes` if this repo treats external PRs as feature requests; `/triage` reads this flag.)_

When set to `yes`, PRs run through the same labels and states as issues, using the `gh pr` equivalents:

- **Read a PR**: `gh pr view <number> --repo $R --comments` and `gh pr diff <number> --repo $R` for the diff.
- **List external PRs for triage**: `gh api --paginate "repos/$R/pulls?state=open" --jq '.[] | select(.author_association | IN("OWNER","MEMBER","COLLABORATOR") | not) | {number, title, author: .user.login, author_association, labels: [.labels[].name]}'`.
- **Comment / label / close**: `gh pr comment`, `gh pr edit --add-label`/`--remove-label`, `gh pr close`, each with `--repo $R`.

GitHub shares one number space across issues and PRs, so a bare `#42` may be either: resolve with `gh pr view 42 --repo $R` and fall back to `gh issue view 42 --repo $R`.

## When a skill says "publish to the issue tracker"

Create a GitHub issue on the fork (`--repo $R`).

## When a skill says "fetch the relevant ticket"

Read it as in **Read an issue** above.

## Wayfinding operations

Used by `/wayfinder`. The **map** is a single issue with **child** issues as tickets.

- **Map**: a single issue labelled `wayfinder:map`, holding the Notes / Decisions-so-far / Fog body. `gh issue create --repo $R --label wayfinder:map`.
- **Child ticket**: an issue linked to the map as a GitHub sub-issue (see **Make an issue a sub-issue of a parent**). Where sub-issues aren't enabled, add the child to a task list in the map body and put `Part of #<map>` at the top of the child body. Labels: `wayfinder:<type>` (`research`/`prototype`/`grilling`/`task`). Once claimed, the ticket is assigned to the driving dev.
- **Blocking**: GitHub's **native issue dependencies**, the canonical, UI-visible representation. Add an edge with `gh api --method POST repos/$R/issues/<child>/dependencies/blocked_by -F issue_id=<blocker-db-id>`, where `<blocker-db-id>` is the blocker's numeric **database id** (`gh api repos/$R/issues/<n> --jq .id`, _not_ the `#number` or `node_id`). GitHub reports `issue_dependencies_summary.blocked_by` (open blockers only, the live gate). Where dependencies aren't available, fall back to a `Blocked by: #<n>, #<n>` line at the top of the child body. A ticket is unblocked when every blocker is closed.
- **Frontier query**: list the map's open children (`gh issue list --repo $R --state open`, scoped to the map's sub-issues / task list), drop any with an open blocker (`issue_dependencies_summary.blocked_by > 0`, or an open issue in the `Blocked by` line) or an assignee; first in map order wins.
- **Claim**: `gh issue edit <n> --repo $R --add-assignee @me`, the session's first write.
- **Resolve**: `gh issue comment <n> --repo $R --body "<answer>"`, then `gh issue close <n> --repo $R`, then append a context pointer (gist + link) to the map's Decisions-so-far.
