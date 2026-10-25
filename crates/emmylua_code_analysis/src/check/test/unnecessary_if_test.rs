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
}
