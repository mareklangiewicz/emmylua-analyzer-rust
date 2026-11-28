use std::collections::HashSet;

use crate::InFiled;
use crate::semantic_model;
use emmylua_parser::{LuaAssignStat, LuaAstNode, LuaDocType, LuaTableExpr, LuaVarExpr};
use rowan::TextRange;

use crate::DiagnosticCode;
use crate::semantic_db::def::{DeclKind, SemanticId, TypeDef, TypeDefKind};
use crate::semantic_model::SemanticModel;
use crate::semantic_model::member::class_fields;
use crate::semantic_model::type_eval::{expand_alias_branches, expand_alias_head};
use crate::{FileId, LuaMemberKey, LuaType, LuaTypeDeclId, LuaUnionType};

use super::undefined_field::table_binding;
use super::{CheckContext, Checker, Mismatch};
use crate::semantic_model::render::humanize_type;
use crate::semantic_model::table_constructor::{TableConstructor, static_member_key, table_expr};
use crate::semantic_model::type_check::table_index_accepts;

pub struct CheckFieldChecker;

impl Checker for CheckFieldChecker {
    const CODES: &[DiagnosticCode] = &[DiagnosticCode::InjectField];

    fn check(context: &mut CheckContext<'_>, semantic_model: &SemanticModel<'_>) {
        let Some(tree) = semantic_model.syntax_tree() else {
            return;
        };
        for stat in tree
            .get_red_root()
            .descendants()
            .filter_map(LuaAssignStat::cast)
        {
            check_index_assign(context, semantic_model, &stat);
        }
    }
}

/// `test.a = 1` (test is a named type): if the target type has no such static member -> InjectField.
/// M0: only handle named classes without a parent type; dynamic keys, index signatures, and Object types are left for later.
fn check_index_assign(
    context: &mut CheckContext<'_>,
    semantic_model: &SemanticModel<'_>,
    assign_stat: &LuaAssignStat,
) {
    let (vars, _) = assign_stat.get_var_and_expr_list();
    for var in vars {
        let LuaVarExpr::IndexExpr(index_expr) = var else {
            continue;
        };
        let Some(prefix) = index_expr.get_prefix_expr() else {
            continue;
        };
        let prefix_ty = semantic_model.type_of_expr(prefix.get_syntax_id());
        let Some(index_key) = index_expr.get_index_key() else {
            continue;
        };
        let Some(key) = static_member_key(semantic_model, &index_key) else {
            continue;
        };
        // Aliases must be expanded before deciding whether a field can be injected:
        // `---@alias Anything unknown` should allow `a.foo = 1`, while a class alias should
        // use the underlying class member surface.
        let prefix_ty = expand_alias_type(semantic_model, &prefix_ty).unwrap_or(prefix_ty);
        // `---@type { [number]: number }`: keys outside the object's index signature report injection errors.
        if let LuaType::Object(object) = &prefix_ty {
            if object.get_field(&key).is_none()
                && semantic_model
                    .applicable_index_infos(
                        &prefix_ty,
                        &key.to_index_type().unwrap_or(LuaType::Unknown),
                    )
                    .is_ok_and(|infos| infos.is_empty())
            {
                context.add_diagnostic(
                    DiagnosticCode::InjectField,
                    index_key
                        .get_range()
                        .unwrap_or_else(|| index_expr.get_range()),
                    t!(
                        "Fields cannot be injected into the reference of `%{class}` for `%{field}`. ",
                        class = humanize_type(semantic_model, &prefix_ty),
                        field = index_key.get_path_part()
                    ),
                );
            }
            continue;
        }
        let Some(def) = target_def(semantic_model, &prefix_ty) else {
            continue;
        };
        // Classes with a parent type or index signature (`[string]`) are handled broadly (M0).
        if !def.base_types.is_empty() || is_mapped_alias(semantic_model, &def) {
            continue;
        }
        if semantic_model.member_info(&prefix_ty, &key).is_some()
            || is_global_table_surface_member(semantic_model, &def.id, &key)
        {
            continue;
        }
        match semantic_model
            .applicable_index_infos(&prefix_ty, &key.to_index_type().unwrap_or(LuaType::Unknown))
        {
            Ok(infos) if infos.is_empty() => {}
            _ => continue,
        }
        context.add_diagnostic(
            DiagnosticCode::InjectField,
            index_key
                .get_range()
                .unwrap_or_else(|| index_expr.get_range()),
            t!(
                "Fields cannot be injected into the reference of `%{class}` for `%{field}`. ",
                class = humanize_type(semantic_model, &prefix_ty),
                field = index_key.get_path_part()
            ),
        );
    }
}

