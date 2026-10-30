#[cfg(test)]
mod tests {
    use crate::{DiagnosticCode, LuaType, LuaUnionType, VirtualWorkspace};

    #[test]
    fn test_issue_231() {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::AssignTypeMismatch,
            r#"

            --- @type [boolean, string]
            local ret = { coroutine.resume(coroutine.create(function () end), ...) }
            "#
        ));
    }

    #[test]
    fn test_union_tuple() {
        let mut ws = VirtualWorkspace::new();
        ws.def(
            r#"
                local Pos = {
                    [1] = {
                        { 36,  777 },
                    },
                    [2] = {
                        { 826, 244 },
                    },
                }
                ---@type int
                local cur
                ---@type int
                local index

                local points = Pos[cur]
                ---@cast points -?
                local point = points[index] ---@cast point -?
                A = point[1]

            "#,
        );
        let ty = ws.expr_ty("A");
        let expected_ty = LuaType::Union(
            LuaUnionType::from_vec(vec![LuaType::IntegerConst(36), LuaType::IntegerConst(826)])
                .into(),
        );
        assert_eq!(ty, expected_ty);
    }

    #[test]
    fn test_issue_595() {
        let mut ws = VirtualWorkspace::new();
        ws.has_no_diagnostic(
            DiagnosticCode::AssignTypeMismatch,
            r#"
                local ret           --- @type [integer?]
                local h = ret[#ret] -- type is integer??
                if h then
                    --- @type integer
                    local _ = h
                end
            "#,
        );
    }

    #[test]
    fn test_declared_tuple_index() {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();
        ws.def(
            r#"
            ---@return [integer, string]
            local function pos() end

            ---@param t [integer, string]
            local function f(t)
                A = t[1]
                B = t[2]
            end

            local p = pos()
            C = p[1]
            D = pos()[2]
            "#,
        );

        assert_eq!(ws.expr_ty("A"), LuaType::Integer);
        assert_eq!(ws.expr_ty("B"), LuaType::String);
        assert_eq!(ws.expr_ty("C"), LuaType::Integer);
        assert_eq!(ws.expr_ty("D"), LuaType::String);
    }
}
