use crate::semantic_model::type_check::TypeCheckFailReason;
use crate::{InFiled, IndexInfo, MemberList, OperatorDef};

use super::prelude::*;

// Object annotations lose their fields when lowered to `table` or `table | nil`.
pub(crate) fn needs_rich_doc_type(ty: &LuaType) -> bool {
    match ty {
        LuaType::Unknown | LuaType::Table => true,
        LuaType::Union(union) => union
            .iter()
            .all(|part| matches!(part, LuaType::Table | LuaType::Nil)),
        // `{ x: T }[]` lowers to `table[]` the same way.
        LuaType::Array(array) => needs_rich_doc_type(array.get_base()),
        _ => false,
    }
}

impl<'db> SemanticModel<'db> {
    /// Members whose owner is a `SemanticId` (cross-file).
    pub fn members_of_owner(&self, owner: &SemanticId) -> MemberList {
        if let Some(cached) = self.cache.borrow().members_of_owner.get(owner) {
            return cached.clone();
        }
        let result = self.analysis().members_of_owner(owner.clone());
        self.cache
            .borrow_mut()
            .members_of_owner
            .insert(owner.clone(), result.clone());
        result
    }
    /// Members of an owner with a specific name.
    pub fn members_of_owner_named(&self, owner: &SemanticId, name: &str) -> MemberList {
        let key = (owner.clone(), SmolStr::new(name));
        if let Some(cached) = self.cache.borrow().members_of_owner_named.get(&key) {
            return cached.clone();
        }
        let result = self
            .analysis()
            .members_of_owner_named(owner.clone(), SmolStr::new(name));
        self.cache
            .borrow_mut()
            .members_of_owner_named
            .insert(key, result.clone());
        result
    }
    /// Constructor attributes for a type definition (`---@[constructor("init")]` from the `meta("Class")` factory).
    pub fn constructor_attribute_of_type(
        &self,
        type_def: &SemanticId,
    ) -> Option<ConstructorAttribute> {
        self.analysis()
            .constructor_attribute_of_type(type_def.clone())
    }
    /// A function's return type (doc annotation takes priority; otherwise scan function body returns).
    pub fn return_type(&self, closure_syntax: LuaSyntaxId) -> Option<LuaType> {
        let shell = self
            .analysis()
            .signature_return(self.view.file_id(), closure_syntax)?;
        let generic_names = self.signature_generic_names(closure_syntax);
        let mut ty = self
            .analysis()
            .type_shell_lua_in(self.view.file_id(), &shell, &generic_names);
        // Object annotations lower to broad `table` in TypeShell. Keep their
        // fields and index signatures when checking a function's return value.
        if needs_rich_doc_type(&ty)
            && let Some(docs) = self
                .file_facts()?
                .signature_by_closure(closure_syntax)?
                .docs
                .as_ref()
            && docs.returns.len() == 1
        {
            ty = self.rich_doc_type_or(self.view.file_id(), docs.returns[0], ty);
        }
        // `self` in method annotations is the receiver instance; concretize it by the owner type before return checks.
        if let Some(owner_ty) = self.method_owner_type(closure_syntax) {
            ty = infer::vm::replace_self_type(&ty, &owner_ty);
        }
        ty = type_eval::expand_alias_generic(self, &ty);
        (!matches!(ty, LuaType::Unknown)).then_some(ty)
    }
    /// Type of the `param_index`-th function parameter (`---@param` annotation + member field signature fallback).
    pub fn param_type(&self, closure_syntax: LuaSyntaxId, param_index: usize) -> Option<LuaType> {
        let shell = self
            .analysis()
            .param_type(self.view.file_id(), closure_syntax, param_index)?;
        let generic_names = self.signature_generic_names(closure_syntax);
        let ty = self
            .analysis()
            .type_shell_lua_in(self.view.file_id(), &shell, &generic_names);
        if !matches!(ty, LuaType::Unknown) {
            return Some(ty);
        }
        // For `function a.aaa(x)` without `---@param`, fill in from the function signature of the same-named field on the owner type.
        self.expected_member_param_for_closure(closure_syntax, param_index)
    }
    /// Generic parameter names from signature docs (`---@generic T`), used as the shell projection context.
    pub(crate) fn signature_generic_names(&self, closure_syntax: LuaSyntaxId) -> Vec<SmolStr> {
        let Some(signatures) = self.signatures() else {
            return Vec::new();
        };
        let Some(signature) = signatures
            .iter()
            .find(|sig| sig.closure_syntax == closure_syntax)
        else {
            return Vec::new();
        };
        signature
            .docs
            .as_ref()
            .map(|docs| docs.generic_params.iter().map(|g| g.name.clone()).collect())
            .unwrap_or_default()
    }
    /// Type of the module's exported value.
    pub fn type_of_module_export(&self) -> Option<LuaType> {
        self.module_export_lua(self.view.file_id())
    }
    fn module_export_lua(&self, module_file: FileId) -> Option<LuaType> {
        if let Some(ty) = self.module_export_as_cast(module_file) {
            return Some(ty);
        }
        if let Some(ty) = self.module_export_from_call(module_file) {
            return Some(ty);
        }
        let shell = self.analysis().module_export_type(module_file)?;
        Some(self.analysis().type_shell_lua(module_file, &shell))
    }
    /// `local M = defer('m', { sub = 1 }); return M` (Neovim's `vim._defer_require`): the
    /// flow-free shell cannot type a call initializer that needs the VM (a generic call), so
    /// the export kept only `M`'s name, a global `Ref("M")` that resolves nothing. The export is
    /// the VM's type of the call plus the table identity of `M` itself (keyed by its value
    /// range, as `decl_by_value_range` reads it), which carries the `function M.f()` members.
    fn module_export_from_call(&self, module_file: FileId) -> Option<LuaType> {
        let ModuleExport::Decl { decl, .. } = self.analysis().module_export(module_file)? else {
            return None;
        };
        let facts = self.file_facts_of(module_file)?;
        let decl_info = facts.decl_by_id(decl)?;
        let value_syntax = decl_info.value_expr_syntax?;
        let has_type_def = decl_info
            .owner_syntax
            .and_then(|owner| facts.type_def_by_owner_syntax(owner))
            .is_some();
        if decl_info.doc_type_syntax.is_some()
            || has_type_def
            || decl_info.multi_return_index.is_some_and(|index| index > 0)
            || value_syntax.get_kind() != LuaSyntaxKind::CallExpr
            || !self
                .analysis()
                .decl_type(module_file, decl.clone())?
                .is_unknown()
        {
            return None;
        }
        let ty = self.model_for(module_file).type_of_decl(decl)?;
        if ty.is_unknown() || ty.is_any() || ty.contain_tpl() {
            return None;
        }
        let identity = LuaType::TableConst(InFiled::new(module_file, value_syntax.get_range()));
        Some(LuaType::from_vec(vec![ty, identity]))
    }
    /// `local M = f() --[[@as T]]; return M` exports `T`. The TypeShell export cannot see the
    /// cast (it only knows a `---@class` attached to `M`), so the module would export `M`'s name.
    fn module_export_as_cast(&self, module_file: FileId) -> Option<LuaType> {
        let ModuleExport::Decl { decl, .. } = self.analysis().module_export(module_file)? else {
            return None;
        };
        let value_syntax = self
            .file_facts_of(module_file)?
            .decl_by_id(decl)?
            .value_expr_syntax?;
        let ty = flow::apply_expr_casts(
            &self.model_for(module_file),
            value_syntax,
            LuaType::Unknown,
            flow::ExprCastMode::AsOnly,
        );
        (!matches!(ty, LuaType::Unknown)).then_some(ty)
    }
    /// require module name -> module export type (cross-file, projected as `LuaType`).
    pub fn require_module_type(&self, module_name: &str) -> LuaType {
        let Some(module_file) = self
            .analysis()
            .module_file_from(module_name, self.view.file_id())
        else {
            return LuaType::Unknown;
        };
        self.module_export_lua(module_file)
            .unwrap_or(LuaType::Unknown)
    }
    /// require module name -> module file id (consumed by require_module_visibility checks).
    pub fn module_file_of(&self, module_name: &str) -> Option<FileId> {
        self.analysis()
            .module_file_from(module_name, self.view.file_id())
    }
    // -- Type / member association --

