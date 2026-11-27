use crate::ast::{FileRole, ModulePath};
use std::path::{Component, Path};

/// Injectable layout, test-marker, React/TSX wrapper, and boundary package
/// conventions for [`super::TypeScriptFrontend`].
#[derive(Debug, Clone)]
pub struct TypeScriptConventions {
    pub extensions: Vec<String>,
    pub tsx_extensions: Vec<String>,
    pub ambient_declaration_suffixes: Vec<String>,
    pub top_level_integration_test_dirs: Vec<String>,
    pub nested_unit_test_dirs: Vec<String>,
    pub unit_test_stem_suffixes: Vec<String>,
    pub test_support_file_stems: Vec<String>,
    pub entrypoint_stems: Vec<String>,
    pub library_root_stems: Vec<String>,
    pub component_wrapper_functions: Vec<String>,
    pub ref_forwarding_wrappers: Vec<String>,
    pub lazy_wrapper_functions: Vec<String>,
    pub component_type_wrappers: Vec<String>,
    pub props_type_wrappers: Vec<String>,
    pub component_base_classes: Vec<String>,
    pub component_lifecycle_methods: Vec<String>,
    pub state_hook_names: Vec<String>,
    pub max_prop_shape_depth: usize,
    pub external_boundary_packages: Vec<String>,
    pub asset_extensions: Vec<String>,
    pub asset_component_queries: Vec<String>,
}

fn owned_list(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| (*s).to_string()).collect()
}

pub(super) fn default_external_boundary_packages() -> Vec<String> {
    [
        "fs",
        "http",
        "https",
        "http2",
        "net",
        "tls",
        "dgram",
        "dns",
        "child_process",
        "worker_threads",
        "cluster",
        "sqlite",
        "axios",
        "got",
        "undici",
        "node-fetch",
        "ky",
        "superagent",
        "nodemailer",
        "pg",
        "mysql",
        "mysql2",
        "sqlite3",
        "better-sqlite3",
        "mongodb",
        "mongoose",
        "redis",
        "ioredis",
        "prisma",
        "@prisma/",
        "drizzle-orm",
        "typeorm",
        "knex",
        "sequelize",
        "kafkajs",
        "amqplib",
        "@aws-sdk/",
        "aws-sdk",
        "@google-cloud/",
        "@azure/",
        "stripe",
        "twilio",
        "@sendgrid/",
        "firebase",
        "@firebase/",
        "@supabase/",
        "ws",
        "socket.io",
        "socket.io-client",
        "grpc",
        "@grpc/",
    ]
    .into_iter()
    .map(String::from)
    .collect()
}

impl Default for TypeScriptConventions {
    fn default() -> Self {
        Self {
            extensions: owned_list(&["ts", "tsx", "mts", "cts"]),
            tsx_extensions: owned_list(&["tsx"]),
            ambient_declaration_suffixes: owned_list(&[".d.ts", ".d.mts", ".d.cts"]),
            top_level_integration_test_dirs: owned_list(&["tests", "__tests__", "e2e"]),
            nested_unit_test_dirs: owned_list(&["__tests__", "__mocks__"]),
            unit_test_stem_suffixes: owned_list(&[".test", ".spec", ".stories"]),
            test_support_file_stems: owned_list(&[
                "test-utils",
                "testUtils",
                "setupTests",
                "setup-tests",
                "vitest.setup",
                "jest.setup",
            ]),
            entrypoint_stems: owned_list(&["main", "cli"]),
            library_root_stems: owned_list(&["index"]),
            component_wrapper_functions: owned_list(&["memo", "forwardRef"]),
            ref_forwarding_wrappers: owned_list(&["forwardRef"]),
            lazy_wrapper_functions: owned_list(&["lazy"]),
            component_type_wrappers: owned_list(&[
                "FC",
                "FunctionComponent",
                "VFC",
                "NamedExoticComponent",
                "MemoExoticComponent",
                "ForwardRefExoticComponent",
            ]),
            props_type_wrappers: owned_list(&[
                "PropsWithChildren",
                "PropsWithRef",
                "PropsWithoutRef",
                "Readonly",
                "Required",
                "Partial",
            ]),
            component_base_classes: owned_list(&[
                "Component",
                "PureComponent",
                "React::Component",
                "React::PureComponent",
            ]),
            component_lifecycle_methods: owned_list(&[
                "render",
                "componentDidMount",
                "componentDidUpdate",
                "componentWillUnmount",
                "componentDidCatch",
                "getDerivedStateFromError",
                "shouldComponentUpdate",
                "getSnapshotBeforeUpdate",
            ]),
            state_hook_names: owned_list(&["useState"]),
            max_prop_shape_depth: 6,
            external_boundary_packages: default_external_boundary_packages(),
            asset_extensions: owned_list(&[
                ".css", ".scss", ".sass", ".less", ".styl", ".svg", ".png", ".jpg", ".jpeg",
                ".gif", ".webp", ".avif", ".ico", ".woff", ".woff2", ".ttf", ".eot", ".mp3",
                ".mp4", ".webm", ".wasm", ".json",
            ]),
            asset_component_queries: owned_list(&["react"]),
        }
    }
}

