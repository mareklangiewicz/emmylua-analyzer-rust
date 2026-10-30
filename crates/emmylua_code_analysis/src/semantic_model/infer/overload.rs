//! # OverloadResolver
//!
//! Extracts the logic for selecting the best candidate from "multiple callable
//! candidates + actual args" out of the VM. Normal calls, pcall, and future
//! diagnostics/completions should reuse this instead of maintaining their own argument
//! matching.

use std::cmp::Reverse;
use std::rc::Rc;

use emmylua_parser::{LuaAstNode, LuaExpr, LuaSyntaxId};

use crate::semantic_db::def::SemanticId;
use crate::semantic_model::SemanticModel;
use crate::{
    LuaArrayType, LuaFunctionType, LuaGenericType, LuaMemberKey, LuaObjectType, LuaTupleStatus,
    LuaTupleType, LuaType, LuaTypeDeclId, TypeDefKind, VariadicType,
};

use super::unify::{self, TplBindings};
use super::vm::{
    bind_signature_generics, is_empty_table_literal, score_param_match, shallow_closure_signature,
    unify_call_bindings, variadic_base_type, widen_const, widen_variadic_const,
};
use crate::semantic_model::type_eval;

/// Minimal representation of a call argument (type + whether it is a closure literal).
#[derive(Debug, Clone)]
pub struct CallArg {
    pub ty: LuaType,
    pub closure_syntax: Option<LuaSyntaxId>,
}

impl CallArg {
    pub fn new(ty: LuaType) -> Self {
        Self {
            ty,
            closure_syntax: None,
        }
    }
}

/// Table literal -> Object structure (for unify with `{__index: T}` params).
pub fn structural_table_type(model: &SemanticModel, ty: &LuaType) -> LuaType {
    let LuaType::TableConst(table) = ty else {
        return ty.clone();
    };
    let owner = SemanticId::member(table.file_id, table.value);
    let mut fields = hashbrown::HashMap::new();
    for member in model.members_of_owner(&owner).iter() {
        let key = LuaMemberKey::Name(member.name.clone());
        let mut member_ty = model.type_of_member(&member.id).unwrap_or(LuaType::Unknown);
        if matches!(member_ty, LuaType::Unknown)
            && let Some(facts) = model.file_facts_of(member.file_id)
            && let Some(member_def) = facts.member_by_id(&member.id)
            && let Some(value_syntax) = member_def.value_syntax
        {
            member_ty = model.type_of_expr(value_syntax);
            if matches!(member_ty, LuaType::Unknown)
                && let Some(node) = model
                    .syntax_tree()
                    .and_then(|tree| value_syntax.to_node_from_root(&tree.get_red_root()))
                && let Some(LuaExpr::NameExpr(name_expr)) = LuaExpr::cast(node)
                && let Some(name) = name_expr.get_name_text()
                && let Some(def) = model.resolve_type_def(&name)
            {
                member_ty = model.type_def_ref(&def);
            }
        }
        fields.insert(key, member_ty);
    }
    LuaType::Object(Rc::new(LuaObjectType::new_with_fields(fields, Vec::new())))
}

/// Table literal -> Array (for structural unification with `T[]` params).
/// Only positive integer-keyed members are collected and widened to the array element
/// base type (`{1,2,3}` -> `integer[]`). A table with no members at all is an empty array
/// of any element (`{}` -> `any[]`).
fn table_literal_as_array(model: &SemanticModel, ty: &LuaType) -> Option<LuaType> {
    table_literal_as_array_by(model, ty, |syntax| model.type_of_expr(syntax))
}

/// `table_literal_as_array`, typing each member's value expression with `type_of_value`.
pub(crate) fn table_literal_as_array_by(
    model: &SemanticModel,
    ty: &LuaType,
    type_of_value: impl Fn(LuaSyntaxId) -> LuaType,
) -> Option<LuaType> {
    if is_empty_table_literal(model, ty) {
        return Some(LuaType::Array(Rc::new(LuaArrayType::from_base_type(
            LuaType::Any,
        ))));
    }
    let members = table_literal_members(
        model,
        ty,
        |key| matches!(key, LuaMemberKey::Integer(key) if *key > 0),
        type_of_value,
    )?;
    let mut base_types = Vec::new();
    for (_, member_ty) in members {
        push_known_type(&mut base_types, member_ty);
    }
    if base_types.is_empty() {
        return None;
    }
    let base = if base_types.len() == 1 {
        base_types.pop()?
    } else {
        LuaType::from_vec(base_types)
    };
    Some(LuaType::Array(Rc::new(LuaArrayType::from_base_type(base))))
}

