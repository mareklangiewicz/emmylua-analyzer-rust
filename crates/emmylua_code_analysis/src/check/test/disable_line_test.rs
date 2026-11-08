#[cfg(test)]
mod tests {
    use crate::DiagnosticCode;

    use crate::Emmyrc;
    use crate::check::test::{check_source, check_source_with_emmyrc, count_by_code};

    /// No disable annotation: UndefinedGlobal is reported.
    #[test]
    fn test_undefined_global_without_disable() {
        let diags = check_source("local a = missing_global");
        assert_eq!(count_by_code(&diags, DiagnosticCode::UndefinedGlobal), 1);
    }

    /// `---@diagnostic disable-next-line: undefined-global`: next line is not reported.
    #[test]
    fn test_disable_next_line() {
        let diags = check_source(
            "---@diagnostic disable-next-line: undefined-global\nlocal a = missing_global",
        );
        assert_eq!(count_by_code(&diags, DiagnosticCode::UndefinedGlobal), 0);
    }

    /// Trailing `---@diagnostic disable-next-line`: applies to the next line, not this line (mirrors issue 158).
    #[test]
    fn test_disable_next_line_trailing() {
        let diags = check_source(
            "local a = missing_global ---@diagnostic disable-next-line: undefined-global\nlocal b = another_missing",
        );
        // This line's `missing_global` is still reported; the next line's `another_missing` is disabled.
        assert_eq!(count_by_code(&diags, DiagnosticCode::UndefinedGlobal), 1);
    }

    /// `---@diagnostic disable-line: undefined-global`: this line is not reported.
    #[test]
    fn test_disable_line() {
        let diags =
            check_source("local a = missing_global ---@diagnostic disable-line: undefined-global");
        assert_eq!(count_by_code(&diags, DiagnosticCode::UndefinedGlobal), 0);
    }

    /// No code list = disables all diagnostic codes (DisableAll).
    #[test]
    fn test_disable_all_next_line() {
        let diags = check_source("---@diagnostic disable-next-line\nlocal a = missing_global");
        assert_eq!(count_by_code(&diags, DiagnosticCode::UndefinedGlobal), 0);
    }

    /// Disabling other codes does not affect this code.
    #[test]
    fn test_disable_other_code_no_effect() {
        let diags =
            check_source("---@diagnostic disable-next-line: unused\nlocal a = missing_global");
        assert_eq!(count_by_code(&diags, DiagnosticCode::UndefinedGlobal), 1);
    }

    /// File-level `---@diagnostic disable: undefined-global`: whole file is not reported.
    #[test]
    fn test_file_level_disable() {
        let diags =
            check_source("---@diagnostic disable: undefined-global\nlocal a = missing_global");
        assert_eq!(count_by_code(&diags, DiagnosticCode::UndefinedGlobal), 0);
    }

    /// 文件级 disable 不带 code 时应压制整个文件的全部诊断, 而非个别 code
    #[test]
    fn test_file_level_disable_all_codes() {
        let body = r#"local unused_local = 1
local a = missing_global
---@param s string
local function f(s) end
f(1)
local b = a .. "x""#;
        let baseline = check_source(body);
        assert!(!baseline.is_empty(), "diagnostics: {baseline:#?}");

        let diags = check_source(&format!("---@diagnostic disable\n{body}"));
        assert!(diags.is_empty(), "diagnostics: {diags:#?}");
    }

    /// 文件级 disable 全部后再 enable 全部应恢复诊断
    #[test]
    fn test_file_level_disable_all_then_enable_all() {
        let diags = check_source(
            r#"---@diagnostic disable
---@diagnostic enable
local a = missing_global"#,
        );
        assert_eq!(
            count_by_code(&diags, DiagnosticCode::UndefinedGlobal),
            1,
            "diagnostics: {diags:#?}"
        );
    }

    /// A chunk-level bare `disable … enable` pair silences only the lines between them
    /// (neovim `runtime/lua/vim/glob.lua`: the pair used to cancel out and silence nothing).
    #[test]
    fn test_file_level_disable_all_enable_all_is_positional() {
        let diags = check_source(
            r#"local a = before_missing
---@diagnostic disable
local b = inside_missing
---@diagnostic enable
local c = after_missing"#,
        );
        assert_eq!(
            count_by_code(&diags, DiagnosticCode::UndefinedGlobal),
            2,
            "diagnostics: {diags:#?}"
        );
    }

    /// The same pair with a code list is positional too.
    #[test]
    fn test_file_level_disable_code_enable_code_is_positional() {
        let diags = check_source(
            r#"local a = before_missing
---@diagnostic disable: undefined-global
local b = inside_missing
---@diagnostic enable: undefined-global
local c = after_missing"#,
        );
        assert_eq!(
            count_by_code(&diags, DiagnosticCode::UndefinedGlobal),
            2,
            "diagnostics: {diags:#?}"
        );
    }

    /// A chunk-level `disable` without a matching `enable` runs from the comment to the end of file.
    #[test]
    fn test_file_level_disable_runs_to_eof() {
        let diags = check_source(
            r#"local a = before_missing
---@diagnostic disable
local b = inside_missing
local c = also_inside_missing"#,
        );
        assert_eq!(
            count_by_code(&diags, DiagnosticCode::UndefinedGlobal),
            1,
            "diagnostics: {diags:#?}"
        );
    }

    /// `enable: code` after a bare `disable` re-enables only that code; the rest stay silenced.
    #[test]
    fn test_file_level_disable_all_then_enable_one_code() {
        let diags = check_source(
            r#"---@param s string
local function f(s) end
---@diagnostic disable
---@diagnostic enable: undefined-global
local a = missing_global
f(1)"#,
        );
        assert_eq!(
            count_by_code(&diags, DiagnosticCode::UndefinedGlobal),
            1,
            "diagnostics: {diags:#?}"
        );
        assert_eq!(
            count_by_code(&diags, DiagnosticCode::ParamTypeMismatch),
            0,
            "diagnostics: {diags:#?}"
        );
    }

    /// File-level `---@diagnostic enable: undefined-global` overrides a global config disable.
    #[test]
    fn test_file_level_enable_overrides_config() {
        let mut emmyrc = Emmyrc::default();
        emmyrc
            .diagnostics
            .disable
            .push(DiagnosticCode::UndefinedGlobal);
        let diags = check_source_with_emmyrc(
            "---@diagnostic enable: undefined-global\nlocal a = missing_global",
            emmyrc,
        );
        assert_eq!(count_by_code(&diags, DiagnosticCode::UndefinedGlobal), 1);
    }
}
