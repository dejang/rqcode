use rqcode::ast::{BarrelKind, FileAst, SyntaxTree};
use rqcode::config::{Config, ModuleBoundaryConfig};
use rqcode::lang::LanguageFrontend;
use rqcode::lang::rust::{RustFrontend, rust_language_spec};
use rqcode::lang::swift::{SwiftFrontend, swift_language_spec};
use rqcode::lang::typescript::{TypeScriptFrontend, typescript_language_spec};
use std::error::Error;
use std::path::Path;

fn parse_ts(path: &str, src: &str) -> FileAst {
    let tree = SyntaxTree::parse(path, path, typescript_language_spec(), src);
    TypeScriptFrontend.extract(&tree, Path::new("."))
}

#[test]
fn typescript_frontend_classifies_directory_and_pure_reexport_barrels() {
    for (label, path, src) in [
        (
            "direct internal index",
            "src/models/index.ts",
            "export { Order } from './order';\n",
        ),
        (
            "external-only index",
            "src/ext/index.ts",
            "export * from 'external-sdk';\n",
        ),
        (
            "imported-binding index",
            "src/models/index.ts",
            "import { Order } from './order';\nexport { Order };\n",
        ),
        (
            "mixed-code index",
            "src/models/index.ts",
            "export const V = 1;\nexport { Order } from './order';\n",
        ),
    ] {
        let ast = parse_ts(path, src);
        assert_eq!(ast.barrel_kind, BarrelKind::DirectoryIndex, "{label}");
        assert!(ast.is_barrel(), "{label}");
        assert!(ast.is_directory_barrel(), "{label}");
    }

    for (label, path, src) in [
        (
            "wildcard pure re-export",
            "src/models/all.ts",
            "export * from './order';\n",
        ),
        (
            "side-effect import plus re-export",
            "src/models/setup.ts",
            "import './polyfill';\nexport * from './order';\n",
        ),
    ] {
        let ast = parse_ts(path, src);
        assert_eq!(ast.barrel_kind, BarrelKind::PureReexport, "{label}");
        assert!(ast.is_barrel(), "{label}");
        assert!(!ast.is_directory_barrel(), "{label}");
    }
}

#[test]
fn typescript_frontend_classifies_non_barrel_files_as_none() {
    for (label, path, src) in [
        (
            "non-reexport index",
            "src/app/index.ts",
            "export class App {}\n",
        ),
        (
            "local-only export clause in index",
            "src/app/index.ts",
            "function run() {}\nexport { run };\n",
        ),
        (
            "ambient .d.ts index",
            "src/types/index.d.ts",
            "export * from './order';\n",
        ),
        ("empty file", "src/empty.ts", ""),
        (
            "comment-only file",
            "src/comments.ts",
            "// just a comment\n",
        ),
        (
            "side-effect import only",
            "src/polyfill.ts",
            "import './setup';\n",
        ),
        (
            "import-only file",
            "src/unused.ts",
            "import { Order } from './order';\n",
        ),
        ("empty export clause", "src/marker.ts", "export {};\n"),
    ] {
        let ast = parse_ts(path, src);
        assert_eq!(ast.barrel_kind, BarrelKind::None, "{label}");
        assert!(!ast.is_barrel(), "{label}");
        assert!(!ast.is_directory_barrel(), "{label}");
    }
}

#[test]
fn rust_and_swift_frontends_classify_all_files_as_non_barrel() {
    for (path, src) in [
        (
            "src/models/mod.rs",
            "pub mod order;\npub use order::Order;\n",
        ),
        ("src/prelude.rs", "pub use crate::order::Order;\n"),
    ] {
        let rust_tree = SyntaxTree::parse(path, path, rust_language_spec(), src);
        let rust_ast = RustFrontend.extract(&rust_tree, Path::new("."));
        assert_eq!(rust_ast.barrel_kind, BarrelKind::None, "{path}");
        assert!(!rust_ast.is_barrel(), "{path}");
    }

    let swift_tree = SyntaxTree::parse(
        "Sources/Core/Exports.swift",
        "Sources/Core/Exports.swift",
        swift_language_spec(),
        "@_exported import Foundation\n",
    );
    let swift_ast = SwiftFrontend::new().extract(&swift_tree, Path::new("."));
    assert_eq!(swift_ast.barrel_kind, BarrelKind::None);
    assert!(!swift_ast.is_barrel());
}

#[test]
fn module_boundary_config_defaults_and_toml_enforce_no_barrel_policy() -> Result<(), Box<dyn Error>>
{
    let cfg = ModuleBoundaryConfig::default();
    assert!(cfg.barrels.flag_barrel_modules);
    assert!(!cfg.barrels.allow_library_root_barrel);
    assert_eq!(cfg.barrels.max_barrel_related, 3);

    let full = Config::from_toml(
        "[cohesion.boundaries]\nflag_barrel_modules = false\nallow_library_root_barrel = true\nmax_barrel_related = 5\n",
    )?;
    assert!(!full.cohesion.boundaries.barrels.flag_barrel_modules);
    assert!(full.cohesion.boundaries.barrels.allow_library_root_barrel);
    assert_eq!(full.cohesion.boundaries.barrels.max_barrel_related, 5);

    let partial = Config::from_toml(
        "[cohesion.boundaries]\nflag_cyclic_modules = false\nallow_library_root_barrel = true\n",
    )?;
    assert!(!partial.cohesion.boundaries.flag_cyclic_modules);
    assert!(partial.cohesion.boundaries.flag_leaky_boundaries);
    assert!(partial.cohesion.boundaries.flag_stateless_impl_methods);
    assert!(partial.cohesion.boundaries.barrels.flag_barrel_modules);
    assert!(
        partial
            .cohesion
            .boundaries
            .barrels
            .allow_library_root_barrel
    );
    assert_eq!(partial.cohesion.boundaries.barrels.max_barrel_related, 3);
    Ok(())
}
