#[cfg(test)]
mod test {
    use crate::{DiagnosticCode, VirtualWorkspace};
    use lsp_types::NumberOrString;
    use tokio_util::sync::CancellationToken;

    #[test]
    fn test_vararg_first_function_is_callable() {
        let diagnostics = crate::check::test::check_source(
            r#"
            ---@param ... fun(value: integer): integer
            local function apply(...)
                return (...)(1)
            end
            print(apply(function(value) return value end))
        "#,
        );
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    /// With every checker on, param-type-mismatch types the closure argument first. Its
    /// return inference read `w`, which answered `unknown` while that return was in
    /// progress; the answer was cached, so the later checkers saw an untyped `w`.
    #[test]
    fn test_closure_arg_param_typed_after_param_type_check() {
        let diagnostics = crate::check::test::check_source(
            r#"
            ---@param f fun(prev: integer): integer?
            local function plain(f) return f(1) end
            plain(function(w) w() return w end)

            ---@generic V
            ---@param seed V
            ---@param f fun(prev: V): V?
            ---@return V
            local function iterate(seed, f) return f(seed) or seed end
            iterate(1, function(want) want() return want end)
        "#,
        );
        assert_eq!(
            crate::check::test::count_by_code(&diagnostics, DiagnosticCode::CallNonCallable),
            2,
            "{diagnostics:#?}"
        );
    }

    #[test]
    fn test_vararg_first_function_checks_arguments() {
        let diagnostics = crate::check::test::check_source(
            r#"
            ---@param ... fun(value: integer): integer
            local function apply(...)
                return (...)('bad')
            end
            print(apply(function(value) return value end))
        "#,
        );
        assert_eq!(
            crate::check::test::count_by_code(&diagnostics, DiagnosticCode::ParamTypeMismatch),
            1
        );
        assert_eq!(
            crate::check::test::count_by_code(&diagnostics, DiagnosticCode::CallNonCallable),
            0
        );
    }

    #[test]
    fn test_vararg_first_number_is_not_callable() {
        let diagnostics = crate::check::test::check_source(
            r#"
            ---@param ... integer
            local function apply(...)
                return (...)(1)
            end
            print(apply(2))
        "#,
        );
        assert_eq!(
            crate::check::test::count_by_code(&diagnostics, DiagnosticCode::CallNonCallable),
            1
        );
    }

    #[test]
    fn test_callable_table_accepts_valid_arguments() {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();
        let file_id = ws.def(
            r#"
            local command = setmetatable({}, {
                ---@param arg string
                __call = function(_, arg) print(arg) end,
            })
            command('hello')
            local api = { cmd = command }
            api.cmd('hello')
            "#,
        );
        let diagnostics = ws
            .analysis
            .diagnose_file(file_id, CancellationToken::new())
            .unwrap();
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn test_callable_table_preserves_invalid_call_diagnostics() {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();
        let file_id = ws.def(
            r#"
            local command = setmetatable({}, {
                ---@param arg string
                __call = function(_, arg) print(arg) end,
            })
            command(42)
            local plain = {}
            plain()
            local without_call = setmetatable({}, {})
            without_call()
            "#,
        );
        let mut diagnostics = ws
            .analysis
            .diagnose_file(file_id, CancellationToken::new())
            .unwrap();
        diagnostics.sort_by_key(|diagnostic| diagnostic.range.start);
        let codes: Vec<_> = diagnostics
            .iter()
            .map(|diagnostic| diagnostic.code.clone())
            .collect();
        assert_eq!(
            codes,
            vec![
                Some(NumberOrString::String("param-type-mismatch".into())),
                Some(NumberOrString::String("call-non-callable".into())),
                Some(NumberOrString::String("call-non-callable".into())),
            ],
            "{diagnostics:#?}"
        );
    }

    #[test]
    fn test_callable_table_member_assignment_across_files() {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();
        ws.def_file(
            "api.lua",
            r#"
            local api = {}
            api.cmd = setmetatable({}, {
                ---@param arg string
                __call = function(_, arg) print(arg) end,
            })
            api.plain = {}
            return api
            "#,
        );
        let file_id = ws.def(
            r#"
            local api = require('api')
            api.cmd('hello')
            api.cmd(42)
            api.plain()
            "#,
        );
        let mut diagnostics = ws
            .analysis
            .diagnose_file(file_id, CancellationToken::new())
            .unwrap();
        diagnostics.sort_by_key(|diagnostic| diagnostic.range.start);
        let codes: Vec<_> = diagnostics
            .iter()
            .map(|diagnostic| diagnostic.code.clone())
            .collect();
        assert_eq!(
            codes,
            vec![
                Some(NumberOrString::String("param-type-mismatch".into())),
                Some(NumberOrString::String("call-non-callable".into())),
            ],
            "{diagnostics:#?}"
        );
    }

    #[test]
    fn test_call_non_callable() {
        let mut ws = VirtualWorkspace::new();

        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::CallNonCallable,
            r#"
                local i = 1
                i()
            "#
        ));

        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::CallNonCallable,
            r#"
                local s = "hi"
                s()
            "#
        ));

        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::CallNonCallable,
            r#"
                local b = true
                b()
            "#
        ));

        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::CallNonCallable,
            r#"
                ---@type thread
                local t
                t()
            "#
        ));

        assert!(ws.has_no_diagnostic(
            DiagnosticCode::CallNonCallable,
            r#"
                local function f() end
                f()
            "#
        ));

        assert!(ws.has_no_diagnostic(
            DiagnosticCode::CallNonCallable,
            r#"
                ---@type function
                local f
                f()
            "#
        ));

        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::CallNonCallable,
            r#"
                ---@type function|integer
                local f
                f()
            "#
        ));

        assert!(ws.has_no_diagnostic(
            DiagnosticCode::CallNonCallable,
            r#"
                local f ---@type { field: string } & fun()
                f()
            "#
        ));

        // nil is covered by need-check-nil instead of call-non-callable
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::CallNonCallable,
            r#"
                ---@type function|nil
                local f
                f()
            "#
        ));

        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::CallNonCallable,
            r#"
                (1)()
            "#
        ));

        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::CallNonCallable,
            r#"
                ("hi")()
            "#
        ));

        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::CallNonCallable,
            r#"
                (true)()
            "#
        ));

        assert!(ws.has_no_diagnostic(
            DiagnosticCode::CallNonCallable,
            r#"
                (function() end)()
            "#
        ));

        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::CallNonCallable,
            r#"
                local i --- @type integer|fun():string
                _ = i()
            "#
        ));

        assert!(ws.has_no_diagnostic(
            DiagnosticCode::CallNonCallable,
            r#"
                ---@class Callable
                ---@operator call: string
                ---@type Callable
                local c
                c()
            "#
        ));

        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::CallNonCallable,
            r#"
                local c = {}
                c()
            "#
        ));

        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::CallNonCallable,
            r#"
                ---@class NonCallable
                ---@type NonCallable
                local c = {}
                c()
            "#
        ));

        assert!(ws.has_no_diagnostic(
            DiagnosticCode::CallNonCallable,
            r#"
                ---@type any
                local c
                c()
            "#
        ));
    }

    #[test]
    fn test_call_non_callable_fallback_from_initializer() {
        let mut ws = VirtualWorkspace::new();
        ws.enable_check(DiagnosticCode::CallNonCallable);
        let file_id = ws.def("local i = 1\n i()\n");

        let diagnostics = ws
            .analysis
            .diagnose_file(file_id, CancellationToken::new())
            .unwrap();
        let code = Some(NumberOrString::String(
            DiagnosticCode::CallNonCallable.get_name().to_string(),
        ));
        assert!(diagnostics.iter().any(|diag| diag.code == code));
    }

    #[test]
    fn test_no_call_non_callable_on_class_overload() {
        let mut ws = VirtualWorkspace::new();
        // `---@overload` on a class makes both the class table and its instances callable.
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::CallNonCallable,
            r#"
            ---@class Point
            ---@field x integer
            ---@overload fun(x: integer): Point
            local Point = {}
            local p = Point(1)
            "#
        ));
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::CallNonCallable,
            r#"
            ---@class Logger
            ---@overload fun(msg: string)
            ---@type Logger
            local log
            log('hi')
            "#
        ));
    }

    /// snacks declares `---@class snacks.picker` twice: `---@meta` field docs in one file, the
    /// `---@overload fun(...)` on the runtime table in another. Only the first declaration was
    /// read for call candidates, so `Snacks.picker { ... }` was not callable.
    #[test]
    fn test_no_call_non_callable_on_class_overload_in_another_declaration() {
        let mut ws = VirtualWorkspace::new();
        ws.def_files(vec![
            (
                "picker_meta.lua",
                r#"
                ---@meta _
                ---@class Picker
                ---@field files fun(): integer
                "#,
            ),
            (
                "picker.lua",
                r#"
                ---@class Picker
                ---@overload fun(opts: table): integer
                local M = setmetatable({}, { __call = function(_, opts) return 1 end })
                return M
                "#,
            ),
        ]);
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::CallNonCallable,
            r#"
            ---@type Picker
            local p
            local n = p({})
            "#
        ));
        ws.def(
            r#"
            ---@type Picker
            local p
            picked = p({})
            "#,
        );
        let picked = ws.expr_ty("picked");
        assert_eq!(ws.humanize_type(picked), "integer");
    }

    #[test]
    fn test_no_call_non_callable_on_generic_class_instance() {
        let mut ws = VirtualWorkspace::new();
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::CallNonCallable,
            r#"
            ---@class Box<T>
            ---@operator call(T): T
            ---@type Box<integer>
            local b
            b(1)
            "#
        ));
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::CallNonCallable,
            r#"
            ---@class Set<T>
            ---@overload fun(v: T): boolean
            ---@type Set<string>
            local s
            s('a')
            "#
        ));
        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::CallNonCallable,
            r#"
            ---@class Plain<T>
            ---@type Plain<string>
            local p
            p('a')
            "#
        ));
    }

    #[test]
    fn test_no_call_non_callable_on_undefined_type() {
        let mut ws = VirtualWorkspace::new();
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::CallNonCallable,
            r#"
            local a --- @type NotDefined
            a()
            "#
        ));
    }

    #[test]
    fn test_no_call_non_callable_on_alias() {
        let mut ws = VirtualWorkspace::new();
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::CallNonCallable,
            r#"
            ---@alias FunAlias function
            local a --- @type FunAlias
            a()
            "#
        ));
    }

    #[test]
    fn test_no_call_non_callable_for_generic_function_param_in_a_lua() {
        let mut ws = VirtualWorkspace::new();
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::CallNonCallable,
            r#"
            --- @generic F: function
            --- @param fn F
            --- @return F
            function once(fn)
              return function(...)
                return fn(...)
              end
            end
            "#,
        ));
    }

    /// #1334: a `table<string, integer>` read is `integer`, which is not callable.
    #[test]
    fn test_index_signature_value_is_not_callable() {
        let diagnostics = crate::check::test::check_source(
            r#"
            ---@param pt table<string, integer>
            local function g(pt)
                return pt.X()
            end
            return g
        "#,
        );
        assert_eq!(
            crate::check::test::count_by_code(&diagnostics, DiagnosticCode::CallNonCallable),
            1
        );
    }

    /// A local initializer reads the callee's declaration, so a callable table literal keeps its
    /// `__call` there: the assignment contract widens it to `table`, the call must not.
    #[test]
    fn test_callable_table_called_in_a_local_initializer() {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::CallNonCallable,
            r#"
            local command = setmetatable({}, { __call = function() return 1 end })
            local x = command()
            return x
            "#,
        ));
    }

    #[test]
    fn test_type_guard_narrows_an_access_path_to_its_callable_part() {
        // An object type's field and a dynamic key have no member declaration: their access path
        // carries the `type()` guard, which keeps only the functions.
        let mut ws = VirtualWorkspace::new();
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::CallNonCallable,
            r#"
            ---@param o { x: string|fun(): string }
            ---@param answers table<string, string|fun(): string>
            ---@param s string
            local function f(o, answers, s)
                if type(o.x) == 'function' then o.x() end
                if type(answers[s]) == 'function' then return answers[s]() end
            end
            return f
            "#,
        ));
        // The other branch, a different key, and a union of guards that lets a string through.
        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::CallNonCallable,
            r#"
            ---@param answers table<string, string|fun(): string>
            ---@param s string
            local function f(answers, s)
                if type(answers[s]) ~= 'function' then return answers[s]() end
            end
            return f
            "#,
        ));
        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::CallNonCallable,
            r#"
            ---@param answers table<string, string|fun(): string>
            ---@param s string
            ---@param t string
            local function f(answers, s, t)
                if type(answers[s]) == 'function' then return answers[t]() end
            end
            return f
            "#,
        ));
        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::CallNonCallable,
            r#"
            ---@param o { x: string|fun(): string }
            local function f(o)
                if type(o.x) == 'function' or type(o.x) == 'string' then o.x() end
            end
            return f
            "#,
        ));
    }
}
