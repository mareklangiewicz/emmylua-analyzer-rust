#[cfg(test)]
mod test {
    use lsp_types::NumberOrString;
    use tokio_util::sync::CancellationToken;

    use crate::{DiagnosticCode, VirtualWorkspace};

    fn check_class_array_guard(element_type: &str, body: &str, expected_codes: &[DiagnosticCode]) {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();
        let mut emmyrc = ws.get_emmyrc();
        emmyrc.strict.array_index = true;
        ws.update_emmyrc(emmyrc);
        let file_id = ws.def(&format!(
            r#"
            ---@class Container
            ---@field items {element_type}[]
            local _Container = {{}}

            ---@param t Container
            ---@param _other Container
            ---@param _index integer
            local function read(t, _other, _index)
                {body}
            end
            read({{ items = {{ 'hello' }} }}, {{ items = {{}} }}, 1)
            "#,
        ));
        let diagnostics = ws
            .analysis
            .diagnose_file(file_id, CancellationToken::new())
            .expect("diagnostics");
        let expected = expected_codes
            .iter()
            .map(|code| Some(NumberOrString::String(code.get_name().to_string())))
            .collect::<Vec<_>>();
        assert_eq!(
            diagnostics
                .iter()
                .map(|d| d.code.clone())
                .collect::<Vec<_>>(),
            expected,
            "{body}: {diagnostics:#?}"
        );
    }

    #[test]
    fn test_class_array_element_guards() {
        for body in [
            "if t.items[1] then print(t.items[1]:sub(1)) end",
            "if not t.items[1] then return end; print(t.items[1]:sub(1))",
            "if t.items[1] ~= nil then print(t.items[1]:sub(1)) end",
            "if t.items[1] == nil then return end; print(t.items[1]:sub(1))",
            "if t.items[1] then t.items[2] = 'other'; print(t.items[1]:sub(1)) end",
            "t.items = {}; if t.items[1] then print(t.items[1]:sub(1)) end",
            "if t.items[1] then t.items[1] = t.items[1]:sub(1) end",
            "t.items[1] = t.items[2]; if t.items[1] then print(t.items[1]:sub(1)) end",
        ] {
            check_class_array_guard("string", body, &[]);
        }
    }

    #[test]
    fn test_class_array_element_guard_writes() {
        for body in [
            "if t.items[1] then t.items[1] = nil; print(t.items[1]:sub(1)) end",
            "if t.items[1] then t.items[1.0] = nil; print(t.items[1]:sub(1)) end",
            "if t.items[1] then t.items[1e0] = nil; print(t.items[1]:sub(1)) end",
            "if t.items[1] then t.items = {}; print(t.items[1]:sub(1)) end",
            "if t.items[1] then t = _other; print(t.items[1]:sub(1)) end",
            "if t.items[1] then t.items[_index] = nil; print(t.items[1]:sub(1)) end",
            "if t.items[1] then if _index == 1 then t.items = {} end; print(t.items[1]:sub(1)) end",
        ] {
            check_class_array_guard(
                "string",
                body,
                &[
                    DiagnosticCode::ParamTypeMismatch,
                    DiagnosticCode::NeedCheckNil,
                ],
            );
        }
    }

    #[test]
    fn test_class_array_element_guard_scope() {
        for body in [
            "print(t.items[1]:sub(1))",
            "if t.items[1] then print(t.items[2]:sub(1)) end",
            "if t.items[1] then end; print(t.items[1]:sub(1))",
            "if t.items[1] then print(_other.items[1]:sub(1)) end",
            "if t.items[1] then local function later() print(t.items[1]:sub(1)) end; t.items = {}; later() end",
            "if (t.items[1] ~= nil) ~= nil then print(t.items[1]:sub(1)) end",
            "if (not not t.items[1]) ~= nil then print(t.items[1]:sub(1)) end",
        ] {
            check_class_array_guard(
                "string",
                body,
                &[
                    DiagnosticCode::ParamTypeMismatch,
                    DiagnosticCode::NeedCheckNil,
                ],
            );
        }
        check_class_array_guard(
            "string",
            "if t.items[1] then local t = _other; print(t.items[1]:sub(1)) end",
            &[
                DiagnosticCode::RedefinedLocal,
                DiagnosticCode::ParamTypeMismatch,
                DiagnosticCode::NeedCheckNil,
            ],
        );
    }

