//! Regression guard for a cubic cost on a local reassigned many times.
//!
//! ```lua
//! local function f()
//!   local x = 0
//!   x = x + 1 -- n times
//! end
//! ```
//!
//! Every read of `x` asked, for every write of `x`, which closure and which loops
//! enclose it, each by a descent from the file root through a block of `n`
//! statements: `n` reads × `n` writes × `n` children. At n = 400 `emmylua_check`
//! took 9 s on this file, against 0.3 s without the capture and loop rules. The
//! enclosing closure and loops of each write are syntax facts, so they are now
//! looked up once per declaration.

use crate::VirtualWorkspace;
use crate::semantic_model::flow::flow_metrics;
use emmylua_parser::{LuaAstNode, LuaNameExpr};

fn reassigned_chain(n: usize) -> String {
    let mut content = String::from("local function f()\n  local x = 0\n");
    for _ in 0..n {
        content.push_str("  x = x + 1\n");
    }
    content.push_str("  return x\nend\nreturn f\n");
    content
}

/// Read every `x`, as the checkers do for each assignment's value.
fn write_site_lookups(n: usize) -> u64 {
    let mut ws = VirtualWorkspace::new();
    let fid = ws.def_file("chain.lua", &reassigned_chain(n));
    let model = ws.analysis.semantic_model(fid);
    flow_metrics::reset();
    for name in model.chunk().expect("chunk").descendants::<LuaNameExpr>() {
        let _ = model.type_of_expr_at(name.get_syntax_id(), name.get_position());
    }
    flow_metrics::write_site_lookups()
}

#[test]
fn write_sites_of_a_reassigned_local_are_looked_up_once() {
    let measurements: Vec<_> = [16usize, 64, 256]
        .into_iter()
        .map(|n| (n, write_site_lookups(n)))
        .collect();
    for (n, lookups) in &measurements {
        let bound = 2 * *n as u64 + 8;
        assert!(
            *lookups <= bound,
            "{lookups} write-site lookups for {n} writes (bound {bound}): \
             the enclosing closure and loops are looked up per read. All: {measurements:?}"
        );
    }
}

/// Flow steps to read every `x`: each read used to walk back through every earlier write.
fn chain_trace_steps(n: usize) -> u64 {
    let mut ws = VirtualWorkspace::new();
    let fid = ws.def_file("chain.lua", &reassigned_chain(n));
    let model = ws.analysis.semantic_model(fid);
    flow_metrics::reset();
    for name in model.chunk().expect("chunk").descendants::<LuaNameExpr>() {
        let _ = model.type_of_expr_at(name.get_syntax_id(), name.get_position());
    }
    flow_metrics::trace_steps()
}

#[test]
fn reads_of_a_reassigned_local_stop_at_the_previous_write() {
    let measurements: Vec<_> = [16usize, 64, 256]
        .into_iter()
        .map(|n| (n, chain_trace_steps(n)))
        .collect();
    let (n_first, steps_first) = measurements[0];
    for (n, steps) in &measurements {
        let bound = steps_first * (*n as u64) / (n_first as u64) * 2;
        assert!(
            *steps <= bound,
            "{steps} flow steps for {n} writes (bound {bound}, linear from n = {n_first}): \
             each read walks back through every earlier write. All: {measurements:?}"
        );
    }
}
