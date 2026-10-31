#[cfg(test)]
mod tests {
    use crate::DiagnosticCode;

    use crate::check::test::{check_source, count_by_code};

    #[test]
    fn test_redefined_local_in_same_block() {
        let diags = check_source("local a = 1\nlocal a = 2\nprint(a)");
        assert_eq!(count_by_code(&diags, DiagnosticCode::RedefinedLocal), 1);
    }

    #[test]
    fn test_redefined_in_nested_scope() {
        // Outer `a`, inner `a` declared again → treated as redefinition.
        let diags = check_source("local a = 1\nif true then\n  local a = 2\nend\nprint(a)");
        assert_eq!(count_by_code(&diags, DiagnosticCode::RedefinedLocal), 1);
    }

    #[test]
    fn test_distinct_names_ok() {
        let diags = check_source("local a = 1\nlocal b = 2\nprint(a + b)");
        assert_eq!(count_by_code(&diags, DiagnosticCode::RedefinedLocal), 0);
    }

    #[test]
    fn test_later_local_does_not_shadow_an_earlier_body() {
        // `need` the local function is declared AFTER `gate`, so `gate`'s parameter shadows nothing.
        let diags = check_source(
            "local function gate(need) return need end\nlocal function need() return 1 end\nreturn gate(need())\n",
        );
        assert_eq!(
            count_by_code(&diags, DiagnosticCode::RedefinedLocal),
            0,
            "{diags:?}"
        );
        // Control: declared before, it is shadowed.
        let diags = check_source(
            "local function need() return 1 end\nlocal function gate(need) return need end\nreturn gate(need())\n",
        );
        assert_eq!(
            count_by_code(&diags, DiagnosticCode::RedefinedLocal),
            1,
            "{diags:?}"
        );
    }
}
