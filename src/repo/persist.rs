//! Surgical TOML persistence for `[paths] ignored` hide/unhide toggles.

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use toml_edit::{Array, DocumentMut, Item, Table, Value};

/// Append `entry` to `[paths] ignored`, creating the file/table/array as
/// needed. No-op when the exact entry is already present.
pub(crate) fn add_ignored_entry(config_file: &Path, entry: &str) -> io::Result<()> {
    let mut doc = load_document(config_file)?;
    let array = ignored_array(&mut doc)?;
    if array.iter().any(|value| value.as_str() == Some(entry)) {
        return Ok(());
    }
    array.push(entry);
    atomic_write(config_file, doc.to_string().as_bytes())
}

/// Remove every element of `[paths] ignored` equal to `entry`. Returns `true`
/// when at least one element was removed; `false` means the entry was not
/// present verbatim (e.g. the path is hidden by a broader glob instead).
pub(crate) fn remove_ignored_entry(config_file: &Path, entry: &str) -> io::Result<bool> {
    let mut doc = load_document(config_file)?;
    let array = ignored_array(&mut doc)?;
    let before = array.len();
    array.retain(|value| value.as_str() != Some(entry));
    if array.len() == before {
        return Ok(false);
    }
    atomic_write(config_file, doc.to_string().as_bytes())?;
    Ok(true)
}

fn load_document(config_file: &Path) -> io::Result<DocumentMut> {
    let raw = match fs::read_to_string(config_file) {
        Ok(raw) => raw,
        Err(err) if err.kind() == io::ErrorKind::NotFound => String::new(),
        Err(err) => return Err(err),
    };
    if raw.trim().is_empty() {
        return Ok(DocumentMut::new());
    }
    raw.parse::<DocumentMut>()
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
}

fn ignored_array(doc: &mut DocumentMut) -> io::Result<&mut Array> {
    let table = doc.as_table_mut();
    let paths = table
        .entry("paths")
        .or_insert_with(|| Item::Table(Table::new()));
    let Some(paths) = paths.as_table_mut() else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "config path `paths` is not a table",
        ));
    };
    let ignored = paths
        .entry("ignored")
        .or_insert_with(|| Item::Value(Value::Array(Array::default())));
    ignored
        .as_value_mut()
        .and_then(|value| value.as_array_mut())
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "config path `paths.ignored` is not an array",
            )
        })
}

/// Write `bytes` to a sibling temp file, then atomically rename it over
/// `path`.
///
/// Temp-file cleanup: a successful `rename` consumes the temp file (it becomes
/// `path`), so nothing is left behind. If the `rename` fails, the temp file is
/// explicitly removed before returning the error. The only path that can leak
/// a temp file is a failure in `create`/`write_all`/`sync_all` before the
/// rename; the process-id-suffixed name keeps such a stray file from colliding
/// with a later write, and it is overwritten by the next successful toggle.
fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = tmp_path(path);
    let mut file = File::create(&tmp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    match fs::rename(&tmp, path) {
        Ok(()) => Ok(()),
        Err(err) => {
            let _ = fs::remove_file(&tmp);
            Err(err)
        }
    }
}

fn tmp_path(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("config.toml");
    path.with_file_name(format!(".{name}.tmp-{}", std::process::id()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_config(prefix: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};

        static NEXT: AtomicU64 = AtomicU64::new(1);
        let dir = std::env::temp_dir().join(format!(
            "pb-repo-persist-{prefix}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir.join("config.toml")
    }

    #[test]
    fn add_creates_file_table_and_array_and_is_idempotent() {
        let config = temp_config("add");

        add_ignored_entry(&config, "team/secret").unwrap();
        add_ignored_entry(&config, "team/secret").unwrap();

        let saved = fs::read_to_string(&config).unwrap();
        assert_eq!(saved.matches("team/secret").count(), 1);
        let parsed: toml::Value = toml::from_str(&saved).unwrap();
        assert_eq!(
            parsed["paths"]["ignored"],
            toml::Value::Array(vec![toml::Value::String("team/secret".into())])
        );
    }

    #[test]
    fn remove_deletes_matching_entries_and_reports_missing() {
        let config = temp_config("remove");
        fs::write(
            &config,
            "# keep me\n[paths]\nignored = [\"a\", \"team/secret\", \"b\"]\nsnippets = [\"/x\"]\n",
        )
        .unwrap();

        assert!(remove_ignored_entry(&config, "team/secret").unwrap());
        assert!(!remove_ignored_entry(&config, "not-there").unwrap());

        let saved = fs::read_to_string(&config).unwrap();
        assert!(saved.contains("# keep me"));
        assert!(saved.contains("snippets = [\"/x\"]"));
        assert!(!saved.contains("team/secret"));
        assert!(saved.contains("\"a\""));
        assert!(saved.contains("\"b\""));
    }

    #[test]
    fn invalid_ignored_type_is_an_error() {
        let config = temp_config("bad-type");
        fs::write(&config, "[paths]\nignored = \"nope\"\n").unwrap();
        let err = add_ignored_entry(&config, "x").unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }
}