/// Table literal -> `table<K, V>` (for unification with `table<K, V>` params): `K` is the
/// union of its key types, `V` the union of its widened value types.
fn table_literal_as_map(
    model: &SemanticModel,
    ty: &LuaType,
    map: &LuaGenericType,
) -> Option<LuaType> {
    let members = table_literal_members(
        model,
        ty,
        |key| matches!(key, LuaMemberKey::Name(_) | LuaMemberKey::Integer(_)),
        |syntax| model.type_of_expr(syntax),
    )?;
    let mut key_types = Vec::new();
    let mut value_types = Vec::new();
    for (key, value_ty) in members {
        let key_ty = match key {
            LuaMemberKey::Name(_) => LuaType::String,
            _ => LuaType::Integer,
        };
        if !key_types.contains(&key_ty) {
            key_types.push(key_ty);
        }
        push_known_type(&mut value_types, value_ty);
    }
    if key_types.is_empty() || value_types.is_empty() {
        return None;
    }
    Some(LuaType::Generic(Rc::new(LuaGenericType::new(
        map.get_base_type_id(),
        vec![LuaType::from_vec(key_types), LuaType::from_vec(value_types)],
    ))))
}

/// The members of a table literal type whose key passes `keep_key`, each with its widened
/// value type: `type_of_value` on the value expression, else the member's own type.
fn table_literal_members(
    model: &SemanticModel,
    ty: &LuaType,
    keep_key: impl Fn(&LuaMemberKey) -> bool,
    type_of_value: impl Fn(LuaSyntaxId) -> LuaType,
) -> Option<Vec<(LuaMemberKey, LuaType)>> {
    let LuaType::TableConst(table) = ty else {
        return None;
    };
    let owner = SemanticId::member(table.file_id, table.value);
    let mut out = Vec::new();
    for member_ref in model.members_of_owner(&owner).iter() {
        let Some(facts) = model.file_facts_of(member_ref.file_id) else {
            continue;
        };
        let Some(member) = facts.member_by_id(&member_ref.id) else {
            continue;
        };
        if !keep_key(&member.key) {
            continue;
        }
        let member_ty = if let Some(value_syntax) = member.value_syntax {
            type_of_value(value_syntax)
        } else {
            model
                .type_of_member(&member_ref.id)
                .unwrap_or(LuaType::Unknown)
        };
        out.push((member.key.clone(), widen_const(&member_ty)));
    }
    Some(out)
}

/// Collect a type into a deduplicated union list, skipping `unknown` / `any`.
fn push_known_type(types: &mut Vec<LuaType>, ty: LuaType) {
    if !matches!(ty, LuaType::Unknown | LuaType::Any) && !types.contains(&ty) {
        types.push(ty);
    }
}

/// Expand a non-generic named alias (`LocalTimer.OnTimer` -> `DocFunction`).
fn expand_non_generic_alias(model: &SemanticModel, ty: &LuaType) -> LuaType {
    let (LuaType::Ref(id) | LuaType::Def(id)) = ty else {
        return ty.clone();
    };
    let Some(def) = model.type_def_of(id) else {
        return ty.clone();
    };
    if def.kind != TypeDefKind::Alias {
        return ty.clone();
    }
    model.alias_target(&def).unwrap_or_else(|| ty.clone())
}

/// Resolve the `self` type of a callable type.
///
/// Mirrors the old `call_operator_self_type`: only `---@overload` /
/// `---@operator call` on a `---@class` provides `self`; plain function types do not.
/// A union keeps only callable members, aliases pass through to the real callable type,
/// and if an intersection is callable as a whole, `self` is the whole intersection.
pub(crate) fn call_operator_self_type(model: &SemanticModel, ty: &LuaType) -> Option<LuaType> {
    let mut visited = Vec::new();
    call_operator_self_type_inner(model, ty, &mut visited)
}

