use super::{
    PythonPackageManager, PythonPackageManagerResolution, PythonResolutionKind,
    detect_python_package_manager,
};
use std::fs;
use std::path::Path;

pub fn resolve_python_package_manager(
    repo_root: &Path,
    workdir: &Path,
) -> Option<PythonPackageManagerResolution> {
    let direct =
        detect_python_package_manager(workdir).map(|manager| PythonPackageManagerResolution {
            manager,
            resolved_from: workdir.to_path_buf(),
            kind: PythonResolutionKind::DirectRoot,
            ambiguous: false,
        });
    let has_manifest = workdir.join("pyproject.toml").is_file()
        || workdir.join("requirements.txt").is_file()
        || workdir.join("Pipfile").is_file();
    let ancestor = find_workspace_ancestor_resolution(repo_root, workdir);
    match (direct, ancestor) {
        (Some(direct), Some(ancestor))
            if direct.manager == PythonPackageManager::Pip
                && ancestor.manager == PythonPackageManager::Uv =>
        {
            Some(PythonPackageManagerResolution {
                ambiguous: true,
                ..ancestor
            })
        }
        (Some(direct), Some(mut ancestor))
            if ancestor.manager != direct.manager
                && ancestor.kind == PythonResolutionKind::WorkspaceAncestor =>
        {
            ancestor.ambiguous = true;
            Some(direct)
        }
        (Some(direct), _) => Some(direct),
        (None, Some(ancestor)) => Some(ancestor),
        (None, None) if has_manifest => Some(PythonPackageManagerResolution {
            manager: PythonPackageManager::Pip,
            resolved_from: workdir.to_path_buf(),
            kind: PythonResolutionKind::Fallback,
            ambiguous: false,
        }),
        (None, None) => None,
    }
}

fn find_workspace_ancestor_resolution(
    repo_root: &Path,
    workdir: &Path,
) -> Option<PythonPackageManagerResolution> {
    let mut current = workdir.parent();
    while let Some(path) = current {
        if !path.starts_with(repo_root) {
            break;
        }
        if path.join("uv.lock").is_file() || pyproject_has_uv_workspace(path) {
            return Some(PythonPackageManagerResolution {
                manager: PythonPackageManager::Uv,
                resolved_from: path.to_path_buf(),
                kind: PythonResolutionKind::WorkspaceAncestor,
                ambiguous: false,
            });
        }
        current = path.parent();
    }
    None
}

fn pyproject_has_uv_workspace(root: &Path) -> bool {
    let pyproject_path = root.join("pyproject.toml");
    let Ok(content) = fs::read_to_string(pyproject_path) else {
        return false;
    };
    let Ok(value) = toml::from_str::<toml::Value>(&content) else {
        return false;
    };
    value
        .get("tool")
        .and_then(|value| value.get("uv"))
        .and_then(|value| value.get("workspace"))
        .is_some()
}
