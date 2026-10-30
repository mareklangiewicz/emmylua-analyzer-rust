#[cfg(test)]
mod test {
    use crate::{DiagnosticCode, VirtualWorkspace};

    #[test]
    fn test_vararg_unknown_and_boolean_conditions_are_not_constant() {
        let diagnostics = crate::check::test::check_source(
            r#"
            local function unknown(...)
                if ... then print('present') end
                assert(...)
            end
            ---@param ... boolean
            local function boolean(...)
                if ... then print('true') end
                assert(...)
            end
            unknown(false)
            boolean(false)
        "#,
        );
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn test_vararg_false_conditions_still_report_impossible_branches() {
        let diagnostics = crate::check::test::check_source(
            r#"
            ---@param ... false
            local function always_false(...)
                if ... then print('unreachable') end
                assert(...)
            end
            always_false(false)
        "#,
        );
        let messages: Vec<_> = diagnostics
            .iter()
            .map(|d| (d.code, d.message.as_str()))
            .collect();
        assert_eq!(
            messages,
            vec![
                (
                    DiagnosticCode::UnnecessaryAssert,
                    "Impossible assert: this expression is always falsy; prefer `error()`"
                ),
                (
                    DiagnosticCode::UnnecessaryIf,
                    "Impossible `if` statement: this condition is always falsy"
                ),
            ]
        );
    }

    #[test]
    fn test_issue_392() {
        let mut ws = VirtualWorkspace::new();
        assert!(ws.has_no_diagnostic(DiagnosticCode::UnnecessaryIf,
        r#"
        local a = false ---@type boolean|nil
        if a == nil or a then -- Unnecessary `if` statement: this condition is always truthy [unnecessary-if]
            print('a is not false')
        end
        "#
        ));
    }

    #[test]
    fn test_issue_396() {
        let mut ws = VirtualWorkspace::new();
        assert!(ws.has_no_diagnostic(DiagnosticCode::UnnecessaryIf,
        r#"
        local a = false ---@type 'a'|'b'
        if a ~= 'a' then -- Unnecessary `if` statement: this condition is always truthy [unnecessary-if]
        end
        "#
        ));
    }
    /// #1319: a declared `---@type` on a member wins over the metatable's `__index` return
    /// (Neovim declares `vim.env` this way).
    #[test]
    fn test_issue_1319_declared_type_wins_over_metatable_index() {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();
        ws.def_file(
            "def.lua",
            r#"
            G = {}

            --- @type table<string, string?>
            G.envA = setmetatable({}, {
              __index = function(_, k)
                local v = os.getenv(k)
                if v == nil then return nil end
                return v
              end,
            })

            --- @type table<string, string?>
            G.envD = setmetatable({}, { __index = function(_, k) return 'always' end })
            "#,
        );
        for member in ["G.envA", "G.envD"] {
            let ty = ws.expr_ty(member);
            assert_eq!(ws.humanize_type(ty), "table<string,string?>", "{member}");
        }
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::UnnecessaryIf,
            r#"
            if G.envA.X then print 'A' end
            if G.envD.X then print 'D' end
            "#
        ));
    }

    /// #1319: `if v == nil then return nil end return v` returns `string?`, not `nil`, also
    /// as an `__index` function and when the inner condition is checked first.
    #[test]
    fn test_issue_1319_guarded_return_keeps_both_paths() {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();
        ws.def_file(
            "def.lua",
            r#"
            F = function(_, k)
              local v = os.getenv(k)
              if v == nil then return nil end
              return v
            end
            ENV = setmetatable({}, { __index = F })
            "#,
        );
        let fun = ws.expr_ty("F");
        assert!(ws.humanize_type(fun).ends_with("-> string?"));
        let field = ws.expr_ty("ENV.X");
        assert_eq!(ws.humanize_type(field), "string?");
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::UnnecessaryIf,
            r#"
            local env = setmetatable({}, {
              __index = function(_, k)
                local v = os.getenv(k)
                if v == nil then return nil end
                return v
              end,
            })
            if env.X then print 'A' end
            "#
        ));
    }

    /// A local captured by a closure that reassigns it is read, inside the closure, as any
    /// value it can hold: `if ended` in a later call can be true.
    #[test]
    fn test_captured_reassigned_local_is_not_its_initializer() {
        let mut ws = VirtualWorkspace::new();
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::UnnecessaryIf,
            r#"
                local function mk()
                    local ended = false
                    local n = 0
                    return function()
                        if ended then return nil end
                        if n > 3 then ended = true end
                        n = n + 1
                        return n
                    end
                end
            "#
        ));
    }

    /// A captured local declared without a value (`local relay`) and set outside the closure
    /// is not just `nil` inside it: `if relay` can be true.
    #[test]
    fn test_captured_nil_local_set_outside_is_not_nil() {
        let mut ws = VirtualWorkspace::new();
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::UnnecessaryIf,
            r#"
                ---@param c boolean
                ---@param load fun(): fun(): integer
                local function mk(c, load)
                    local relay
                    if c then relay = load() end
                    return function()
                        if relay then return relay() end
                        return 0
                    end
                end
            "#
        ));
    }

    /// A captured counter (`i = i + 1` inside the closure) stays an `integer`: a value
    /// computed from the local itself adds nothing to what it can hold.
    #[test]
    fn test_captured_counter_stays_integer() {
        let mut ws = VirtualWorkspace::new();
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::AssignTypeMismatch,
            r#"
                ---@param s any[]
                local function mk(s)
                    local i, n = 0, #s
                    return function()
                        if i >= n then return nil end
                        i = i + 1
                        local v = s[i]
                        if v == nil then i = n end
                        return v
                    end
                end
            "#
        ));
    }

    /// An optional function field (`---@field f? fun()`) keeps its `nil`: `if t.f` can be false.
    #[test]
    fn test_optional_function_field_can_be_nil() {
        let mut ws = VirtualWorkspace::new();
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::UnnecessaryIf,
            r#"
                ---@class OptFunField
                ---@field f? fun()

                ---@param t OptFunField
                local function a(t)
                    if t.f then t.f() end
                end
            "#
        ));
    }

    /// A local written inside a closure is read after a call that may run it: the read is
    /// not just its initializer.
    #[test]
    fn test_local_written_by_closure_is_not_its_initializer() {
        let mut ws = VirtualWorkspace::new();
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::UnnecessaryIf,
            r#"
                ---@param f fun()
                local function run(f) f() end
                local function mk()
                    local ran = false
                    run(function() ran = true end)
                    if ran then return 1 end
                end
            "#
        ));
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::UnnecessaryIf,
            r#"
                ---@param f fun(): string?
                local function run(f) f() end
                local function mk()
                    local cancelled = false
                    run(function()
                        local text = nil ---@type string?
                        cancelled = cancelled or text == nil
                        return text
                    end)
                    if cancelled then return nil end
                end
            "#
        ));
    }

    /// The field resolves through a nullable prefix too (`t: T?`, narrowed): `if t.f` can be false.
    #[test]
    fn test_optional_function_field_of_nullable_prefix_can_be_nil() {
        let mut ws = VirtualWorkspace::new();
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::UnnecessaryIf,
            r#"
                ---@class OptFunField
                ---@field f? fun()

                ---@param t OptFunField?
                local function a(t)
                    if not t then return end
                    if t.f then t.f() end
                end
            "#
        ));
    }

    /// `self` in `function M.E:m()` is the value at `M.E`, not the root local `M`: `self.cause`
    /// reads `E`'s optional field.
    #[test]
    fn test_nested_path_method_self_is_its_prefix() {
        let mut ws = VirtualWorkspace::new();
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::UnnecessaryIf,
            r#"
                ---@class NestedSelf
                ---@field cause NestedSelf?

                local function mk() return {} end
                local M = {}
                M.E = mk() --[[@as NestedSelf]]

                function M.E:str()
                    if self.cause then return 1 end
                end
            "#
        ));
    }

    /// A table built from `{}` by dynamic-key writes (`seen[k] = true`, which record no member)
    /// has no known shape: `seen[k2]` may hold a value.
    #[test]
    fn test_dynamic_key_read_of_empty_table_is_not_nil() {
        let mut ws = VirtualWorkspace::new();
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::UnnecessaryIf,
            r#"
                ---@param k string
                ---@param k2 string
                local function a(k, k2)
                    local seen = {}
                    seen[k] = true
                    if seen[k2] then return 1 end
                end
            "#
        ));
    }

    /// A local annotated with an object type reads its field from that type, not from the
    /// initializer's literal field: `{ to = nil } ---@type { to: fun()? }` can be set later.
    #[test]
    fn test_object_type_field_over_initializer_literal() {
        let mut ws = VirtualWorkspace::new();
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::UnnecessaryIf,
            r#"
                local out = { to = nil } ---@type { to: fun(line: string)? }
                local function write(line)
                    if out.to then out.to(line) end
                end
            "#
        ));
    }

    /// The falsy branch of a member test keeps an owner whose member CAN be falsy: after
    /// `if self.flag then return end` (`flag: boolean`), `self` is still the class, not `never`.
    #[test]
    fn test_member_falsy_branch_keeps_owner_with_boolean_member() {
        let mut ws = VirtualWorkspace::new();
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::UnnecessaryIf,
            r#"
                ---@class FalsyGuard
                ---@field flag boolean
                ---@field thunk (fun())?
                local Job = {}

                function Job:cancel()
                    if self.flag then return end
                    if self.thunk then
                        self.thunk = nil
                    end
                end
            "#
        ));
    }

    /// A param documented with an object type keeps its fields in the body; the shell lowers
    /// `{ since: integer? }` to a bare `table`, so the field read the body's own later write.
    #[test]
    fn test_param_object_doc_type_field() {
        let mut ws = VirtualWorkspace::new();
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::UnnecessaryIf,
            r#"
                ---@param t { since: integer? }
                local function f(t)
                    if t.since then t.since = nil end
                end
            "#
        ));
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::UnnecessaryIf,
            r#"
                ---@param t { since: integer? }
                local function g(t)
                    if t.since then t.since = 1 end
                end
            "#
        ));
        // An optional param keeps its nil.
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::UnnecessaryIf,
            r#"
                ---@param t? { since: integer }
                local function h(t)
                    if t then return t.since end
                end
            "#
        ));
    }

    #[test]
    fn test_equality_against_a_literal_outside_the_union_is_impossible() {
        let mut ws = VirtualWorkspace::new();
        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::UnnecessaryIf,
            r#"
            ---@alias Why 'target'|'ledger-light'
            ---@param why Why
            local function f(why)
                if why == 'ledger-ligth' then return 1 end
            end
            "#
        ));
        // A literal that is in the union, and a non-literal side, keep the branch.
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::UnnecessaryIf,
            r#"
            ---@alias Why2 'target'|'ledger-light'
            ---@param why Why2
            ---@param s string
            local function g(why, s)
                if why == 'ledger-light' then return 1 end
                if why == s then return 2 end
                if why ~= 'ledger-light' then return 3 end
            end
            "#
        ));
        // A local reassigned in a loop: it holds more than its initial literal on later passes.
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::UnnecessaryIf,
            r#"
            ---@param pats string[]
            local function h(pats)
                local num = ''
                for _, p in ipairs(pats) do
                    if num == '' then num = p end
                end
                return num
            end
            "#
        ));
    }

    /// A test that swaps a module's state table wholesale (`sp.state = { active = true }`) from
    /// another file must not replace the member's declared `---@type`: every other reader of
    /// `sp.state.active` still sees `boolean` (myhypr stickysuper.lua vs stickysuper_spec.lua).
    /// Both file orders, so "first member wins" cannot pass by luck of the file ids.
    #[test]
    fn test_declared_member_type_wins_over_cross_file_literal_assign() {
        let module = (
            "sp.lua",
            r#"
            local M = {}
            ---@type { active: boolean, prev: string? }
            M.state = { active = false, prev = nil }
            return M
            "#,
        );
        let spec = (
            "spec.lua",
            r#"
            local sp = require 'sp'
            sp.state = { active = true, prev = nil }
            "#,
        );
        for files in [vec![module, spec], vec![spec, module]] {
            let order: Vec<_> = files.iter().map(|(name, _)| *name).collect();
            let mut ws = VirtualWorkspace::new();
            ws.def_files(files);
            let ty = ws.expr_ty("require('sp').state.active");
            assert_eq!(ws.humanize_type(ty), "boolean", "{order:?}");
            assert!(
                ws.has_no_diagnostic(
                    DiagnosticCode::UnnecessaryIf,
                    r#"
                    local sp = require 'sp'
                    local function leave()
                        if sp.state.active then print 'y' end
                    end
                    "#
                ),
                "{order:?}"
            );
        }
    }
}