fn call_operator_self_type_inner(
    model: &SemanticModel,
    ty: &LuaType,
    visited: &mut Vec<LuaTypeDeclId>,
) -> Option<LuaType> {
    match ty {
        LuaType::Ref(id) | LuaType::Def(id) => {
            if visited.contains(id) {
                return None;
            }
            visited.push(id.clone());
            let result = (|| {
                let def = model.callable_type_def_of(id)?;
                if def.kind == TypeDefKind::Alias {
                    let target = model.alias_target(&def)?;
                    return call_operator_self_type_inner(model, &target, visited);
                }
                if !def.call_overloads.is_empty() {
                    return Some(ty.clone());
                }
                let facts = model.file_facts_of(def.file_id)?;
                if facts.operator_of(&def.id, "call").is_some() {
                    return Some(ty.clone());
                }
                None
            })();
            visited.pop();
            result
        }
        LuaType::Generic(generic) => {
            let base_id = generic.get_base_type_id();
            if visited.contains(&base_id) {
                return None;
            }
            let def = model.callable_type_def_of(&base_id)?;
            if def.kind == TypeDefKind::Alias {
                visited.push(base_id.clone());
                let expanded = type_eval::expand_alias_generic(model, ty);
                let result = call_operator_self_type_inner(model, &expanded, visited);
                visited.pop();
                return result;
            }
            if !def.call_overloads.is_empty() {
                return Some(ty.clone());
            }
            let facts = model.file_facts_of(def.file_id)?;
            if facts.operator_of(&def.id, "call").is_some() {
                return Some(ty.clone());
            }
            None
        }
        LuaType::Union(union) => {
            let mut callable = Vec::new();
            for member in union.into_vec() {
                if let Some(self_ty) = call_operator_self_type_inner(model, &member, visited) {
                    if !callable.contains(&self_ty) {
                        callable.push(self_ty);
                    }
                }
            }
            match callable.len() {
                0 => None,
                1 => callable.pop(),
                _ => Some(LuaType::from_vec(callable)),
            }
        }
        LuaType::Intersection(intersection) => intersection
            .get_types()
            .iter()
            .any(|member| call_operator_self_type_inner(model, member, visited).is_some())
            .then(|| ty.clone()),
        LuaType::Instance(instance) => {
            call_operator_self_type_inner(model, instance.get_base(), visited).map(|_| ty.clone())
        }
        _ => None,
    }
}

/// How well a candidate fits a call; greater is better. Args past the last param outrank any
/// score, so `fun()` never beats a signature that takes them (`f:seek('end')` vs
/// `fun(whence?, offset?)`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct CallRank {
    surplus: Reverse<usize>,
    score: i32,
}

/// Candidate selection: among candidates where all params unify successfully, take the
/// highest score (literal/function structural exact matches are preferred).
pub fn select_callable(
    model: &SemanticModel,
    candidates: &[LuaFunctionType],
    args: &[CallArg],
    colon_call: bool,
    receiver: Option<&LuaType>,
) -> Option<(LuaFunctionType, TplBindings)> {
    let mut best: Option<(LuaFunctionType, TplBindings, CallRank)> = None;
    for candidate in candidates {
        if let Some((bindings, rank)) =
            match_call_candidate(model, candidate, args, colon_call, receiver)
        {
            let replace = match &best {
                Some((_, _, best_rank)) => rank > *best_rank,
                None => true,
            };
            if replace {
                best = Some((candidate.clone(), bindings, rank));
            }
        }
    }
    best.map(|(fun, bindings, _)| (fun, bindings))
}

/// Return all highest-scoring candidates (a directly callable union needs to merge the
/// returns of multiple same-priority candidates).
pub fn select_callable_all(
    model: &SemanticModel,
    candidates: &[LuaFunctionType],
    args: &[CallArg],
    colon_call: bool,
    receiver: Option<&LuaType>,
) -> Vec<(LuaFunctionType, TplBindings)> {
    let mut best: Vec<(LuaFunctionType, TplBindings, CallRank)> = Vec::new();
    for candidate in candidates {
        if let Some((bindings, rank)) =
            match_call_candidate(model, candidate, args, colon_call, receiver)
        {
            if let Some((_, _, best_rank)) = best.first() {
                if rank < *best_rank {
                    continue;
                }
                if rank > *best_rank {
                    best.clear();
                }
            }
            best.push((candidate.clone(), bindings, rank));
        }
    }
    best.into_iter()
        .map(|(fun, bindings, _)| (fun, bindings))
        .collect()
}

/// Match one candidate against actual args: unify param by param; variadic params
/// consume the remaining args.
pub fn match_call_candidate(
    model: &SemanticModel,
    fun: &LuaFunctionType,
    args: &[CallArg],
    colon_call: bool,
    receiver: Option<&LuaType>,
) -> Option<(TplBindings, CallRank)> {
    match_call_candidate_impl(model, fun, args, colon_call, false, receiver)
}

