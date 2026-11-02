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

    #[test]
    fn test_inline_closures_are_not_declarations() {
        let mut ws = VirtualWorkspace::new();
        ws.enable_full_diagnostic();

        // A closure passed as an argument, stored in a table constructor or returned has no declaration
        // to carry a doc comment; only `function`/`local function` statements and the values of
        // local/assign statements are checked.
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::IncompleteSignatureDoc,
            r#"
                pcall(function(a) return a end)
                local handlers = { on_line = function(line) return line end }
                local mt = setmetatable({}, { __call = function(_, x) return x end })
                ---@return fun(o: any): any
                local function make()
                    return function(o) return o end
                end
                return handlers, mt, make
            "#
        ));

        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::IncompleteSignatureDoc,
            r#"
                local t = {}
                t.m = function(x) return x end
            "#
        ));
    }

    /// A closure that implements a declared field (`self.next = function(s)`, a subclass
    /// overriding a `---@field init fun(...)`) is documented by that field (0.25.1 parity).
    #[test]
    fn test_declared_member_implementation_needs_no_comment() {
        let mut ws = VirtualWorkspace::new();
        ws.enable_full_diagnostic();

        assert!(ws.has_no_diagnostic(
            DiagnosticCode::IncompleteSignatureDoc,
            r#"
                ---@class Box
                ---@field next fun(s: any): any the pull
                local Box = {}

                ---@return Box self it
                function Box:wrap()
                    local next = self.next
                    self.next = function(s)
                        return next(s)
                    end
                    return self
                end
            "#
        ));
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::IncompleteSignatureDoc,
            r#"
                ---@class Base
                ---@field init fun(self: Base, name?: string) ctor
                local Base = {}
                ---@class Sub : Base
                local Sub = {}
                function Sub:init(name) self.name = name end
            "#
        ));
        // Control: an undeclared member of a class still needs its comment.
        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::IncompleteSignatureDoc,
            r#"
                ---@class Other
                local Other = {}
                function Other:make(name) return name end
            "#
        ));
    }

    /// A plain description comment is a comment: the `@param`/`@return` checks run instead of
    /// "Missing comment" (0.25.1 parity).
    #[test]
    fn test_description_only_comment_counts() {
        let mut ws = VirtualWorkspace::new();
        ws.enable_full_diagnostic();

        assert!(ws.has_no_diagnostic(
            DiagnosticCode::IncompleteSignatureDoc,
            r#"
                local M = {}
                ---Return all arguments unchanged.
                function M.id(...) return ... end
                return M
            "#
        ));
        // Control: a named param still needs its `@param` under a plain comment.
        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::IncompleteSignatureDoc,
            r#"
                local M = {}
                ---Double it.
                function M.twice(x) return x end
                return M
            "#
        ));
    }

    /// A trailing `...` may be zero values, so `return x, ...` fits one `@return`.
    #[test]
    fn test_trailing_vararg_return_counts_its_minimum() {
        let mut ws = VirtualWorkspace::new();
        ws.enable_full_diagnostic();

        assert!(ws.has_no_diagnostic(
            DiagnosticCode::IncompleteSignatureDoc,
            r#"
                ---@param x integer the x
                ---@return integer x it
                local function step(x, ...)
                    return x, ...
                end
                return step
            "#
        ));
        // Control: a fixed second value still needs its `@return`.
        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::IncompleteSignatureDoc,
            r#"
                ---@param x integer the x
                ---@return integer x it
                local function step2(x)
                    return x, x
                end
                return step2
            "#
        ));
    }

    /// Without a comment, a non-global function is reported only when its signature cannot be
    /// inferred: a parameterless helper with returns is fine (0.25.1 parity).
    #[test]
    fn test_inferable_undocumented_local_needs_no_comment() {
        let mut ws = VirtualWorkspace::new();
        ws.enable_full_diagnostic();

        assert!(ws.has_no_diagnostic(
            DiagnosticCode::IncompleteSignatureDoc,
            r#"
                local n = 0
                local function fire()
                    n = n + 1
                    return n
                end
                return fire
            "#
        ));
    }

    /// A trailing comment of an earlier statement is not the function's comment.
    #[test]
    fn test_trailing_comment_of_previous_statement_is_not_attached() {
        let mut ws = VirtualWorkspace::new();
        ws.enable_full_diagnostic();

        assert!(ws.has_no_diagnostic(
            DiagnosticCode::IncompleteSignatureDoc,
            r#"
                ---@class E
                local E = {}
                ---@param self E it
                function E.throw(self) error(self) end -- error() takes any value

                function E:loc() return 'here' end
                return E
            "#
        ));
    }

    /// An override's parameters past a declared `fun(self: Base, ...: any)` field's fixed ones
    /// take the vararg's type, so `function Sub:init(name, id)` is typed by the field.
    #[test]
    fn test_override_params_typed_by_declared_vararg() {
        let mut ws = VirtualWorkspace::new();
        ws.enable_full_diagnostic();

        assert!(ws.has_no_diagnostic(
            DiagnosticCode::IncompleteSignatureDoc,
            r#"
                ---@class Base
                ---@field init fun(self: Base, ...: any) ctor
                local Base = {}
                ---@class Sub : Base
                local Sub = {}
                function Sub:init(name, id) self.name = name; self.id = id end
            "#
        ));
        // A nested owner path (`function M.Sub:init`) finds the field through its prefix's type.
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::IncompleteSignatureDoc,
            r#"
                ---@class NBase
                ---@field init fun(self: NBase, ...: any) ctor
                local NBase = {}
                local M = {}
                ---@class NSub : NBase
                ---@type NSub
                M.Sub = setmetatable({}, { __index = NBase })
                function M.Sub:init(name, id) self.name = name; self.id = id end
                return M
            "#
        ));
        // Control: a fixed-arity field gives no type past its last parameter.
        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::IncompleteSignatureDoc,
            r#"
                ---@class Base2
                ---@field init fun(self: Base2, name: string) ctor
                local Base2 = {}
                ---@class Sub2 : Base2
                local Sub2 = {}
                function Sub2:init(name, id) self.name = name; self.id = id end
            "#
        ));
    }
    /// A variadic main `@return` (`---@return any ...`) has no upper bound (0.25.1 parity), so any
    /// number of returned values fits it.
    #[test]
    fn test_variadic_doc_return_has_no_limit() {
        let mut ws = VirtualWorkspace::new();
        ws.enable_full_diagnostic();

        assert!(ws.has_no_diagnostic(
            DiagnosticCode::IncompleteSignatureDoc,
            r#"
                ---@param x integer the x
                ---@return any ... the values
                local function var(x)
                    if x then return 1, 2 end
                    return nil, 'e'
                end
                return var
            "#
        ));
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::IncompleteSignatureDoc,
            r#"
                ---@param x integer the x
                ---@return integer first
                ---@return string... rest
                local function var2(x)
                    return x, 'a', 'b', 'c'
                end
                return var2
            "#
        ));
        // Control: a non-variadic `@return any` still bounds the count to one.
        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::IncompleteSignatureDoc,
            r#"
                ---@param x integer the x
                ---@return any v the value
                local function fixed(x)
                    return x, x
                end
                return fixed
            "#
        ));
    }
}
