# Measuring, per path

The rules every measurement needs (the three-way table, the corpus diff, a fixture's reach, the
narrow loop) are in `AGENTS.md` "Measuring". This file holds the rest, one section per path:
read the section for the path you are on.

## Corpus bases and diffs

- Corpus trees are live plugin checkouts: a base run is valid while no tree moved since it ran,
  whatever its date. `gndx emmylua corpus valid LABEL…` checks that (#67); before diffing,
  `corpus refresh LABEL…` re-runs only a STALE base's moved trees, with its own binary and config.
- `corpus diff` exit 2 means "not comparable", not "no difference".
- A passed `emmylua gate` keeps its corpus step as the base `gate-<commit8>-<MMDD>`: the installed
  build's base, with no second run.
- `corpus diff` pairs hits by `file:line` and folds small line moves, so it reads a change to a
  diagnostic's range as no change. Judge a range or column change with `corpus diff --exact`,
  which keys on the whole range and folds nothing (#73: 0 changed lines by default, 144 +/- pairs
  exact).
- `corpus run` of a `+dirty` build (`gndx emmylua build .` with uncommitted changes) refuses
  mason.nvim, and says so before the first tree: its commit is unknown, so the lab vouches for no
  blowup fix. Diff it against a base on the trees it ran.
- A scratch run goes with `corpus drop LABEL… --apply` (a plan without it); it refuses `gate-*` runs
  and #1's base set.
- Every corpus run keeps its binary at `~/.cache/gndx/emmylua-corpus/<LABEL>/emmylua_check`: probe
  an older rule with `--bin=` on that copy instead of rebuilding it.

## Fixtures and `probe`

- `probe DIR… --bin=CANDIDATE` prints the three-way table (plus the candidate) for any fixture.
  Each row ends with its source line. With several cells on one line, add `--exact` so each row
  carries its `L:C-L:C` range. Parsing the JSON by hand is a sign that `probe` lacks a flag.
- A fixture reduced from a corpus line stands in for that line only until the fix: the fix is
  confirmed when the original FILE:LINE leaves the corpus diff (`corpus diff A B
  --line=FILE:LINE`). #83's reduction kept the message and slid onto another mechanism; its fix
  turned the fixture green while snacks `lsp/init.lua:452` kept firing (#84).
- Before pinning a fixture as a test, map its class: `probe` variants that each change one feature
  (annotated or not, before or after the write, straight-line or conditional), then pin the
  smallest cells that tell the builds apart (#41: a 36-variant grid showed that on main, `---@type`
  on the write decides whether a read before it goes wrong).
- A cause stays a hypothesis until its control goes green: the bug's own cell minus the suspected
  feature, nothing else changed. A control that varies another feature says nothing about the cause
  (#78 blamed the receiver of `M:f()`; its controls `M:g()` and `M.f()` changed the params and the
  argument count, and `M.f(1)`, the bug without a receiver, stayed red).
- `probe` and `reduce` run a fixture alone under an empty config, so a gate hit that needs the Lua
  repo's libraries (snacks, `$VIMRUNTIME`) does not show there: rebuild it in library form, a
  `lib/` dir with the library file and a `ws/.emmyrc.json` naming `../lib` (relative to the
  config's dir), probe it with `--root=ws --config=DIR/ws/.emmyrc.json`, then shrink the library
  file. The same goes for a one-file `VirtualWorkspace` test: a hit that needs the class in another
  file (`ws.def`) or a `@class` name other than its local (`---@class vim.Iter` on `local Iter`)
  passes in one file.

## Mutation evidence for a test

`gndx emmylua test --filter=… --red-patch=FILE.diff` runs the filtered tests on HEAD with a mutant
applied, in the scratch worktree. `gndx emmylua mutant F OLD NEW` writes FILE.diff: F as of HEAD
with the literal OLD (exactly once) replaced by NEW. Each filter must catch it: a NOT RED filter is
a test that cannot see that mutation (`colon_extra = 0` in `param_count.rs` is caught by
`redundant_parameter_test` alone).

A refactor has no red phase: its evidence is a mutant of the refactored code that the EXISTING tests
catch (RED with the patch, all green on HEAD), recorded in the commit message. A new guard test earns
its place only by going RED on some mutant. `--red-patch` refuses a mutant of a file with uncommitted
changes (`mutant` warns), so commit the refactor first, run the mutant, then amend the message
(c0e09ab1: the `V[]` arm put back to bare `numeric_key_expanded` goes RED in
`enum_value_keys_a_table_of_its_base_type`).

## A `resources/std` annotation edit

It goes through the narrow loop too. First `probe` fixtures of the callers' shapes (a callback
with one param, with several, variadic, a std function passed as the argument), then `--trees=` a
few trees that use the symbol heavily. #76's first `repl` spelling went straight to a full run and
added 11 hits (snacks, plenary, sidekick, lspconfig, argparse);
`test_gsub_callback_takes_every_capture` pins the shapes they came from.

## Porting a fork fix upstream

- Re-measure on upstream main (corpus diff with main as base, and every "before" test run against
  main's source: `test --red-at=MAIN:FILE`). Fork numbers do not carry over.
- Before the PR goes ready, land the same rule on the fork and diff the fork corpus too. It covers
  the trees main crashes or blows up on (tuis, neotest, bufferline, async.nvim, neovim-runtime), so
  upstream's diff is blind there: 1447 went ready with a rule the fork corpus then changed twice
  (#41).
- A lowering fix ("spelling X now means Y"): copy a corpus tree, respell X as Y in its
  annotations, run the base build on the copy and the candidate on the original. Equal hits mean
  the new hits are base's own Y behaviour, not the fix (#37: `|?` → `|nil`, 1053 = 1053; 1184 on
  the original is the control).
- Duplicates: search upstream issue titles by words; GitHub search drops punctuation (`|?` finds
  nothing). Find code overlap by grepping open upstream PR diffs (`gh pr diff`) for the changed
  symbol.

## Timings

Trust only interleaved `gndx emmylua ab` with resolved ranges; re-run anything marked `LOADED`.
Other agents load this machine.

## A crash, hang or blowup on a tree

`outcome` on all three builds → `stack` the failing one → `gndx emmylua fork` for the fix touching
those frames → cherry-pick onto upstream in a scratch worktree, re-run `outcome` → `reduce` last.

## An editor fix (completion, hover, gd)

Pin it as a `myemmylab.FEATURE_PROBES` probe and prove it RED on a revert build. For `gd`/hover
gaps, fix in `emmylua_ls` first so diagnostics cannot move.
