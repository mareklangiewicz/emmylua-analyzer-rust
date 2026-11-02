#[cfg(test)]
mod test {
    use crate::{DiagnosticCode, Emmyrc, VirtualWorkspace};

    #[test]
    fn test_1() {
        let mut ws = VirtualWorkspace::new();
        let mut config = Emmyrc::default();
        config.strict.array_index = true;
        ws.analysis.update_config(config.into());

        assert!(ws.has_no_diagnostic(
            DiagnosticCode::UnnecessaryAssert,
            r#"
            ---@type boolean
            local b
            assert(b)

            ---@type any
            local c
            assert(c)

            ---@type unknown
            local d
            assert(d)

            ---@type boolean
            local e
            assert(e)

            ---@type number?
            local f
            assert(f)

            ---@type integer[]
            local ints = {1, 2}
            assert(ints[3])

            ---@type [integer, integer]
            local enum = {1, 2}
            assert(enum[3])
        "#
        ));
    }

    #[test]
    fn test_2() {
        let mut ws = VirtualWorkspace::new();

        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::UnnecessaryAssert,
            r#"
            assert(true)
        "#
        ));

        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::UnnecessaryAssert,
            r#"
            ---@return integer
            local function hi()
              return 1
            end
            assert(hi(1))
            "#
        ));

        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::UnnecessaryAssert,
            r#"
            assert({}, 'hi')
            "#
        ));

        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::UnnecessaryAssert,
            r#"
            ---@type [integer, integer]
            local enum = {1, 2}
            assert(enum[2])
            "#
        ));
    }

    #[test]
    fn test_impossible_assert() {
        let mut ws = VirtualWorkspace::new();

        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::UnnecessaryAssert,
            r#"
            assert(false)
            "#
        ));

        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::UnnecessaryAssert,
            r#"
            assert(nil)
            "#
        ));

        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::UnnecessaryAssert,
            r#"
            assert(nil and 5)
            "#
        ));

        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::UnnecessaryAssert,
            r#"
            local a = false ---@type false
            assert(a)
            "#
        ));

        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::UnnecessaryAssert,
            r#"
            ---@type integer[]
            local a = {1,2,3}
            local b = a[2] ---@type integer|nil
            if not b then
              assert(b, "No second element!")
            end
            "#
        ));
    }

    /// A method's own `...` after fixed params is not the field's first slots: `init(issue, ...)`
    /// overriding `fun(self: Obj, ...: any)` keeps a vararg of any length, so
    /// `select('#', ...) == 0` can hold.
    #[test]
    fn test_override_vararg_keeps_its_length_open() {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();
        let code = r#"
            ---@class Obj
            ---@field init fun(self: Obj, ...: any) ctor
            local Obj = {}

            ---@class Ask : Obj
            local Ask = setmetatable({}, { __index = Obj })

            DOC
            function Ask:init(issue, ...)
                assert(select('#', ...) == 0, 'no extra')
                self.issue = issue
            end
            return Ask
        "#;
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::UnnecessaryAssert,
            &code.replace(
                "DOC",
                "---@param issue string what\n---@param ... any must be empty"
            )
        ));
        assert!(ws.has_no_diagnostic(DiagnosticCode::UnnecessaryAssert, &code.replace("DOC", "")));
    }
}