/// A class bound to a GLOBAL table (`---@class packagelib` + `package = {}`) has that table's
/// assignments (`package.path = ""`) as members, owned by `Name("package")` rather than by the class.
/// Only the binding file's assignments are the surface; one another file adds stays an injection.
fn is_global_table_surface_member(
    semantic_model: &SemanticModel<'_>,
    target_id: &SemanticId,
    key: &LuaMemberKey,
) -> bool {
    let LuaMemberKey::Name(name) = key else {
        return false;
    };
    semantic_model
        .resolve_owner_set(target_id.clone())
        .iter()
        .filter_map(|owner| match owner {
            SemanticId::Decl(decl_key) => semantic_model
                .file_facts_of(decl_key.file_id)
                .and_then(|facts| facts.decl_by_id(owner))
                .filter(|decl| matches!(decl.kind, DeclKind::Global))
                .map(|decl| (decl_key.file_id, decl.name.clone())),
            _ => None,
        })
        .any(|(file_id, global)| {
            semantic_model
                .members_of_owner_named(&SemanticId::name(global), name)
                .iter()
                .any(|member_ref| member_ref.file_id == file_id)
        })
}

/// Resolve alias heads through the shared evaluator.
fn expand_alias_type(semantic_model: &SemanticModel<'_>, ty: &LuaType) -> Option<LuaType> {
    let expanded = expand_alias_head(semantic_model, ty)?;
    (expanded != *ty).then_some(expanded)
}

/// Whether the alias target is a mapped type.
fn is_mapped_alias(semantic_model: &SemanticModel<'_>, def: &TypeDef) -> bool {
    let Some(syntax) = def.alias_type else {
        return false;
    };
    let Some(tree) = semantic_model.syntax_tree_of(def.file_id) else {
        return false;
    };
    let Some(node) = syntax.to_node_from_root(&tree.get_red_root()) else {
        return false;
    };
    matches!(LuaDocType::cast(node), Some(LuaDocType::Mapped(_)))
}

pub(super) fn table_literal_mismatches(
    semantic_model: &SemanticModel<'_>,
    target: &LuaType,
    table: &LuaTableExpr,
) -> Vec<Mismatch> {
    let mut mismatches = Vec::new();
    collect_table_mismatches(&mut mismatches, semantic_model, target, table);
    mismatches
}

fn collect_table_mismatches(
    mismatches: &mut Vec<Mismatch>,
    semantic_model: &SemanticModel<'_>,
    target: &LuaType,
    table: &LuaTableExpr,
) {
    // Resolve union aliases once. A remaining alias is an unresolved cycle;
    // following it again here would bypass the evaluator's active alias path.
    let Some(target) = expand_alias_branches(semantic_model, target) else {
        return;
    };
    check_table_head(mismatches, semantic_model, &target, table);
}

fn check_table_head(
    mismatches: &mut Vec<Mismatch>,
    semantic_model: &SemanticModel<'_>,
    target: &LuaType,
    table: &LuaTableExpr,
) {
    check_table_part(mismatches, semantic_model, target, table, &[]);
}