/// Fallback candidate selection: allow a plain `... T` to keep bindings inferred from
/// the first actual arg even when later params do not fully match.
pub fn select_callable_partial(
    model: &SemanticModel,
    candidates: &[LuaFunctionType],
    args: &[CallArg],
    colon_call: bool,
    receiver: Option<&LuaType>,
) -> Option<(LuaFunctionType, TplBindings)> {
    let mut best: Option<(LuaFunctionType, TplBindings, CallRank)> = None;
    for candidate in candidates {
        if let Some((bindings, rank)) =
            match_call_candidate_impl(model, candidate, args, colon_call, true, receiver)
        {
            let replace = match &best {
                Some((_, _, best_rank)) => rank > *best_rank,
                None => true,
            };
            if replace {
                best = Some((candidate.clone(), bindings, rank));
            }
        }
    }
    best.map(|(fun, bindings, _)| (fun, bindings))
}

fn normalize_missing_generic_arg(
    model: &SemanticModel,
    arg_ty: &LuaType,
    param_ty: &LuaType,
) -> LuaType {
    let (LuaType::Generic(param_generic), LuaType::Ref(arg_id) | LuaType::Def(arg_id)) =
        (param_ty, arg_ty)
    else {
        return arg_ty.clone();
    };
    if param_generic.get_base_type_id() != *arg_id {
        return arg_ty.clone();
    }
    let Some(def) = model.type_def_of(arg_id) else {
        return arg_ty.clone();
    };
    let params = def
        .generic_params
        .iter()
        .map(|param| {
            param
                .default
                .map(|s| model.doc_type_lua_in(def.file_id, s, &[]))
                .or_else(|| {
                    param
                        .constraint
                        .map(|s| model.doc_type_lua_in(def.file_id, s, &[]))
                })
                .unwrap_or(LuaType::Unknown)
        })
        .collect();
    LuaType::Generic(Rc::new(LuaGenericType::new(arg_id.clone(), params)))
}

/// Whether the type contains an unbound generic parameter (used to avoid binding `T`
/// to `unknown` when an argument is missing).
pub(crate) fn type_contains_tpl_ref(ty: &LuaType) -> bool {
    use LuaType::*;
    match ty {
        TplRef(_) => true,
        Array(array) => type_contains_tpl_ref(array.get_base()),
        Tuple(tuple) => tuple.get_types().iter().any(type_contains_tpl_ref),
        Union(union) => union.into_vec().iter().any(type_contains_tpl_ref),
        Intersection(intersection) => intersection.get_types().iter().any(type_contains_tpl_ref),
        Generic(generic) => generic.get_params().iter().any(type_contains_tpl_ref),
        TableGeneric(generic) => generic.iter().any(type_contains_tpl_ref),
        Variadic(variadic) => match variadic.as_ref() {
            VariadicType::Base(base) | VariadicType::Repeat(base) => type_contains_tpl_ref(base),
            VariadicType::Multi(types) => types.iter().any(type_contains_tpl_ref),
        },
        DocFunction(fun) => {
            fun.get_params()
                .iter()
                .any(|(_, ty)| ty.as_ref().is_some_and(type_contains_tpl_ref))
                || type_contains_tpl_ref(fun.get_ret())
        }
        Object(object) => {
            object.get_fields().values().any(type_contains_tpl_ref)
                || object
                    .get_index_access()
                    .iter()
                    .any(|(k, v)| type_contains_tpl_ref(k) || type_contains_tpl_ref(v))
        }
        Call(call) => call.get_operands().iter().any(type_contains_tpl_ref),
        Mapped(mapped) => type_contains_tpl_ref(&mapped.value),
        Conditional(conditional) => {
            type_contains_tpl_ref(conditional.get_checked_type())
                || type_contains_tpl_ref(conditional.get_extends_type())
                || type_contains_tpl_ref(conditional.get_true_type())
                || type_contains_tpl_ref(conditional.get_false_type())
        }
        _ => false,
    }
}

