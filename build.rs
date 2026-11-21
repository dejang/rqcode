use std::env;
use std::error::Error;
use std::fmt::Write as _;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

fn main() -> Result<(), Box<dyn Error>> {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR")?);
    let rules_dir = manifest_dir.join("rules");

    println!("cargo:rerun-if-changed={}", rules_dir.display());

    let mut rule_files = Vec::new();
    if rules_dir.is_dir() {
        collect_yaml_files(&rules_dir, &mut rule_files)?;
    }
    rule_files.sort();

    let mut generated = String::from("pub const EMBEDDED_AST_GREP_RULES: &[(&str, &str)] = &[\n");
    for abs_path in &rule_files {
        println!("cargo:rerun-if-changed={}", abs_path.display());
        let rel = abs_path
            .strip_prefix(&manifest_dir)
            .unwrap_or(abs_path)
            .to_string_lossy()
            .replace('\\', "/");
        let abs_str = abs_path.to_string_lossy();
        let _ = writeln!(generated, "    ({rel:?}, include_str!({abs_str:?})),");
    }
    generated.push_str("];\n");

    let out_dir = PathBuf::from(env::var("OUT_DIR")?);
    fs::write(out_dir.join("embedded_ast_grep_rules.rs"), generated)?;
    Ok(())
}

fn collect_yaml_files(dir: &Path, out: &mut Vec<PathBuf>) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            println!("cargo:rerun-if-changed={}", path.display());
            collect_yaml_files(&path, out)?;
        } else if path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("yml") || ext.eq_ignore_ascii_case("yaml"))
        {
            out.push(path);
        }
    }
    Ok(())
}