/// `target` as one part of an intersection whose parts are `parts` (none outside one): a field
/// one part declares is not injected into another (`---@type IA & IB` takes `a` and `b`).
fn check_table_part(
    mismatches: &mut Vec<Mismatch>,
    semantic_model: &SemanticModel<'_>,
    target: &LuaType,
    table: &LuaTableExpr,
    parts: &[LuaType],
) {
    match target {
        LuaType::Array(array) => {
            for (field, key) in table.get_fields_with_keys() {
                if table_index_accepts(semantic_model, &LuaType::Integer, &key)
                    && let Some(value) = field.get_value_expr().and_then(|expr| table_expr(&expr))
                {
                    collect_table_mismatches(mismatches, semantic_model, array.get_base(), &value);
                }
            }
            return;
        }
        LuaType::Intersection(intersection) => {
            let parts = intersection.get_types();
            for component in parts {
                check_table_part(mismatches, semantic_model, component, table, parts);
            }
            return;
        }
        LuaType::Union(union) => {
            let source =
                LuaType::TableConst(InFiled::new(semantic_model.file_id(), table.get_range()));
            let mut best: Option<Vec<Mismatch>> = None;
            for component in union.iter() {
                // An unresolved alias must not restart expansion of this branch.
                if target_def(semantic_model, component)
                    .is_some_and(|def| def.kind == TypeDefKind::Alias)
                    || semantic_model::type_check::check_type_at(
                        semantic_model,
                        &source,
                        component,
                        table.get_range(),
                        false,
                    )
                    .is_err()
                {
                    continue;
                }
                let mut errors = Vec::new();
                check_table_head(&mut errors, semantic_model, component, table);
                if errors.is_empty() {
                    return;
                }
                if best
                    .as_ref()
                    .is_none_or(|current| errors.len() < current.len())
                {
                    best = Some(errors);
                }
            }
            if let Some(errors) = best {
                mismatches.extend(errors);
            } else if let Some(array) = union_array_for_sequence(union, table) {
                // A sequence literal no member takes is the array's: its elements are what
                // fail, not the record's missing fields (`Plain|integer[]` with `{ 1, 'two' }`).
                check_table_part(mismatches, semantic_model, &array, table, &[]);
            } else if let Some(component) = union.iter().find(|component| {
                matches!(component, LuaType::Object(_))
                    || target_def(semantic_model, component)
                        .is_some_and(|def| def.kind == TypeDefKind::Class)
            }) {
                // Keep a missing-field diagnostic when no alternative accepts the record.
                check_table_head(mismatches, semantic_model, component, table);
            }
            return;
        }
        _ => {}
    }
    let class;
    let mut bound_keys = HashSet::new();
    let (shape, closed) = if let LuaType::Object(object) = target {
        (object.as_ref(), false)
    } else {
        let Some(def) = target_def(semantic_model, target) else {
            return;
        };
        if def.kind != TypeDefKind::Class {
            return;
        }
        // Only the standard-library dictionary bypasses closed-record checks.
        if let LuaType::Generic(generic) = target
            && generic.get_base_type_id() == LuaTypeDeclId::global("table")
            && semantic_model.is_std_file(def.file_id)
        {
            return;
        }
        class = class_fields(semantic_model, target);
        bound_keys = class_literal_keys(semantic_model, target, table);
        (&class, true)
    };
    let members = shape.get_fields();
    let constructor = TableConstructor::new(semantic_model, table);
    let mut missing = Vec::new();
    for (key, ty) in members {
        match constructor.member_value(key) {
            Ok(None) if !is_optional(semantic_model, ty) => missing.push(key.to_path()),
            Ok(Some(value)) => {
                if let Some(child) = value.expr.and_then(table_expr) {
                    collect_table_mismatches(mismatches, semantic_model, ty, &child);
                }
            }
            _ => {}
        }
    }

    if closed {
        for (field, key) in table.get_fields_with_keys() {
            if static_member_key(semantic_model, &key).is_some_and(|key| {
                members.contains_key(&key)
                    || bound_keys.contains(&key)
                    || part_declares(semantic_model, parts, &key)
            }) {
                continue;
            }
            let expected = shape
                .get_index_access()
                .iter()
                .find(|(index, _)| table_index_accepts(semantic_model, index, &key))
                .map(|(_, value)| value);
            if let Some(expected) = expected {
                if let Some(value) = field.get_value_expr().and_then(|expr| table_expr(&expr)) {
                    collect_table_mismatches(mismatches, semantic_model, expected, &value);
                }
            } else {
                mismatches.push(Mismatch {
                    code: DiagnosticCode::InjectField,
                    range: table.get_range(),
                    message: format!(
                        "Field `{}` cannot be injected into the target type.",
                        key.get_path_part()
                    ),
                });
            }
        }
    }

    missing.sort_unstable();
    report_missing_fields(mismatches, table, missing);
}

