#[cfg(test)]
mod tests {

    use crate::{DiagnosticCode, VirtualWorkspace};

    #[test]
    fn test_290() {
        let mut ws = VirtualWorkspace::new();
        ws.enable_full_diagnostic();

        assert!(ws.has_no_diagnostic(
            DiagnosticCode::IncompleteSignatureDoc,
            r#"
                ---@param a string
                local function foo(_, a)
                    _ = a
                end
            "#
        ));
    }

    #[test]
    fn test_return() {
        let mut ws = VirtualWorkspace::new();
        ws.enable_full_diagnostic();

        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::IncompleteSignatureDoc,
            r#"
            local c = function(x, y)
                return x + y
            end
            "#
        ));

        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::IncompleteSignatureDoc,
            r#"
            local function do_add(x, y)
                return x + y
            end
            "#
        ));

        assert!(ws.has_no_diagnostic(
            DiagnosticCode::IncompleteSignatureDoc,
            r#"
            local function noop()
            end
            "#
        ));

        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::IncompleteSignatureDoc,
            r#"
            ---@param p number
            local function FLPR3(p, e)
                return 0
            end
            "#
        ));

        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::IncompleteSignatureDoc,
            r#"
            ---@param p number
            local function FLPR3(p)
                return 0
            end
            "#
        ));

        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::IncompleteSignatureDoc,
            r#"
            --- function without param signature
            local function FLPR3(p)
                return 0
            end
            "#
        ));

        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::IncompleteSignatureDoc,
            r#"

                ---@class Test
                local Test = {}

                ---@param test Test
                function Test:add(test, c)
                end
            "#
        ));
    }

    #[test]
    fn test_return_overload() {
        let mut ws = VirtualWorkspace::new();
        ws.enable_full_diagnostic();

        assert!(ws.has_no_diagnostic(
            DiagnosticCode::IncompleteSignatureDoc,
            r#"
            ---@return_overload true, integer
            ---@return_overload false, string
            local function f()
                return true, 1
            end
            "#
        ));

        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::IncompleteSignatureDoc,
            r#"
            ---@return_overload true, integer
            ---@return_overload false, string
            local function f()
                return true, 1, "extra"
            end
            "#
        ));
    }

    #[test]
    fn test_variadic_return_overload_does_not_trigger_incomplete_signature_doc() {
        let mut ws = VirtualWorkspace::new();
        ws.enable_full_diagnostic();

        assert!(ws.has_no_diagnostic(
            DiagnosticCode::IncompleteSignatureDoc,
            r#"
            ---@return_overload true, integer...
            ---@return_overload false, string
            local function f()
                return true, 1, 2, 3, 4
            end
            "#
        ));
    }

    #[test]
    fn test_global() {
        let mut ws = VirtualWorkspace::new();
        ws.enable_full_diagnostic();

        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::MissingGlobalDoc,
            r#"
                function FLPR1()
                end
            "#
        ));

        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::MissingGlobalDoc,
            r#"
                ---
                function FLPR1(a)
                end
            "#
        ));
        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::MissingGlobalDoc,
            r#"
                ---
                function FLPR1()
                    return 1
                end
            "#
        ));

        assert!(ws.has_no_diagnostic(
            DiagnosticCode::MissingGlobalDoc,
            r#"
                ---
                function FLPR2()
                end
            "#
        ));
    }

    #[test]
    fn test_incomplete_signature_doc_no_param_no_return() {
        let mut ws = VirtualWorkspace::new();
        ws.enable_full_diagnostic();

        assert!(ws.has_no_diagnostic(
            DiagnosticCode::IncompleteSignatureDoc,
            r#"
                local function a()
                end
            "#
        ));
    }

    #[test]
    fn test_nested_closure_returns_do_not_count_for_outer() {
        let mut ws = VirtualWorkspace::new();
        ws.enable_full_diagnostic();

        // `outer` has no params and returns nothing itself: the documented inner function's return is not its own.
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::IncompleteSignatureDoc,
            r#"
                local function outer()
                    ---@return integer
                    local function inner()
                        return 1
                    end
                    inner()
                end
            "#
        ));

        // `outer` documents its one return; the inner function's two returns are not checked against it.
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::IncompleteSignatureDoc,
            r#"
                ---@return integer
                local function outer()
                    ---@return integer
                    ---@return integer
                    local function inner()
                        return 1, 2
                    end
                    return (inner())
                end
            "#
        ));
    }
}