fn match_call_candidate_impl(
    model: &SemanticModel,
    fun: &LuaFunctionType,
    args: &[CallArg],
    colon_call: bool,
    allow_partial_variadic: bool,
    receiver: Option<&LuaType>,
) -> Option<(TplBindings, CallRank)> {
    let mut bindings = TplBindings::new();
    let mut score = 0i32;
    let params = fun.get_params().to_vec();
    let has_self_param = params
        .first()
        .is_some_and(|(name, ty)| name == "self" || matches!(ty, Some(LuaType::SelfInfer)));
    // Method-call argument alignment:
    // - colon calls implicitly pass the receiver, so an explicit `self` parameter is
    //   skipped as one logical argument;
    // - dot calls explicitly pass the receiver; if the method type does not list `self`
    //   (semantic signatures contain only user params), one actual arg must be skipped.
    // - when a dot-defined function is called with colon syntax (`a:create()`), the
    //   receiver is the first implicit argument.
    let param_start = usize::from(has_self_param);
    let mut effective_args_storage: Vec<CallArg> = Vec::new();
    let args: &[CallArg] = if colon_call && !has_self_param && !fun.is_colon_define() {
        if let Some(receiver) = receiver {
            effective_args_storage.push(CallArg::new(receiver.clone()));
            effective_args_storage.extend(args.iter().cloned());
            effective_args_storage.as_slice()
        } else {
            args
        }
    } else {
        args
    };
    // A multi-return call as the last argument passes all its values, one per remaining
    // param: `id(f())` binds `act: T` to `f()`'s first value only, not the whole tuple.
    let mut spread_args_storage: Vec<CallArg> = Vec::new();
    let args: &[CallArg] = if let Some((last, init)) = args.split_last()
        && last.closure_syntax.is_none()
        && let LuaType::Variadic(variadic) = &last.ty
        && let VariadicType::Multi(values) = variadic.as_ref()
    {
        spread_args_storage.extend(init.iter().cloned());
        spread_args_storage.extend(values.iter().cloned().map(CallArg::new));
        &spread_args_storage
    } else {
        args
    };
    let arg_start = if has_self_param && colon_call {
        0
    } else if !has_self_param && fun.is_colon_define() && !colon_call {
        1
    } else {
        param_start
    };

    // Handle call-site variadic generic args (`... T...`) first. When callback params
    // (`fun(...: T...)`) contain the same T, the actual T value must be taken from the
    // tail of the call-site args so consecutive variadic generic slots like
    // `fun(_: T1..., _: T2...)` can be sliced correctly.
    prebind_variadic_generics(fun, args, param_start, arg_start, &mut bindings);

    // A method's class generics stay named refs (`fun(v: T)`); the receiver instance binds them,
    // so a function value unifies with `fun(v: string)`, not with an unbindable `T`.
    let class_map = receiver.and_then(|receiver| type_eval::class_generic_map(model, receiver));

    // Also set in the loop: an alias param can expand to `T...`.
    let mut takes_rest = fun.takes_rest();
    for (param_index, (name, param_ty)) in params.iter().enumerate().skip(param_start) {
        let index = arg_start + (param_index - param_start);
        let Some(param_ty) = param_ty else {
            continue;
        };
        let mut param_ty = bind_signature_generics(param_ty, fun.get_generic_params());
        if let Some(class_map) = &class_map {
            param_ty = type_eval::substitute_named_refs(&param_ty, class_map);
        }
        let raw_param_ty = param_ty.clone();
        param_ty = unify::substitute(&param_ty, &bindings);
        // Only expand conditional aliases; plain structural aliases (e.g. `Arrayable<T>`)
        // keep their Generic form so unify can back-infer T from the generic-parameter
        // structure instead of binding the whole union to T.
        let expanded = if matches!(raw_param_ty, LuaType::Ref(_) | LuaType::Def(_)) {
            expand_non_generic_alias(model, &param_ty)
        } else {
            type_eval::expand_alias_generic(model, &param_ty)
        };
        // Pure pass-through aliases (`Id<T> = T`) expand directly to TplRef; structural
        // aliases (`Arrayable<T> = T | T[]`) keep Generic form so unify can back-infer
        // from the generic-parameter structure instead of binding the whole union to the
        // generic. Non-generic aliases (`LocalTimer.OnTimer`) also expand to a concrete
        // structure, otherwise closure args cannot match the alias param.
        if (matches!(raw_param_ty, LuaType::Ref(_) | LuaType::Def(_)) && expanded != raw_param_ty)
            || matches!(expanded, LuaType::TplRef(_))
        {
            param_ty = expanded;
        } else if type_eval::contains_conditional_infer(&expanded) {
            param_ty = type_eval::eval_conditionals(model, &expanded);
        }
        if name == "..." || matches!(param_ty, LuaType::Variadic(_)) {
            takes_rest = true;
            let base = variadic_base_type(&param_ty).unwrap_or(LuaType::Unknown);
            // `T...` variadic generic (param itself is Variadic): bind the remaining args
            // as a whole tuple instead of binding T one arg at a time.
            // For a plain `... T`, `param_ty` is not Variadic, so each arg still unifies T.
            if matches!(param_ty, LuaType::Variadic(_)) {
                // After prebinding, `T...`'s T has been replaced with a concrete tuple/type;
                // do not unify each remaining arg with the tuple base again; just treat it
                // as an already-bound variadic.
                if let Some(LuaType::TplRef(raw_tpl)) = variadic_base_type(&raw_param_ty)
                    && bindings.contains_key(&raw_tpl.get_tpl_id())
                {
                    score += 2;
                    break;
                }
                if let LuaType::TplRef(tpl) = &base {
                    // Prebinding already handled this; just avoid unify-ing the tuple
                    // element-by-element again.
                    if bindings.contains_key(&tpl.get_tpl_id()) {
                        score += 2;
                        break;
                    }
                    let rest_types = args
                        .get(index..)
                        .unwrap_or(&[])
                        .iter()
                        .map(|arg| widen_variadic_const(&arg.ty))
                        .collect::<Vec<_>>();
                    let tuple_ty = LuaType::Tuple(Rc::new(LuaTupleType::new(
                        rest_types,
                        LuaTupleStatus::InferResolve,
                    )));
                    if !unify_call_bindings(
                        model,
                        &LuaType::TplRef(tpl.clone()),
                        &tuple_ty,
                        &mut bindings,
                    ) {
                        return None;
                    }
                    score += 2;
                    break;
                }
            }
            for arg in args.get(index..).unwrap_or(&[]) {
                let arg_ty = structural_table_type(model, &arg.ty);
                if !unify_call_bindings(model, &base, &arg_ty, &mut bindings) {
                    if allow_partial_variadic {
                        // Only a fallback when no complete candidate match exists: keep
                        // bindings already inferred from the first arg so diagnostics can
                        // still report the mismatch while return-type inference continues.
                        break;
                    }
                    return None;
                }
            }
            score += 2;
            break;
        }

        let arg = args.get(index);
        let arg_ty = arg
            .map(|arg| {
                if matches!(param_ty, LuaType::TplRef(_)) {
                    // For generic value params (`---@generic T; ---@param a T`), when the
                    // arg is a closure literal use a shallow signature: do not leak
                    // body-return/unannotated-param inference into outer generics like `Mock<T>`.
                    if let Some(closure) = arg.closure_syntax {
                        return shallow_closure_signature(model, closure);
                    }
                    return arg.ty.clone();
                }
                if arg.closure_syntax.is_some() && matches!(param_ty, LuaType::DocFunction(_)) {
                    // Only variadic generic callbacks (`fun(...: T...)`) need to infer T
                    // from the closure literal's real signature; ordinary callbacks still use
                    // the expected function type to avoid treating not-yet-inferred closure
                    // param types as Unknown.
                    let is_variadic_callback = matches!(
                        &param_ty,
                        LuaType::DocFunction(fun)
                            if fun.get_params().iter().any(|(_, ty)| matches!(ty, Some(LuaType::Variadic(_))))
                    );
                    if is_variadic_callback
                        && let Some(closure) = arg.closure_syntax
                        && let Some(fun) = model.type_of_signature(closure)
                    {
                        LuaType::DocFunction(Rc::new(fun))
                    } else {
                        param_ty.clone()
                    }
                } else if matches!(param_ty, LuaType::TplRef(_)) {
                    // Keep the original table identity for generic args so later expansion
                    // like std.Unpack still works.
                    arg.ty.clone()
                } else if matches!(param_ty, LuaType::Array(_)) {
                    // Array param: structurally unify table literals by element type
                    // (`map({1,2,3}, ...)`).
                    table_literal_as_array(model, &arg.ty).unwrap_or_else(|| arg.ty.clone())
                } else if let LuaType::Generic(map) = &param_ty
                    && map.get_base_type_id().get_name() == "table"
                    && map.get_params().len() == 2
                {
                    // `table<K, V>` param: unify table literals by key and value types.
                    table_literal_as_map(model, &arg.ty, map)
                        .unwrap_or_else(|| structural_table_type(model, &arg.ty))
                } else {
                    structural_table_type(model, &arg.ty)
                }
            })
            .unwrap_or(LuaType::Unknown);
        // Keep generics inside function signatures (`fun(...: T...)`) unsubstituted so
        // `unify_call_bindings` can use prebound values in `bindings` to split consecutive
        // variadic generic slots (`fun(_: T1..., _: T2...)`).
        let unify_param = if matches!(raw_param_ty, LuaType::DocFunction(_)) {
            &raw_param_ty
        } else {
            &param_ty
        };
        let arg_ty = normalize_missing_generic_arg(model, &arg_ty, &param_ty);
        let call_bind_ok = if arg.is_none() && type_contains_tpl_ref(&param_ty) {
            // When an arg is missing, do not bind `T` to Unknown: leave it to call-site
            // default/constraint filling, otherwise `---@param a T?` returns would turn
            // `Mock<Procedure>` into `Mock<unknown>`.
            true
        } else if matches!(arg_ty, LuaType::Object(_))
            && matches!(
                raw_param_ty,
                LuaType::Ref(_) | LuaType::Def(_) | LuaType::Generic(_)
            )
        {
            // Named-type param + table-literal arg: structural matching is handled by type
            // checking/context inference, so accept here to avoid generic unification
            // rejecting `fnA({ hook = function(obj) ... end })`.
            true
        } else {
            unify_call_bindings(model, unify_param, &arg_ty, &mut bindings)
        };
        if !call_bind_ok {
            return None;
        }
        // `{}` says nothing about the element: it binds a `V[]` element generic to `any`.
        if let (Some(arg), LuaType::Array(array)) = (arg, &param_ty)
            && let LuaType::TplRef(tpl) = array.get_base()
            && !bindings.contains_key(&tpl.get_tpl_id())
            && is_empty_table_literal(model, &arg.ty)
        {
            bindings.insert(tpl.get_tpl_id(), LuaType::Any);
        }
        score += if arg.is_some() {
            score_param_match(&param_ty, &arg_ty)
        } else {
            -50
        };
    }
    let surplus = if takes_rest {
        0
    } else {
        args.len()
            .saturating_sub(arg_start + params.len().saturating_sub(param_start))
    };

    Some((
        bindings,
        CallRank {
            surplus: Reverse(surplus),
            score,
        },
    ))
}