#[googletest::gtest]
fn closure_local_from_module_field_keeps_declared_type() {
    use crate::{DiagnosticCode, VirtualWorkspace};
    use googletest::prelude::*;
    let mut ws = VirtualWorkspace::new();
    assert_that!(
        ws.has_no_diagnostic(
            DiagnosticCode::UnnecessaryIf,
            r#"
            ---@type { panel: string? }
            local st = { panel = nil }
            ---@return string? p
            local function get()
              local p = st.panel
              if p then return p end
              return nil
            end
            return get
            "#
        ),
        eq(true)
    );
}

#[googletest::gtest]
fn closure_local_from_annotated_member_table_keeps_declared_type() {
    use crate::{DiagnosticCode, VirtualWorkspace};
    use googletest::prelude::*;
    let mut ws = VirtualWorkspace::new();
    assert_that!(
        ws.has_no_diagnostic(
            DiagnosticCode::UnnecessaryIf,
            r#"
            local M = {}
            ---@type { panel: string? }
            M.state = { panel = nil }
            ---@return string? p
            function M.get()
              local p = M.state.panel
              if p then return p end
              return nil
            end
            return M
            "#
        ),
        eq(true)
    );
}

#[googletest::gtest]
fn rawget_of_a_key_no_member_answers_is_not_always_truthy() {
    use crate::{DiagnosticCode, VirtualWorkspace};
    use googletest::prelude::*;
    let mut ws = VirtualWorkspace::new_with_init_std_lib();
    assert_that!(
        ws.has_no_diagnostic(
            DiagnosticCode::UnnecessaryIf,
            r#"
            ---@type table
            local t = {}
            local v = rawget(t, 'x')
            if v then print(v) end
            ---@param p table
            local function f(p)
              local w = rawget(p, 'x')
              if w then return w end
            end
            return f
            "#
        ),
        eq(true)
    );
}

