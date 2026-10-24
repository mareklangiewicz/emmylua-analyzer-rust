#[cfg(test)]
mod test {
    use crate::{DiagnosticCode, VirtualWorkspace};

    #[test]
    fn test_pcall_return() {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();

        ws.def(
            r#"
        a, b = pcall(string.rep, "a", 1000000000)
        "#,
        );

        let ty = ws.expr_ty("b");
        let expected = ws.ty("string");
        assert_eq!(ty, expected);
    }

    #[test]
    fn test_unpack_return() {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();

        ws.def(
            r#"
        a, b, c = table.unpack({1, 2, 3})
        "#,
        );

        let a_ty = ws.expr_ty("a");
        let b_ty = ws.expr_ty("b");
        let c_ty = ws.expr_ty("c");
        let a_expected = ws.expr_ty("1");
        let b_expected = ws.expr_ty("2");
        let c_expected = ws.expr_ty("3");
        assert_eq!(a_ty, a_expected);
        assert_eq!(b_ty, b_expected);
        assert_eq!(c_ty, c_expected);
    }

    #[test]
    fn test_assert_return() {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();

        ws.def(
            r#"
        ---@return string?
        ---@return string?
        function cwd() end

        a, b = assert(cwd())
        "#,
        );

        let a_ty = ws.expr_ty("a");
        let b_ty = ws.expr_ty("b");
        let a_expected = ws.ty("string");
        let b_expected = ws.ty("string?");
        assert_eq!(a_ty, a_expected);
        assert_eq!(b_ty, b_expected);
    }

    #[test]
    fn test_issue_237() {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();

        assert!(ws.has_no_diagnostic(
            DiagnosticCode::UnbalancedAssignments,
            r#"
        local fmt = ""
        local scol, ecol, match, key, time_fmt = fmt:find('(<([^:>]+):?([^>]*)>)')
        "#,
        ));
    }

    #[test]
    fn test_issue_244() {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();
        ws.def(
            r#"
        ---@return string
        local function foo()
            return "ok"
        end
        ok, err = pcall(foo)
        "#,
        );

        let err_ty = ws.expr_ty("err");
        let expected = ws.ty("string");
        assert_eq!(err_ty, expected);
    }

    #[test]
    fn test_issue_342() {
        let mut ws = VirtualWorkspace::new();
        ws.def(
            r#"
        function bar()
            local a, b = foo()
            e = b
        end

        --- @return string, integer
        function foo() end
        "#,
        );

        let e_ty = ws.expr_ty("e");
        let expected = ws.ty("integer");
        assert_eq!(e_ty, expected);
    }

    /// An empty table literal is an empty array: passing `{}` (or a local still holding `{}`) to a
    /// `number[]` / `T[]` parameter keeps the call's return types, slot by slot.
    #[test]
    fn test_empty_table_to_array_param_keeps_returns() {
        let mut ws = VirtualWorkspace::new();
        ws.def(
            r#"
        ---@param xs number[]
        ---@return string text
        ---@return number median
        local function spread(xs) return 'a', 1 end

        ---@generic T
        ---@param xs T[]
        ---@return string text
        ---@return number count
        local function count(xs) return 'a', 1 end

        local t = {}
        local a1, b1 = spread({})
        local a2, b2 = spread(t)
        local a3, b3 = count({})
        A1, B1, A2, B2, A3, B3 = a1, b1, a2, b2, a3, b3
        "#,
        );

        let string = ws.ty("string");
        let number = ws.ty("number");
        for (a, b) in [("A1", "B1"), ("A2", "B2"), ("A3", "B3")] {
            assert_eq!(ws.expr_ty(a), string, "{a}");
            assert_eq!(ws.expr_ty(b), number, "{b}");
        }
    }

    /// `{}` says nothing about an element type: a `V[]` parameter given it binds `V` to `any`,
    /// not `unknown` (which would type every later callback parameter `unknown`).
    #[test]
    fn test_empty_table_binds_array_element_to_any() {
        let mut ws = VirtualWorkspace::new();
        ws.def(
            r#"
        ---@class Box<T>
        ---@overload fun<V>(src: V[]): Box<V>
        local Box = {}

        B = Box({})
        "#,
        );

        assert_eq!(ws.expr_ty("B"), ws.ty("Box<any>"));
    }
}