impl TypeScriptConventions {
    pub(super) fn is_tsx_path(&self, path: &Path) -> bool {
        path.extension()
            .and_then(|e| e.to_str())
            .is_some_and(|ext| {
                self.tsx_extensions
                    .iter()
                    .any(|cand| cand.eq_ignore_ascii_case(ext))
            })
    }

    pub(super) fn strip_extension<'a>(&self, file_name: &'a str) -> Option<&'a str> {
        let (stem, ext) = file_name.rsplit_once('.')?;
        self.extensions
            .iter()
            .any(|cand| cand.eq_ignore_ascii_case(ext))
            .then_some(stem)
    }

    pub(super) fn is_ambient_declaration(&self, file_name: &str) -> bool {
        self.ambient_declaration_suffixes
            .iter()
            .any(|suffix| ends_with_ignore_ascii_case(file_name, suffix))
    }

    pub(super) fn module_path(&self, relative_path: &Path) -> ModulePath {
        let components: Vec<String> = relative_path
            .components()
            .filter_map(|c| match c {
                Component::Normal(seg) => seg.to_str().map(String::from),
                _ => None,
            })
            .collect();

        let Some((last, dirs)) = components.split_last() else {
            return ModulePath::new(vec![String::from("index")]);
        };

        let mut segments = Vec::with_capacity(components.len());
        segments.extend(dirs.iter().cloned());
        let without_ext = self.strip_extension(last).unwrap_or(last.as_str());
        let stem = self
            .unit_test_stem_suffixes
            .iter()
            .find_map(|suffix| without_ext.strip_suffix(suffix.as_str()))
            .unwrap_or(without_ext);
        let is_lib_root = self.library_root_stems.iter().any(|r| r == stem);
        if !is_lib_root && !stem.is_empty() {
            segments.push(stem.to_string());
        }

        if segments.is_empty() {
            segments.push(String::from("index"));
        }

        ModulePath::new(segments)
    }

    pub(super) fn file_role(&self, relative_path: &Path) -> FileRole {
        let normal: Vec<&str> = relative_path
            .components()
            .filter_map(|c| match c {
                Component::Normal(seg) => seg.to_str(),
                _ => None,
            })
            .collect();
        if normal.is_empty() {
            return FileRole::Module;
        }
        if normal.len() > 1
            && self
                .top_level_integration_test_dirs
                .iter()
                .any(|dir| dir == normal[0])
        {
            return FileRole::IntegrationTest;
        }
        if normal.len() > 2
            && normal[1..normal.len() - 1]
                .iter()
                .any(|dir| self.nested_unit_test_dirs.iter().any(|d| d == dir))
        {
            return FileRole::UnitTest;
        }

        let file_name = normal[normal.len() - 1];
        if let Some(stem) = self.strip_extension(file_name) {
            if self
                .unit_test_stem_suffixes
                .iter()
                .any(|suffix| stem.ends_with(suffix.as_str()))
                || self.test_support_file_stems.iter().any(|s| s == stem)
            {
                return FileRole::UnitTest;
            }
            if self.entrypoint_stems.iter().any(|s| s == stem) {
                return FileRole::BinaryEntrypoint;
            }
            let at_root = normal.len() == 1 || (normal.len() == 2 && normal[0] == "src");
            if at_root && self.library_root_stems.iter().any(|s| s == stem) {
                return FileRole::LibraryRoot;
            }
        }
        FileRole::Module
    }

    pub(super) fn is_external_boundary_package(&self, specifier: &str) -> bool {
        let normalized = specifier.strip_prefix("node:").unwrap_or(specifier);
        self.external_boundary_packages.iter().any(|pkg| {
            if pkg.ends_with('/') {
                normalized.starts_with(pkg.as_str())
            } else {
                normalized == pkg.as_str()
                    || normalized
                        .strip_prefix(pkg.as_str())
                        .is_some_and(|rest| rest.starts_with('/'))
            }
        })
    }

    pub(super) fn is_asset_specifier(&self, specifier: &str) -> bool {
        let trimmed = specifier.trim();
        let (path_part, query_part) = trimmed
            .split_once('?')
            .map_or((trimmed, None), |(p, q)| (p, Some(q)));
        if let Some(query) = query_part
            && query.split('&').any(|part| {
                let key = part.split('=').next().unwrap_or("");
                self.asset_component_queries
                    .iter()
                    .any(|cand| cand.eq_ignore_ascii_case(key))
            })
        {
            return false;
        }
        let path_part = path_part.split('#').next().unwrap_or(path_part);
        self.asset_extensions
            .iter()
            .any(|ext| ends_with_ignore_ascii_case(path_part, ext))
    }

    pub(super) fn is_component_wrapper(&self, name: &str) -> bool {
        self.component_wrapper_functions.iter().any(|w| w == name)
    }

    pub(super) fn is_ref_forwarding_wrapper(&self, name: &str) -> bool {
        self.ref_forwarding_wrappers.iter().any(|w| w == name)
    }

    pub(super) fn is_lazy_wrapper(&self, name: &str) -> bool {
        self.lazy_wrapper_functions.iter().any(|w| w == name)
    }

    pub(super) fn is_component_type_wrapper(&self, name: &str) -> bool {
        self.component_type_wrappers.iter().any(|w| w == name)
    }

    pub(super) fn is_props_type_wrapper(&self, name: &str) -> bool {
        self.props_type_wrappers.iter().any(|w| w == name)
    }

    pub(super) fn is_component_base_class(&self, name: &str) -> bool {
        self.component_base_classes.iter().any(|b| b == name)
    }

    pub(super) fn is_component_lifecycle_method(&self, name: &str) -> bool {
        self.component_lifecycle_methods.iter().any(|m| m == name)
    }

    pub(super) fn is_state_hook(&self, name: &str) -> bool {
        self.state_hook_names.iter().any(|h| h == name)
    }
}

