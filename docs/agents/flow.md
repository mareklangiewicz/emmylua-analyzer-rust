# The member flow model (`semantic_model/flow.rs`)

How a member read like `t.x` gets its flow-sensitive type, and what each piece assumes. Read this
before fixing a narrowing bug (need-check-nil, unnecessary-if, a lost guard, a wrong value after a
write), then place the bug in it: name the piece and the assumption it breaks. Written for #43;
line numbers are at fork `4f768591`, "main" means upstream main `41366714`.

## 1. The read layer picks a subject (`type_of_expr_at_node`, flow.rs:987)

An `IndexExpr` read is not one query. The read layer (IndexExpr arm, flow.rs:999–1213) picks
**one member identity** and traces it, then may replace the result from other sources:

| step | source | where |
|---|---|---|
| a | `resolve_member(t.x).member_id`, else `narrowed_member_id` (first `member_infos_with_key` id of the prefix's type, non-union prefixes only) | 1006, 3531 |
| b | trace that id: `trace_member(id, access = Some(t.x))` when the file has conditions or complex control, else `type_of_member_access_at` | 1012–1028 |
| c | the resolved id is a runtime member and the prefix is a declared `Ref/Def/Generic`: the **declared field** of the prefix wins, nil dropped when the trace is more specific | 1065–1090 |
| d | the resolved id is **not** a runtime member, and some runtime member of the owner with that name has a write before `offset` anywhere (`latest_member_assignment`, by position, not by path): **trace that runtime member** with `type_of_member_at`, i.e. `access = None` | 1118–1147 |
| e | the prefix's type projects a different, more specific member: that wins | 1150–1188 |
| f | no member id at all: the VM's flow-insensitive type (`type_of_expr_node`) | 1199 |

Every exit passes through `guarded_index_type` (the `guarded` closure, fork 8f94b3fe; on main only
the tail does).

Assumptions:
- **A1.** One access has one identity. It doesn't: a class field, one runtime member *per write
  site* (`s.desc = "x"` and `s.desc = nil` are two ids), and an access path `(decl, keys)`. Steps
  b–e each trace or project a different one, and the first that answers wins.
- **A2.** A runtime member that was written *somewhere before* the read (step d) holds the read's
  value. Position is not path: the write may sit on a branch the read's path skips.
- **A3.** A member with no id (step f) is the VM's business. The VM types a missing member of a
  table as `Nil`.

## 2. The subject: what a member trace tracks

`trace_member(model, member, access, tree, flow_id, visited, path)` (flow.rs:3152) tracks one
`SemanticId` (`TraceSubject::Member`, cache.rs:244), plus the optional read expression `access`.

- With `access = Some`, writes are matched both by id and by access path (`member_write_match`,
  flow.rs:2161: `Exact` / `Invalidates` / `ReplacesOwner` / `Unrelated`), and guards are matched
  syntactically (`same_member_access`).
- With `access = None`, only a write whose `FlowEffect::AssignMember.member` **equals the id** is
  seen (`member_write_match` returns `Unrelated` for everything else, 2170–2172).

Assumptions:
- **S1.** `access = None` is safe because the id names every write to the member. False for a
  runtime member: each write site has its own id (see A1).
- **S2.** The entry type (`member_entry_type`, 744) is what the member held before any write on
  the path. With `access = Some` and a `Decl`-owned member, it is the read's flow-insensitive
  type. Otherwise `flow_member_value_type` (692): for a runtime member owned by a `Decl`, that is
  **the member's own written value** (`type_of_member`), a write that hasn't happened on this path.

## 3. Walking back: collecting guards (`trace_member_uncached`, flow.rs:3216)

From the read's flow node, walk antecedents:

- `Assignment` with an `Exact` write: the result is the written value (with a declared field
  annotation: widened when it fits, else the annotation), finalized with the path's **casts
  only**: guards met before the write are dropped (3260–3283). `Invalidates`/`ReplacesOwner`:
  the owner's new field or the member's value type.
- `TrueCondition`/`FalseCondition`: unless `path.ignore_guards` (an assign-target query), push the
  `Narrowing` from `narrow_member_condition` (4903) onto `PathState.narrowings` (2425).
- `TagCast`: push a `CastOp` with the narrowing count when met.
- `Start`/`Unreachable` or no antecedent: the entry type, finalized with the whole path.

`finalize` (3398) applies casts and guards in program order (far to near).

Assumptions:
- **G1.** A guard narrows the member's *value*. Guards that narrow the *owner* by a member
  (`MemberTruthy`, `MemberDiscriminant`) go through `apply_narrowing` on the owner's own trace
  (`filter_type_by_member_truthiness`, 6238).
- **G1b.** Owner narrowing by a member test (`if t.k then`, dropping the components of `t` with
  no `k`) works when `t` is a local, not when it is itself a member (`if a.t.k then`): the
  missing-union-member rule (§6) then reports `a.t.k` as `found nil` inside its own guard.
  Fixture: snacks `notifier.lua:700` reduced to a conditional `a.t = size()` (a type without
  `k`), then `a.t.k = v`, then `if a.t.k then use(a.t.k) end`. Found on branch
  `fix-runtime-member-writes`, where `a.t` first reads as the union. **Fixed**: the member trace
  had no owner arm; `narrow_member_condition` now turns `<access>.k` into `MemberTruthy`, as
  `direct_narrow_condition` does for a local (test `test_member_test_narrows_a_member_owner`).
- **G2.** An `Unreachable` node returns the entry type like `Start` (3314). Unlike `walk_decl`,
  the member walk has no `TraceMode::MergeBranch`, so a branch that ends at `Unreachable` still
  contributes to a merge. Not yet tied to a bug: note it when placing one.

## 4. Merges (`walk_member`, flow.rs:6652)

At a `Multiple` antecedent:
1. If every branch is a straight line back to one common node (`local_merge_common`, 6400) with
   no write to, guard on, or cast of the member (`member_branch_is_pure`, 6586): skip the merge
   and continue at the common node.
2. Else trace each branch with a clone of `visited` and `path`, and `merge_types` the results.
   `None` (cycle, unknown node) is skipped; a `never` branch is not filtered out (the decl walk
   filters it).

Assumptions:
- **M1.** Every branch's answer is about the same subject. True by construction inside one trace,
  but which subject the trace runs on is chosen above it, in the read layer (A1, S1).
- **M2.** Guards distribute over the merge: narrowing each branch, then merging, equals narrowing
  the merge. That is what lets the memo (below) apply guards to a cached merged state.

## 5. The memo (`trace_member` / `trace_decl`, flow.rs:3152 / 2682; `TraceStateKey`, cache.rs:265)

Key: `(file, subject, flow node)`. For members, the subject includes `access` (the read's syntax
id) and `ignore_guards`, so a member entry is reused only by the same read in one query.

- Off for nodes on a cycle (`tree.is_in_cycle`), and when the path holds a `Correlated` narrowing.
- **The memo holds the state for an empty incoming path.** A query arriving with guards or casts
  computes (or fetches) that state, then `finalize`s its own path on top (3171–3184, 2715–2729).
- An entry is `Ready` only if no `InProgress`/`Provisional` entry was read while computing it
  (`is_complete(revision)`); otherwise it is `Provisional`, reused only within the current outer
  query and dropped at its end (`QueryCacheScope`).

On main, the **decl** memo (main flow.rs:2207) has no empty-path split: a hit returns the state
computed with the first query's guards, and a later query's guards are lost. That is upstream
1360. The member memo on main already has the split.

Assumptions:
- **C1.** The state for an empty path depends only on `(node, subject)`. It also depends on which
  queries are in progress when it is computed (`InProgress` reads as `None`), which the
  `Provisional` state contains within one outer query.
- **C2.** M2 holds for every `Narrowing` the path can carry. Upstream 1360 lists the variants it
  checked (`narrows_a_merged_state`); `Correlated` is excluded explicitly.

## 6. After the read: checkers that re-derive member facts

**First: does the checker reach the read layer at all?** §1–5 run only under
`type_of_expr_at` (types.rs:1591 → `type_of_expr_at_node`, types.rs:1658). Plain `type_of_expr`
(signature.rs:198) runs the inference VM: a static member read `t.x` there is `IndexMember(Static)`
(vm.rs:528), the prefix's VM type indexed, with no member trace. Only a dynamic key (vm.rs:518) and
a call argument (vm.rs:571) re-enter flow. So instrumenting §1 for a checker on the VM path shows
nothing fire (#49: `unnecessary_if.rs:39` types its condition with `type_of_expr`). At fork
0883e61a, `grep -rn 'type_of_expr_at(' src/check/checker/`: only need_check_nil,
unnecessary_assert, assign_type_mismatch, call_non_callable, undefined_field,
return_type_mismatch and check_return_count call it (some also call `type_of_expr`, so check
the call site); every other checker (unnecessary_if,
param_type_check, cast_type_mismatch, …) reads member values without flow. Grep again before
relying on this list.

- `member_missing_in_union` (access.rs:694), used by `param_type_check.rs:838`: for an
  index-expression argument whose prefix is a union with a component lacking the key, the checker
  reports `found nil` itself. The read's type never contains that `nil`: the read layer takes the
  other components' field type. The fork adds `&& !is_compatible(nil, param)` (upstream 1423).

Assumption:
- **X1.** "A missing member reads as nil" lives in the read type. It doesn't: it lives in two
  other places with two answers (the VM's `Nil` for a plain table, A3; the checker's special case
  for a union), and the read layer gives a third (the present components' type).

## Bug placement (#43 step 3)

| bug | piece, assumption broken | evidence |
|---|---|---|
| #41 conditional member write drops the skip path (main + fork) | §1 step d + §2 S1/S2: the override traces the write site's runtime member with `access = None`; on the path that skips the write, the entry type is that write's own value | instrumented read (fork tip): step b traced the class field and got `string?`; step d replaced it with runtime member `141..145` traced to `String`. Variant `---@field desc? string\|integer` reads `string`, not `string\|integer`, so it isn't step c |
| **new**: straight-line `s.desc = "x"; s.desc = nil` reads `"x"` (main + fork; 0.25.1 `nil`) | same: step d takes the *first* runtime member; with `access = None` the later write (another id) is `Unrelated` | predicted from the #41 mechanism, then confirmed; the reverse order reads `nil` |
| upstream 1360 (decl memo loses a later query's guards) | §5 C1 on main's decl memo: no empty-path split | main `trace_decl` vs fork's; 1360's PR body, oracle build (#44) |
| upstream 1361 (false branch of `res and res.ok` drops `Res`) | §3 G1: owner narrowing by member truthiness asked "can be truthy" on both branches | fixed on the fork in 4c1949d5 (same diff as 1361) |
| #42 `if not t.x then return end` doesn't narrow on main | §1: one of the two guard sources (trace narrowing, `guarded_index_type`) misses this shape on main. Candidate fork fix: 8f94b3fe ("every exit passes through `guarded_index_type`"). G2 (an `Unreachable` branch still merges) is the other candidate | **not verified**: the commit search stays parked until a fix is chosen (#43 step 4) |
| `self.hunk_cache may be nil` (diffs.nvim `runtime/cache.lua`) | §1 step f + A3/X1, **not** a non-optional field: `self` is the un-annotated `local Cache = {}`, not `diffs.Cache`. `resolve_member` finds owner `Decl(Cache)` and no member id; the VM types the missing member `Nil` | 28-line reduction (`---@class diffs.Cache` beside a bare `local Cache`, constructor `setmetatable({ hunk_cache = {} }, Cache)`) reports on main and fork, clean on 0.25.1; instrumented read: `resolved=(None, Decl(Cache))`, `vm-fallback ty=Nil`. Without the constructor the fork reports `undefined-field` only |
| missing union member rule (upstream 1423, #40) | §6 X1: the nil is added by the checker, not by the read | access.rs:694, param_type_check.rs:838 |
| its range on a colon call (which-key `mappings.lua:258:39`) | not flow: `check_arg_pairs` pairs a colon call's receiver with the first argument's range | not traced in this pass |
| #9 which failing constructor field gets reported flips | doesn't fit the flow walk: files are checked serially, so a flip needs per-process order. Leads: hashbrown containers (randomly seeded) in member collection for the table-vs-class check (`type_check/simple_type.rs`, `member_infos`); C1's order dependence would not flip between runs on its own | not traced |

## Fix ranking (#43 step 4)

1. **Member identity (A1, S1, S2): #41, the straight-line write bug, and likely part of the
   which-key `unnecessary-if` lines in upstream 1360's corpus diff.** The coarsest sound rule: the
   step-d override traces with `access = Some(index_expr)` (`type_of_member_access_at`), so writes
   through any id are seen by access path, and the entry type is the declared field. One-line
   change at the root; check whether steps c–e then still add anything. Highest value: wrong
   values in straight-line code, on main and the fork.
2. **The memo's empty-path state (C1/C2): upstream 1360.** Fixed on the fork; upstream PR open.
   Nothing new to do.
3. **Where "missing member ⇒ nil" lives (A3, X1): the diffs.nvim hits, upstream 1423.** Unifying
   it (the read's type carries the `nil`, or neither does) deletes the checker's special case but
   moves many hits; corpus-measure before choosing. Park until 1 is done.
4. **Guard sources (G1, G2): #42, 1361.** 1361 is fixed on the fork. #42: test 8f94b3fe on main
   first; if it doesn't fix it, look at G2.
5. **Outside flow: #9, the colon-call range.** Separate issues.
