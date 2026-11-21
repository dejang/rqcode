use crate::ast::{LanguageId, LanguageSpec};
use crate::lang::LanguageRegistry;
use ignore::WalkBuilder;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SourceError {
    #[error("path does not exist: {0}")]
    MissingRoot(PathBuf),
    #[error("failed to walk directory: {0}")]
    Walk(#[from] ignore::Error),
    #[error("failed to read source file {path}: {source}")]
    ReadFile {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// A discovered source file ready to be parsed into a `SyntaxTree`.
#[derive(Debug, Clone)]
pub struct SourceFile {
    path: PathBuf,
    relative_path: PathBuf,
    language: LanguageSpec,
    content: String,
}

impl SourceFile {
    pub fn new(
        path: impl Into<PathBuf>,
        relative_path: impl Into<PathBuf>,
        language: LanguageSpec,
        content: impl Into<String>,
    ) -> Self {
        Self {
            path: path.into(),
            relative_path: relative_path.into(),
            language,
            content: content.into(),
        }
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    #[must_use]
    pub fn relative_path(&self) -> &Path {
        &self.relative_path
    }

    #[must_use]
    pub fn language(&self) -> &LanguageSpec {
        &self.language
    }

    #[must_use]
    pub fn language_id(&self) -> LanguageId {
        self.language.id()
    }

    #[must_use]
    pub fn content(&self) -> &str {
        &self.content
    }
}

/// Collection of discovered source files rooted at a project directory.
#[derive(Debug, Clone)]
pub struct SourceTree {
    root: PathBuf,
    files: Vec<SourceFile>,
}

impl SourceTree {
    pub fn from_files(root: impl Into<PathBuf>, files: Vec<SourceFile>) -> Self {
        Self {
            root: root.into(),
            files,
        }
    }

    /// Recursively discover supported source files under `root`, honoring gitignore and `excludes`.
    ///
    /// # Errors
    /// Returns [`SourceError::MissingRoot`] if `root` does not exist, [`SourceError::Walk`] on directory
    /// traversal errors, or [`SourceError::ReadFile`] if a discovered file cannot be read.
    pub fn discover(
        root: &Path,
        registry: &LanguageRegistry,
        excludes: &[String],
    ) -> Result<Self, SourceError> {
        if !root.exists() {
            return Err(SourceError::MissingRoot(root.to_path_buf()));
        }

        let canonical_root = root.to_path_buf();
        let mut files = Vec::new();

        if root.is_file() {
            if let Some(frontend) = registry.frontend_for_path(root) {
                let content =
                    std::fs::read_to_string(root).map_err(|source| SourceError::ReadFile {
                        path: root.to_path_buf(),
                        source,
                    })?;
                let rel = root
                    .file_name()
                    .map_or_else(|| root.to_path_buf(), PathBuf::from);
                files.push(SourceFile::new(root, rel, frontend.grammar(), content));
            }
            return Ok(Self {
                root: root.parent().unwrap_or(root).to_path_buf(),
                files,
            });
        }

        let default_excludes = registry.default_excludes();
        let excluded_components: HashSet<&str> = excludes
            .iter()
            .map(String::as_str)
            .chain(default_excludes.iter().copied())
            .map(|p| p.trim_matches('/'))
            .filter(|p| !p.is_empty())
            .collect();
        let excluded_prefixes: Vec<&str> = excluded_components
            .iter()
            .copied()
            .filter(|p| p.contains('/'))
            .collect();

        let walker = WalkBuilder::new(root).hidden(true).git_ignore(true).build();

        for entry in walker {
            let entry = entry?;
            let path = entry.path();
            if !path.is_file() {
                continue;
            }

            let relative = path
                .strip_prefix(&canonical_root)
                .unwrap_or(path)
                .to_path_buf();

            let mut is_skipped = false;
            for component in relative.components() {
                if let Some(seg) = component.as_os_str().to_str()
                    && excluded_components.contains(seg)
                {
                    is_skipped = true;
                    break;
                }
            }
            if !is_skipped && !excluded_prefixes.is_empty() {
                for prefix in &excluded_prefixes {
                    if relative.starts_with(prefix) {
                        is_skipped = true;
                        break;
                    }
                }
            }
            if is_skipped {
                continue;
            }

            let Some(frontend) = registry.frontend_for_path(path) else {
                continue;
            };

            let content =
                std::fs::read_to_string(path).map_err(|source| SourceError::ReadFile {
                    path: path.to_path_buf(),
                    source,
                })?;

            files.push(SourceFile::new(path, relative, frontend.grammar(), content));
        }

        files.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));

        Ok(Self {
            root: canonical_root,
            files,
        })
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    #[must_use]
    pub fn files(&self) -> &[SourceFile] {
        &self.files
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovers_supported_files_and_applies_exclusions() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::create_dir_all(dir.path().join("generated")).unwrap();
        std::fs::create_dir_all(dir.path().join("target")).unwrap();

        std::fs::write(dir.path().join("src/lib.rs"), "pub fn ok() {}").unwrap();
        std::fs::write(dir.path().join("generated/auto.rs"), "pub fn gen() {}").unwrap();
        std::fs::write(dir.path().join("target/build.rs"), "pub fn built() {}").unwrap();
        std::fs::write(dir.path().join("README.md"), "# docs").unwrap();

        let registry = LanguageRegistry::with_defaults();
        let tree = SourceTree::discover(dir.path(), &registry, &["generated".to_string()])
            .expect("discover succeeds");

        assert_eq!(tree.files().len(), 1);
        assert_eq!(tree.files()[0].relative_path(), Path::new("src/lib.rs"));
    }
}