    pub fn member_keys_of_owner(&self, owner: &SemanticId) -> Vec<SmolStr> {
        self.analysis().member_keys_of_owner(owner.clone())
    }
    pub fn resolve_owner(&self, owner: &SemanticId) -> Option<SemanticId> {
        self.analysis().resolve_owner(owner.clone())
    }
    pub(crate) fn resolve_owner_set(&self, owner: SemanticId) -> Vec<SemanticId> {
        if let Some(cached) = self.cache.borrow().resolve_owner_set.get(&owner) {
            return cached.clone();
        }
        let result = self.analysis().resolve_owner_set(owner.clone());
        self.cache
            .borrow_mut()
            .resolve_owner_set
            .insert(owner, result.clone());
        result
    }
    pub fn module_export(&self) -> Option<&'db ModuleExport> {
        self.view.module_export()
    }
    // -- Control flow --

    pub fn flow_tree(&self) -> Option<&'db FlowTree> {
        self.analysis().flow_tree(self.view.file_id())
    }
    // -- Convenience predicates --

    /// Type of an expression (VM inference; `Unknown` = no type / inference failed).
    pub fn type_of_expr(&self, expr_syntax: LuaSyntaxId) -> LuaType {
        self.type_of_expr_impl(expr_syntax)
    }
    pub(crate) fn type_of_expr_impl(&self, expr_syntax: LuaSyntaxId) -> LuaType {
        self.type_of_expr_cached(expr_syntax, || infer::infer_expr(self, expr_syntax))
    }
    /// Reuse a resolved node while keeping the ordinary query's cycle guard.
    pub(crate) fn type_of_expr_node(&self, expr: &LuaExpr) -> LuaType {
        self.type_of_expr_cached(expr.get_syntax_id(), || {
            infer::vm::infer_expr_node(self, expr.clone())
        })
    }
    fn type_of_expr_cached(
        &self,
        expr_syntax: LuaSyntaxId,
        infer: impl FnOnce() -> LuaType,
    ) -> LuaType {
        let key = (self.view.file_id(), expr_syntax);
        {
            let cache = self.cache.borrow();
            if let Some(entry) = cache.expr_type.get(&key) {
                return entry
                    .get_inference_seed(&cache)
                    .cloned()
                    .unwrap_or(LuaType::Unknown);
            }
        }
        let _query = cache::QueryCacheScope::new(&self.cache);
        self.cache
            .borrow_mut()
            .expr_type
            .insert(key, cache::CacheEntry::InProgress);
        let revision = self.cache.borrow().revision();
        let ty = infer();
        let mut cache = self.cache.borrow_mut();
        let entry = cache::CacheEntry::finish(ty.clone(), cache.is_complete(revision));
        cache.expr_type.insert(key, entry);
        ty
    }
    /// Hover/display layer only: expands generic alias instances (`Pick<...>` -> `T[K]` structure).
    pub fn expand_alias_for_hover(&self, ty: &LuaType) -> LuaType {
        type_eval::expand_alias_generic(self, ty)
    }
    /// Hover/display layer only: evaluates conditional types (`A extends B ? X : Y`).
    pub fn eval_conditionals_for_hover(&self, ty: &LuaType) -> LuaType {
        type_eval::eval_conditionals(self, ty)
    }

    /// Whether a value of `source` can be used where `target` is expected.
    pub fn type_check(&self, source: &LuaType, target: &LuaType) -> bool {
        self.type_check_cached(source, target, false)
            .unwrap_or(false)
    }
    /// Precise subtype proof, including object fields and generic arguments.
    pub fn type_check_subtype(&self, source: &LuaType, target: &LuaType) -> bool {
        self.type_check_cached(source, target, true)
            .unwrap_or(false)
    }
    pub(super) fn type_check_cached(
        &self,
        source: &LuaType,
        target: &LuaType,
        subtype: bool,
    ) -> Option<bool> {
        let key = (self.file_id(), source.clone(), target.clone(), subtype);
        {
            let cache = self.cache.borrow();
            if let Some(entry) = cache.type_check.get(&key) {
                return entry.get_complete(&cache).copied();
            }
        }
        let _query = cache::QueryCacheScope::new(&self.cache);
        self.cache
            .borrow_mut()
            .type_check
            .insert(key.clone(), cache::CacheEntry::InProgress);
        let result = if subtype {
            type_check::check_type_subtype(self, source, target)
        } else {
            type_check::check_type(self, source, target)
        };
        let result = match result {
            Ok(()) => Some(true),
            Err(err) if err.is_type_not_match() => Some(false),
            Err(_) => None,
        };
        let mut cache = self.cache.borrow_mut();
        if result.is_none() {
            cache.mark_incomplete();
        }
        // Comparison outcomes carry their own completeness. An unfinished
        // alternative does not invalidate a proof from another alternative.
        cache.type_check.insert(
            key,
            cache::CacheEntry::finish(result.unwrap_or(false), result.is_some()),
        );
        result
    }

    /// Index signatures of `typ` whose key type accepts `key_type`.
    pub fn applicable_index_infos(
        &self,
        typ: &LuaType,
        key_type: &LuaType,
    ) -> Result<Vec<IndexInfo>, TypeCheckFailReason> {
        let _query = cache::QueryCacheScope::new(&self.cache);
        let revision = self.cache.borrow().revision();
        let infos = self.index_infos(typ);
        {
            let cache = self.cache.borrow();
            if cache.evaluation_exhausted {
                return Err(TypeCheckFailReason::TooComplex);
            }
            if !cache.is_complete(revision) {
                return Err(TypeCheckFailReason::Unresolved);
            }
        }
        // 收集必须完整, 每个比较是否已有确定结论则由类型检查器判断.
        infos
            .into_iter()
            .filter_map(
                |info| match self.type_check_cached(key_type, &info.key_type, false) {
                    Some(true) => Some(Ok(info)),
                    Some(false) => None,
                    None => Some(Err(if self.cache.borrow().evaluation_exhausted {
                        TypeCheckFailReason::TooComplex
                    } else {
                        TypeCheckFailReason::Unresolved
                    })),
                },
            )
            .collect()
    }

    /// Multi-value expression list types (mirrors old `infer_expr_list_types`; trailing multi-returns expand according to var_count).
    pub fn infer_expr_list_types(
        &self,
        exprs: &[LuaExpr],
        var_count: Option<usize>,
    ) -> Vec<(LuaType, rowan::TextRange)> {
        infer::infer_expr_list_types(self, exprs, var_count)
    }
    /// Unified signature return type projection: TypeShell takes priority, with rich projection as fallback for Unknown.
    pub(crate) fn signature_return_type(
        &self,
        file_id: FileId,
        closure_syntax: LuaSyntaxId,
        signature: &Signature,
        generic_names: &[SmolStr],
    ) -> LuaType {
        let return_shells = self
            .analysis()
            .signature_returns(file_id, closure_syntax)
            .unwrap_or_default();
        let mut ret = if return_shells.len() > 1 {
            LuaType::Variadic(Rc::new(VariadicType::Multi(
                return_shells
                    .iter()
                    .map(|shell| {
                        self.analysis()
                            .type_shell_lua_in(file_id, shell, generic_names)
                    })
                    .collect(),
            )))
        } else if let Some(shell) = return_shells.first() {
            self.analysis()
                .type_shell_lua_in(file_id, shell, generic_names)
        } else {
            LuaType::Unknown
        };
        // Conditional types and `L - R` cannot be expressed by TypeShell; whenever the return type contains one,
        // prefer rich projection to avoid `Test<T extends ...>` being downgraded to `Test<unknown>`, or
        // `T -?` to `T`, by shell.
        if let Some(docs) = signature.docs.as_ref()
            && docs.returns.len() == 1
        {
            let rich = self.doc_type_lua_rich_in(file_id, docs.returns[0]);
            if !matches!(rich, LuaType::Unknown)
                && rich.any_type(|ty| match ty {
                    LuaType::Conditional(_) => true,
                    LuaType::Call(call) => call.get_call_kind() == LuaAliasCallKind::Sub,
                    _ => false,
                })
            {
                ret = rich;
            }
        }
        // Rich projection fallback: structures like `T[K]` / mapped may be Unknown at the TypeShell layer.
        if matches!(ret, LuaType::Unknown)
            && let Some(docs) = signature.docs.as_ref()
            && docs.returns.len() == 1
        {
            let rich = self.doc_type_lua_rich_in(file_id, docs.returns[0]);
            if !matches!(rich, LuaType::Unknown) {
                ret = rich;
            }
        }
        // `self` in method annotations is the receiver instance; concretize the signature return type by the method owner.
        if signature.is_method
            && let Some(owner_ty) = self.method_owner_type(closure_syntax)
        {
            ret = infer::vm::replace_self_type(&ret, &owner_ty);
        }
        ret
    }
    /// Use the same call-operator signature for inference and diagnostics.
    pub(crate) fn call_operator_type(&self, file_id: FileId, op: &OperatorDef) -> LuaFunctionType {
        let params = op
            .params
            .iter()
            .map(|syntax| {
                (
                    String::new(),
                    Some(self.doc_type_lua_rich_in(file_id, *syntax)),
                )
            })
            .collect::<Vec<_>>();
        let is_variadic = params
            .last()
            .is_some_and(|(_, ty)| matches!(ty, Some(LuaType::Variadic(_))));
        LuaFunctionType::new(
            AsyncState::None,
            false,
            is_variadic,
            params,
            self.doc_type_lua_rich_in(file_id, op.returns),
            None,
        )
    }

    /// Function signature (by closure syntax location) -> structured function type (`DocFunction`).
    pub fn type_of_signature(&self, closure_syntax: LuaSyntaxId) -> Option<LuaFunctionType> {
        let signature = self.file_facts()?.signature_by_closure(closure_syntax)?;
        // `---@generic T: Base` -> unified projection context + GenericTpl list (including constraints/defaults).
        let generic_params: Vec<GenericTpl> = signature
            .docs
            .as_ref()
            .map(|docs| self.generic_tpls_with_metadata(self.view.file_id(), &docs.generic_params))
            .unwrap_or_default();
        let nullable_params: Vec<SmolStr> = signature
            .docs
            .as_ref()
            .map(|docs| docs.nullable_params.clone())
            .unwrap_or_default();
        let mut params = Vec::new();
        for (index, name) in signature.param_names.iter().enumerate() {
            let mut ty = self
                .param_type(closure_syntax, index)
                .unwrap_or(LuaType::Unknown);
            if needs_rich_doc_type(&ty)
                && let Some(docs) = signature.docs.as_ref()
                && let Some((_, syntax)) = docs
                    .param_types
                    .iter()
                    .find(|(param_name, _)| param_name == name)
            {
                ty = self.rich_doc_type_or(self.view.file_id(), *syntax, ty);
            }
            // `function a.aaa(x)`: if this closure implements a member and the owner type has
            // a same-named field with a function type, use the field signature to fill in missing `---@param` types.
            if matches!(ty, LuaType::Unknown)
                && let Some(expected_param_ty) =
                    self.expected_member_param_for_closure(closure_syntax, index)
            {
                ty = expected_param_ty;
            }
            // Rich object projection keeps names; restore the function's generic scope.
            ty = infer::vm::bind_signature_generics(&ty, &generic_params);
            ty = type_eval::expand_alias_generic(self, &ty);
            // Do not evaluate conditional types here: before function generics are substituted, `T extends ...` would incorrectly take the false branch.
            // Call sites/diagnostics should call `eval_conditionals` after bindings are substituted.
            if nullable_params.iter().any(|n| n == name) && !ty.is_nullable() {
                ty = LuaType::Union(Rc::new(LuaUnionType::from_vec(vec![ty, LuaType::Nil])));
            }
            params.push((name.to_string(), Some(ty)));
        }
        // `function M:event_on(...)` only has unpacked `...`, but `---@overload` / class fields give concrete slots.
        // Here overload function types project `...` into Variadic(Multi(...)) so `local a,b,c = ...` can take slots.
        if signature.is_variadic {
            let overload_funcs: Vec<LuaFunctionType> = if let Some(docs) = signature.docs.as_ref()
                && !docs.overloads.is_empty()
            {
                docs.overloads
                    .iter()
                    .filter_map(|syntax| {
                        match self.doc_type_lua_rich_in(self.view.file_id(), *syntax) {
                            LuaType::DocFunction(fun) => Some(fun.as_ref().clone()),
                            _ => None,
                        }
                    })
                    .collect()
            } else {
                self.expected_member_signatures_for_closure(closure_syntax)
                    .unwrap_or_default()
            };
            if !overload_funcs.is_empty() {
                let has_self = overload_funcs.iter().any(|fun| {
                    fun.get_params().first().is_some_and(|(name, ty)| {
                        name == "self" || matches!(ty, Some(LuaType::SelfInfer))
                    })
                });
                let param_start = usize::from(has_self);
                let max_len = overload_funcs
                    .iter()
                    .map(|fun| fun.get_params().len().saturating_sub(param_start))
                    .max()
                    .unwrap_or(0);
                let mut slots = Vec::new();
                for slot in 0..max_len {
                    let mut parts = Vec::new();
                    for fun in &overload_funcs {
                        if let Some((_, Some(ty))) = fun.get_params().get(param_start + slot) {
                            let ty = type_eval::expand_alias_generic(self, ty);
                            let ty = if ty.any_type(|t| matches!(t, LuaType::Any)) {
                                LuaType::Any
                            } else {
                                ty
                            };
                            if !parts.contains(&ty) {
                                parts.push(ty);
                            }
                        }
                    }
                    // Unions containing any, such as `any[] | any`, should collapse to any directly.
                    if parts.iter().any(|ty| matches!(ty, LuaType::Any)) {
                        parts = vec![LuaType::Any];
                    }
                    slots.push(LuaType::from_vec(parts));
                }
                if let Some((_, ty)) = params.iter_mut().find(|(name, _)| name == "...") {
                    *ty = Some(LuaType::Variadic(Rc::new(VariadicType::Multi(slots))));
                }
            }
        }
        let generic_names: Vec<SmolStr> = generic_params
            .iter()
            .map(|param| SmolStr::new(param.get_name()))
            .collect();
        let mut ret = self.signature_return_type(
            self.view.file_id(),
            closure_syntax,
            signature,
            &generic_names,
        );
        // Bind the generic names first: an unbound `Ref("T")` is evaluated as a type, so `T -?` would become `T`.
        ret = infer::vm::bind_signature_generics(&ret, &generic_params);
        ret = type_eval::expand_alias_generic(self, &ret);
        // Without an explicit `---@return`, keep the return type declared by the member on the owner type.
        // This keeps member docs like class field `---@field f fun(): never` effective on the implementation function's return.
        if matches!(ret, LuaType::Unknown)
            && signature.docs.is_none()
            && let Some(expected_fun) = self.expected_member_signature_for_closure(closure_syntax)
        {
            ret = expected_fun.get_ret().clone();
        }
        if matches!(ret, LuaType::Unknown) && signature.docs.is_none() {
            let inferred = infer::closure_return_lua(self, closure_syntax);
            if !matches!(inferred, LuaType::Unknown | LuaType::Any) {
                ret = inferred;
            }
        }
        let is_variadic = signature.is_variadic;
        // `---@async` -> AsyncState::Async (consumed by await_in_sync checks).
        let async_state = signature
            .docs
            .as_ref()
            .map(|docs| {
                if docs.is_async {
                    AsyncState::Async
                } else {
                    AsyncState::None
                }
            })
            .unwrap_or(AsyncState::None);
        Some(LuaFunctionType::new(
            async_state,
            signature.is_method,
            is_variadic,
            params,
            ret,
            Some(generic_params),
        ))
    }
    /// Instance type of the owner that a method closure belongs to (the `self` type).
    /// Consistent with `method_self_return_shell`: if the method owner is a runtime declaration
    /// and that declaration has an attached `---@class/@enum` type definition (same owner_syntax), use the type definition.
    /// Finds the owning method closure from the implicit `self` parameter declaration.
    pub(crate) fn method_closure_for_self_decl(&self, decl: &Decl) -> Option<LuaSyntaxId> {
        let closure_syntax = decl.owner_syntax?;
        let facts = self.file_facts()?;
        let signature = facts.signature_by_closure(closure_syntax)?;
        if signature.is_method {
            Some(closure_syntax)
        } else {
            None
        }
    }
    pub(crate) fn method_owner_type(&self, closure_syntax: LuaSyntaxId) -> Option<LuaType> {
        // `function M.E:m()`: a nested runtime path is owned by its root local (`M`), but the
        // method belongs to the value at its prefix (`M.E`).
        if let Some(prefix) = self.nested_method_prefix(closure_syntax) {
            let ty = self.type_of_expr(prefix.get_syntax_id());
            if !ty.is_unknown() {
                return Some(ty);
            }
        }
        let facts = self.file_facts()?;
        let member = facts.member_by_value_syntax(closure_syntax)?;
        let owner = member.owner.clone();
        let (owner_ty, owner_value) = match &owner {
            SemanticId::Decl(decl) => (
                self.type_of_decl(&SemanticId::Decl(decl.clone()))?,
                owner.clone(),
            ),
            SemanticId::TypeDef(def) => {
                let id = match &def.scope {
                    TypeScope::Global => LuaTypeDeclId::global(&def.full_name),
                    TypeScope::Internal(workspace_id) => {
                        LuaTypeDeclId::internal(*workspace_id, &def.full_name)
                    }
                    TypeScope::File(file_id) => LuaTypeDeclId::file(*file_id, &def.full_name),
                };
                let def = self.type_def_of(&id)?;
                (self.type_def_ref(&def), owner.clone())
            }
            SemanticId::Name(name) => {
                let resolved = self.resolve_owner(&SemanticId::Name(name.clone()))?;
                let ty = match &resolved {
                    SemanticId::Decl(decl) => self.type_of_decl(&SemanticId::Decl(decl.clone()))?,
                    SemanticId::TypeDef(def) => {
                        let id = match &def.scope {
                            TypeScope::Global => LuaTypeDeclId::global(&def.full_name),
                            TypeScope::Internal(workspace_id) => {
                                LuaTypeDeclId::internal(*workspace_id, &def.full_name)
                            }
                            TypeScope::File(file_id) => {
                                LuaTypeDeclId::file(*file_id, &def.full_name)
                            }
                        };
                        let def = self.type_def_of(&id)?;
                        self.type_def_ref(&def)
                    }
                    _ => return None,
                };
                (ty, resolved)
            }
            _ => return None,
        };
        let owner_ty = if let SemanticId::Decl(decl_id) = &owner_value {
            if let Some(facts) = self.file_facts_of(decl_id.file_id)
                && let Some(decl) = facts.decl_by_id(&SemanticId::Decl(decl_id.clone()))
                && let Some(def) = facts
                    .type_defs
                    .iter()
                    .find(|def| def.owner_syntax.is_some() && def.owner_syntax == decl.owner_syntax)
            {
                self.type_def_ref(def)
            } else {
                owner_ty
            }
        } else {
            owner_ty
        };
        Some(owner_ty)
    }
    /// The prefix `a.b` of a method closure named by a nested path (`function a.b:m()`).
    fn nested_method_prefix(&self, closure_syntax: LuaSyntaxId) -> Option<LuaIndexExpr> {
        let root = self.syntax_tree()?.get_red_root();
        let func_stat =
            emmylua_parser::LuaFuncStat::cast(closure_syntax.to_node_from_root(&root)?.parent()?)?;
        let Some(emmylua_parser::LuaVarExpr::IndexExpr(name)) = func_stat.get_func_name() else {
            return None;
        };
        match name.get_prefix_expr()? {
            LuaExpr::IndexExpr(prefix) => Some(prefix),
            _ => None,
        }
    }
    /// If the closure is a member implementation like `function a.aaa(...)`, return the function signature of the same-named field on the owner type.
    /// Used to fill in parameter types from field declarations when `---@param` is absent.
    pub(crate) fn expected_member_param_for_closure(
        &self,
        closure_syntax: LuaSyntaxId,
        param_index: usize,
    ) -> Option<LuaType> {
        let is_method = self
            .file_facts()
            .and_then(|facts| facts.signature_by_closure(closure_syntax))
            .is_some_and(|sig| sig.is_method);
        let mut types = Vec::new();
        for expected_fun in self.expected_member_signatures_for_closure(closure_syntax)? {
            let has_self_in_expected = expected_fun
                .get_params()
                .first()
                .is_some_and(|(name, ty)| name == "self" || matches!(ty, Some(LuaType::SelfInfer)));
            // `:` methods don't write the implicit self in source, while `---@field` function types usually list self as the first parameter.
            // When inferring implementation function parameters, skip that self parameter.
            let expected_index = if is_method && has_self_in_expected {
                param_index + 1
            } else {
                param_index
            };
            if let Some(ty) = expected_fun
                .get_params()
                .get(expected_index)
                .and_then(|(_, ty)| ty.clone())
                && !types.contains(&ty)
            {
                types.push(ty);
            }
        }
        if types.is_empty() {
            None
        } else {
            Some(LuaType::from_vec(types))
        }
    }
    /// If the closure is a member implementation like `function a.aaa(...)`, return the function signature of the same-named field on the owner type.
    /// Used to fill in parameter types from field declarations when `---@param` is absent.
    pub(crate) fn expected_member_signature_for_closure(
        &self,
        closure_syntax: LuaSyntaxId,
    ) -> Option<LuaFunctionType> {
        self.expected_member_signatures_for_closure(closure_syntax)?
            .into_iter()
            .next()
    }
    /// Returns all usable function signatures for this member implementation:
    /// - `---@overload` as the overload set;
    /// - repeated `---@field` (non-overload) follows old semantics where the last one overrides previous ones;
    /// - a normal single field uses that field's signature directly.
    pub(crate) fn expected_member_signatures_for_closure(
        &self,
        closure_syntax: LuaSyntaxId,
    ) -> Option<Vec<LuaFunctionType>> {
        let facts = self.file_facts()?;
        let member = facts
            .members
            .iter()
            .find(|member| member.value_syntax == Some(closure_syntax))?;
        let member_file = self.view.file_id();
        // 0. Inline `---@type` on a table field (`{ ---@type test A = function(a, b) ... }`) directly gives the function type.
        if let Some(doc_syntax) = member.doc_type_syntax {
            let mut doc_ty = type_eval::expand_alias_generic(
                self,
                &self.doc_type_lua_rich_in(member_file, doc_syntax),
            );
            // When an inline field `---@type test` points to a plain alias, `expand_alias_generic` only expands
            // the `Alias<...>` form; bare `Ref(test)` needs to continue along the alias chain to the function type.
            let mut alias_visited = Vec::new();
            while let LuaType::Ref(id) | LuaType::Def(id) = &doc_ty {
                let alias_id = id.clone();
                if alias_visited.contains(&alias_id) {
                    break;
                }
                let Some(def) = self.type_def_of(&alias_id) else {
                    break;
                };
                if def.kind != TypeDefKind::Alias {
                    break;
                }
                let Some(target) = self.alias_target(&def) else {
                    break;
                };
                alias_visited.push(alias_id);
                doc_ty = type_eval::expand_alias_generic(self, &target);
            }
            let mut docs_out = Vec::new();
            match &doc_ty {
                LuaType::DocFunction(fun) => docs_out.push(fun.as_ref().clone()),
                LuaType::Union(union) => {
                    for ty in union.into_vec() {
                        if let LuaType::DocFunction(fun) = ty {
                            docs_out.push(fun.as_ref().clone());
                        }
                    }
                }
                _ => {}
            }
            if !docs_out.is_empty() {
                return Some(docs_out);
            }
        }
        // 1. `---@overload` on the function's own doc: each overload is an independent candidate.
        if let Some(signature) = facts.signature_by_closure(closure_syntax)
            && let Some(docs) = signature.docs.as_ref()
            && !docs.overloads.is_empty()
        {
            let mut out = Vec::new();
            for syntax in &docs.overloads {
                if let LuaType::DocFunction(fun) = self.doc_type_lua_rich_in(member_file, *syntax) {
                    out.push(fun.as_ref().clone());
                }
            }
            if !out.is_empty() {
                return Some(out);
            }
        }

        let owner = member.owner.clone();
        let key = member.key.clone();

        let owner_ty = match &owner {
            SemanticId::Decl(decl) => {
                let mut owner_ty = self.type_of_decl(&SemanticId::Decl(decl.clone()))?;
                // `---@class ClosureTest` + `local Test`: differently named locals should also be associated with the class definition,
                // otherwise `function Test:e` cannot fill parameter types from class fields.
                if !matches!(
                    owner_ty,
                    LuaType::Ref(_) | LuaType::Def(_) | LuaType::Generic(_)
                ) && let Some(facts) = self.file_facts_of(decl.file_id)
                    && let Some(decl_info) = facts.decl_by_id(&SemanticId::Decl(decl.clone()))
                    && let Some(def) = decl_info
                        .owner_syntax
                        .and_then(|syntax| facts.type_def_by_owner_syntax(syntax))
                {
                    owner_ty = self.type_def_ref(def);
                }
                owner_ty
            }
            SemanticId::TypeDef(def) => {
                let id = match &def.scope {
                    TypeScope::Global => LuaTypeDeclId::global(&def.full_name),
                    TypeScope::Internal(workspace_id) => {
                        LuaTypeDeclId::internal(*workspace_id, &def.full_name)
                    }
                    TypeScope::File(file_id) => LuaTypeDeclId::file(*file_id, &def.full_name),
                };
                let def = self.type_def_of(&id)?;
                self.type_def_ref(&def)
            }
            SemanticId::Name(name) => {
                let owner_id = SemanticId::Name(name.clone());
                match self.resolve_owner(&owner_id) {
                    Some(SemanticId::Decl(decl)) => self.type_of_decl(&SemanticId::Decl(decl))?,
                    Some(SemanticId::TypeDef(def)) => {
                        let id = match &def.scope {
                            TypeScope::Global => LuaTypeDeclId::global(&def.full_name),
                            TypeScope::Internal(workspace_id) => {
                                LuaTypeDeclId::internal(*workspace_id, &def.full_name)
                            }
                            TypeScope::File(file_id) => {
                                LuaTypeDeclId::file(*file_id, &def.full_name)
                            }
                        };
                        let def = self.type_def_of(&id)?;
                        self.type_def_ref(&def)
                    }
                    _ => return None,
                }
            }
            SemanticId::Member(table_key) => {
                // Table-literal field (`---@type D31; local f = { func = function(...) end }`):
                // the member owner is a synthetic table identity; go back to the declaration that initializes the table and use its declared type as the field type source.
                let decl = facts.decl_by_value_range(table_key.key_range)?;
                self.type_of_decl(&decl.id)?
            }
            _ => return None,
        };

        let infos = member::member_infos_with_key_all(self, &owner_ty, &key);
        let mut out = Vec::new();
        for info in &infos {
            let typ = match &info.typ {
                LuaType::Ref(id) | LuaType::Def(id)
                    if self
                        .type_def_of(id)
                        .is_some_and(|def| def.kind == TypeDefKind::Alias) =>
                {
                    let def = self.type_def_of(id)?;
                    self.alias_target(&def)
                        .map(|target| type_eval::expand_alias_generic(self, &target))
                        .unwrap_or_else(|| info.typ.clone())
                }
                _ => type_eval::expand_alias_generic(self, &info.typ),
            };
            match &typ {
                LuaType::DocFunction(fun) => out.push(fun.as_ref().clone()),
                LuaType::Union(union) => {
                    for ty in union.into_vec() {
                        if let LuaType::DocFunction(fun) = ty {
                            out.push(fun.as_ref().clone());
                        }
                    }
                }
                _ => {}
            }
        }
        // Repeated `@field` non-overload: the last one overrides previous ones (expected by the third test case).
        if out.len() > 1 {
            out = vec![out.pop()?];
        }
        Some(out)
    }
    /// `---@param` / `---@return` annotations on table-literal fields override the closure's bare signature.
    /// semantic currently does not merge these field-level docs into `Signature.docs`, but `setmetatable`'s
    /// `__call` / `__index` metamethod fields depend on them to preserve the function signature.
    pub(crate) fn table_field_signature_override(
        &self,
        file_id: FileId,
        closure_syntax: LuaSyntaxId,
        mut params: Vec<(String, Option<LuaType>)>,
        mut ret: LuaType,
    ) -> Option<(Vec<(String, Option<LuaType>)>, LuaType)> {
        let facts = self.file_facts_of(file_id)?;
        facts
            .members
            .iter()
            .find(|member| member.value_syntax == Some(closure_syntax))?;
        let tree = self.syntax_tree_of(file_id)?;
        let node = closure_syntax.to_node_from_root(&tree.get_red_root())?;
        let field = node.ancestors().find_map(LuaTableField::cast)?;
        let mut has_param = false;
        let mut has_return = false;
        let mut returns = Vec::new();
        for comment in field.get_comments() {
            for tag in comment.get_doc_tags() {
                match tag {
                    LuaDocTag::Param(param) => {
                        if let Some(name_token) = param.get_name_token()
                            && let Some(doc_ty) = param.get_type()
                        {
                            let mut ty = self.doc_type_lua_rich_in(file_id, doc_ty.get_syntax_id());
                            if param.is_nullable() {
                                ty = LuaType::from_vec(vec![ty, LuaType::Nil]);
                            }
                            let name = name_token.get_name_text().to_string();
                            if let Some(slot) = params.iter_mut().find(|(n, _)| n == &name) {
                                slot.1 = Some(ty);
                                has_param = true;
                            }
                        }
                    }
                    LuaDocTag::Return(return_tag) => {
                        has_return = true;
                        for doc_ty in return_tag.get_types() {
                            let ty = self.doc_type_lua_rich_in(file_id, doc_ty.get_syntax_id());
                            if !matches!(ty, LuaType::Unknown) {
                                returns.push(ty);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        if has_return && !returns.is_empty() {
            ret = if returns.len() == 1 {
                returns.pop()?
            } else {
                LuaType::Variadic(Rc::new(VariadicType::Multi(returns)))
            };
        }
        if has_param || has_return {
            Some((params, ret))
        } else {
            None
        }
    }
    /// Signature of the function an alias value names (`M.run = verbs.run`, `local f = M.run`,
    /// `local f = require 'm'`): follow member reads, names and `require` to the closure at the end of
    /// the chain. A closure-valued member reads as
    /// a broad `Function` and only a call recovers its signature, through the member's identity; storing
    /// the read elsewhere drops that identity, so the stored member re-derives it here. A closure literal
    /// at `value_syntax` itself is not an alias (`None`): its member keeps whatever projection it has.
    pub fn alias_signature_in_file(
        &self,
        file_id: FileId,
        value_syntax: LuaSyntaxId,
    ) -> Option<LuaFunctionType> {
        let (mut file_id, mut syntax) = (file_id, value_syntax);
        // Bounded: an alias cycle (`a.f = b.f; b.f = a.f`) names no closure.
        for hop in 0..8 {
            if hop > 0
                && let Some(fun) = self.type_of_signature_in_file(file_id, syntax)
            {
                return Some(fun);
            }
            let model = self.model_for(file_id);
            let root = model.syntax_tree_of(file_id)?.get_red_root();
            (file_id, syntax) = match LuaExpr::cast(syntax.to_node_from_root(&root)?)? {
                LuaExpr::IndexExpr(index_expr) => {
                    let resolved = model.resolve_member(&index_expr)?;
                    let (member_file, member_id) = (resolved.file_id?, resolved.member_id?);
                    let member = model.file_facts_of(member_file)?.member_by_id(&member_id)?;
                    (member_file, member.value_syntax?)
                }
                LuaExpr::NameExpr(name_expr) => {
                    let decl_id = model.resolve_name(name_expr.get_position())?;
                    model.decl_value_syntax(&decl_id)?
                }
                // `require 'm'` names what module `m` returns.
                expr @ LuaExpr::CallExpr(_) => {
                    let module_file = model.require_module_file_of_expr(&expr)?;
                    match &model.file_facts_of(module_file)?.module_export {
                        ModuleExport::Expr { value_syntax } => (module_file, *value_syntax),
                        ModuleExport::Decl { decl, .. } => model.decl_value_syntax(decl)?,
                        _ => return None,
                    }
                }
                _ => return None,
            };
        }
        None
    }

    /// The file and initializer of a declaration.
    fn decl_value_syntax(&self, decl_id: &SemanticId) -> Option<(FileId, LuaSyntaxId)> {
        let SemanticId::Decl(decl_key) = decl_id else {
            return None;
        };
        let decl = self.file_facts_of(decl_key.file_id)?.decl_by_id(decl_id)?;
        Some((decl_key.file_id, decl.value_expr_syntax?))
    }

    /// Signature structure of a closure in any file (used for cross-file member/global function signature resolution).
    pub fn type_of_signature_in_file(
        &self,
        file_id: FileId,
        closure_syntax: LuaSyntaxId,
    ) -> Option<LuaFunctionType> {
        let facts = self.file_facts_of(file_id)?;
        let signature = facts.signature_by_closure(closure_syntax)?;
        let generic_params: Vec<GenericTpl> = signature
            .docs
            .as_ref()
            .map(|docs| self.generic_tpls_with_metadata(file_id, &docs.generic_params))
            .unwrap_or_default();
        let generic_names: Vec<SmolStr> = generic_params
            .iter()
            .map(|param| SmolStr::new(param.get_name()))
            .collect();
        let nullable_params: Vec<SmolStr> = signature
            .docs
            .as_ref()
            .map(|docs| docs.nullable_params.clone())
            .unwrap_or_default();
        let mut params = Vec::new();
        for (index, name) in signature.param_names.iter().enumerate() {
            let mut ty = self
                .analysis()
                .param_type(file_id, closure_syntax, index)
                .map(|shell| {
                    self.analysis()
                        .type_shell_lua_in(file_id, &shell, &generic_names)
                })
                .filter(|ty| !matches!(ty, LuaType::Unknown))
                .or_else(|| {
                    signature.docs.as_ref().and_then(|docs| {
                        docs.param_types
                            .iter()
                            .find(|(param_name, _)| param_name == name)
                            .map(|(_, syntax)| self.doc_type_lua_rich_in(file_id, *syntax))
                    })
                })
                .unwrap_or(LuaType::Any);
            // Rich object projection keeps names; restore the function's generic scope.
            ty = infer::vm::bind_signature_generics(&ty, &generic_params);
            ty = type_eval::expand_alias_generic(self, &ty);
            // See `type_of_signature`: do not evaluate conditional types before function generics are substituted.
            if nullable_params.iter().any(|n| n == name) && !ty.is_nullable() {
                ty = LuaType::Union(Rc::new(LuaUnionType::from_vec(vec![ty, LuaType::Nil])));
            }
            params.push((name.to_string(), Some(ty)));
        }
        let mut ret =
            self.signature_return_type(file_id, closure_syntax, signature, &generic_names);
        // Bind the generic names first: an unbound `Ref("T")` is evaluated as a type, so `T -?` would become `T`.
        ret = infer::vm::bind_signature_generics(&ret, &generic_params);
        ret = type_eval::expand_alias_generic(self, &ret);
        if matches!(ret, LuaType::Unknown)
            && signature.docs.is_none()
            && file_id == self.view.file_id()
        {
            let inferred = infer::closure_return_lua(self, closure_syntax);
            if !matches!(inferred, LuaType::Unknown | LuaType::Any) {
                ret = inferred;
            }
        }
        if signature.docs.is_none()
            && let Some((new_params, new_ret)) = self.table_field_signature_override(
                file_id,
                closure_syntax,
                params.clone(),
                ret.clone(),
            )
        {
            params = new_params;
            ret = new_ret;
        }
        let is_variadic = signature.is_variadic;
        let async_state = signature
            .docs
            .as_ref()
            .map(|docs| {
                if docs.is_async {
                    AsyncState::Async
                } else {
                    AsyncState::None
                }
            })
            .unwrap_or(AsyncState::None);
        Some(LuaFunctionType::new(
            async_state,
            signature.is_method,
            is_variadic,
            params,
            ret,
            Some(generic_params),
        ))
    }
    /// Signature structure of a declaration (cross-file: the `Decl` key carries file_id).
    /// One unified entry: declaration identity -> locate `(file, closure)` -> `type_of_signature_in_file`.
    pub fn type_of_decl_signature(&self, decl: &SemanticId) -> Option<LuaFunctionType> {
        let SemanticId::Decl(decl_key) = decl else {
            return None;
        };
        let facts = self.file_facts_of(decl_key.file_id)?;
        let decl = facts.decl_by_id(decl)?;
        let closure_syntax = decl.value_expr_syntax?;
        self.type_of_signature_in_file(decl_key.file_id, closure_syntax)
    }
    /// Structured function type for a `LuaType::Signature` payload.
    ///
    /// This is only a bridge for consuming the legacy value variant; the main
    /// signature API is `type_of_signature(closure_syntax)`. Lookup is O(1)
    /// through `FileFacts::signature_by_position`.
    pub(crate) fn function_type_by_signature_id(
        &self,
        signature_id: &LuaSignatureId,
    ) -> Option<LuaFunctionType> {
        let file_id = signature_id.get_file_id();
        let facts = self.file_facts_of(file_id)?;
        let signature = facts.signature_by_position(signature_id.get_position())?;
        self.type_of_signature_in_file(file_id, signature.closure_syntax)
    }
}
