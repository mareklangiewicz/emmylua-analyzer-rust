#[cfg(test)]
mod test {
    use crate::{DiagnosticCode, ModuleVisibility, VirtualWorkspace};

    #[test]
    fn test_module_annotation() {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();

        ws.def_files(vec![(
            "a.lua",
            r#"
                local a = {
                }
                return a
                "#,
        )]);

        ws.def(
            r#"
            ---@module "a"
            aaa = {}
            "#,
        );

        let aaa_ty = ws.expr_ty("aaa");
        assert!(aaa_ty.is_module_ref());
    }

    #[test]
    fn test_module_no_require() {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();

        // ---@meta no-require has the highest priority
        let file_id = ws.def_file(
            "a.lua",
            r#"
                ---@meta no-require

                ---@public
                A = {
                }

                return A
                "#,
        );
        let module = ws.analysis.db.module_info_of(file_id);
        assert!(module.is_some());
        assert!(module.as_ref().unwrap().visible.is_hidden());
    }

    #[test]
    fn test_module_default_visibility() {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();

        let file_id = ws.def_file(
            "a.lua",
            r#"
                A = {
                }

                return A
                "#,
        );
        let module = ws.analysis.db.module_info_of(file_id);
        assert!(module.is_some());
        assert!(module.as_ref().unwrap().visible == ModuleVisibility::Public);
    }

    #[test]
    fn test_module_internal() {
        let mut ws = VirtualWorkspace::new();
        {
            let file_id = ws.def_file(
                "a.lua",
                r#"
                ---@internal
                A = {
                }

                return A
                "#,
            );
            let module = ws.analysis.db.module_info_of(file_id);
            assert!(module.is_some());
            assert!(module.as_ref().unwrap().visible == ModuleVisibility::Internal);
        }
        {
            // Visibility must be attached to the definition statement
            let file_id = ws.def_file(
                "b.lua",
                r#"
                B = {
                }

                ---@internal
                return B
                "#,
            );
            let module = ws.analysis.db.module_info_of(file_id);
            assert!(module.is_some());
            assert!(module.as_ref().unwrap().visible == ModuleVisibility::Public);
        }

        {
            // When return returns an anonymous structure, visibility may be attached to it
            let file_id = ws.def_file(
                "c.lua",
                r#"

                ---@internal
                return {
                }
                "#,
            );
            let module = ws.analysis.db.module_info_of(file_id);
            assert!(module.is_some());
            assert!(module.as_ref().unwrap().visible == ModuleVisibility::Internal);
        }
    }

    #[test]
    fn test_module_return_from_truthy_while_block() {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();

        ws.def(
            r#"
                while {} do
                    return 1
                end
                "#,
        );

        // `def()` creates `virtual_0.lua`, so the block is requireable as `virtual_0`.
        let ty = ws.expr_ty(r#"require("virtual_0")"#);
        let integer = ws.ty("integer");
        assert_eq!(ty, integer);
    }

    // Migrated to semantic check/ (new pipeline); module export metadata block semantics pending semantic support.
    #[test]
    fn test_module_multiple_return_paths_preserve_export_metadata_block() {
        let mut ws = VirtualWorkspace::new();

        ws.def(
            r#"
                ---@class (partial) ModuleExport
                ---@field private hidden integer
                local export = {}

                if flag then
                    return export
                end

                return export
                "#,
        );

        // `AccessInvisible` only fires if the export still points at `export`.
        assert!(!ws.has_no_diagnostic(
            DiagnosticCode::AccessInvisible,
            r#"
                local export = require("virtual_0")
                export.hidden = 1
                "#,
        ));
    }
    #[test]
    fn test_require_inference_uses_source_root() {
        use emmylua_parser::{LuaAstToken, LuaLocalName};
        let mut ws = VirtualWorkspace::new_with_init_std_lib();
        let base = ws.virtual_url_generator.base.clone();
        ws.analysis.add_main_workspace(base.join("a"));
        ws.analysis.add_main_workspace(base.join("b"));
        ws.def_files(vec![
            ("a/helper.lua", "return 11"),
            ("b/helper.lua", "return 'from-b'"),
        ]);
        for (path, expected) in [("a/control.lua", "integer"), ("b/control.lua", "string")] {
            let file = ws.def_file(path, "local value = require('helper')");
            let local = ws.get_node::<LuaLocalName>(file);
            let model = ws.analysis.semantic_model(file);
            let decl = model
                .decl_by_offset(local.get_name_token().unwrap().get_position())
                .unwrap();
            let typ = model.type_of_decl(&decl).unwrap();
            assert_eq!(ws.humanize_type(typ), expected);
        }
    }
    /// `local M = defer('m', { sub = 1 })` (Neovim's `vim._defer_require`): the export's flow-free
    /// shell cannot instantiate the generic call, so `require` lost the literal's fields AND, held
    /// by a member (`vim.lsp = require('vim.lsp')`), the `function M.get()` members too.
    #[test]
    fn test_require_export_initialized_by_generic_call() {
        let mut ws = VirtualWorkspace::new_with_init_std_lib();
        ws.def_files(vec![(
            "mod.lua",
            r#"
                ---@generic T
                ---@param mod T
                ---@return T
                local function defer(root, mod) return mod end
                local M = defer('mod', { sub = 1 })
                function M.get() return 'x' end
                return M
            "#,
        )]);
        assert!(ws.has_no_diagnostic(
            DiagnosticCode::UndefinedField,
            r#"
                local m = require('mod')
                local a = m.sub
                local b = m.get()
                vv = {}
                vv.m = require('mod')
                local c = vv.m.sub
                local d = vv.m.get()
            "#,
        ));
        ws.def(
            r#"
                local m = require('mod')
                vv2 = {}
                vv2.m = require('mod')
                sub_ty = m.sub
                get_ty = vv2.m.get()
            "#,
        );
        let sub_ty = ws.expr_ty("sub_ty");
        assert_eq!(ws.humanize_type(sub_ty), "1");
        let get_ty = ws.expr_ty("get_ty");
        assert_eq!(ws.humanize_type(get_ty), "string");
    }
}
