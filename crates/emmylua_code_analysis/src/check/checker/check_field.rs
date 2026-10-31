use std::collections::{HashMap, HashSet};

use emmylua_parser::{
    LuaAssignStat, LuaAst, LuaAstNode, LuaCallExpr, LuaDocType, LuaExpr, LuaIndexKey, LuaLocalStat,
    LuaTableExpr, LuaVarExpr,
};
use rowan::TextRange;

use crate::DiagnosticCode;
use crate::semantic_db::def::{DeclKind, SemanticId, TypeDef, TypeDefKind};
use crate::semantic_model::SemanticModel;
use crate::semantic_model::type_eval::expand_alias_branches;
use crate::{FileId, LuaMemberKey, LuaType, LuaTypeDeclId, LuaUnionType};

use super::assign_type_mismatch::index_key_type;
use super::param_count::call_arg_param_index;
use super::param_type_check::callable_candidates;
use super::undefined_field::table_binding;
use super::{CheckContext, Checker};
use crate::semantic_model::render::humanize_type;

pub struct CheckFieldChecker;

impl Checker for CheckFieldChecker {
    const CODES: &[DiagnosticCode] = &[DiagnosticCode::MissingFields, DiagnosticCode::InjectField];

    fn check(context: &mut CheckContext<'_>, semantic_model: &SemanticModel<'_>) {
        let Some(tree) = semantic_model.syntax_tree() else {
            return;
        };
        let root = tree.get_red_root();
        for node in root.descendants().filter_map(LuaAst::cast) {
            match node {
                LuaAst::LuaLocalStat(local_stat) => {
                    check_local(context, semantic_model, &local_stat);
                }
                LuaAst::LuaAssignStat(assign_stat) => {
                    check_assign(context, semantic_model, &assign_stat);
                    check_index_assign(context, semantic_model, &assign_stat);
                }
                LuaAst::LuaCallExpr(call_expr) => {
                    check_call_args(context, semantic_model, &call_expr)
                }
                _ => {}
            }
        }
    }
}

/// `---@type C\nlocal c = { ... }`.
fn check_local(
    context: &mut CheckContext<'_>,
    semantic_model: &SemanticModel<'_>,
    local_stat: &LuaLocalStat,
) {
    let name_list = local_stat.get_local_name_list().collect::<Vec<_>>();
    let value_exprs = local_stat.get_value_exprs().collect::<Vec<_>>();
    for (index, local_name) in name_list.iter().enumerate() {
        let Some(LuaExpr::TableExpr(table)) = value_exprs.get(index) else {
            continue;
        };
        let Some(decl_id) = semantic_model.decl_by_offset(local_name.get_range().start()) else {
            continue;
        };
        check_against(context, semantic_model, &decl_id, table);
    }
}

