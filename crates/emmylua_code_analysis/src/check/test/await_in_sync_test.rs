#[cfg(test)]
mod test {
    use crate::DiagnosticCode;
    use crate::VirtualWorkspace;

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
}