    #[test]
    fn test_class_array_element_guard_preserves_assignment_type() {
        for (value, expected) in [
            ("'hello'", &[] as &[DiagnosticCode]),
            ("1", &[DiagnosticCode::AssignTypeMismatch]),
        ] {
            check_class_array_guard(
                "(string|number)",
                &format!(
                    r#"
                    t.items[1] = {value}
                    if t.items[1] then
                        ---@type string
                        local _s = t.items[1]
                    end
                    "#,
                ),
                expected,
            );
        }
    }

    #[test]
    fn test_issue_245() {
        let mut ws = VirtualWorkspace::new();

        assert!(ws.has_no_diagnostic(
            DiagnosticCode::NeedCheckNil,
            r#"
        local a --- @type table?
        local _ = (a and a.type == 'change') and a.field
        "#
        ));
    }
    #[test]
    fn test_issue_402() {
        let mut ws = VirtualWorkspace::new();
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::NeedCheckNil,
            r#"
            ---@class A
            local a = {}

            ---@param self table?
            function a.new(self)
                if self then
                    self.a = 1
                end
            end
        "#
        ));
    }

    #[test]
    fn test_issue_474() {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::NeedCheckNil,
            r#"
            ---@class Range4
            ---@class TSNode: userdata
            ---@field range fun(self: TSNode): Range4

            ---@param node_or_range TSNode|Range4
            ---@return Range4
            function foo(node_or_range)
                if type(node_or_range) == 'table' then
                    return node_or_range
                else
                    return node_or_range:range()
                end
            end
            "#
        ));
    }

    #[test]
    fn test_cast() {
        let mut ws = VirtualWorkspace::new();
        ws.def(
            r#"
            ---@class Cast1
            ---@field get fun(self: self, a: number): Cast1?
            ---@field get2 fun(self: self, a: number): Cast1?
        "#,
        );
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::NeedCheckNil,
            r#"
                ---@type Cast1
                local A

                local a = A:get(1) --[[@cast -?]]
                    :get2(2)
            "#
        ));
    }

    #[test]
    fn test_issue_895_891() {
        let mut ws = VirtualWorkspace::new();
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::NeedCheckNil,
            r#"
        local t = {
        123,
        234,
        345,
        }

        ---@param id number
        function test(id) end

        for i = 1, #t do
            test(t[i]) -- expected 'number' but found (123|234|345)?
        end
        "#,
        ));
    }

    #[test]
    fn test_issue_886() {
        let mut ws = VirtualWorkspace::new();
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::AssignTypeMismatch,
            r#"
        ---@type string[]
        local a = {}

        -- if #a == 0 then return end
        if not a[1] then return end

        -- ---@type string
        -- local s = a[1]

        ---@type string
        local s = a[#a]
        "#,
        ));
    }

    #[test]
    fn test_asserted_index_assignment_prefix_is_not_nil() {
        let mut ws = VirtualWorkspace::new();
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::NeedCheckNil,
            r#"
        ---@type integer[][]
        local res
        res[1][1] = assert(res[1])[2]
        "#,
        ));
    }

    #[test]
    fn test_numeric_for_len_expr_narrows_loop_body_value_to_non_nil() {
        let mut ws = VirtualWorkspace::new();
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::NeedCheckNil,
            r#"
        ---@type false|fun(...)[]?
        local calls

        -- `#calls` itself may raise; the body read is what this pins.
        ---@diagnostic disable-next-line: need-check-nil
        for i = 1, #calls do
            calls[i](...)
        end
        "#,
        ));
    }

    #[test]
    fn test_different_asserted_index_assignment_prefix_still_needs_nil_check() {
        let mut ws = VirtualWorkspace::new();
        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::NeedCheckNil,
            r#"
        ---@type integer[][]
        local res
        res[1][1] = assert(res[2])[2]
        "#,
        ));
    }

    #[test]
    fn test_metatable_index_takes_declared_type_of_table() {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();
        // `__index = G` where `---@type Api` declares `G = {}`: the proxy reads `Api`'s fields.
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::NeedCheckNil,
            r#"
            ---@class Dsp
            ---@field exec fun(s: string): integer

            ---@class Api
            ---@field dsp Dsp

            ---@type Api
            G = {}

            ---@type Api
            local L = {}

            local hl = setmetatable({ bind = 1 }, { __index = G })
            local n = hl.dsp.exec('x')
            local hl2 = setmetatable({}, { __index = L })
            local m = hl2.dsp.exec('y')
        "#
        ));
    }

    #[test]
    fn test_numeric_for_bound_by_field_path_length() {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();
        // `for i = 1, #t.list` bounds `t.list[i]` as `for i = 1, #list` bounds `list[i]`.
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::NeedCheckNil,
            r#"
            ---@class Holder
            ---@field list string[]

            ---@type Holder
            local t = { list = {} }
            for i = 1, #t.list do
                print(t.list[i]:upper())
            end
        "#
        ));
        // A different path is not bounded by it.
        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::NeedCheckNil,
            r#"
            ---@class Holder2
            ---@field list string[]
            ---@field other string[]

            ---@type Holder2
            local t = { list = {}, other = {} }
            for i = 1, #t.list do
                print(t.other[i]:upper())
            end
        "#
        ));
    }

    #[test]
    fn test_and_narrows_after_merge_of_earlier_guard() {
        // After `if prev and ok()` merges back to `St?`, the later `prev and prev.sig` still
        // narrows its right side, even when an unguarded read of `prev` memoized the merge
        // first. Needs the std library and every check: with less, nothing reads `prev`
        // before the guarded read, and the test passes without the fix.
        let mut ws = VirtualWorkspace::new_with_init_std_lib();
        let file_id = ws.def(
            r#"
            ---@class St
            ---@field sig string

            ---@return St?
            local function decode() end

            ---@return boolean
            local function ok() return true end

            local prev = decode()
            if prev and ok() then end
            print(prev and prev.sig)
        "#,
        );
        let diagnostics = ws
            .analysis
            .diagnose_file(file_id, CancellationToken::new())
            .unwrap_or_default();
        let code = Some(NumberOrString::String(
            DiagnosticCode::NeedCheckNil.get_name().to_string(),
        ));
        assert!(
            !diagnostics.iter().any(|diag| diag.code == code),
            "{diagnostics:#?}"
        );
    }

    /// A generic `T -?` return drops `nil` also when the function is a member of a
    /// `---@class` table defined in another file.
    #[test]
    fn test_required_class_module_not_null_return() {
        let mut ws = VirtualWorkspace::new();
        ws.def_file(
            "chk.lua",
            r#"
            ---@class Chk
            local chk = {}
            ---@generic T
            ---@param act T
            ---@return T -?
            function chk.truthy(act) return act end
            return chk
            "#,
        );
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::NeedCheckNil,
            r#"
            local chk = require 'chk'
            ---@type string?
            local q
            print(chk.truthy(q):len())
            "#,
        ));
    }

    /// An `--[[@as T]]` inside parentheses casts the parenthesized value: `(ctx --[[@as Sub]]).f`
    /// reads `Sub`'s field, not the declared type's (where a missing field reads nil).
    #[test]
    fn test_paren_inner_cast_types_index_prefix() {
        let mut ws = VirtualWorkspace::new();
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::NeedCheckNil,
            r#"
            ---@class Api
            ---@field dispatch fun(d: any)

            ---@class Base
            ---@field log any

            ---@class Sub : Base
            ---@field hl Api

            ---@param ctx Base
            local function f(ctx)
                local hl = (ctx --[[@as Sub]]).hl
                hl.dispatch(1)
            end
            "#,
        ));
    }

    /// `return M` where `local M = f() --[[@as T]]`: the module exports `T` also when no
    /// `---@class` is attached to `M` (here a blank line detaches it) and `f()` is opaque.
    #[test]
    fn test_required_module_export_keeps_as_cast() {
        let mut ws = VirtualWorkspace::new();
        ws.def_file(
            "mylog.lua",
            r#"
            ---@class Base
            local Base = {}
            function Base:i() end

            ---@class MyLog : Base

            local log = Base.new() --[[@as MyLog]]
            return log
            "#,
        );
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::NeedCheckNil,
            r#"
            local log = require 'mylog'
            log:i()
            "#,
        ));
    }

    /// An optional function field may be nil: an unguarded call is flagged.
    #[test]
    fn test_optional_function_field_call_needs_check() {
        let mut ws = VirtualWorkspace::new();
        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::NeedCheckNil,
            r#"
            ---@class OptFunCall
            ---@field f? fun()

            ---@param t OptFunCall
            local function a(t)
                t.f()
            end
            "#
        ));
        // A guard still narrows it.
        for body in [
            "if not t.f then return end\nt.f()",
            "if _flag or not t.f then return end\nt.f()",
            "if t.f then t.f() end",
        ] {
            assert!(
                ws.has_no_diagnostic(
                    DiagnosticCode::NeedCheckNil,
                    &format!(
                        "---@param t OptFunCall\n---@param _flag boolean\nlocal function a(t, _flag)\n{body}\nend"
                    ),
                ),
                "{body}"
            );
        }
    }

    #[test]
    fn test_numeric_for_bound_through_a_local_alias_of_the_path() {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();
        // `#list` bounds `t.list[i]` when `list` is a local alias of `t.list`, as member
        // narrowing reads such aliases.
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::NeedCheckNil,
            r#"
            ---@class Holder
            ---@field list string[]

            ---@type Holder
            local t = { list = {} }
            local list = t.list
            for i = 1, #list do
                print(t.list[i]:upper())
            end
        "#
        ));
        // A path written inside the loop is no longer the alias's.
        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::NeedCheckNil,
            r#"
            ---@class Holder3
            ---@field list string[]

            ---@type Holder3
            local t = { list = {} }
            local list = t.list
            for i = 1, #list do
                t.list = {}
                print(t.list[i]:upper())
            end
        "#
        ));
    }
    #[test]
    fn test_guard_narrows_field_written_in_another_function() {
        // `init` writes the field, so the read takes the runtime member's type; the `and`
        // guard must still narrow it, through `Store.x` and through a method's `self.x`.
        let mut ws = VirtualWorkspace::new();
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::NeedCheckNil,
            r#"
            ---@class Checks
            ---@field safe boolean?

            ---@class Store
            ---@field checks Checks?
            local Store = {}

            ---@param checks Checks?
            function Store.init(checks) Store.checks = checks end

            function Store.use()
                if Store.checks and Store.checks.safe then return 1 end
            end
        "#
        ));
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::NeedCheckNil,
            r#"
            ---@class Checks2
            ---@field safe boolean?

            ---@class Store2
            ---@field checks Checks2?
            local Store2 = {}

            ---@param checks Checks2?
            function Store2:init(checks) self.checks = checks end

            function Store2:use()
                if self.checks and self.checks.safe then return 1 end
            end
        "#
        ));
    }

    #[test]
    fn test_strict_array_index_of_any_or_unknown_elements_is_not_nullable() {
        // `any` and `unknown` already admit nil, so a strict array index adds nothing to them: a
        // call through `{ ... }` of unannotated varargs is not "may be nil".
        let mut ws = VirtualWorkspace::new();
        let mut emmyrc = ws.get_emmyrc();
        emmyrc.strict.array_index = true;
        ws.update_emmyrc(emmyrc);
        for body in [
            "---@type any[]\nlocal a = {}\na[n]()\nprint(a[n].x)",
            "---@type unknown[]\nlocal u = {}\nu[n]()\nprint(u[n].x)",
            "local ps = { ... }\nlocal last = ps[n]\nlast()\nps[n]()",
            "local ps = { ... }\nps[1]()",
        ] {
            assert!(
                ws.has_no_diagnostic(
                    DiagnosticCode::NeedCheckNil,
                    &format!("local function f(n, ...)\n---@cast n integer\n{body}\nend"),
                ),
                "{body}"
            );
        }
        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::NeedCheckNil,
            r#"
            ---@param n integer
            local function f(n)
                ---@type string[]
                local s = {}
                print(s[n]:upper())
            end
        "#
        ));
    }

    #[test]
    fn test_call_through_an_undefined_field_is_left_to_undefined_field() {
        // An undefined member reads as `nil`; undefined-field already names that site, and a
        // second "may be nil" for it would survive the undefined-field suppression meant for it.
        let mut ws = VirtualWorkspace::new();
        let source = r#"
            ---@class Obj
            ---@field cls table

            ---@class Cls
            ---@field new fun(self: Cls): Obj

            ---@param C Cls
            local function f(C)
                local p = C:new()
                p:sum()
                print(p.sum.x)
                ---@diagnostic disable-next-line: undefined-field
                p.other()
            end
        "#;
        assert!(ws.has_no_diagnostic(DiagnosticCode::NeedCheckNil, source));
        assert!(!ws.has_no_diagnostic(DiagnosticCode::UndefinedField, source));
        // A declared optional member still needs its check.
        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::NeedCheckNil,
            r#"
            ---@class Obj2
            ---@field sum (fun(self: Obj2): integer)?

            ---@param p Obj2
            local function f(p)
                p:sum()
            end
        "#
        ));
    }

    #[test]
    fn test_gmatch_captures_after_the_first_are_strings() {
        // The loop ends when the first capture is nil; while it runs, every capture matched.
        let mut ws = VirtualWorkspace::new_with_init_std_lib();
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::NeedCheckNil,
            r#"
            local src = 'a=b'
            for k, v in src:gmatch '(%w+)=(%w+)' do
                print(k:upper(), v:upper())
            end
        "#
        ));
        // A single call outside a loop still yields a possibly-nil first capture.
        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::NeedCheckNil,
            r#"
            local src = 'a=b'
            local k = src:gmatch '(%w+)'()
            print(k:upper())
        "#
        ));
    }

    #[test]
    fn test_dynamic_index_guard_by_local_key() {
        // A dynamic key named by a local identifies the access: the guard covers the same `toks[k]`.
        let mut ws = VirtualWorkspace::new();
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::NeedCheckNil,
            r#"
            ---@class Tok
            ---@field line integer

            ---@param toks Tok[]
            ---@param k integer
            local function f(toks, k)
                local line = toks[k] and toks[k].line
                while toks[k] and toks[k].line == line do
                    k = k + 1
                end
                if toks[k] ~= nil then return toks[k].line end
            end
            return f
        "#
        ));
    }

    #[test]
    fn test_dynamic_index_guard_lost_on_key_or_slot_write() {
        let mut ws = VirtualWorkspace::new();
        // The key moved: `toks[k]` is now another slot.
        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::NeedCheckNil,
            r#"
            ---@class Tok
            ---@field line integer

            ---@param toks Tok[]
            ---@param k integer
            local function f(toks, k)
                if toks[k] then
                    k = k + 1
                    return toks[k].line
                end
            end
            return f
        "#
        ));
        // A different local key is a different access.
        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::NeedCheckNil,
            r#"
            ---@class Tok
            ---@field line integer

            ---@param toks Tok[]
            ---@param k integer
            ---@param j integer
            local function f(toks, k, j)
                if toks[k] then return toks[j].line end
            end
            return f
        "#
        ));
        // A literal-key write may be the same slot.
        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::NeedCheckNil,
            r#"
            ---@class Tok
            ---@field line integer

            ---@param toks Tok[]
            ---@param k integer
            local function f(toks, k)
                if toks[k] then
                    toks[1] = nil
                    return toks[k].line
                end
            end
            return f
        "#
        ));
    }

    #[test]
    fn test_dynamic_index_guard_by_length_key() {
        let mut ws = VirtualWorkspace::new();
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::NeedCheckNil,
            r#"
            ---@class Stage
            ---@field final boolean

            ---@param stages Stage[]
            local function f(stages)
                return stages[#stages] and stages[#stages].final
            end
            return f
        "#
        ));
        // Any write below the length's path may move it.
        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::NeedCheckNil,
            r#"
            ---@class Stage
            ---@field final boolean

            ---@param stages Stage[]
            ---@param other Stage[]
            local function f(stages, other)
                if stages[#other] then
                    other[#other + 1] = stages[1]
                    return stages[#other].final
                end
            end
            return f
        "#
        ));
    }

    #[test]
    fn test_length_of_nullable_operand() {
        let mut ws = VirtualWorkspace::new();
        // `#nil` raises, as `nil.x` does.
        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::NeedCheckNil,
            r#"
            ---@param w string?
            local function f(w) return #w end
            return f
        "#
        ));
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::NeedCheckNil,
            r#"
            ---@param w string?
            local function f(w) return w and #w end
            return f
        "#
        ));
        // Where it yields a value, the length is an integer, not the operand's `string?`.
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::ParamTypeMismatch,
            r#"
            ---@param n integer
            local function take(n) end
            ---@param w string?
            local function f(w) take(#w) take(1 + #w - 1) end
            return f
        "#
        ));
    }

    #[test]
    fn test_guard_after_cast_narrows_the_cast_type() {
        let mut ws = VirtualWorkspace::new();
        // The guard follows the cast, so it narrows what the cast set (both read the decl's trace).
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::NeedCheckNil,
            r#"
            ---@return string[]?
            local function rd() return nil end
            local function f()
                local missing = rd()
                ---@cast missing string[]?
                if missing and missing[1] then return 1 end
                if missing and missing[2] then return 2 end
            end
            return f
        "#
        ));
    }

    #[test]
    fn test_integer_key_read_of_an_empty_table_literal_is_unknown_not_nil() {
        let mut ws = VirtualWorkspace::new();
        // Dynamic-key writes record no member, so an empty `{}` has no known shape: the string-key
        // read already says `unknown`; the integer-key read said `nil`.
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::NeedCheckNil,
            r#"
            local g = {}
            for y = 1, 3 do
                g[y] = {}
                for x = 1, 3 do
                    g[y][x] = ' '
                end
            end
            return g
        "#
        ));
    }
}
