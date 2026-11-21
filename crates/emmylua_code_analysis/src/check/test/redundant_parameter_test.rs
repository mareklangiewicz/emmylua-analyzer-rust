#[cfg(test)]
mod test {
    use crate::check::test::{check_source, texts_by_code};
    use crate::{DiagnosticCode, VirtualWorkspace};

    #[test]
    fn test() {
        let mut ws = VirtualWorkspace::new();

        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::RedundantParameter,
            r#"
            ---@class Test
            local Test = {}

            ---@param a string
            function Test.name(a)
            end

            Test:name("")
        "#
        ));

        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::RedundantParameter,
            r#"
            ---@class Test2
            local Test = {}

            ---@param a string
            function Test.name(a)
            end

            Test.name("", "")
        "#
        ));

        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::RedundantParameter,
            r#"
            ---@class A
            ---@field event fun()

            ---@type A
            local a = {
                event = function(aaa)
                end,
            }
        "#
        ));
    }

    #[test]
    fn test_1() {
        let mut ws = VirtualWorkspace::new();

        assert!(ws.has_no_diagnostic(
            DiagnosticCode::RedundantParameter,
            r#"
                ---@type fun(...)[]
                local a = {}

                a[1] = function(ccc, ...)
                end
        "#
        ));
    }

    #[test]
    fn test_dots() {
        let mut ws = VirtualWorkspace::new();

        assert!(ws.has_no_diagnostic(
            DiagnosticCode::RedundantParameter,
            r#"
            ---@class Test
            local Test = {}

            ---@param a string
            ---@param ... any
            function Test.dots(a, ...)
                print(a, ...)
            end

            Test.dots(1, 2, 3)
            Test:dots(1, 2, 3)
        "#
        ));
    }

    #[test]
    fn test_named_dots() {
        let mut ws = VirtualWorkspace::new();

        assert!(ws.has_no_diagnostic(
            DiagnosticCode::RedundantParameter,
            r#"
            ---@class Test
            local Test = {}

            ---@param ... any
            function Test.dots(...args)
            end

            Test.dots(1, 2, 3)
            Test:dots(1, 2, 3)
        "#
        ));

        assert!(ws.has_no_diagnostic(
            DiagnosticCode::ParamTypeMismatch,
            r#"
            ---@param ... number
            local function f(...args)
            end

            f(1, 2, 3)
        "#
        ));
    }

    #[test]
    fn test_variadic_call_operator() {
        for (class, instance) in [
            ("Callable", "Callable"),
            ("Callable<T>", "Callable<integer>"),
        ] {
            let source = format!(
                r#"
                ---@class {class}
                ---@operator call(string...): string

                ---@type {instance}
                local callable

                callable()
                callable("a", "b", "c")
                callable("a", 1)
                "#
            );
            let diagnostics = check_source(&source);
            assert_eq!(
                diagnostics
                    .iter()
                    .filter(|d| {
                        matches!(
                            d.code,
                            DiagnosticCode::ParamTypeMismatch
                                | DiagnosticCode::MissingParameter
                                | DiagnosticCode::RedundantParameter
                        )
                    })
                    .map(|d| (d.code, usize::from(d.range.start())))
                    .collect::<Vec<_>>(),
                vec![(
                    DiagnosticCode::ParamTypeMismatch,
                    source.find("1)").unwrap()
                )],
                "{class}: {diagnostics:#?}"
            );
        }
    }

    #[test]
    fn test_issue_360() {
        let mut ws = VirtualWorkspace::new();
        let source = r#"
            ---@alias buz number

            ---@param a buz
            ---@overload fun(): number
            function test(a)
            end

            local c = test({'test'})
        "#;

        assert!(ws.has_no_diagnostic(DiagnosticCode::RedundantParameter, source));
        assert!(!ws.has_no_diagnostic(DiagnosticCode::ParamTypeMismatch, source));
    }

    #[test]
    fn test_function_param() {
        let mut ws = VirtualWorkspace::new();
        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::RedundantParameter,
            r#"
                ---@param callback fun()
                local function with_local(callback)
                end

                with_local(function(local_player) end)
        "#
        ));
    }

    #[test]
    fn test_generic_infer_function() {
        // temp disable this test, future fix this
        let mut ws = VirtualWorkspace::new();
        ws.def(
            r#"
            ---@alias Parameters<T extends function> T extends (fun(...: infer P): any) and P or never

            ---@alias Procedure fun(...: any[]): any

            ---@alias MockParameters<T> T extends Procedure and Parameters<T> or never

            ---@class Mock<T>
            ---@overload fun(...: MockParameters<T>...)

            ---@generic T: Procedure
            ---@param a T
            ---@return Mock<T>
            function fn(a)
            end

            sum = fn(function(a, b)
                return a + b
            end)
            "#,
        );
        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::RedundantParameter,
            r#"
            sum(1, 2, 3)
        "#
        ));
    }

    #[test]
    fn test_generic_variadic_instantiated_params_reports_redundant_parameter() {
        let mut ws = VirtualWorkspace::new();
        ws.def(
            r#"
            ---@generic T
            ---@param ... T...
            ---@return fun(...: T...)
            local function bind(...)
            end

            bound = bind(1, "a")
            "#,
        );

        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::RedundantParameter,
            r#"
            bound(1, "a", true)
        "#
        ));
    }

    #[test]
    fn test_issue_894() {
        let mut ws = VirtualWorkspace::new();
        ws.def(
            r#"
            _nop = function() end
            "#,
        );
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::RedundantParameter,
            r#"
            function a(...) _nop(...) end
        "#
        ));
    }

    #[test]
    fn test_issue_1274_luajit_loadstring() {
        use crate::{Emmyrc, EmmyrcLuaVersion};

        let mut ws = VirtualWorkspace::new();
        let mut config = Emmyrc::default();
        config.runtime.version = EmmyrcLuaVersion::LuaJIT;
        ws.update_emmyrc(config);
        ws.analysis.init_std_lib(None);

        // LuaJIT: `loadstring` is an alias for `load` and accepts `mode`/`env`.
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::RedundantParameter,
            r#"
            local storage = { env = {} }
            local _ = loadstring("return 42", nil, nil, storage.env)
        "#
        ));

        // Guard against a vacuous pass: a 5th argument must still be redundant.
        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::RedundantParameter,
            r#"
            loadstring("return 42", nil, nil, {}, {})
        "#
        ));
    }

    #[test]
    fn test_issue_1274_luajit_load() {
        use crate::{Emmyrc, EmmyrcLuaVersion};

        let mut ws = VirtualWorkspace::new();
        let mut config = Emmyrc::default();
        config.runtime.version = EmmyrcLuaVersion::LuaJIT;
        ws.update_emmyrc(config);
        ws.analysis.init_std_lib(None);

        // LuaJIT: `load` accepts `chunk, chunkname?, mode?, env?`.
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::RedundantParameter,
            r#"
            local storage = { env = {} }
            local _ = load("return 42", nil, nil, storage.env)
        "#
        ));

        // Guard against a vacuous pass: a 5th argument must still be redundant.
        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::RedundantParameter,
            r#"
            load("return 42", nil, nil, {}, {})
        "#
        ));
    }

    #[test]
    fn test_issue_1274_luajit_loadfile() {
        use crate::{Emmyrc, EmmyrcLuaVersion};

        let mut ws = VirtualWorkspace::new();
        let mut config = Emmyrc::default();
        config.runtime.version = EmmyrcLuaVersion::LuaJIT;
        ws.update_emmyrc(config);
        ws.analysis.init_std_lib(None);

        // LuaJIT: `loadfile` accepts `filename?, mode?, env?`.
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::RedundantParameter,
            r#"
            local storage = { env = {} }
            local _ = loadfile("chunk.lua", nil, storage.env)
        "#
        ));

        // Guard against a vacuous pass: a 4th argument must still be redundant.
        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::RedundantParameter,
            r#"
            loadfile("chunk.lua", nil, {}, {})
        "#
        ));
    }

    #[test]
    fn test_named_dots_zero_args() {
        let mut ws = VirtualWorkspace::new();

        assert!(ws.has_no_diagnostic(
            DiagnosticCode::MissingParameter,
            r#"
            function f(...args)
            end

            f()
        "#
        ));

        assert!(ws.has_no_diagnostic(
            DiagnosticCode::RedundantParameter,
            r#"
            local function g(...args)
            end

            g(1, 2, 3)
        "#
        ));
    }

    #[test]
    fn test_overload_dots_and_optional_params() {
        let mut ws = VirtualWorkspace::new();

        assert!(ws.has_no_diagnostic(
            DiagnosticCode::RedundantParameter,
            r#"
            ---@overload fun(...: integer): integer
            ---@param ... number
            ---@return number
            local function mx(...) return 0 end

            mx(1, 2, 3)
        "#
        ));

        assert!(ws.has_no_diagnostic(
            DiagnosticCode::RedundantParameter,
            r#"
            ---@overload fun(x: string, ...: integer)
            ---@param a string
            local function f(a) end

            f('a', 1, 2)
        "#
        ));

        assert!(ws.has_no_diagnostic(
            DiagnosticCode::MissingParameter,
            r#"
            ---@overload fun(x?: integer)
            ---@param a string
            ---@param b string
            local function g(a, b) end

            g()
        "#
        ));

        // Control: a fixed-arity overload still reports extra arguments.
        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::RedundantParameter,
            r#"
            ---@overload fun(x: integer)
            ---@param a string
            local function h(a) end

            h(1, 2)
        "#
        ));
    }

    /// A closure argument meets the parameter the shared receiver rule (`call_arg_param_index`)
    /// gives it (#76). The colon/dot offsets were the param-to-argument ones used backwards, so a
    /// colon call to a dot-defined function, and a dot call to a colon-defined method, took the
    /// callback's type from no parameter, and its extra parameter went unreported.
    #[test]
    fn test_closure_arg_meets_its_param_through_the_receiver_rule() {
        let source = r#"
            ---@class M
            local M = {}

            ---@param cb fun(a: integer)
            function M:colon_def(cb) end

            ---@param o M
            ---@param cb fun(a: integer)
            function M.dot_def(o, cb) end

            ---@param self M
            ---@param cb fun(a: integer)
            function M.named_self(self, cb) end

            ---@param x self
            ---@param cb fun(a: integer)
            function M.self_typed(x, cb) end

            M:colon_def(function(a, b1) end)
            M.colon_def(M, function(a, b2) end)
            M:dot_def(function(a, b3) end)
            M.dot_def(M, function(a, b4) end)
            M:named_self(function(a, b5) end)
            M.named_self(M, function(a, b6) end)
            M:self_typed(function(a, b7) end)
            M.self_typed(M, function(a, b8) end)
        "#;
        let diags = check_source(source);
        let mut extra = texts_by_code(source, &diags, DiagnosticCode::RedundantParameter);
        extra.sort();
        assert_eq!(
            extra,
            vec!["b1", "b2", "b3", "b4", "b5", "b6", "b7", "b8"],
            "{diags:#?}"
        );
    }

    /// `string.gsub` calls a function `repl` with one argument per capture, so a callback taking
    /// two captures is not redundant in either call style (std typed `repl` as one-param
    /// `fun(param: string)`; #76 made the colon call read it too: blink.cmp, compat53).
    #[test]
    fn test_gsub_callback_takes_every_capture() {
        // `string` is std's: check_source has none.
        let mut ws = VirtualWorkspace::new_with_init_std_lib();
        for call in [
            r#"string.gsub("ab", "(a)(b)", function(x, y) return x .. y end)"#,
            r#"("ab"):gsub("(a)(b)", function(x, y) return x .. y end)"#,
        ] {
            assert!(
                ws.has_no_diagnostic(DiagnosticCode::RedundantParameter, call),
                "{call}"
            );
        }
        // ... and every callback `repl` accepted before still type-checks (the corpus shapes of
        // a first, all-variadic `fun(...: string)`: snacks, plenary, sidekick, lspconfig).
        for call in [
            r#"("ab"):gsub("a", function(v) return v end)"#,
            r#"("ab"):gsub("a", string.upper)"#,
            r#"string.gsub("ab", "a", string.upper)"#,
            r#"local r = ("ab"):gsub("a", function(...) return ... end)"#,
            "local r = (\"ab\"):gsub(\"a\",\n---@param v string\nfunction(v) return v end)",
        ] {
            assert!(
                ws.has_no_diagnostic(DiagnosticCode::ParamTypeMismatch, call),
                "{call}"
            );
        }
    }
}
