#[cfg(test)]
mod test {
    use crate::DiagnosticCode;
    use crate::VirtualWorkspace;

    #[test]
    fn test_feat_724() {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();

        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::PreferredLocalAlias,
            r#"
            local gsub = string.gsub
            print(string.gsub("hello", "l", "0"))
            "#,
        ));

        assert!(ws.has_no_diagnostic(
            DiagnosticCode::PreferredLocalAlias,
            r#"
            local t = {
                a = ""
            }
            local h = t.a
            t.a = 'h'
            print(t.a)
            "#,
        ));
    }

    #[test]
    fn test_function_stat_redefining_the_field_is_not_an_alias_use() {
        let mut ws = VirtualWorkspace::new();
        // `function s.next()` replaces `s.next`: it cannot be spelled through the alias of the old one.
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::PreferredLocalAlias,
            "---@param s table\nlocal function f(s)\n  local pull = s.next\n  function s.next(self) return pull(self) end\n  return s\nend\nreturn f\n"
        ));
        // Control: a READ of the aliased field still prefers the alias.
        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::PreferredLocalAlias,
            "---@param s table\nlocal function f(s)\n  local pull = s.next\n  return pull, s.next\nend\nreturn f\n"
        ));
    }
}