/// Prebind T from call-site variadic generic params (`... T...`).
///
/// Only top-level variadic slots participate; `T...` nested inside callback function
/// signatures is left for later `DocFunction` structural unification to split. The
/// bindings here are only seeds for later matching, so callback parameter grouping is not
/// fixed prematurely.
fn prebind_variadic_generics(
    fun: &LuaFunctionType,
    args: &[CallArg],
    param_start: usize,
    arg_start: usize,
    bindings: &mut TplBindings,
) {
    for (param_index, (name, param_ty)) in fun.get_params().iter().enumerate().skip(param_start) {
        let Some(param_ty) = param_ty else {
            continue;
        };
        let is_variadic = name == "..." || matches!(param_ty, LuaType::Variadic(_));
        if !is_variadic || !matches!(param_ty, LuaType::Variadic(_)) {
            continue;
        }
        let Some(LuaType::TplRef(tpl)) = variadic_base_type(param_ty) else {
            continue;
        };
        if bindings.contains_key(&tpl.get_tpl_id()) {
            continue;
        }
        let index = arg_start + (param_index - param_start);
        let rest_types: Vec<LuaType> = args
            .get(index..)
            .unwrap_or(&[])
            .iter()
            .map(|arg| widen_variadic_const(&arg.ty))
            .collect();
        if rest_types.is_empty()
            || rest_types
                .iter()
                .all(|ty| matches!(ty, LuaType::Unknown | LuaType::Any))
        {
            continue;
        }
        let bound = if rest_types.len() == 1 {
            rest_types[0].clone()
        } else {
            LuaType::Tuple(Rc::new(LuaTupleType::new(
                rest_types,
                LuaTupleStatus::InferResolve,
            )))
        };
        let _ = unify::unify_bindings(&LuaType::TplRef(tpl.clone()), &bound, bindings);
    }
}

