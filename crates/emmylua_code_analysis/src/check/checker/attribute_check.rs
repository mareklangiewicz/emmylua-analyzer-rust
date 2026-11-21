//! # attribute_check — parameter checks for `---@[attr(...)]` attribute annotations
//!
//! Mirrors legacy `diagnostic::checker::attribute_check`:
//! - the attribute type must inherit from `Attribute`;
//! - constructors are `---@overload fun(...)` overloads on the type (extracted by the semantic layer as
//!   `TypeDef.call_overloads`), selected by argument count/type;
//! - too few / too many parameters → AttributeMissingParameter / AttributeRedundantParameter;
//! - argument type mismatch → AttributeParamTypeMismatch.

use emmylua_parser::{
    LuaAstNode, LuaDocAttributeUse, LuaDocTagAttributeUse, LuaExpr, LuaLiteralExpr,
};

use crate::DiagnosticCode;
use crate::semantic_model::SemanticModel;
use crate::semantic_model::attribute_overload::select_attribute_overload;
use crate::semantic_model::type_check::check_type_detail;
use crate::{LuaType, LuaTypeDeclId};

use super::{CheckContext, Checker};
use crate::semantic_model::render::humanize_type;

pub struct AttributeCheckChecker;

impl Checker for AttributeCheckChecker {
    const CODES: &[DiagnosticCode] = &[
        DiagnosticCode::AttributeParamTypeMismatch,
        DiagnosticCode::AttributeMissingParameter,
        DiagnosticCode::AttributeRedundantParameter,
    ];

    fn check(context: &mut CheckContext<'_>, semantic_model: &SemanticModel<'_>) {
        let Some(tree) = semantic_model.syntax_tree() else {
            return;
        };
        let root = tree.get_red_root();
        for tag_use in root.descendants().filter_map(LuaDocTagAttributeUse::cast) {
            for attribute_use in tag_use.get_attribute_uses() {
                check_attribute_use(context, semantic_model, &attribute_use);
            }
        }
    }
}

fn check_attribute_use(
    context: &mut CheckContext<'_>,
    semantic_model: &SemanticModel<'_>,
    attribute_use: &LuaDocAttributeUse,
) {
    let Some(name_type) = attribute_use.get_type() else {
        return;
    };
    let attribute_ty = semantic_model.doc_type_lua(name_type.get_syntax_id());
    let decl_id = match &attribute_ty {
        LuaType::Ref(id) | LuaType::Def(id) => id,
        _ => return,
    };
    if !is_attribute_class(semantic_model, decl_id) {
        return;
    }
    let args = attribute_use
        .get_arg_list()
        .map(|arg_list| arg_list.get_args().collect::<Vec<_>>())
        .unwrap_or_default();
    let call_arg_types = args
        .iter()
        .map(|arg| semantic_model.type_of_expr(LuaExpr::LiteralExpr(arg.clone()).get_syntax_id()))
        .collect::<Vec<_>>();
    let Some(def) = semantic_model.type_def_of(decl_id) else {
        return;
    };
    let Some(func) = select_attribute_overload(semantic_model, &def, &call_arg_types) else {
        return;
    };
    let def_params = func.get_params().to_vec();
    check_param_count(context, &def_params, attribute_use, &args);
    check_param(context, semantic_model, &def_params, &args, &call_arg_types);
}

/// Whether the type (transitively) inherits from `Attribute`.
fn is_attribute_class(semantic_model: &SemanticModel<'_>, decl_id: &LuaTypeDeclId) -> bool {
    let Some(def) = semantic_model.type_def_of(decl_id) else {
        return false;
    };
    let mut visited = vec![def.id.clone()];
    let mut stack = vec![def];
    while let Some(def) = stack.pop() {
        if def.full_name == "Attribute" {
            return true;
        }
        for super_def in semantic_model.base_type_defs(&def.type_decl_id()) {
            if !visited.contains(&super_def.id) {
                visited.push(super_def.id.clone());
                stack.push(super_def);
            }
        }
    }
    false
}

/// Check whether parameter count matches.
fn check_param_count(
    context: &mut CheckContext<'_>,
    def_params: &[(String, Option<LuaType>)],
    attribute_use: &LuaDocAttributeUse,
    args: &[LuaLiteralExpr],
) {
    let call_args_count = args.len();
    if call_args_count < def_params.len() {
        for def_param in def_params[call_args_count..].iter() {
            if def_param.0 == "..." {
                break;
            }
            if def_param.1.as_ref().is_some_and(LuaType::is_optional) {
                continue;
            }
            context.add_diagnostic(
                DiagnosticCode::AttributeMissingParameter,
                args.last()
                    .map(|arg| arg.get_range())
                    .unwrap_or_else(|| attribute_use.get_range()),
                t!(
                    "expected %{expected} parameters but found %{found}",
                    expected = def_params.len(),
                    found = call_args_count
                ),
            );
        }
    } else if call_args_count > def_params.len() {
        if def_params.last().is_some_and(|(name, typ)| {
            name == "..." || typ.as_ref().is_some_and(LuaType::is_variadic)
        }) {
            return;
        }
        for arg in args[def_params.len()..].iter() {
            context.add_diagnostic(
                DiagnosticCode::AttributeRedundantParameter,
                arg.get_range(),
                t!(
                    "expected %{expected} parameters but found %{found}",
                    expected = def_params.len(),
                    found = call_args_count
                ),
            );
        }
    }
}

/// Check whether parameter types match.
fn check_param(
    context: &mut CheckContext<'_>,
    semantic_model: &SemanticModel<'_>,
    def_params: &[(String, Option<LuaType>)],
    args: &[LuaLiteralExpr],
    call_arg_types: &[LuaType],
) {
    for (idx, param) in def_params.iter().enumerate() {
        if param.0 == "..." {
            if call_arg_types.len() < idx {
                break;
            }
            if let Some(variadic_type) = param.1.as_ref() {
                for (arg_idx, arg_type) in call_arg_types[idx..].iter().enumerate() {
                    if let Some(arg) = args.get(idx + arg_idx) {
                        add_type_check_diagnostic(
                            context,
                            semantic_model,
                            arg.get_range(),
                            arg_type,
                            variadic_type,
                        );
                    }
                }
            }
            break;
        }
        if let Some(param_type) = param.1.as_ref() {
            let arg_type = call_arg_types.get(idx).unwrap_or(&LuaType::Any);
            if let Some(arg) = args.get(idx) {
                add_type_check_diagnostic(
                    context,
                    semantic_model,
                    arg.get_range(),
                    arg_type,
                    param_type,
                );
            }
        }
    }
}

fn add_type_check_diagnostic(
    context: &mut CheckContext<'_>,
    semantic_model: &SemanticModel<'_>,
    range: rowan::TextRange,
    arg_type: &LuaType,
    param_type: &LuaType,
) {
    let reason = match check_type_detail(semantic_model, arg_type, param_type) {
        Err(err) if err.is_type_not_match() => err
            .reason()
            .map_or(String::new(), |reason| format!(" {reason}")),
        _ => return,
    };
    context.add_diagnostic(
        DiagnosticCode::AttributeParamTypeMismatch,
        range,
        t!(
            "expected `%{expected}` but found `%{found}`.%{reason}",
            expected = humanize_type(semantic_model, param_type),
            found = humanize_type(semantic_model, arg_type),
            reason = reason
        ),
    );
}
