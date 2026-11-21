#[cfg(test)]
mod test {
    use crate::DiagnosticCode;
    use crate::VirtualWorkspace;

    // 函数字段及其别名都必须拒绝把异步函数传给同步回调参数.
    #[test]
    fn test_doc_projection_sync_callback_field() {
        for field_type in ["fun(callback: sync fun())", "Run"] {
            for (annotation, accepted) in [("---@async", false), ("", true)] {
                let mut ws = VirtualWorkspace::new();
                assert_eq!(
                    ws.has_no_diagnostic(
                        DiagnosticCode::AwaitInSync,
                        &format!(
                            r#"
                            ---@alias Run fun(callback: sync fun())
                            ---@class Runner
                            ---@field run {field_type}
                            ---@type Runner
                            local runner
                            {annotation}
                            local function callback() end
                            runner.run(callback)
                            "#,
                        ),
                    ),
                    accepted,
                    "{field_type}, {annotation}",
                );
            }
        }
    }

    #[test]
    fn test_await_in_sync() {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();

        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::AwaitInSync,
            r#"
        local function name(callback)

        end

        name(function()
            local a = coroutine.yield(1)
        end)
        "#
        ));

        assert!(ws.has_no_diagnostic(
            DiagnosticCode::AwaitInSync,
            r#"
        local function name(callback)

        end

        ---@async
        name(function()
            local a = coroutine.yield(1)
        end)
        "#
        ));

        assert!(ws.has_no_diagnostic(
            DiagnosticCode::AwaitInSync,
            r#"
        ---@param callback async fun()
        local function name(callback)

        end

        name(function()
            local a = coroutine.yield(1)
        end)
        "#
        ));

        assert!(ws.has_no_diagnostic(
            DiagnosticCode::AwaitInSync,
            r#"
            ---@generic T, R
            ---@param call async fun(...: T...): R...
            ---@return async fun(...: T...): R...
            local function name(call)

            end

            local d = name(function()
                local a = coroutine.yield(1)
            end)
            "#
        ));

        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::AwaitInSync,
            r#"
        ---@param callback async fun()
        local function name(callback)
            callback()
        end
        "#
        ));
    }

    #[test]
    fn test_issue_99() {
        let mut ws = VirtualWorkspace::new();

        assert!(ws.has_no_diagnostic(
            DiagnosticCode::AwaitInSync,
            r#"
        ---@async
        local function foo()

        end

        ---@async
        return function()
            foo()
        end
        "#
        ));
    }

    #[test]
    fn test_issue_101() {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();

        ws.def(
            r#"
        --- @async
        function bar() end
        "#,
        );

        assert!(ws.has_no_diagnostic(
            DiagnosticCode::AwaitInSync,
            r#"
        --- @async
        function foo()
            pcall(function()
                bar() -- EXPECTED OK
            end)
        end
        "#
        ));

        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::AwaitInSync,
            r#"
        function baz()
            pcall(bar)
        end
        "#
        ));
    }

    #[test]
    fn test_issue_161() {
        let mut ws = VirtualWorkspace::new();
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::AwaitInSync,
            r#"
        local function create(_f) end

        local _a = create(
        --- @async
        function()
            coroutine.yield()
        end
        )

        --- @async
        local _b = create(function()
        coroutine.yield()
        end)
        "#
        ));
    }

    #[test]
    fn test_issue_721() {
        let mut ws = VirtualWorkspace::new();
        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::AwaitInSync,
            r#"
            --- @param f sync fun()
            local function run(f) end

            --- @async
            local function async_fn() end

            run(async_fn) -- error required
        "#
        ));
    }

    /// A closure takes async-ness from the type its context expects: a parameter reached through a
    /// member alias (`J.run = M.run`), or a table field whose table is typed by `---@type` or by the
    /// parameter it is passed to.
    #[test]
    fn test_closure_async_from_expected_type() {
        let pre = r#"
            ---@async
            local function aw() end
            local M = {}
            ---@param fn async fun(): any?
            function M.run(fn) end
            local J = {}
            J.run = M.run
            ---@class AsyncField
            ---@field call async fun(a: string): any
            ---@class SyncField
            ---@field call fun(a: string): any
            ---@param c AsyncField
            local function use(c) end
            ---@param c SyncField
            local function use_sync(c) end
        "#;
        let ok = |code: &str| {
            let mut ws = VirtualWorkspace::new_with_init_std_lib();
            ws.has_no_diagnostic(DiagnosticCode::AwaitInSync, &format!("{pre}\n{code}"))
        };
        assert!(ok("J.run(function() aw() end)"));
        assert!(ok("use({ call = function(a) aw() end })"));
        assert!(ok(
            "---@type AsyncField\nlocal c = { call = function(a) aw() end }"
        ));
        assert!(!ok("use_sync({ call = function(a) aw() end })"));
        assert!(!ok(
            "---@type SyncField\nlocal c = { call = function(a) aw() end }"
        ));
    }

    /// #58: an expected type spelled through an alias of an `async fun` makes the closure async
    /// just like the inline spelling: as a parameter, an optional parameter (`Alias|nil`) and a
    /// table field. An alias of a plain `fun` keeps the closure sync.
    #[test]
    fn test_closure_async_from_alias_expected_type() {
        let pre = r#"
            ---@async
            local function aw() end
            ---@alias AsyncFn async fun(x: integer)
            ---@alias PlainFn fun(x: integer)
            ---@param fn AsyncFn
            local function run(fn) end
            ---@param fn? AsyncFn
            local function run_opt(fn) end
            ---@param fn PlainFn
            local function run_plain(fn) end
            ---@class AliasField
            ---@field call AsyncFn
            ---@param c AliasField
            local function use(c) end
        "#;
        let ok = |code: &str| {
            let mut ws = VirtualWorkspace::new_with_init_std_lib();
            ws.has_no_diagnostic(DiagnosticCode::AwaitInSync, &format!("{pre}\n{code}"))
        };
        assert!(ok("run(function(x) aw() end)"));
        assert!(ok("run_opt(function(x) aw() end)"));
        assert!(ok("use({ call = function(x) aw() end })"));
        assert!(!ok("run_plain(function(x) aw() end)"));
    }

    /// A closure passed for an `async fun()` parameter is async whatever the call style: the
    /// argument meets the parameter the shared receiver rule (`call_arg_param_index`) gives it
    /// (#76). The colon/dot offsets were the param-to-argument ones used backwards, so three
    /// cells reported `await-in-sync` inside an async callback.
    #[test]
    fn test_closure_arg_async_param_through_the_receiver_rule() {
        let source = r#"
            ---@class N
            local N = {}

            ---@async
            local function w1() end
            ---@async
            local function w2() end
            ---@async
            local function w3() end
            ---@async
            local function w4() end
            ---@async
            local function w5() end
            ---@async
            local function w6() end

            ---@param cb async fun()
            function N:colon_def(cb) end

            ---@param o N
            ---@param cb async fun()
            function N.dot_def(o, cb) end

            ---@param x self
            ---@param cb async fun()
            function N.self_typed(x, cb) end

            N:colon_def(function() w1() end)
            N.colon_def(N, function() w2() end)
            N:dot_def(function() w3() end)
            N.dot_def(N, function() w4() end)
            N:self_typed(function() w5() end)
            N.self_typed(N, function() w6() end)
        "#;
        let diags = crate::check::test::check_source(source);
        let reported =
            crate::check::test::texts_by_code(source, &diags, DiagnosticCode::AwaitInSync);
        assert_eq!(reported, Vec::<&str>::new(), "{diags:#?}");
    }

    /// An async function passed for a `sync fun()` parameter is reported in every call style,
    /// a colon-defined method whose docs also name `self` included: the argument meets the
    /// parameter `call_arg_param_index` gives it (0.25.1, upstream and the fork all agree).
    #[test]
    fn test_async_arg_to_sync_param_in_every_call_style() {
        let source = r#"
            ---@class Q
            local Q = {}

            ---@param cb sync fun()
            function Q:colon_def(cb) end

            ---@param o Q
            ---@param cb sync fun()
            function Q.plain(o, cb) end

            ---@param x self
            ---@param cb sync fun()
            function Q.self_typed(x, cb) end

            ---@param self Q
            ---@param cb sync fun()
            function Q:explicit_self(cb) end

            ---@async
            local function a1() end
            ---@async
            local function a2() end
            ---@async
            local function a3() end
            ---@async
            local function a4() end
            ---@async
            local function a5() end
            ---@async
            local function a6() end
            ---@async
            local function a7() end
            ---@async
            local function a8() end

            Q:colon_def(a1)
            Q.colon_def(Q, a2)
            Q:plain(a3)
            Q.plain(Q, a4)
            Q:self_typed(a5)
            Q.self_typed(Q, a6)
            Q:explicit_self(a7)
            Q.explicit_self(Q, a8)
        "#;
        let diags = crate::check::test::check_source(source);
        let mut reported =
            crate::check::test::texts_by_code(source, &diags, DiagnosticCode::AwaitInSync);
        reported.sort();
        assert_eq!(
            reported,
            vec!["a1", "a2", "a3", "a4", "a5", "a6", "a7", "a8"],
            "{diags:#?}"
        );
    }
}
