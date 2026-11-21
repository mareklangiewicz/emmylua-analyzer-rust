//! Which of an attribute's `---@overload`s an attribute use `---@[name(args)]` calls: the attribute
//! checker checks against it and hover shows it, so both read this one pick (#81).
use crate::semantic_model::SemanticModel;
use crate::semantic_model::type_check;
use crate::{LuaFunctionType, LuaType, TypeDef};

/// Select an overload following legacy `select_attribute_constructor_func`:
/// exact type match > count match > first.
pub fn select_attribute_overload(
    semantic_model: &SemanticModel<'_>,
    def: &TypeDef,
    arg_types: &[LuaType],
) -> Option<LuaFunctionType> {
    let overloads: Vec<LuaFunctionType> = def
        .call_overloads
        .iter()
        .filter_map(|syntax| {
            // Overloads live in the file that declares the attribute class (for built-in
            // attributes this is the standard library), so project in `def.file_id`.
            match semantic_model.doc_type_lua_in(def.file_id, *syntax, &def.generic_params) {
                LuaType::DocFunction(func) => Some(func.as_ref().clone()),
                _ => None,
            }
        })
        .collect();
    if overloads.is_empty() {
        return None;
    }

    let arg_count = arg_types.len();
    let only_candidate = overloads.len() == 1;
    let mut fallback = None;
    let mut count_fallback = None;
    for func in &overloads {
        fallback.get_or_insert_with(|| func.clone());
        if !attribute_params_accept_arg_count(func.get_params(), arg_count) {
            continue;
        }
        count_fallback.get_or_insert_with(|| func.clone());
        if only_candidate || callable_accepts_args(semantic_model, func, arg_types) {
            return Some(func.clone());
        }
    }
    count_fallback.or(fallback)
}

/// Whether `arg_count` arguments fit an attribute signature's params: the attribute checker's
/// overload pick (shared with hover). Arguments are positional, so the minimum is the last required
/// param's position + 1: `(a?, x)` needs two (#81, as #78 for calls). An untyped param counts as
/// required, as `check_param_count` reports it.
fn attribute_params_accept_arg_count(
    def_params: &[(String, Option<LuaType>)],
    arg_count: usize,
) -> bool {
    let fixed = def_params
        .iter()
        .take_while(|(name, typ)| name != "..." && !typ.as_ref().is_some_and(LuaType::is_variadic))
        .count();
    let required_count = def_params[..fixed]
        .iter()
        .rposition(|(_, typ)| !typ.as_ref().is_some_and(LuaType::is_optional))
        .map_or(0, |last| last + 1);
    let allows_more = def_params
        .last()
        .is_some_and(|(name, typ)| name == "..." || typ.as_ref().is_some_and(LuaType::is_variadic));
    arg_count >= required_count && (allows_more || arg_count <= def_params.len())
}

fn callable_accepts_args(
    semantic_model: &SemanticModel<'_>,
    func: &LuaFunctionType,
    arg_types: &[LuaType],
) -> bool {
    for (index, (name, param_ty)) in func.get_params().iter().enumerate() {
        if name == "..." {
            if let Some(param_ty) = param_ty {
                return arg_types[index..]
                    .iter()
                    .all(|arg| type_check::is_compatible(semantic_model, arg, param_ty));
            }
            return true;
        }
        let Some(param_ty) = param_ty else {
            continue;
        };
        let Some(arg) = arg_types.get(index) else {
            return true;
        };
        if !type_check::is_compatible(semantic_model, arg, param_ty) {
            return false;
        }
    }
    true
}
