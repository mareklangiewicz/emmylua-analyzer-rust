#[cfg(test)]
mod test {
    use crate::{DiagnosticCode, LuaType, VirtualWorkspace};

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

    /// A member assigned a call holds the call's FIRST value (`M.ok = detect()`), as a local does:
    /// the member's own type, which `t.x = v` checks are made against, is not the whole list.
    #[test]
    fn test_member_assigned_call_holds_first_value() {
        use emmylua_parser::{LuaAstNode, LuaIndexExpr};

        let mut ws = VirtualWorkspace::new();
        let file_id = ws.def(
            r#"
        ---@return boolean ok
        ---@return string why
        local function detect() end

        ---@param n number
        ---@return boolean ok
        ---@return string why
        local function probe(n) end

        local M = {}
        M.ok = detect()
        M.bad = probe('x')
        "#,
        );
        let model = ws.analysis.semantic_model(file_id);
        // `M.bad`'s call matches no candidate: the VM gives up and the database types the member.
        for index_expr in model.chunk().expect("chunk").descendants::<LuaIndexExpr>() {
            let member_id = model
                .resolve_member(&index_expr)
                .and_then(|resolved| resolved.member_id)
                .expect("member");
            assert_eq!(
                model.type_of_member(&member_id),
                Some(LuaType::Boolean),
                "{}",
                index_expr.syntax().text()
            );
        }
    }
}
