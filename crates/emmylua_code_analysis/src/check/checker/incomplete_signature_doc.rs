//! # incomplete_signature_doc - functions missing doc annotations / parameter annotations / return annotations
//!
//! Uses only semantic facts: `Signature.docs` is `None` = no comment (global functions report `MissingGlobalDoc`,
//! others report `IncompleteSignatureDoc`); additionally report missing `@param` and return values exceeding the documented count.

use emmylua_parser::{
    LuaAstNode, LuaAstToken, LuaClosureExpr, LuaComment, LuaExpr, LuaLiteralToken, LuaStat,
    LuaSyntaxId, LuaSyntaxKind,
};

use crate::DiagnosticCode;
use crate::LuaType;
use crate::semantic_db::def::{DeclKind, Signature};
use crate::semantic_model::SemanticModel;

use super::{CheckContext, Checker, own_return_stats};

pub struct IncompleteSignatureDocChecker;

impl Checker for IncompleteSignatureDocChecker {
    const CODES: &[DiagnosticCode] = &[
        DiagnosticCode::IncompleteSignatureDoc,
        DiagnosticCode::MissingGlobalDoc,
    ];

    fn check(context: &mut CheckContext<'_>, semantic_model: &SemanticModel<'_>) {
        let Some(signatures) = semantic_model.signatures() else {
            return;
        };
        let signatures = signatures.to_vec();
        for signature in &signatures {
            check_signature(context, semantic_model, signature);
        }
    }
}

fn check_signature(
    context: &mut CheckContext<'_>,
    semantic_model: &SemanticModel<'_>,
    signature: &Signature,
) {
    let is_global = semantic_model
        .file_facts()
        .and_then(|facts| {
            signature
                .name
                .as_ref()
                .and_then(|name| facts.decl_named(name))
                .map(|decl| decl.kind == DeclKind::Global)
        })
        .unwrap_or(false);
    let code = if is_global {
        DiagnosticCode::MissingGlobalDoc
    } else {
        DiagnosticCode::IncompleteSignatureDoc
    };

    let Some(tree) = semantic_model.syntax_tree() else {
        return;
    };
    let root = tree.get_red_root();
    let Some(node) = signature.closure_syntax.to_node_from_root(&root) else {
        return;
    };
    let Some(closure) = LuaClosureExpr::cast(node) else {
        return;
    };
    if !is_declared_function(&closure) {
        return;
    }

    let Some(docs) = &signature.docs else {
        // A plain description comment is a comment: check its (absent) `@param`/`@return`s.
        if has_attached_comment(&closure) {
            check_params(context, &closure, &[], code);
            check_returns(context, semantic_model, &closure, 0, code);
            return;
        }
        // No comment: a global function always needs one; another only when its signature
        // cannot be inferred, i.e. some named parameter's type is unknown. A parameter typed
        // by a declared field (`self.next = function(s)`, an override of `---@field init
        // fun(...)`) or a call site's callback type is known; returns are inferred.
        if is_global || !param_types_known(semantic_model, &closure) {
            context.add_diagnostic(
                code,
                closure.get_range(),
                t!(
                    "Missing comment for function `%{name}`.",
                    name = signature.name.as_deref().unwrap_or("")
                ),
            );
        }
        return;
    };

    let doc_param_names: Vec<&str> = docs
        .param_types
        .iter()
        .map(|(name, _)| name.as_str())
        .collect();
    check_params(context, &closure, &doc_param_names, code);

    // The actual number of return values exceeds the documented count. return_overload is computed per line (taking the widest line);
    // If the main `@return` rows or any overload line end variadic (`any ...`, `integer...`), there is no limit.
    let mut doc_return_len = docs.returns.len();
    for row_len in &docs.return_overload_rows {
        doc_return_len = doc_return_len.max(*row_len);
    }
    let is_variadic = |syntax: &LuaSyntaxId| {
        matches!(semantic_model.doc_type_lua(*syntax), LuaType::Variadic(_))
            || syntax
                .to_node_from_root(&root)
                .is_some_and(|node| node.text().to_string().ends_with("..."))
    };
    if docs.returns.last().is_some_and(is_variadic)
        || docs
            .return_overloads
            .iter()
            .any(|(_, syntax)| is_variadic(syntax))
    {
        return;
    }
    check_returns(context, semantic_model, &closure, doc_return_len, code);
}