/// Collect pcall callback returns: supports callable members of functions, aliases, and
/// unions/intersections. Uses OverloadResolver to filter callable candidates by args.
pub fn pcall_callback_ret(
    model: &SemanticModel,
    arg_ty: &LuaType,
    owner: Option<&SemanticId>,
    closure_syntax: Option<LuaSyntaxId>,
    call_args: &[CallArg],
) -> Option<(LuaType, bool)> {
    pcall_callback_ret_inner(model, arg_ty, owner, closure_syntax, call_args)
}

/// Build pcall's full return type from its callback return:
/// `[Boolean, R|String?]` (with String when the error slot is explicitly retained).
pub fn pcall_return_type(callback_ret: LuaType, include_error_string: bool) -> LuaType {
    // A variadic return slot (`R...`) may be zero values in Lua; when pcall retains that
    // slot, use nil as a fallback.
    fn with_nil_for_variadic(ty: LuaType) -> LuaType {
        if ty.contain_multi_return() {
            LuaType::from_vec(vec![ty, LuaType::Nil])
        } else {
            ty
        }
    }

    // Only Multi lists separate result slots. Keep a repeated tail as a sequence.
    let callback_slots = if let LuaType::Variadic(variadic) = &callback_ret
        && let VariadicType::Multi(types) = variadic.as_ref()
    {
        types.clone()
    } else {
        vec![callback_ret]
    };
    let mut slots = vec![LuaType::Boolean];
    for (index, ty) in callback_slots.into_iter().enumerate() {
        let slot = if index == 0 && include_error_string {
            LuaType::from_vec(vec![ty, LuaType::String])
        } else {
            ty
        };
        slots.push(with_nil_for_variadic(slot));
    }
    LuaType::Variadic(Rc::new(VariadicType::Multi(slots)))
}

