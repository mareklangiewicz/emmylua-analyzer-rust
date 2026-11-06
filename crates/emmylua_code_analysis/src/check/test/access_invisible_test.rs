#[cfg(test)]
mod tests {
    use crate::{DiagnosticCode, Emmyrc, EmmyrcLuaVersion, VirtualWorkspace};

    #[test]
    fn test_issue_289() {
        let mut ws = VirtualWorkspace::new();
        let mut config = Emmyrc::default();
        config.runtime.version = EmmyrcLuaVersion::LuaJIT;
        ws.analysis.update_config(config.into());
        assert!(ws.has_no_diagnostic_in_namespace(
            DiagnosticCode::AccessInvisible,
            r#"
            local file = io.open("test.txt", "r")
            if file then
                file:close()
            end
            "#
        ));
    }

    #[test]
    fn test_private_local_table_keeps_its_fields_public() {
        let mut ws = VirtualWorkspace::new();
        ws.def_file(
            "cfg.lua",
            r#"
            ---@class Cfg
            ---@field discovery Disc

            ---@class Disc
            ---@field enabled boolean

            ---@private
            ---@type Cfg
            local default = {
                discovery = {
                    enabled = true,
                },
            }

            ---@private
            ---@type Cfg
            local M = {}
            setmetatable(M, { __index = function(_, k) return default[k] end })
            return M
            "#,
        );
        assert!(ws.has_no_diagnostic_in_namespace(
            DiagnosticCode::AccessInvisible,
            r#"
            local cfg = require("cfg")
            local a = cfg.discovery
            local b = cfg.discovery.enabled
            "#
        ));
    }

    #[test]
    fn test_1() {
        let mut ws = VirtualWorkspace::new();
        assert!(ws.has_no_diagnostic_in_namespace(
            DiagnosticCode::AccessInvisible,
            r#"
                ---@class (partial) Log
                ---@field private logLevel table<Log.Level, integer>
                local M

                ---@enum (key) Log.Level
                M.logLevel = {
                    trace = 1,
                }
            "#
        ));
    }

    #[test]
    fn test_2() {
        let mut ws = VirtualWorkspace::new();
        ws.def_file(
            "test.lua",
            r#"
            ---@class (partial) Log
            ---@field private logLevel table<Log.Level, integer>
            local M

            ---@enum (key) Log.Level
            M.logLevel = {
                trace = 1,
            }

            return M
        "#,
        );

        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::AccessInvisible,
            r#"
                local Log = require("test")
                Log.logLevel = 1
            "#
        ));
    }

    #[test]
    fn test_3() {
        let mut ws = VirtualWorkspace::new();
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::AccessInvisible,
            r#"
                local M = {}
                ---@private
                function M.init()
                    ---@private
                    M.log = 1
                end

                local function step_collector()
                    if M.log then
                    end
                end
            "#
        ));
    }

    #[test]
    fn test_4_statement_private_cross_file() {
        let mut ws = VirtualWorkspace::new();
        ws.def_file(
            "mod.lua",
            r#"
            local M = {}
            ---@private
            function M.init() end
            return M
            "#,
        );
        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::AccessInvisible,
            r#"
            local M = require("mod")
            M.init()
            "#,
        ));
    }
}
