//! Ref/Def type checks: nominal + inheritance (`is_sub_type_of` via semantic `super_names`).
//!
//! Parts still missing from semantic fall back: alias origin, enum field unions, call operator.

use crate::{LuaGenericType, LuaMemberKey, LuaObjectType, LuaType, LuaTypeDeclId, TypeDefKind};

use super::context::TypeCheckContext;
use super::sub_type::{get_base_type_id, is_sub_type_of};
use super::{
    RequiredChecks, TypeCheckResult, check_general_type_compact, table_associated_type_def,
};

/// Generic inheritance matching: whether sub's inheritance chain contains a generic parent matching the target generic instance arguments.
fn is_sub_type_of_generic(
    context: &TypeCheckContext,
    sub: &LuaTypeDeclId,
    target: &LuaGenericType,
) -> bool {
    use std::collections::HashSet;
    let target_base = target.get_base_type_id();
    if sub == &target_base && target.get_params().is_empty() {
        return true;
    }
    let mut stack = vec![sub.clone()];
    let mut visited = HashSet::new();
    visited.insert(sub.clone());
    while let Some(current) = stack.pop() {
        for super_type in context.super_types_of(&current) {
            match super_type {
                LuaType::Generic(super_generic)
                    if super_generic.get_base_type_id() == target_base =>
                {
                    if super_generic.get_params() == target.get_params() {
                        return true;
                    }
                }
                LuaType::Ref(super_id) => {
                    if super_id == target_base && target.get_params().is_empty() {
                        return true;
                    }
                    if visited.insert(super_id.clone()) {
                        stack.push(super_id);
                    }
                }
                _ => {}
            }
        }
    }
    false
}

pub fn check_ref_type_compact(
    context: &mut TypeCheckContext,
    source_id: &LuaTypeDeclId,
    compact_type: &LuaType,
) -> TypeCheckResult {
    // The type definition must exist.
    let Some(type_def) = context.type_def_of(source_id) else {
        return Err(context.mismatch(&LuaType::Ref(source_id.clone()), compact_type));
    };

    if type_def.kind == TypeDefKind::Enum {
        return check_ref_enum(context, source_id, compact_type);
    }

    check_ref_class(context, source_id, compact_type)
}

fn check_ref_enum(
    context: &mut TypeCheckContext,
    source_id: &LuaTypeDeclId,
    compact_type: &LuaType,
) -> TypeCheckResult {
    // Directly match the same type.
    if matches!(compact_type, LuaType::Def(id) | LuaType::Ref(id) if id == source_id) {
        return Ok(());
    }
    // Integer enum: broad Integer participates (semantic has no enum field union, so this is relaxed).
    if matches!(
        compact_type,
        LuaType::Integer | LuaType::IntegerConst(_) | LuaType::DocIntegerConst(_)
    ) {
        return Ok(());
    }
    // An enum's runtime table is still a table, so `pairs(DamageType)` etc. should be accepted.
    if matches!(compact_type, LuaType::Table) {
        return Ok(());
    }
    if let Some(signatures) = index_signatures(compact_type) {
        return check_member_map(context, source_id, compact_type, &signatures);
    }
    // Same base type name.
    if let Some(base_id) = get_base_type_id(compact_type)
        && is_sub_type_of(context, source_id, &base_id)
    {
        return Ok(());
    }
    Err(context.mismatch(&LuaType::Ref(source_id.clone()), compact_type))
}