/// `c = { ... }` (c has a doc type).
fn check_assign(
    context: &mut CheckContext<'_>,
    semantic_model: &SemanticModel<'_>,
    assign_stat: &LuaAssignStat,
) {
    let (vars, exprs) = assign_stat.get_var_and_expr_list();
    for (var, expr) in vars.iter().zip(exprs.iter()) {
        let LuaExpr::TableExpr(table) = expr else {
            continue;
        };
        let LuaVarExpr::NameExpr(name_expr) = var else {
            continue;
        };
        let Some(decl_id) = semantic_model.resolve_name(name_expr.get_position()) else {
            continue;
        };
        check_against(context, semantic_model, &decl_id, table);
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
                    .applicable_index_infos(&prefix_ty, &index_key_type(semantic_model, &index_key))
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
        // `table<K, V>` is a map: a key `K` accepts is an entry, not an injected field.
        if prefix_ty.table_generic_params().is_some()
            && semantic_model
                .applicable_index_infos(&prefix_ty, &index_key_type(semantic_model, &index_key))
                .is_ok_and(|infos| !infos.is_empty())
        {
            continue;
        }
        let Some(target_id) = resolve_target(semantic_model, &prefix_ty) else {
            continue;
        };
        let Some(def) = find_type_def(semantic_model, &target_id) else {
            continue;
        };
        // Classes with a parent type or index signature (`[string]`) are handled broadly (M0).
        if !def.base_types.is_empty() || is_mapped_alias(semantic_model, &def) {
            continue;
        }
        if semantic_model.member_info(&prefix_ty, &key).is_some()
            || is_global_table_surface_member(semantic_model, &target_id, &key)
        {
            continue;
        }
        match semantic_model
            .applicable_index_infos(&prefix_ty, &index_key_type(semantic_model, &index_key))
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

/// Follows alias definitions until a non-alias type is reached. Returns `None` when the
/// input is not an alias (or an alias cycle/unresolved target prevents expansion).
fn expand_alias_type(semantic_model: &SemanticModel<'_>, ty: &LuaType) -> Option<LuaType> {
    let mut current = ty.clone();
    let mut visited = Vec::new();
    loop {
        let id = match &current {
            LuaType::Ref(id) | LuaType::Def(id) => id.clone(),
            _ => {
                return if visited.is_empty() {
                    None
                } else {
                    Some(current)
                };
            }
        };
        let def = semantic_model.type_def_of(&id)?;
        if def.kind != TypeDefKind::Alias {
            return if visited.is_empty() {
                None
            } else {
                Some(current)
            };
        }
        if visited.contains(&id) {
            return None;
        }
        visited.push(id.clone());
        current = semantic_model.alias_target(&def)?;
    }
}

/// Static table key -> `LuaMemberKey` (dynamic keys are not checked).
pub(super) fn static_member_key(
    semantic_model: &SemanticModel<'_>,
    key: &LuaIndexKey,
) -> Option<LuaMemberKey> {
    match key {
        LuaIndexKey::Name(name) => Some(LuaMemberKey::Name(name.get_name_text().into())),
        LuaIndexKey::String(string) => Some(LuaMemberKey::Name(string.get_value().into())),
        LuaIndexKey::Integer(integer) => match integer.get_number_value() {
            emmylua_parser::NumberResult::Int(idx) => Some(LuaMemberKey::Integer(idx)),
            // The upper bound rounds up to 2^63, so it must be exclusive.
            emmylua_parser::NumberResult::Float(idx)
                if idx.fract() == 0.0 && (i64::MIN as f64..i64::MAX as f64).contains(&idx) =>
            {
                Some(LuaMemberKey::Integer(idx as i64))
            }
            _ => None,
        },
        LuaIndexKey::Idx(idx) => Some(LuaMemberKey::Integer(*idx as i64)),
        LuaIndexKey::Expr(expr) => match semantic_model.type_of_expr(expr.get_syntax_id()) {
            LuaType::StringConst(s) | LuaType::DocStringConst(s) => {
                Some(LuaMemberKey::Name(s.as_ref().clone()))
            }
            LuaType::IntegerConst(i) | LuaType::DocIntegerConst(i) => {
                Some(LuaMemberKey::Integer(i))
            }
            _ => None,
        },
    }
}

/// The array-like member of a union that a sequence literal (`{ 1, 2 }`) is checked
/// against: `Plain|integer[]` takes `{ 1, 2 }` as the array, not as a `Plain` with fields
/// `[1]`, `[2]` and none of its own.
pub(super) fn union_array_for_sequence(
    union: &LuaUnionType,
    table: &LuaTableExpr,
) -> Option<LuaType> {
    if !table.is_array() {
        return None;
    }
    union
        .into_vec()
        .into_iter()
        .find(|component| match component {
            LuaType::Array(_) | LuaType::Tuple(_) | LuaType::Table => true,
            LuaType::Generic(generic) => {
                matches!(generic.get_base_type_id().get_name(), "array" | "table")
            }
            _ => false,
        })
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

/// `f({ ... })`: check call-argument table literals for missing/extra fields against the parameter type.
fn check_call_args(
    context: &mut CheckContext<'_>,
    semantic_model: &SemanticModel<'_>,
    call_expr: &LuaCallExpr,
) {
    let Some(callee) = call_expr.get_prefix_expr() else {
        return;
    };
    let candidates = callable_candidates(semantic_model, &callee);
    let Some(args) = call_expr.get_args_list() else {
        return;
    };
    let colon_call = call_expr.is_colon_call();
    for candidate in &candidates {
        let params = candidate.get_params();
        for (index, arg) in args.get_args().enumerate() {
            let LuaExpr::TableExpr(table) = arg else {
                continue;
            };
            let Some((_, Some(param_ty))) = call_arg_param_index(candidate, index, colon_call)
                .and_then(|param_index| params.get(param_index))
            else {
                continue;
            };
            check_against_type(context, semantic_model, param_ty, &table);
        }
    }
}

fn check_against(
    context: &mut CheckContext<'_>,
    semantic_model: &SemanticModel<'_>,
    decl_id: &SemanticId,
    table: &LuaTableExpr,
) {
    // Target type: projection of the declared doc annotation.
    let Some(facts) = semantic_model.file_facts() else {
        return;
    };
    let Some(decl) = facts.decl_by_id(decl_id) else {
        return;
    };
    let Some(type_syntax) = decl.doc_type_syntax else {
        return;
    };
    // The shell lowering loses `A & B` (to unknown); the rich projection keeps it.
    let shell = semantic_model.doc_type_lua(type_syntax);
    let target = semantic_model.rich_doc_type_or(semantic_model.file_id(), type_syntax, shell);
    check_against_type(context, semantic_model, &target, table);
}

fn check_against_type(
    context: &mut CheckContext<'_>,
    semantic_model: &SemanticModel<'_>,
    target: &LuaType,
    table: &LuaTableExpr,
) {
    // Expand aliases before checking table literals against them. This makes alias-to-object,
    // alias-to-class and alias-to-unknown behave like their target types.
    if let Some(expanded) = expand_alias_type(semantic_model, target) {
        check_against_type(context, semantic_model, &expanded, table);
        return;
    }
    match target {
        LuaType::Array(array) => {
            for (field, key) in table.get_fields_with_keys() {
                if let Some(LuaExpr::TableExpr(value)) = field.get_value_expr()
                    && semantic_model
                        .applicable_index_infos(target, &index_key_type(semantic_model, &key))
                        .is_ok_and(|infos| !infos.is_empty())
                {
                    check_against_type(context, semantic_model, array.get_base(), &value);
                }
            }
            return;
        }
        LuaType::Intersection(intersection) => {
            for component in intersection.get_types() {
                check_against_type(context, semantic_model, component, table);
            }
            return;
        }
        LuaType::Union(union) => {
            if let Some(array) = union_array_for_sequence(union, table) {
                check_against_type(context, semantic_model, &array, table);
                return;
            }
            // The literal fits the union when it fits one table-shaped component cleanly
            // (`Text|Extmark` takes `{ 'x', 'Hl' }` as a Text). Otherwise report against the
            // first named-class component; empty tables pass through the array component.
            let fields_count = table.get_fields_with_keys().len();
            let components = union.into_vec();
            let shaped: Vec<_> = components
                .iter()
                .filter(|component| match component {
                    LuaType::Ref(_) | LuaType::Def(_) | LuaType::Generic(_) => true,
                    // The Object arm never reports a foreign key, so check its keys here.
                    LuaType::Object(_) => object_has_keys(semantic_model, component, table),
                    _ => false,
                })
                .collect();
            // With a single candidate there is nothing to choose: the fallback checks it.
            if fields_count > 0
                && shaped.len() > 1
                && shaped.into_iter().any(|component| {
                    let mut trial = CheckContext::new(semantic_model, context.config.clone());
                    check_against_type(&mut trial, semantic_model, component, table);
                    trial.diagnostics.is_empty()
                })
            {
                return;
            }
            for component in union.into_vec() {
                let named = matches!(
                    component,
                    LuaType::Ref(_) | LuaType::Def(_) | LuaType::Generic(_)
                );
                if named && fields_count > 0 {
                    check_against_type(context, semantic_model, &component, table);
                    return;
                }
                if matches!(component, LuaType::Array(_)) && fields_count == 0 {
                    return;
                }
            }
            return;
        }
        LuaType::Object(object) => {
            // By member key: a positional `{ 'x' }` sets `[1]` though it spells no key.
            let field_keys: HashSet<_> = table
                .get_fields_with_keys()
                .iter()
                .filter_map(|(_, key)| static_member_key(semantic_model, key))
                .collect();
            let mut missing = Vec::new();
            for (key, ty) in object.get_fields() {
                let name = match key {
                    LuaMemberKey::Name(name) => name.to_string(),
                    LuaMemberKey::Integer(i) => i.to_string(),
                    _ => continue,
                };
                if !field_keys.contains(key) && !ty.is_nullable() {
                    missing.push(name);
                }
            }
            if !missing.is_empty() {
                context.add_diagnostic(
                    DiagnosticCode::MissingFields,
                    table.get_range(),
                    format!(
                        "Missing required fields: {}",
                        missing
                            .iter()
                            .map(|name| format!("`{}`", name))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                );
            }
            return;
        }
        _ => {}
    }
    let Some(target_id) = resolve_target(semantic_model, target) else {
        return;
    };

    // Only the canonical standard-library dictionary bypasses closed-record
    // checks. A user-defined `table<T>` still has a normal class field surface.
    // This is after compound-type handling, so an intersection checks its other
    // components even when its dictionary component returns here.
    if let LuaType::Generic(generic) = target
        && generic.get_base_type_id() == LuaTypeDeclId::global("table")
        && find_type_def(semantic_model, &target_id)
            .is_some_and(|def| semantic_model.is_std_file(def.file_id))
    {
        return;
    }

    // Target type's @field members (including inheritance). A field of the literal the class is
    // bound to (`---@class C` + `local c = { x = 1 }`) may be set too, but it is a default value,
    // not a field each instance must restate. Any other literal (`---@type C local t = {...}`,
    // this table among them) cannot vouch for its own fields.
    let mut class_literal_keys = HashSet::new();
    let members: HashMap<_, _> = semantic_model
        .member_infos(target)
        .into_iter()
        .filter_map(|info| {
            let declaration = info.id.as_ref().and_then(|id| {
                semantic_model
                    .file_facts_of(info.file_id?)
                    .and_then(|facts| facts.member_by_id(id))
            });
            if let Some(member) = declaration
                && !matches!(member.owner, SemanticId::TypeDef(_))
            {
                // The literal's fields are owned by the literal (`Member` keyed by its range).
                let class_bound = match &member.owner {
                    SemanticId::Member(literal) => {
                        let at = (literal.file_id, literal.key_range);
                        at != (semantic_model.file_id(), table.get_range())
                            && is_class_literal(semantic_model, at.0, at.1)
                    }
                    _ => false,
                };
                if class_bound {
                    class_literal_keys.insert(info.key);
                }
                return None;
            }
            let typ = if declaration.is_some_and(|member| member.is_nullable)
                && !info.typ.is_nullable()
            {
                LuaType::from_vec(vec![info.typ, LuaType::Nil])
            } else {
                info.typ
            };
            Some((info.key, typ))
        })
        .collect();
    let fields = table.get_fields_with_keys();
    let mut field_keys = HashSet::with_capacity(fields.len());

    // InjectField: table field is not a member of the target.
    for (field, key) in &fields {
        // Expression keys still need the index-signature checks for their values.
        let member_key = match key {
            LuaIndexKey::Expr(_) => None,
            _ => static_member_key(semantic_model, key),
        };
        if !member_key
            .as_ref()
            .is_some_and(|key| members.contains_key(key) || class_literal_keys.contains(key))
            && let Ok(infos) =
                semantic_model.applicable_index_infos(target, &index_key_type(semantic_model, key))
        {
            if let Some(info) = infos.first() {
                if let Some(LuaExpr::TableExpr(value)) = field.get_value_expr() {
                    check_against_type(context, semantic_model, &info.value_type, &value);
                }
            } else {
                context.add_diagnostic(
                    DiagnosticCode::InjectField,
                    table.get_range(),
                    format!(
                        "Field `{}` cannot be injected into the target type.",
                        key.get_path_part()
                    ),
                );
            }
        }
        if let Some(key) = member_key {
            field_keys.insert(key);
        }
    }

    // MissingFields: required (non-nullable) members are missing.
    let mut missing: Vec<_> = members
        .iter()
        .filter(|(key, ty)| !field_keys.contains(*key) && !is_optional(semantic_model, ty))
        .map(|(key, _)| key.to_path())
        .collect();
    missing.sort_unstable();
    if !missing.is_empty() {
        context.add_diagnostic(
            DiagnosticCode::MissingFields,
            table.get_range(),
            format!(
                "Missing required fields: {}",
                missing
                    .iter()
                    .map(|name| format!("`{}`", name))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        );
    }
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

/// Every key of `table` is a field of the object type `ty` or fits its index signature.
fn object_has_keys(semantic_model: &SemanticModel<'_>, ty: &LuaType, table: &LuaTableExpr) -> bool {
    let LuaType::Object(object) = ty else {
        return false;
    };
    table.get_fields_with_keys().iter().all(|(_, key)| {
        let named = match key {
            LuaIndexKey::Expr(_) => None,
            _ => static_member_key(semantic_model, key),
        };
        named.is_some_and(|key| object.get_field(&key).is_some())
            || semantic_model
                .applicable_index_infos(ty, &index_key_type(semantic_model, key))
                .is_ok_and(|infos| !infos.is_empty())
    })
}

/// Target type -> named type id (Ref/Def/Generic base class).
fn resolve_target(semantic_model: &SemanticModel<'_>, ty: &LuaType) -> Option<SemanticId> {
    match ty {
        LuaType::Ref(id) | LuaType::Def(id) => semantic_model.type_def_of(id).map(|def| def.id),
        LuaType::Generic(generic) => semantic_model
            .type_def_of(&generic.get_base_type_id())
            .map(|def| def.id),
        _ => None,
    }
}

/// Find a type definition by id (walk the workspace type index).
fn find_type_def(semantic_model: &SemanticModel<'_>, type_def_id: &SemanticId) -> Option<TypeDef> {
    let SemanticId::TypeDef(key) = type_def_id else {
        return None;
    };
    let defs = semantic_model.type_defs_in_scope(key.scope, &key.full_name);
    defs.iter().next().cloned()
}

/// Whether the field is optional (nullable / unknown).
pub(super) fn is_optional(semantic_model: &SemanticModel<'_>, ty: &LuaType) -> bool {
    let ty = expand_alias_branches(semantic_model, ty).unwrap_or_else(|| ty.clone());
    match &ty {
        LuaType::Any | LuaType::Unknown | LuaType::Nil => true,
        LuaType::Union(union) => union.into_vec().iter().any(|t| matches!(t, LuaType::Nil)),
        _ => false,
    }
}
