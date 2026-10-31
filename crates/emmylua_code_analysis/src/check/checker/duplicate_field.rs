//! # duplicate_field — duplicate member definitions in types / objects
//!
//! M0: group this file's members by owner; duplicate `@field` on a TypeDef -> `DuplicateDocField`;
//! duplicate runtime members on a Decl (same name defined multiple times) -> `DuplicateSetField`.

use std::collections::HashMap;

use emmylua_parser::{
    LuaAssignStat, LuaAstNode, LuaClosureExpr, LuaExpr, LuaIndexExpr, LuaSyntaxId, LuaSyntaxNode,
    LuaSyntaxTree, LuaVarExpr, PathTrait,
};

use rowan::TextRange;

use crate::DiagnosticCode;
use crate::FileId;
use crate::semantic_db::def::{Member, SemanticId};
use crate::semantic_model::SemanticModel;
use crate::semantic_model::render::humanize_type;
use crate::semantic_model::type_eval::expand_alias_branches;
use crate::semantic_model::type_eval::expand_alias_head;
use crate::{LuaMemberKey, LuaType};

use super::{CheckContext, Checker};

pub struct DuplicateFieldChecker;

impl Checker for DuplicateFieldChecker {
    const CODES: &[DiagnosticCode] = &[
        DiagnosticCode::DuplicateDocField,
        DiagnosticCode::DuplicateSetField,
    ];

    fn check(context: &mut CheckContext<'_>, semantic_model: &SemanticModel<'_>) {
        let Some(facts) = semantic_model.file_facts() else {
            return;
        };
        let tree = semantic_model.syntax_tree();
        let root = tree.map(|tree| tree.get_red_root());
        if let Some(root) = &root {
            check_cross_file_member_assign(context, semantic_model, root);
        }

        let mut by_owner: HashMap<(&SemanticId, Option<LuaSyntaxId>), Vec<&Member>> =
            HashMap::new();
        for member in &facts.members {
            by_owner
                .entry((&member.owner, member.implicit_self))
                .or_default()
                .push(member);
        }
        for ((owner, _), members) in by_owner {
            // Group by key and its index-signature domain, by the table the key is set on (a local
            // root's owner drops the path below it, so `M.A:init` and `M.B:init` share the owner
            // `M`), and by the function body that sets it: sets in different bodies run on
            // different calls.
            let mut by_key: HashMap<
                (LuaMemberKey, Option<SetSite>),
                Vec<(Option<LuaType>, Vec<&Member>)>,
            > = HashMap::new();
            for member in members {
                if root
                    .as_ref()
                    .is_some_and(|root| restores_saved_member(semantic_model, root, member))
                {
                    continue;
                }
                let domain = if let Some(syntax) = member.index_key_syntax {
                    let generics = facts
                        .type_def_by_id(&member.owner)
                        .map(|def| def.generic_params.as_slice())
                        .unwrap_or_default();
                    let key =
                        semantic_model.doc_type_lua_in(semantic_model.file_id(), syntax, generics);
                    let Some(key) = expand_alias_branches(semantic_model, &key) else {
                        continue;
                    };
                    Some(key)
                } else {
                    None
                };
                let site = root.as_ref().and_then(|root| set_site(root, member));
                let groups = by_key.entry((member.key.clone(), site)).or_default();
                if let Some((_, members)) = groups.iter_mut().find(|(key, _)| key == &domain) {
                    members.push(member);
                } else {
                    groups.push((domain, vec![member]));
                }
            }
            let is_type_def = matches!(owner, SemanticId::TypeDef(_));
            for ((key, _), groups) in by_key {
                for (domain, dupes) in groups {
                    let name = domain.map_or_else(
                        || key.to_path(),
                        |ty| format!("[{}]", humanize_type(semantic_model, &ty)),
                    );
                    if dupes.len() <= 1 {
                        continue;
                    }
                    let all_function = dupes
                        .iter()
                        .all(|member| is_function_like(semantic_model, tree, member));
                    // `@field a fun()` may be declared repeatedly (function overloads);
                    // ordinary repeated non-function field writes are also intentional
                    // (for example a global config table assigning the same key twice).
                    if is_type_def && all_function {
                        continue;
                    }
                    if !is_type_def && !all_function {
                        continue;
                    }
                    let code = if is_type_def {
                        DiagnosticCode::DuplicateDocField
                    } else {
                        DiagnosticCode::DuplicateSetField
                    };
                    for member in dupes {
                        if let Some(range) = member.id.member_key_range() {
                            context.add_diagnostic(
                                code,
                                range,
                                t!("Duplicate field `%{name}`.", name = name),
                            );
                        }
                    }
                }
            }
        }
    }
}

