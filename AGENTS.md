# emmylua-analyzer-rust — Marek's permanent fork

Branch `marek-main-fixes`, kept close to upstream main by small, frequent merges. This file and
`docs/agents/` are fork-only. Open work, history and measurements live in the GitHub tracker:
start at pinned issue #1. Commands: `gndx emmylua --help`, `gndx builds emmylua --help`.

## Policy

- **Reliability first.** A new emmylua hit in our Lua repos (luagnd, myhypr, myvi) gets a Lua-side
  answer first: an annotation, a typed local, a cast. Change the engine only for a root-caused bug.
- **Accept upstream's deliberate design**, even when it flags our code. Look for intent (code comments,
  tests that flipped against 0.25.1) before calling something a bug. Known deliberate: an unconstrained
  generic is any-like in the checker; files are checked serially. (LuaLS `T|?` is NOT on this
  list: its loss was a refactor regression, fixed in 14c7537a, #12.)
- **Coarse and sound beats precise.** Fix at the root with the coarsest rule that is still sound
  (widen by kind, give up to the declared type, `unknown`). Park a fix when it adds a fixpoint or
  re-evaluates bodies, costs > ~5% gate time, or removes ≤ ~3 hits at the price of a new concept.
  Go ahead when it deletes a special case, unifies two paths, or is a one-line change at the root.
- **Chase only clear slowdowns:** large and reproducible in one short run. Record the numbers and
  park the rest (#34).
- **Upstream is Marek's call.** Prepare locally; upstream PRs open as drafts. Mention upstream in
  this tracker as plain text ("upstream #1364").

## Measuring: the three-way table comes first

- Before any verdict ("not an engine bug", "regression", "deliberate"), run the fixture on all
  three builds: 0.25.1, upstream main (`~/.local/opt/emmylua-upstream/current`) and the installed fork.
- After any change to `emmylua_code_analysis`, diff the corpus (`gndx emmylua corpus run` +
  `corpus diff`). Green tests and gates say nothing about what plugin trees report: a
  `resolve_member` change once passed everything and added 71 false hits on neotest.
- Corpus trees are live plugin checkouts: re-run the baseline binary today, never diff against an
  old run. `corpus diff` exit 2 means "not comparable", not "no difference".
- Porting a fork fix upstream: re-measure on upstream main (corpus diff with main as base, and
  every "before" test run against main's source). Fork numbers do not carry over.
- Timings: trust only interleaved `gndx emmylua ab` with resolved ranges; re-run anything marked
  `LOADED`. Other agents load this machine.
- Crash, hang or blowup on a tree: `outcome` on all three builds → `stack` the failing one →
  `gndx emmylua fork` for the fix touching those frames → cherry-pick onto upstream in a scratch
  worktree, re-run `outcome` → `reduce` last.
- Pin an editor fix (completion, hover, gd) as a `myemmylab.FEATURE_PROBES` probe and prove it
  RED on a revert build. For `gd`/hover gaps, fix in `emmylua_ls` first so diagnostics cannot move.

## Building and testing

- Every change takes the gated path: `gndx emmylua test` → `lint` → `gndx builds emmylua install`
  (refuses without a passing gate) → `gndx gate all` + `gndx nvim dev check emmylua` → commit.
  What is installed: `gndx builds emmylua status`.
- **One target dir per worktree.** A shared `CARGO_TARGET_DIR` silently runs the previous
  worktree's test binary. After a run, grep the log for one of the branch's new test names.
- **Stage explicit paths.** Workspace `cargo test` rewrites
  `crates/emmylua_code_analysis/resources/std-zh_CN/`; restore it with `git checkout --` and check
  `git show --stat` before pushing.
- In tests, `std-zh_CN/*.lua` loads as MAIN-workspace code, so std definitions count as user writes.
  When a test and `emmylua_check` disagree on one snippet, suspect this before the engine.
- `lint` verdicts depend on the toolchain: keep `rustup update stable` level with upstream CI.
- Keep a current upstream main build: `gndx builds emmylua upstream --fetch` whenever origin/main
  moves. Its worktree `upstream-main` and target stay put.

## Merging upstream

`gndx emmylua sync plan` is the routine. When upstream rewrites a file the fork patched: take
upstream's file whole (`git checkout --theirs`), then replay each fork-only commit touching it,
oldest first (`git show C -- FILE | git apply -3`), led by the fork's failing tests. Behaviour added
inside merge-commit resolutions is in no replayable commit: diff the old fork tip's file for it. A
fork test that fails only because upstream deliberately changed its reporting gets adapted, with
the reason stated in the test.

## Memory safety (the session can die)

A lab run grinding at its memory cap gets **Hyprland** killed by systemd-oomd, not the lab. The lab
enforces headroom and `lab.BLOWUPS` (never upstream/0.25.1 over mason.nvim). Outside the lab, check
`free -g` first and exclude the trees #35 lists as blowups.

## Engine notes

- `rowan` `token_at_offset` scans children linearly: a root lookup per read is O(n²). Use
  `covering_element` (as in the fork's `flow::token_right_of`).
- `collect_named_indexes` marks a query incomplete for an undeclared name, which sweeps the query
  caches. Calling it per read is costly (#34).

## Layout

Main checkout `~/code/rust/emmylua-analyzer-rust` on `main` (upstream); this worktree
`~/code/rust/emmylua-analyzer-rust-worktrees/main`; topic and scratch worktrees beside it, each
with its own `target/`.

## Agent skills

### Issue tracker

GitHub issues on the fork mareklangiewicz/emmylua-analyzer-rust (always pass `--repo`). See `docs/agents/issue-tracker.md`.

### Triage labels

Default five: needs-triage, needs-info, ready-for-agent, ready-for-human, wontfix. See `docs/agents/triage-labels.md`.

### Domain docs

Single-context: root `GLOSSARY.md` + `docs/adr/`, created lazily. See `docs/agents/domain.md`.
