//! Regression guard for a quadratic cost on a chain of conditionals that leave a local alone.
//!
//! ```lua
//! ---@param h H
//! local function f(h)
//!   if h.a then elseif h.b then end -- n times
//! end
//! ```
//!
//! Each merge whose branches do not touch `h` is skipped by jumping to the merge's common
//! predecessor. The skip never met the memo, so each read of `h` walked back through every
//! earlier merge: n reads × n merges. 0.25.1 is linear here; at n = 1600 `emmylua_check` took
//! 5.3 s against 0.25.1's 0.19 s.

use crate::VirtualWorkspace;
use crate::semantic_model::flow::flow_metrics;
use emmylua_parser::{LuaAstNode, LuaNameExpr};

fn untouched_merge_chain(n: usize) -> String {
    let mut content = String::from(
        "---@class H\n---@field a? boolean\n---@field b? boolean\n\n---@param h H\nlocal function f(h)\n",
    );
    for _ in 0..n {
        content.push_str("  if h.a then elseif h.b then end\n");
    }
    content.push_str("end\nreturn f\n");
    content
}

/// Read every `h`, as `unnecessary-if` does for each condition.
fn trace_steps(n: usize) -> u64 {
    let mut ws = VirtualWorkspace::new();
    let fid = ws.def_file("chain.lua", &untouched_merge_chain(n));
    let model = ws.analysis.semantic_model(fid);
    flow_metrics::reset();
    for name in model.chunk().expect("chunk").descendants::<LuaNameExpr>() {
        if name.get_name_text().as_deref() == Some("h") {
            let _ = model.type_of_expr_at(name.get_syntax_id(), name.get_position());
        }
    }
    flow_metrics::trace_steps() + flow_metrics::branch_scan_steps()
}

#[test]
fn reads_past_untouched_merges_stay_linear() {
    let measurements: Vec<_> = [16usize, 64, 256]
        .into_iter()
        .map(|n| (n, trace_steps(n)))
        .collect();
    let (n_first, steps_first) = measurements[0];
    for (n, steps) in &measurements {
        let bound = steps_first * (*n as u64) / (n_first as u64) * 2;
        assert!(
            *steps <= bound,
            "{steps} flow steps for {n} conditionals (bound {bound}, linear from n = {n_first}): \
             each read walks back through every earlier merge. All: {measurements:?}"
        );
    }
}