/// Missing `@param` for each named parameter (`_` is exempt).
fn check_params(
    context: &mut CheckContext<'_>,
    closure: &LuaClosureExpr,
    doc_param_names: &[&str],
    code: DiagnosticCode,
) {
    let Some(params_list) = closure.get_params_list() else {
        return;
    };
    for param in params_list.get_params() {
        let Some(name_token) = param.get_name_token() else {
            continue;
        };
        let name = name_token.get_name_text();
        if name == "_" || doc_param_names.contains(&name) {
            continue;
        }
        context.add_diagnostic(
            code,
            param.get_range(),
            t!(
                "Incomplete signature. Missing @param annotation for parameter `%{name}`.",
                name = name
            ),
        );
    }
}

/// Returned values past the documented count. A trailing `...` or call counts as its minimum
/// number of values, so `return x, ...` fits one `@return`.
fn check_returns(
    context: &mut CheckContext<'_>,
    semantic_model: &SemanticModel<'_>,
    closure: &LuaClosureExpr,
    doc_return_len: usize,
    code: DiagnosticCode,
) {
    for return_stat in own_return_stats(closure) {
        let exprs: Vec<LuaExpr> = return_stat.get_expr_list().collect();
        let value_len = returned_min_len(semantic_model, &exprs);
        if value_len <= doc_return_len {
            continue;
        }
        // Report only the expressions that add values past the documented count.
        let reported = value_len.min(exprs.len());
        for (index, expr) in exprs.iter().enumerate().take(reported).skip(doc_return_len) {
            context.add_diagnostic(
                code,
                expr.get_range(),
                t!(
                    "Incomplete signature. Missing @return annotation at index `%{index}`.",
                    index = index + 1
                ),
            );
        }
    }
}

/// The fewest values `return_stat` yields: one per expression, except a trailing `...` (0) or
/// call (its result's minimum length).
fn returned_min_len(semantic_model: &SemanticModel<'_>, exprs: &[LuaExpr]) -> usize {
    let Some((last, fixed)) = exprs.split_last() else {
        return 0;
    };
    let last_len = match last {
        LuaExpr::LiteralExpr(literal)
            if matches!(literal.get_literal(), Some(LuaLiteralToken::Dots(_))) =>
        {
            0
        }
        LuaExpr::CallExpr(_) => match semantic_model.type_of_expr(last.get_syntax_id()) {
            LuaType::Variadic(variadic) => variadic.get_min_len().unwrap_or(0),
            _ => 1,
        },
        _ => 1,
    };
    fixed.len() + last_len
}

/// Whether the closure is declared by a statement that can carry its doc comment: `function f`,
/// `local function f`, or a value of `local f = ...` / `t.f = ...`. A closure passed as an argument,
/// stored in a table constructor or returned is an inline callback and is not checked.
fn is_declared_function(closure: &LuaClosureExpr) -> bool {
    closure.syntax().parent().is_some_and(|parent| {
        matches!(
            parent.kind().into(),
            LuaSyntaxKind::FuncStat
                | LuaSyntaxKind::LocalFuncStat
                | LuaSyntaxKind::LocalStat
                | LuaSyntaxKind::AssignStat
        )
    })
}

/// Whether a comment is attached to the function statement (a pure `---` also counts). A trailing
/// comment of an earlier statement is not.
fn has_attached_comment(closure: &LuaClosureExpr) -> bool {
    let Some(stat) = closure.ancestors::<LuaStat>().next() else {
        return false;
    };
    let Some(comment) = stat.syntax().prev_sibling().and_then(LuaComment::cast) else {
        return false;
    };
    comment
        .get_owner()
        .is_some_and(|owner| owner.syntax() == stat.syntax())
}

/// Whether every named parameter has a known type (from a call site, a declared field, ...).
fn param_types_known(semantic_model: &SemanticModel<'_>, closure: &LuaClosureExpr) -> bool {
    let Some(params_list) = closure.get_params_list() else {
        return true;
    };
    let Some(facts) = semantic_model.file_facts() else {
        return false;
    };
    params_list.get_params().all(|param| {
        let Some(name_token) = param.get_name_token() else {
            return true;
        };
        facts
            .decl_at_offset(name_token.syntax().text_range().start())
            .and_then(|decl| semantic_model.type_of_decl(&decl.id))
            .is_some_and(|ty| !ty.is_unknown())
    })
}
