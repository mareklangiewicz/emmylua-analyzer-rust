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