/// Where a member is set through an index expression: the access path of the table
/// (`M.A` for `function M.A:init()`) and the function body around the statement.
#[derive(PartialEq, Eq, Hash)]
struct SetSite {
    table_path: String,
    body: Option<TextRange>,
}

/// The set site of a member, or `None` for a table-constructor field.
fn set_site(root: &LuaSyntaxNode, member: &Member) -> Option<SetSite> {
    let key_range = member.id.member_key_range()?;
    let token = root.token_at_offset(key_range.start()).right_biased()?;
    let index_expr = token.parent_ancestors().find_map(LuaIndexExpr::cast)?;
    if index_expr.get_index_key()?.get_range()? != key_range {
        return None;
    }
    let table_path = match index_expr.get_prefix_expr()? {
        LuaExpr::NameExpr(name_expr) => name_expr.get_access_path()?.to_string(),
        LuaExpr::IndexExpr(prefix) => prefix.get_access_path()?.to_string(),
        _ => return None,
    };
    let body = index_expr
        .ancestors::<LuaClosureExpr>()
        .next()
        .map(|closure| closure.get_range());
    Some(SetSite { table_path, body })
}

/// `local A = require("mod"); A.execute = function() end`:
/// the assignment target resolves to an existing method member in the module file -> cross-file duplicate definition DuplicateSetField.
fn check_cross_file_member_assign(
    context: &mut CheckContext<'_>,
    semantic_model: &SemanticModel<'_>,
    root: &LuaSyntaxNode,
) {
    let current_file = semantic_model.file_id();
    for assign_stat in root.descendants().filter_map(LuaAssignStat::cast) {
        let (vars, exprs) = assign_stat.get_var_and_expr_list();
        for (index, var) in vars.iter().enumerate() {
            let LuaVarExpr::IndexExpr(index_expr) = var else {
                continue;
            };
            let Some(prefix) = index_expr.get_prefix_expr() else {
                continue;
            };
            let Some(resolved) = semantic_model.resolve_member(index_expr) else {
                continue;
            };
            // The target must be a require'd module export (cross-file definition).
            let module_file = required_module_file(semantic_model, &prefix)
                .or_else(|| resolved.file_id.filter(|file| *file != current_file));
            let Some(module_file) = module_file else {
                continue;
            };
            if module_file == current_file {
                continue;
            }
            // The module file already has a same-named runtime function member -> this assignment is a duplicate implementation.
            let Some(module_facts) = semantic_model.file_facts_of(module_file) else {
                continue;
            };
            // `self.go = nil` sets no function: clearing a field is not a method to reimplement.
            let module_tree = semantic_model.syntax_tree_of(module_file);
            if !module_facts
                .members_named(resolved.name.as_str())
                .any(|member| {
                    member.value_syntax.is_some()
                        && is_function_like(semantic_model, module_tree, member)
                })
            {
                continue;
            }
            // Only function/closure right-hand sides count as "reimplementing an existing method"; plain field writes are not reported.
            let Some(rhs) = exprs.get(index) else {
                continue;
            };
            if reads_back(semantic_model, root, rhs, index_expr) {
                continue;
            }
            let rhs_ty = semantic_model.type_of_expr(rhs.get_syntax_id());
            if !matches!(
                rhs_ty,
                LuaType::DocFunction(_) | LuaType::Signature(_) | LuaType::Function
            ) {
                continue;
            }
            let Some(key_range) = index_expr.get_index_key().and_then(|key| key.get_range()) else {
                continue;
            };
            context.add_diagnostic(
                DiagnosticCode::DuplicateSetField,
                key_range,
                t!("Duplicate field `%{name}`.", name = resolved.name),
            );
        }
    }
}

