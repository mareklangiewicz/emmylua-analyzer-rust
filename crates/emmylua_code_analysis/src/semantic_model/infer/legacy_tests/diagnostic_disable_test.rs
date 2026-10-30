#[cfg(test)]
mod test {
    use crate::{DiagnosticCode, VirtualWorkspace};

    #[test]
    fn test_disable_nextline() {
        let mut ws = VirtualWorkspace::new();

        assert!(ws.has_no_diagnostic(
            DiagnosticCode::SyntaxError,
            r#"
        ---@diagnostic disable-next-line: syntax-error
        ---@param
        local function f() end
        "#,
        ));
    }

    #[test]
    fn test_file_disable_all() {
        let mut ws = VirtualWorkspace::new();

        assert!(ws.has_no_diagnostic(
            DiagnosticCode::AssignTypeMismatch,
            r#"
        ---@diagnostic disable -- vendored file
        ---@type string
        local x = 1
        "#,
        ));
    }
}