fn check_ref_class(
    context: &mut TypeCheckContext,
    source_id: &LuaTypeDeclId,
    compact_type: &LuaType,
) -> TypeCheckResult {
    // `table<K, V>`, in either spelling, is a map rather than a nominal generic.
    if compact_type.table_generic_params().is_some()
        && let Some(signatures) = index_signatures(compact_type)
    {
        return check_member_map(context, source_id, compact_type, &signatures);
    }
    match compact_type {
        LuaType::Def(id) | LuaType::Ref(id) => {
            if source_id == id {
                return Ok(());
            }
            // Check subtype relationship (bidirectional: nominal assignment allows super ↔ sub).
            if is_sub_type_of(context, id, source_id) || is_sub_type_of(context, source_id, id) {
                return Ok(());
            }
            // Extra handling for enum targets (semantic has no field union, so fall back to base type name).
            if context.is_enum(id) {
                if let Some(base_id) = get_base_type_id(&LuaType::Ref(source_id.clone()))
                    && is_sub_type_of(context, id, &base_id)
                {
                    return Ok(());
                }
            }
            Err(context.mismatch(&LuaType::Ref(source_id.clone()), compact_type))
        }
        LuaType::Array(array) => {
            let value = context
                .require_complete(|| context.model.infer_iter_from_type_def(source_id, 1))?
                .ok_or_else(|| context.mismatch(&LuaType::Ref(source_id.clone()), compact_type))?;
            check_general_type_compact(context, &value, array.get_base())
        }
        LuaType::Table => Ok(()),
        LuaType::Generic(generic) => {
            let base_type_id = generic.get_base_type_id();
            if (context.strict_subtype
                && context
                    .require_complete(|| is_sub_type_of_generic(context, source_id, generic))?)
                || (!context.strict_subtype
                    && (source_id == &base_type_id
                        || is_sub_type_of(context, &base_type_id, source_id)
                        || is_sub_type_of(context, source_id, &base_type_id)))
            {
                Ok(())
            } else {
                Err(context.mismatch(&LuaType::Ref(source_id.clone()), compact_type))
            }
        }
        // Table/object/tuple/intersection structures: M0 uses base type name (`table`) plus nominal fallback (semantic member-structure checks are left for later).
        LuaType::TableConst(table) => {
            /*
               ---@class A
               local A = {}
               A.__index = A
            */
            if let Some(target_def) = table_associated_type_def(context.model, table)
                && target_def.kind == TypeDefKind::Class
            {
                check_general_type_compact(
                    context,
                    &LuaType::Ref(source_id.clone()),
                    &LuaType::Ref(target_def.type_decl_id()),
                )
            } else {
                if let Some(base_id) = get_base_type_id(compact_type)
                    && is_sub_type_of(context, source_id, &base_id)
                {
                    return Ok(());
                }
                Err(context.mismatch(&LuaType::Ref(source_id.clone()), compact_type))
            }
        }
        LuaType::Object(_) | LuaType::Tuple(_) | LuaType::Intersection(_) => {
            if let Some(base_id) = get_base_type_id(compact_type)
                && is_sub_type_of(context, source_id, &base_id)
            {
                return Ok(());
            }
            if let LuaType::Object(object) = compact_type {
                let mut checks = RequiredChecks::default();
                checks.require(check_class_fields(context, source_id, object))?;
                checks.require(check_member_map(
                    context,
                    source_id,
                    compact_type,
                    object.get_index_access(),
                ))?;
                return checks.finish();
            }
            Err(context.mismatch(&LuaType::Ref(source_id.clone()), compact_type))
        }
        _ => {
            if let Some(base_type_id) = get_base_type_id(compact_type) {
                if source_id == &base_type_id
                    || is_sub_type_of(context, &base_type_id, source_id)
                    || is_sub_type_of(context, source_id, &base_type_id)
                {
                    Ok(())
                } else {
                    Err(context.mismatch(&LuaType::Ref(source_id.clone()), compact_type))
                }
            } else {
                Err(context.mismatch(&LuaType::Ref(source_id.clone()), compact_type))
            }
        }
    }
}

/// A class is an object whose fields it has: `---@class W` with `address: string` (and more)
/// passes where `{ address: string }` is expected. A field the class lacks is checked as nil,
/// so it satisfies only an optional one.
fn check_class_fields(
    context: &mut TypeCheckContext,
    source_id: &LuaTypeDeclId,
    object: &LuaObjectType,
) -> TypeCheckResult {
    let source = LuaType::Ref(source_id.clone());
    let members = context.require_complete(|| context.model.member_infos(&source))?;
    let mut checks = RequiredChecks::default();
    for (key, expected) in object.get_fields() {
        let actual = members
            .iter()
            .find(|info| &info.key == key)
            .map(|info| info.typ.clone())
            .unwrap_or(LuaType::Nil);
        checks.require(check_general_type_compact(context, &actual, expected))?;
    }
    checks.finish()
}

/// The `[K]: V` index signatures of a map type: an object's, or `table<K, V>`'s one.
pub(super) fn index_signatures(ty: &LuaType) -> Option<Vec<(LuaType, LuaType)>> {
    match ty {
        LuaType::Object(object) if !object.get_index_access().is_empty() => {
            Some(object.get_index_access().to_vec())
        }
        _ => match ty.table_generic_params()? {
            [key, value] => Some(vec![(key.clone(), value.clone())]),
            _ => None,
        },
    }
}

/// A named table type is a map whose members fit it: each member's key fits some signature's
/// key type, and its value that signature's value type. `:` methods live on the class table, not
/// on the instance a map holds, so they are not checked.
fn check_member_map(
    context: &mut TypeCheckContext,
    source_id: &LuaTypeDeclId,
    map: &LuaType,
    signatures: &[(LuaType, LuaType)],
) -> TypeCheckResult {
    if signatures.is_empty() {
        return Ok(());
    }
    let source = LuaType::Ref(source_id.clone());
    let members = context.require_complete(|| context.model.member_infos(&source))?;
    let mut checks = RequiredChecks::default();
    for member in members.iter().filter(|member| !member.is_method) {
        let key = match (&member.index_key_type, &member.key) {
            (Some(key), _) => key.clone(),
            (None, LuaMemberKey::Name(name)) => LuaType::StringConst(name.clone().into()),
            (None, LuaMemberKey::Integer(i)) => LuaType::IntegerConst(*i),
            (None, LuaMemberKey::None) => continue,
        };
        let mut covered = false;
        for (key_ty, value_ty) in signatures {
            match check_general_type_compact(context, &key, key_ty) {
                Ok(()) => {
                    covered = true;
                    checks.require(check_general_type_compact(context, &member.typ, value_ty))?;
                }
                Err(err) if err.is_unresolved() => {
                    covered = true;
                    checks.require(Err(err))?;
                }
                Err(_) => {}
            }
        }
        if !covered {
            return Err(context.mismatch(&source, map));
        }
    }
    checks.finish()
}