/// require local variable -> module file (`local A = require("mod")`).
fn required_module_file(semantic_model: &SemanticModel<'_>, prefix: &LuaExpr) -> Option<FileId> {
    let LuaExpr::NameExpr(name_expr) = prefix else {
        return None;
    };
    let decl = semantic_model.resolve_name(name_expr.get_position())?;
    let facts = semantic_model.file_facts()?;
    let decl = facts.decl_by_id(&decl)?;
    let call_syntax = decl.value_expr_syntax?;
    let tree = semantic_model.syntax_tree()?;
    let node = call_syntax.to_node_from_root(&tree.get_red_root())?;
    let call = emmylua_parser::LuaCallExpr::cast(node)?;
    let arg = call.get_args_list()?.get_args().next()?;
    let ty = semantic_model.type_of_expr(arg.get_syntax_id());
    let module_name = match ty {
        LuaType::StringConst(s) | LuaType::DocStringConst(s) => s.as_ref().to_string(),
        _ => return None,
    };
    semantic_model.module_file_of(&module_name)
}

/// Whether `member` (set through an index expression) is a restore: `t.f = saved` where
/// `saved` is a local read from that same `t.f` (`local saved = t.f`).
fn restores_saved_member(
    semantic_model: &SemanticModel<'_>,
    root: &LuaSyntaxNode,
    member: &Member,
) -> bool {
    let Some(value_syntax) = member.value_syntax else {
        return false;
    };
    let Some(value) = value_syntax.to_node_from_root(root).and_then(LuaExpr::cast) else {
        return false;
    };
    let Some(key_range) = member.id.member_key_range() else {
        return false;
    };
    let Some(token) = root.token_at_offset(key_range.start()).right_biased() else {
        return false;
    };
    let Some(target) = token.parent_ancestors().find_map(LuaIndexExpr::cast) else {
        return false;
    };
    reads_back(semantic_model, root, &value, &target)
}

/// Whether `rhs` is a local that was initialized from `target`'s own access path and never
/// reassigned: putting a saved original back reimplements nothing.
fn reads_back(
    semantic_model: &SemanticModel<'_>,
    root: &LuaSyntaxNode,
    rhs: &LuaExpr,
    target: &LuaIndexExpr,
) -> bool {
    let LuaExpr::NameExpr(name_expr) = rhs else {
        return false;
    };
    let Some(target_path) = target.get_access_path() else {
        return false;
    };
    let Some(decl_id) = semantic_model.resolve_name(name_expr.get_position()) else {
        return false;
    };
    let Some(decl) = semantic_model
        .file_facts()
        .and_then(|facts| facts.decl_by_id(&decl_id))
    else {
        return false;
    };
    let Some(init) = decl
        .value_expr_syntax
        .and_then(|syntax| syntax.to_node_from_root(root))
        .and_then(LuaIndexExpr::cast)
    else {
        return false;
    };
    if init.get_access_path().as_deref() != Some(target_path.as_str()) {
        return false;
    }
    // A reassigned local may no longer hold the original.
    !root
        .descendants()
        .filter_map(LuaAssignStat::cast)
        .any(|assign| {
            assign.get_var_and_expr_list().0.iter().any(|var| {
                matches!(var, LuaVarExpr::NameExpr(name)
                if semantic_model.resolve_name(name.get_position()).as_ref() == Some(&decl_id))
            })
        })
}

/// Whether a member resolves to a function or has a closure value; `tree` is the member's file.
fn is_function_like(
    semantic_model: &SemanticModel<'_>,
    tree: Option<&LuaSyntaxTree>,
    member: &Member,
) -> bool {
    if let Some(value_syntax) = member.value_syntax
        && let Some(tree) = tree
        && let Some(node) = value_syntax.to_node_from_root(&tree.get_red_root())
        && LuaClosureExpr::cast(node).is_some()
    {
        return true;
    }
    semantic_model
        .type_of_member(&member.id)
        .and_then(|ty| expand_alias_head(semantic_model, &ty))
        .is_some_and(|ty| {
            matches!(
                ty,
                LuaType::DocFunction(_) | LuaType::Signature(_) | LuaType::Function
            )
        })
}