fn report_missing_fields(
    mismatches: &mut Vec<Mismatch>,
    table: &LuaTableExpr,
    missing: Vec<String>,
) {
    if !missing.is_empty() {
        mismatches.push(Mismatch {
            code: DiagnosticCode::MissingFields,
            range: table.get_range(),
            message: format!(
                "Missing required fields: {}",
                missing
                    .iter()
                    .map(|name| format!("`{}`", name))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        });
    }
}

/// The fields of the literal the class `target` is bound to (`---@class C` + `local c = { x = 1 }`,
/// or `M.f = { ... }`): another literal of the class may set them too. They are default values,
/// not fields each instance must restate; `table` itself cannot vouch for its own fields.
fn class_literal_keys(
    semantic_model: &SemanticModel<'_>,
    target: &LuaType,
    table: &LuaTableExpr,
) -> HashSet<LuaMemberKey> {
    let here = (semantic_model.file_id(), table.get_range());
    semantic_model
        .member_infos(target)
        .into_iter()
        .filter(|info| {
            let Some(member) = info.id.as_ref().and_then(|id| {
                semantic_model
                    .file_facts_of(info.file_id?)
                    .and_then(|facts| facts.member_by_id(id))
            }) else {
                return false;
            };
            // The literal's fields are owned by the literal (`Member` keyed by its range).
            let SemanticId::Member(literal) = &member.owner else {
                return false;
            };
            let at = (literal.file_id, literal.key_range);
            at != here && is_class_literal(semantic_model, at.0, at.1)
        })
        .map(|info| info.key)
        .collect()
}

/// The literal at `range` is the value a `---@class` statement binds: `local c = { ... }`
/// (table_binding) or `M.f = { ... }`.
fn is_class_literal(semantic_model: &SemanticModel<'_>, file_id: FileId, range: TextRange) -> bool {
    if table_binding(semantic_model, file_id, range).is_some() {
        return true;
    }
    let Some(facts) = semantic_model.file_facts_of(file_id) else {
        return false;
    };
    facts
        .type_defs
        .iter()
        .filter_map(|def| def.owner_syntax)
        .any(|stat| {
            facts.members_by_owner_syntax(stat).any(|member| {
                member
                    .value_syntax
                    .is_some_and(|value| value.get_range() == range)
            })
        })
}

/// The array-like member of a union that a sequence literal (`{ 1, 2 }`) is checked
/// against: `Plain|integer[]` takes `{ 1, 2 }` as the array, not as a `Plain` with fields
/// `[1]`, `[2]` and none of its own.
fn union_array_for_sequence(union: &LuaUnionType, table: &LuaTableExpr) -> Option<LuaType> {
    if !table.is_array() {
        return None;
    }
    union
        .iter()
        .find(|component| match component {
            LuaType::Array(_) | LuaType::Tuple(_) | LuaType::Table => true,
            LuaType::Generic(generic) => {
                matches!(generic.get_base_type_id().get_name(), "array" | "table")
            }
            _ => false,
        })
        .cloned()
}

/// Whether some part of an intersection declares the field `key`.
fn part_declares(
    semantic_model: &SemanticModel<'_>,
    parts: &[LuaType],
    key: &LuaMemberKey,
) -> bool {
    parts.iter().any(|part| match part {
        LuaType::Object(object) => object.get_fields().contains_key(key),
        _ => {
            target_def(semantic_model, part).is_some_and(|def| def.kind == TypeDefKind::Class)
                && class_fields(semantic_model, part)
                    .get_fields()
                    .contains_key(key)
        }
    })
}

/// The definition of a named target, before alias expansion.
fn target_def(semantic_model: &SemanticModel<'_>, ty: &LuaType) -> Option<TypeDef> {
    match ty {
        LuaType::Ref(id) | LuaType::Def(id) => semantic_model.type_def_of(id),
        LuaType::Generic(generic) => semantic_model.type_def_of(&generic.get_base_type_id()),
        _ => None,
    }
}

/// Whether the field is optional (nullable / unknown).
fn is_optional(semantic_model: &SemanticModel<'_>, ty: &LuaType) -> bool {
    fn contains_nil(ty: &LuaType) -> bool {
        match ty {
            LuaType::Any | LuaType::Unknown | LuaType::Nil => true,
            LuaType::Union(union) => union.iter().any(contains_nil),
            LuaType::Intersection(intersection) => {
                intersection.get_types().iter().all(contains_nil)
            }
            _ => false,
        }
    }

    let ty = expand_alias_branches(semantic_model, ty).unwrap_or_else(|| ty.clone());
    contains_nil(&ty)
}
