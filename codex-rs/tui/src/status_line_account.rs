use std::ffi::OsStr;
use std::path::Path;

pub(crate) fn label_for_codex_home(codex_home: Option<&Path>) -> Option<String> {
    let codex_home = codex_home?;
    if !codex_home.is_dir() {
        return None;
    }
    let instances_dir = codex_home.parent()?;
    if instances_dir.file_name() != Some(OsStr::new("instances")) {
        return None;
    }

    Some(codex_home.file_name()?.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::label_for_codex_home;
    use std::fs;
    use std::path::Path;
    use tempfile::tempdir;

    fn create_instance_home(root: &Path, slug: &str) -> std::path::PathBuf {
        let instance_home = root.join("instances").join(slug);
        fs::create_dir_all(&instance_home).expect("create instance home");
        instance_home
    }

    #[test]
    fn returns_instance_slug_for_instance_codex_home() {
        let root = tempdir().expect("create temp root");
        let instance_home = create_instance_home(root.path(), "ebjd7");
        let label = label_for_codex_home(Some(&instance_home));

        assert_eq!(label, Some("ebjd7".to_string()));
    }

    #[test]
    fn omits_root_codex_home() {
        let root = tempdir().expect("create temp root");
        let label = label_for_codex_home(Some(root.path()));

        assert_eq!(label, None);
    }

    #[test]
    fn omits_non_instance_directory() {
        let root = tempdir().expect("create temp root");
        let instance_home = root.path().join("sessions").join("demo");
        fs::create_dir_all(&instance_home).expect("create instance home");
        let label = label_for_codex_home(Some(&instance_home));

        assert_eq!(label, None);
    }

    #[test]
    fn omits_missing_instance_home() {
        let root = tempdir().expect("create temp root");
        let instance_home = root.path().join("instances").join("demo");
        let label = label_for_codex_home(Some(&instance_home));

        assert_eq!(label, None);
    }
}
