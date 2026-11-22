# Where a diagnostic is decided

Per diagnostic code, the hand-offs between files that grep alone doesn't show: which function
decides *whether* a hit is reported, and which decides *where* and with *what message*. Paths are
under `crates/emmylua_code_analysis/src/`. Add a code here when a session had to trace it.

## `assign-type-mismatch`

1. `check/checker/assign_type_mismatch.rs`: walks `local`, assignment and `return` statements, and
   table fields with their own doc type (`check_inline_table_fields`). Every value goes through
   `check_value_against`.
2. `check_value_against` first asks `check/checker/check_field.rs` `table_literal_mismatches`
   about a table literal value. That walk reports only **nested** literals (a field whose value is
   itself a `{ … }`), plus missing and injected fields. If it reports anything, step 3 is skipped.
3. Otherwise the whole value goes to `is_compatible` / `check_type_at`. On failure,
   `add_mismatch` re-runs `check_type_at` with detail and, given a
   `TypeCheckFailReason::TypeNotMatchAtField`, moves the diagnostic onto that one field's entry.
   So **one** hit per literal, on the first field that fails.
4. That field comes from `semantic_model/type_check/simple_type.rs` `check_table_record`, which
   checks a literal against an object (`{ [1]: T }`) and against a class (`class_fields` turns it
   into a `LuaObjectType` first). It walks the fields in key order (#59: hash order made the line
   move between runs). `check_constructor_value` builds the `TypeNotMatchAtField`.

## `param-type-mismatch` on a table-literal argument

`check/checker/param_type_check.rs` `table_argument_mismatches` reports the whole literal when
`check_type_at(literal, expected)` returns any `Err`, `Unresolved` included. When the field types
look compatible, ask `semantic_model::type_check::check_type_detail(model, actual, expected)` for
the reason first. `Unresolved` comes from an expected type that reaches an undeclared name (an
alias of `Missing|{…}`) or a recursive alias, and is reported on purpose: upstream 1361c218's
recursive-alias tests (`test_recursive_scalar_alias_rejects_table_arguments`) pin it (#84).

## Generic binding from call args (`param-type-mismatch`, `V` left unbound)

`semantic_model/infer/overload.rs` converts each arg before unifying it with the param (a table
literal becomes an array through `table_literal_as_array` only for an `Array` param). Std's own
signatures are the heaviest callers of that path: `ipairs` and `pairs` (`resources/std/global.lua`)
and `table.remove` (`resources/std/table.lua`) take `V[] | table<integer, V> | …` unions. Before a
corpus run of a binding change, grep `resources/std/*.lua` for the param shape it touches: #50's
+33 corpus lines came almost entirely through those three.

## Generic ids (a generic bound or substituted wrong)

A class's doc generics and a method's own doc generics both lower to `Type(index)`, so index
bindings for `C<X, …>` collide with a method's first generics (#71: vim.Iter `fold`'s `A` took
`V1`). Inside a method, the class generics it names are named `Ref`s, substituted by name. A
function field's `generic_params` (`---@field f fun(v: T): T`) also lists the class's generics, so a
"method's own generics" filter has to drop names the class declares: shadowing every listed generic
inside `unify::substitute` broke 11 tests. 99282893 filters in `semantic_model/member.rs` instead.

## Argument/param alignment (which param an argument meets)

A colon call and a colon definition each shift the receiver, so argument `i` meets a different
param per cell. 0.25.1 aligned the call once, up front (`build_call_constraint_context`: it
inserted a `self` param for a dot call to a colon method, or the receiver as argument 0 for a
colon call to a plain function), and plain index pairing was right after that. Upstream's rewrite
(3474c99d) dropped that step, so each site aligns for itself, and a site that pairs by plain index
is wrong in two cells (#75).

The fork's shared rule is `CallAlignment` in `semantic_model/infer/call_alignment.rs`, one variant per cell,
decided once per candidate by `CallAlignment::of(func, colon_call)`:
- `Plain`: argument `i` fills param `i`. `ReceiverIsSelf`: a colon call's receiver fills a first param
  that is `self` (`first_param_is_self`: a param named `self`, `SelfInfer`, or `Ref("self")` from
  `---@param x self`). `ReceiverIsParam0`: a colon call's receiver fills a plain function's param 0
  and is checked against it (`o:f(x)` on `function M.f(a, x)`). `ArgIsReceiver`: a dot call's
  argument 0 is a colon method's receiver and fills no param.
- `param_of(i)` / `param(params, i)`: the param argument `i` fills, or its declared type (`None`
  for `ArgIsReceiver`'s receiver); `arg_of(p)`: the argument that fills param `p`; `receiver_param(params)`: `ReceiverIsParam0`'s param 0, which the
  receiver is checked against; `explicit_params` / `explicit_args`: the two slices a site that pairs
  in order zips.

The sites on the rule: `grep -rn CallAlignment crates/ --exclude='*_test.rs'`. One reads it
differently: `overload.rs` `match_call_candidate_impl` still prepends a `ReceiverIsParam0` receiver
as argument 0 when its type is known, as 0.25.1 did. A new site that pairs arguments with params
reads `CallAlignment`, with a test per cell: a hand-written argument → param map that reused the
param → argument offsets was #76's bug, and in `vm.rs`, #77's.
