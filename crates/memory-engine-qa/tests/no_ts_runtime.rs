use std::{
    fs,
    path::{Path, PathBuf},
};

#[test]
fn no_non_dagger_typescript_runtime_files_remain() {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("qa crate is under crates/");
    let mut offenders = Vec::new();

    collect_typescript_files(repo_root, repo_root, &mut offenders);
    offenders.sort();

    assert!(
        offenders.is_empty(),
        "non-Dagger TypeScript runtime files remain:\n{}",
        offenders
            .iter()
            .map(|path| format!("- {}", path.display()))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

fn collect_typescript_files(root: &Path, directory: &Path, offenders: &mut Vec<PathBuf>) {
    let entries = fs::read_dir(directory)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", directory.display()));

    for entry in entries {
        let entry = entry.expect("directory entry is readable");
        let path = entry.path();
        let relative = path
            .strip_prefix(root)
            .expect("path is under repository root");

        if should_skip(relative) {
            continue;
        }

        let file_type = entry.file_type().expect("file type is readable");
        if file_type.is_dir() {
            collect_typescript_files(root, &path, offenders);
            continue;
        }

        if file_type.is_file() && is_typescript_file(&path) {
            offenders.push(relative.to_path_buf());
        }
    }
}

fn should_skip(relative: &Path) -> bool {
    relative
        .components()
        .next()
        .and_then(|component| component.as_os_str().to_str())
        .is_some_and(|name| matches!(name, ".dagger" | ".git" | "node_modules" | "target"))
}

fn is_typescript_file(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| matches!(extension, "ts" | "tsx" | "mts" | "cts"))
}