#[googletest::gtest]
fn a_false_or_boolean_condition_is_not_constant_after_an_if_on_it() {
    use crate::DiagnosticCode;
    // `a and b` of two booleans is `false|boolean`. Truthy narrowing dropped `boolean` (to
    // Unknown), so the merge after `if on then end` kept only the else branch's `false`.
    crate::check::test::assert_clean_cases(
        DiagnosticCode::UnnecessaryIf,
        "---@param a boolean\n---@param b boolean\nlocal function f(a, b)\n  local on = a and b\n",
        &[
            (
                "  if on then print(1) end\n  if on then print(2) end\nend\nreturn f",
                true,
            ),
            (
                "  if not on then print(1) end\n  if on then print(2) end\nend\nreturn f",
                true,
            ),
            (
                "  if on then print(1) end\n  if not on then print(2) end\nend\nreturn f",
                true,
            ),
            // Still constant inside a branch that decided it.
            (
                "  if on then if on then print(1) end end\nend\nreturn f",
                false,
            ),
        ],
    );
}

#[googletest::gtest]
fn truthiness_narrows_boolean_to_true_and_false() {
    use googletest::prelude::*;
    let diagnostics = crate::check::test::check_source(
        r#"
        ---@param on boolean
        local function f(on)
            if on then
                ---@type false
                local _t = on
            else
                ---@type true
                local _e = on
            end
        end
        return f
        "#,
    );
    let messages: Vec<_> = diagnostics
        .iter()
        .filter(|d| d.code == crate::DiagnosticCode::AssignTypeMismatch)
        .map(|d| d.message.lines().next().unwrap_or_default().to_string())
        .collect();
    assert_that!(
        messages,
        elements_are![
            eq("expected `false` but found `true`."),
            eq("expected `true` but found `false`.")
        ]
    );
}

