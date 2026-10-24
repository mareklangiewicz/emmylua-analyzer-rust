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
}
