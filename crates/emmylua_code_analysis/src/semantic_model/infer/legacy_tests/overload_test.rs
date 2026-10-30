#[cfg(test)]
mod test {

    use crate::{DiagnosticCode, VirtualWorkspace};

    #[test]
    fn test_table() {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();

        assert!(ws.has_no_diagnostic(
            DiagnosticCode::ParamTypeMismatch,
            r#"
        table.concat({'', ''}, ' ')
        "#
        ));
    }

    #[test]
    fn test_sub_string() {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();

        assert!(ws.has_no_diagnostic(
            DiagnosticCode::MissingParameter,
            r#"
        local t = ("m2"):sub(1)
        "#
        ));
    }

    #[test]
    fn test_class_default_constructor() {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();
        ws.def(
            r#"

            ---@generic T
            ---@[constructor("__init")]
            ---@param name `T`
            ---@return T
            function meta(name)
            end
        "#,
        );

        ws.def(
            r#"
        ---@class MyClass
        local M = meta("MyClass")

        function M:__init()
        end

        A = M()
        "#,
        );

        let ty = ws.expr_ty("A");
        let expected = ws.ty("MyClass");
        assert_eq!(ws.humanize_type(ty), ws.humanize_type(expected));
    }

    #[test]
    fn test_issue_770() {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::RedundantParameter,
            r#"
        local table = {1,2}
        if next(table, 2) == '2' then
            print('ok')
        end
        "#
        ));
    }

    #[test]
    fn test_overload_that_cannot_take_the_args_loses() {
        // `fun()` takes no args; the main signature takes `'end'` and only misses the
        // optional `offset`, so its return wins.
        let mut ws = VirtualWorkspace::new();
        ws.def(
            r#"
            ---@class Fh
            local Fh = {}
            ---@overload fun()
            ---@param whence? string
            ---@param offset? integer
            ---@return integer?
            function Fh:seek(whence, offset) end
            ---@type Fh
            F = {}
            "#,
        );
        let ty = ws.expr_ty("F:seek('end')");
        assert_eq!(ws.humanize_type(ty), "integer?");

        let mut ws = VirtualWorkspace::new_with_init_std_lib();
        ws.def("F = assert(io.open('x'))");
        let ty = ws.expr_ty("F:seek('end')");
        assert_eq!(ws.humanize_type(ty), "integer?");
    }

    #[test]
    fn test_overload_with_alias_variadic_param_takes_the_rest() {
        // `xs: Ints` expands to `integer...`, so it takes all three args. Counted as one fixed
        // param, it would lose to the bare main signature `fun(...)`, which returns `unknown`.
        let mut ws = VirtualWorkspace::new();
        ws.def(
            r#"
            ---@alias Ints integer...
            ---@overload fun(a?: integer, b?: integer): string
            ---@overload fun(xs: Ints): boolean
            local function k(...) end
            E = k(1, 2, 3)
            "#,
        );
        let ty = ws.expr_ty("E");
        assert_eq!(ws.humanize_type(ty), "boolean");
    }

    #[test]
    fn test_table_literal_fits_a_map_member_of_a_union_param() {
        // `{ sync = true }` fits the `table<string, boolean>` member, so the 2-arg call keeps
        // the main signature instead of falling to the 1-arg overload (or to nothing).
        let mut ws = VirtualWorkspace::new();
        ws.def(
            r#"
            ---@param line string?
            ---@param bare? table<string, boolean>|true
            ---@return integer
            ---@overload fun(line: string?): string
            local function f(line, bare) end
            ---@param bare table<string, boolean>|true
            ---@return integer
            local function g(bare) end
            A = f('x', { sync = true })
            B = g({ sync = true })
            C = f('x')
            "#,
        );
        let ty = ws.expr_ty("A");
        assert_eq!(ws.humanize_type(ty), "integer");
        let ty = ws.expr_ty("B");
        assert_eq!(ws.humanize_type(ty), "integer");
        let ty = ws.expr_ty("C");
        assert_eq!(ws.humanize_type(ty), "string");
    }
}
