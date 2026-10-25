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