/// Loops have no back-edge in the flow graph: a read inside a loop saw only the first pass, so
/// `if in_string` after `local in_string = false` was "impossible" though the body sets it for the
/// next pass (neospector jsonl.lua `walk_line`). A loop-carried local keeps its kind, not its value.
#[test]
fn test_loop_carried_local_is_widened() {
    use crate::{DiagnosticCode, VirtualWorkspace};
    let mut ws = VirtualWorkspace::new_with_init_std_lib();
    for (label, code) in [
        (
            "for",
            "---@param line string\nlocal function f(line)\n  local s = false\n  for i = 1, #line do\n    if s then print(i) end\n    if line:sub(i, i) == '\"' then s = not s end\n  end\nend",
        ),
        (
            "for-in",
            "---@param t string[]\nlocal function f(t)\n  local s = false\n  for _, v in ipairs(t) do\n    if s then print(v) end\n    s = true\n  end\nend",
        ),
        (
            "while",
            "local function f()\n  local s = false\n  while math.random() > 0.5 do\n    if s then print(1) end\n    s = true\n  end\nend",
        ),
        (
            "repeat",
            "local function f()\n  local s = false\n  repeat\n    if s then print(1) end\n    s = true\n  until math.random() > 0.5\nend",
        ),
        (
            "nested",
            "---@param t string[]\nlocal function f(t)\n  local s = false\n  for _ = 1, 3 do\n    for _, v in ipairs(t) do\n      if s then print(v) end\n    end\n    s = true\n  end\nend",
        ),
        (
            "sibling branch",
            "---@param t string[]\nlocal function f(t)\n  local in_block = false\n  for _, v in ipairs(t) do\n    if v == 'a' then\n      in_block = true\n    elseif in_block then\n      print(v)\n    end\n  end\nend",
        ),
    ] {
        assert!(
            ws.has_no_diagnostic(DiagnosticCode::UnnecessaryIf, code),
            "{label}"
        );
    }
    // The same holds after the loop: a dynamic loop's body may have run, so a read after it sees
    // the body's writes too, not only the value from before the loop (neospector myaihook.lua
    // `if full and not bracketed`).
    for (label, code) in [
        (
            "for after",
            "---@param t string[]\nlocal function f(t)\n  local s = false\n  for i = 1, #t do\n    if t[i] == 'x' then s = true end\n  end\n  if s then print(1) end\nend",
        ),
        (
            "for-in after",
            "---@param t string[]\nlocal function f(t)\n  local n = 0\n  for _, v in ipairs(t) do\n    if v == 'x' then n = n + 1 end\n  end\n  if n > 0 then print(n) end\nend",
        ),
        (
            "while after",
            "local function f()\n  local s = false\n  while math.random() > 0.5 do\n    s = true\n  end\n  if s then print(1) end\nend",
        ),
        (
            "break",
            "---@param t string[]\nlocal function f(t)\n  local s = false\n  for _, v in ipairs(t) do\n    if v == 'x' then\n      s = true\n      break\n    end\n  end\n  if s then print(1) end\nend",
        ),
    ] {
        assert!(
            ws.has_no_diagnostic(DiagnosticCode::UnnecessaryIf, code),
            "{label}"
        );
    }
    // A loop that starts after the read cannot have run before it.
    assert!(!ws.has_no_diagnostic(
        DiagnosticCode::UnnecessaryIf,
        "---@param t string[]\nlocal function f(t)\n  local s = false\n  if s then print(1) end\n  for _ in ipairs(t) do\n    s = true\n  end\nend",
    ));
    // A local declared inside the loop is a fresh one on every pass.
    assert!(!ws.has_no_diagnostic(
        DiagnosticCode::UnnecessaryIf,
        "---@param t string[]\nlocal function f(t)\n  for _, v in ipairs(t) do\n    local s = false\n    if s then print(v) end\n    s = true\n  end\nend",
    ));
    // Widening never evaluates a body write, so a write that reads the local (`j = f(j)`)
    // cannot loop back into itself (gnd/json.lua `decode_array`).
    assert!(ws.has_no_diagnostic(
        DiagnosticCode::ParamTypeMismatch,
        r#"
        ---@type fun(s: string, i: integer): any, integer
        local decode_value
        ---@param s string
        ---@param i integer
        ---@return integer
        local function skip_ws(s, i) return i end
        ---@param n integer
        local function need(n) end
        ---@param s string
        ---@param i integer
        local function decode_array(s, i)
          local j = skip_ws(s, i + 1)
          while true do
            local v
            v, j = decode_value(s, j)
            need(j)
            j = skip_ws(s, j + 1)
          end
        end
        "#
    ));
}