fn ends_with_ignore_ascii_case(haystack: &str, suffix: &str) -> bool {
    haystack.len() >= suffix.len()
        && haystack.is_char_boundary(haystack.len() - suffix.len())
        && haystack[haystack.len() - suffix.len()..].eq_ignore_ascii_case(suffix)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conventions_classify_tsx_paths_and_roles() {
        let conv = TypeScriptConventions::default();

        assert!(conv.is_tsx_path(Path::new("src/App.tsx")));
        assert!(conv.is_tsx_path(Path::new("src/App.TSX")));
        assert!(!conv.is_tsx_path(Path::new("src/app.ts")));
        assert!(!conv.is_tsx_path(Path::new("src/app.mts")));

        assert_eq!(
            conv.module_path(Path::new("src/components/OrderCard.TSX"))
                .to_string(),
            "src::components::OrderCard"
        );
        assert_eq!(
            conv.module_path(Path::new("src/components/OrderCard.test.tsx"))
                .to_string(),
            "src::components::OrderCard"
        );
        assert_eq!(
            conv.module_path(Path::new("src/components/Button.stories.tsx"))
                .to_string(),
            "src::components::Button"
        );
        assert_eq!(
            conv.module_path(Path::new("src/components/index.tsx"))
                .to_string(),
            "src::components"
        );

        assert_eq!(
            conv.file_role(Path::new("src/components/__tests__/OrderCard.tsx")),
            FileRole::UnitTest
        );
        assert_eq!(
            conv.file_role(Path::new("src/__mocks__/api.ts")),
            FileRole::UnitTest
        );
        assert_eq!(
            conv.file_role(Path::new("src/test-utils.tsx")),
            FileRole::UnitTest
        );
        assert_eq!(
            conv.file_role(Path::new("src/main.tsx")),
            FileRole::BinaryEntrypoint
        );
        assert_eq!(
            conv.file_role(Path::new("src/index.tsx")),
            FileRole::LibraryRoot
        );
    }

    #[test]
    fn conventions_classify_boundaries_assets_and_ambient_files() {
        let conv = TypeScriptConventions::default();

        assert!(conv.is_external_boundary_package("node:fs/promises"));
        assert!(conv.is_external_boundary_package("fs"));
        assert!(conv.is_external_boundary_package("axios"));
        assert!(conv.is_external_boundary_package("@aws-sdk/client-s3"));
        assert!(conv.is_external_boundary_package("nodemailer"));
        assert!(!conv.is_external_boundary_package("react"));
        assert!(!conv.is_external_boundary_package("react-router-dom"));
        assert!(!conv.is_external_boundary_package("vitest"));
        assert!(!conv.is_external_boundary_package("ws-mock"));

        assert!(conv.is_asset_specifier("./OrderCard.module.css"));
        assert!(conv.is_asset_specifier("./styles.css?inline"));
        assert!(conv.is_asset_specifier("./logo.SVG?url"));
        assert!(!conv.is_asset_specifier("./icon.svg?react"));
        assert!(!conv.is_asset_specifier("./OrderCard.tsx"));

        assert!(conv.is_ambient_declaration("env.d.ts"));
        assert!(conv.is_ambient_declaration("types.d.mts"));
        assert!(conv.is_ambient_declaration("types.d.cts"));
        assert!(!conv.is_ambient_declaration("types.ts"));
    }
}
