//! Regression guard: a read through a required module must not mark its query incomplete.
//!
//! ```lua
//! -- mod.lua
//! local M = {}
//! M.a = 1
//! return M
//! -- main.lua
//! local mod = require("mod")
//! local x = mod.missing
//! ```
//!
//! `require("mod")` is `(M|{ a: integer })`: `M` is only the export's name identity, which
//! no `---@class` declares. `mod.missing` has no named member, so inference falls back to
//! an index signature on each union member. The index resolver treats the undeclared `M`
//! as "not known yet" and marks the query incomplete, and the outer query then throws
//! away everything it cached. `emmylua_check` did that on every such read: on
//! gitsigns.nvim, 35 000 of 37 000 discards were `M`, and the run took twice upstream's
//! CPU time. A name with no declaration has no index signature, so the fallback no longer
//! asks.

use crate::VirtualWorkspace;
use emmylua_parser::{LuaAstNode, LuaIndexExpr};

#[test]
fn reading_a_missing_member_of_a_required_module_keeps_the_query_complete() {
    let mut ws = VirtualWorkspace::new();
    ws.def_file("mod.lua", "local M = {}\nM.a = 1\nreturn M\n");
    let fid = ws.def_file(
        "main.lua",
        "local mod = require(\"mod\")\nlocal x = mod.missing\nreturn x\n",
    );
    let model = ws.analysis.semantic_model(fid);
    let read = model
        .chunk()
        .expect("chunk")
        .descendants::<LuaIndexExpr>()
        .find(|e| e.syntax().text().to_string() == "mod.missing")
        .expect("the read `mod.missing`");
    let before = model.incomplete_revision();
    let _ = model.type_of_expr_at(read.get_syntax_id(), read.get_position());
    let after = model.incomplete_revision();
    assert_eq!(
        before,
        after,
        "reading `mod.missing` marked the query incomplete {} time(s)",
        after.wrapping_sub(before)
    );
}