/// A bare `local tmp` holds nil until assigned. On a path that skips the assignment the trace
/// reached the declaration with no type at all (`unknown`), which vanished from the merge with
/// the assigned branch, so `if tmp` after `if … then tmp = os.tmpname() end` was "always truthy"
/// (neospector logsync/dbw/init.lua `M.invoke`).
#[test]
fn test_uninitialized_local_is_nil_on_unassigned_path() {
    use crate::{DiagnosticCode, VirtualWorkspace};
    let mut ws = VirtualWorkspace::new_with_init_std_lib();
    assert!(ws.has_no_diagnostic(
        DiagnosticCode::UnnecessaryIf,
        "---@param body string\nlocal function f(body)\n  local tmp\n  if #body > 8000 then\n    tmp = os.tmpname()\n  end\n  if tmp then print(body) end\nend",
    ));
    // An assigned value of unknown type keeps no nil from before it (neospector x/drive `M.dump`).
    assert!(ws.has_no_diagnostic(
        DiagnosticCode::NeedCheckNil,
        "---@param t table\nlocal function g(t)\n  local v\n  if t.a then\n    v = t.b\n  else\n    v = t.c\n  end\n  return v.x\nend",
    ));
    // Assigned on every path: truthy.
    assert!(!ws.has_no_diagnostic(
        DiagnosticCode::UnnecessaryIf,
        "---@param body string\nlocal function f(body)\n  local tmp\n  if #body > 8000 then\n    tmp = 'a'\n  else\n    tmp = 'b'\n  end\n  if tmp then print(body) end\nend",
    ));
}

