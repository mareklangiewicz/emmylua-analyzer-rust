# emmylua-analyzer-rust — Marek's permanent fork

Branch `marek-main-fixes`, kept close to upstream main by small, frequent merges. This file and
`docs/agents/` are fork-only. Open work, history and measurements live in the GitHub tracker:
start at pinned issue #1. Commands: `gndx emmylua --help`, `gndx builds emmylua --help`.

## Policy

- **Reliability first.** A new emmylua hit in our Lua repos (luagnd, myhypr, myvi) gets a Lua-side
  answer first: an annotation, a typed local, a cast. Change the engine only for a root-caused bug.
- **Accept upstream's deliberate design**, even when it flags our code. Intent is evidence in
  upstream's own code (a comment, a test that flipped against 0.25.1); a guess or a commit title is
  not. Each known-deliberate entry names its evidence; an entry without it belongs in an issue:
  - an unconstrained generic is any-like in the checker (`undefined_field.rs` header, 3474c99d);
  - files are checked serially (`emmylua_check/src/lib.rs` "concurrent diagnose calls … not safe", #34).
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
  On the corpus, the third leg is a same-day run tagged per line:
  `gndx emmylua corpus run v0251-<date> ~/.local/opt/emmylua-0.25.1/emmylua_check`, then
  `corpus diff A B --against v0251-<date>` (`agrees` = 0.25.1 reports B's count of that hit).
- After any change to `emmylua_code_analysis`, diff the corpus. Green tests and gates say nothing
  about what plugin trees report: a `resolve_member` change once passed everything and added 71
  false hits on neotest. Judge an engine experiment before the gated install:
  `gndx emmylua build .` (uncommitted changes included) → `corpus run LABEL <its emmylua_check>` →
  `corpus diff BASE LABEL`. Install only what the diff keeps.
- A fixture is evidence for its own shape only. NOT RED on a build (`test --red-revert`, `probe`)
  says that shape passes there, not that the class is absent: the corpus diff decides. For #41,
  `--red-revert` was NOT RED on the fork while the fork corpus held all 146 false hits.
- Spend CPU on the narrow loop. Reduce each added corpus line to a fixture, iterate with
  `gndx emmylua test --filter=…` and `corpus run LABEL BIN --trees=a,b` (diffed against today's full
  run), and run the full corpus and the full test suite once, before the commit.
- `docs/agents/measuring.md`, one section per path: a corpus base or diff (stale base, exit 2,
  `--exact`, `+dirty`); a `probe` fixture (flags, mapping a class, library form); mutation
  evidence (`test --red-patch`); a `resources/std` edit; timings; a crash, hang or blowup; an
  editor fix.
- **Porting a fork fix upstream:** read `measuring.md` § Porting before the first git command. The
  code moves by `git diff A^ B -- F | git apply -3`, one file at a time, into a fresh worktree off
  `origin/main`, so conflict resolutions already made survive (#89).

## Building and testing

- Every change takes the gated path: `gndx emmylua test` → commit → `lint` →
  `gndx builds emmylua install --with-gate` → `gndx gate all` + `gndx nvim dev check emmylua`.
  `lint` and `install` judge the clean committed HEAD (a dirty tree gets no ledger verdict), and
  `--with-gate` runs `gndx emmylua gate` on exactly the built binaries, installing only on a pass.
  A gate failure means amending or a fix-up commit, then lint and install again.
  What is installed: `gndx builds emmylua status`.
- **One target dir per worktree.** A shared `CARGO_TARGET_DIR` silently runs the previous
  worktree's test binary. A passing `gndx emmylua test` prints no test names: confirm with
  `<target>/debug/build/emmylua_code_analysis/*/out/emmylua_code_analysis-* --list | grep NAME`.
- **Stage explicit paths.** Workspace `cargo test` rewrites
  `crates/emmylua_code_analysis/resources/std-zh_CN/`; restore it with `git checkout --` and check
  `git show --stat` before pushing.
- In tests, `std-zh_CN/*.lua` loads as MAIN-workspace code, so std definitions count as user writes.
  When a test and `emmylua_check` disagree on one snippet, suspect this before the engine.
- `check_source` loads no std, so a call to std there checks against nothing and passes. A test
  whose source calls std uses `check_source_with_std`. Tests load `std/` and `std-zh_CN/` both: a
  std edit goes in both twins, and a narrowed red run reverts both
  (`--red-revert=C:…/std/F,…/std-zh_CN/F`; the lab warns when one is left out).
- `lint` verdicts depend on the toolchain: keep `rustup update stable` level with upstream CI.
- Keep a current upstream main build: `gndx builds emmylua upstream --fetch` whenever origin/main
  moves. Its worktree `upstream-main` and target stay put.

## Merging upstream

`gndx emmylua sync plan` is the routine. When upstream rewrites a file the fork patched lightly: take
upstream's file whole (`git checkout --theirs`), then replay each fork-only commit touching it,
oldest first (`git show C -- FILE | git apply -3`), led by the fork's failing tests. When the fork's
side of the file is the larger one (`flow.rs`, `vm.rs`), resolve hunk by hunk with
`git checkout --conflict=diff3`. Behaviour added inside merge-commit resolutions is in no replayable
commit: diff the old fork tip's file for it, and give new resolution code a test of its own. A
fork test that fails only because upstream deliberately changed its reporting gets adapted, with
the reason stated in the test.

## Memory safety (the session can die)

A lab run grinding at its memory cap gets **Hyprland** killed by systemd-oomd, not the lab. The lab
enforces headroom and `lab.BLOWUPS` (never upstream/0.25.1 over mason.nvim). Outside the lab, check
`free -g` first and exclude the trees #35 lists as blowups.

## Engine notes

- A flow or narrowing bug (need-check-nil, unnecessary-if, a lost guard, a wrong value after a
  member write, a local widened in a loop or a closure) starts with `docs/agents/flow.md`: place the bug in it (the piece and the
  assumption it breaks) before fixing a symptom.
- A diagnostic reported on the wrong line, with the wrong message, or not at all, or a generic
  bound or substituted wrong: start with `docs/agents/checkers.md`, which names the function that
  decides each code's hits and how generic ids collide.
- `rowan` `token_at_offset` scans children linearly: a root lookup per read is O(n²). Use
  `covering_element` (as in the fork's `flow::token_right_of`).
- `collect_named_indexes` marks a query incomplete for an undeclared name, which sweeps the query
  caches. Calling it per read is costly (#34).

## Layout

Main checkout `~/code/rust/emmylua-analyzer-rust` on `main` (upstream); this worktree
`~/code/rust/emmylua-analyzer-rust-worktrees/main`; topic and scratch worktrees beside it, each
with its own `target/`. Remotes: `marek` = this fork, `origin` = upstream. An upstream-PR branch
starts with no tracking, so a bare `git push` goes nowhere; its first push is
`git push -u marek <branch>`.

## Agent skills

### Issue tracker

GitHub issues on the fork mareklangiewicz/emmylua-analyzer-rust (always pass `--repo`). See `docs/agents/issue-tracker.md`.

### Triage labels

Default five: needs-triage, needs-info, ready-for-agent, ready-for-human, wontfix. See `docs/agents/triage-labels.md`.

### Domain docs

Single-context: root `GLOSSARY.md` + `docs/adr/`, created lazily. See `docs/agents/domain.md`.