fn pcall_callback_ret_inner(
    model: &SemanticModel,
    ty: &LuaType,
    owner: Option<&SemanticId>,
    closure_syntax: Option<LuaSyntaxId>,
    call_args: &[CallArg],
) -> Option<(LuaType, bool)> {
    match ty {
        LuaType::DocFunction(fun) => {
            let ret = if !call_args.is_empty() {
                if let Some((bindings, _)) =
                    match_call_candidate(model, fun, call_args, false, None)
                {
                    let ret = bind_signature_generics(fun.get_ret(), fun.get_generic_params());
                    let ret = unify::substitute(&ret, &bindings);
                    let ret = type_eval::expand_alias_generic(model, &ret);
                    type_eval::eval_conditionals(model, &ret)
                } else {
                    fun.get_ret().clone()
                }
            } else {
                fun.get_ret().clone()
            };
            Some((ret, true))
        }
        LuaType::Any | LuaType::Unknown => Some((LuaType::Unknown, true)),
        LuaType::Union(union) => {
            let mut types = Vec::new();
            for component in union.into_vec() {
                if let LuaType::DocFunction(fun) = &component
                    && !call_args.is_empty()
                    && match_call_candidate(model, fun, call_args, false, None).is_none()
                {
                    continue;
                }
                if let Some((ret, _)) =
                    pcall_callback_ret_inner(model, &component, owner, closure_syntax, call_args)
                {
                    if !types.contains(&ret) {
                        types.push(ret);
                    }
                }
            }
            if types.is_empty() {
                None
            } else {
                Some((LuaType::from_vec(types), true))
            }
        }
        LuaType::Intersection(intersection) => {
            let mut types = Vec::new();
            for component in intersection.get_types() {
                if let LuaType::DocFunction(fun) = component
                    && !call_args.is_empty()
                    && match_call_candidate(model, fun, call_args, false, None).is_none()
                {
                    continue;
                }
                if let Some((ret, _)) =
                    pcall_callback_ret_inner(model, component, owner, closure_syntax, call_args)
                {
                    if !types.contains(&ret) {
                        types.push(ret);
                    }
                }
            }
            if types.is_empty() {
                None
            } else {
                Some((LuaType::from_vec(types), true))
            }
        }
        LuaType::Ref(id) | LuaType::Def(id) => {
            let def = model.type_def_of(id)?;
            if def.kind == TypeDefKind::Alias {
                let target = model.alias_target(&def)?;
                if let LuaType::DocFunction(fun) = &target
                    && !call_args.is_empty()
                    && match_call_candidate(model, fun, call_args, false, None).is_none()
                {
                    return None;
                }
                let (ret, _) =
                    pcall_callback_ret_inner(model, &target, owner, closure_syntax, call_args)?;
                Some((ret, true))
            } else {
                None
            }
        }
        LuaType::Function | LuaType::Signature(_) => {
            // A closure literal passed directly: return according to its closure signature.
            if let Some(closure_syntax) = closure_syntax
                && let Some(fun) = model.type_of_signature(closure_syntax)
            {
                return Some((fun.get_ret().clone(), false));
            }
            let SemanticId::Member(key) = owner? else {
                return None;
            };
            let facts = model.file_facts_of(key.file_id)?;
            let member = facts.member_by_id(&SemanticId::Member(key.clone()))?;
            let closure = member.value_syntax?;
            let fun = model.type_of_signature_in_file(key.file_id, closure)?;
            Some((fun.get_ret().clone(), false))
        }
        _ => None,
    }
}