/// `fn.ann = nil` deletes a field of a `table` param; it does not declare it nil. The member typed
/// by that write read `nil` everywhere, also before the write, so `if fn.ann and fn.ann.default`
/// was "always falsy" (luagnd cmdscan `command`). Accepted cost: `fn.ann = nil; if fn.ann` is no
/// longer "always falsy" (member reads on a param are flow-insensitive).
#[test]
fn test_nil_write_does_not_type_a_member() {
    use crate::{DiagnosticCode, VirtualWorkspace};
    let mut ws = VirtualWorkspace::new_with_init_std_lib();
    assert!(ws.has_no_diagnostic(
        DiagnosticCode::UnnecessaryIf,
        "---@param fn table\nlocal function f(fn)\n  local d = fn.ann and fn.ann.default\n  if d then\n    fn.ann.default = nil\n    if not next(fn.ann) then fn.ann = nil end\n  end\nend",
    ));
    // A table this function builds still reads the nil it was given.
    assert!(!ws.has_no_diagnostic(
        DiagnosticCode::UnnecessaryIf,
        "local function f()\n  local t = {}\n  t.ann = nil\n  if t.ann then print(1) end\nend",
    ));
}

/// A negative integer in a doc type (`-1`, a unary minus over a literal) is that literal. It lowered
/// to unknown, which a union then dropped: `---@param dir 1|-1` read `1`, so `if dir > 0` was "always
/// truthy" (neospector `table_goto_col`), and a bare `-1` read unknown, so `d > 0` passed unseen.
#[test]
fn test_negative_doc_integer_is_a_literal() {
    use crate::{DiagnosticCode, VirtualWorkspace};
    let mut ws = VirtualWorkspace::new_with_init_std_lib();
    assert!(ws.has_no_diagnostic(
        DiagnosticCode::UnnecessaryIf,
        "---@param dir 1|-1\nlocal function f(dir)\n  if dir > 0 then print(1) end\nend",
    ));
    assert!(!ws.has_no_diagnostic(
        DiagnosticCode::UnnecessaryIf,
        "---@param d -1\nlocal function f(d)\n  if d > 0 then print(1) end\nend",
    ));
}
