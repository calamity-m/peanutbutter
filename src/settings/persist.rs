//! Surgical TOML persistence for settings edits.

use crate::settings::app::{Field, FieldKind};
use std::borrow::Borrow;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use toml_edit::{DocumentMut, Item, Table, Value};

/// Write changed fields to `config_file` while preserving unrelated TOML text.
pub(crate) fn save_changed_fields<I, F>(config_file: &Path, fields: I) -> io::Result<usize>
where
    I: IntoIterator<Item = F>,
    F: Borrow<Field>,
{
    let changed = fields
        .into_iter()
        .filter_map(|field| {
            let field = field.borrow();
            field.changed().then(|| field.clone())
        })
        .collect::<Vec<_>>();
    if changed.is_empty() {
        return Ok(0);
    }

    let raw = match fs::read_to_string(config_file) {
        Ok(raw) => raw,
        Err(err) if err.kind() == io::ErrorKind::NotFound => String::new(),
        Err(err) => return Err(err),
    };
    let mut doc = if raw.trim().is_empty() {
        DocumentMut::new()
    } else {
        raw.parse::<DocumentMut>()
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?
    };

    for field in &changed {
        set_field(&mut doc, field)?;
    }

    atomic_write(config_file, doc.to_string().as_bytes())?;
    Ok(changed.len())
}

fn set_field(doc: &mut DocumentMut, field: &Field) -> io::Result<()> {
    let mut table = doc.as_table_mut();
    for segment in field.toml_path {
        let item = table
            .entry(segment)
            .or_insert_with(|| Item::Table(Table::new()));
        if !item.is_table() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("config path `{}` is not a table", field.toml_path.join(".")),
            ));
        }
        table = item.as_table_mut().expect("checked table");
    }
    let value = match field.kind {
        FieldKind::Float => Item::Value(Value::from(field.value)),
        FieldKind::Int => Item::Value(Value::from(field.value.round() as i64)),
    };
    table[field.key] = value;
    Ok(())
}

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
    use crate::settings::app::{FieldKind, Readout};
    use std::sync::atomic::{AtomicU64, Ordering};

    fn temp_dir(prefix: &str) -> PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let path = std::env::temp_dir().join(format!(
            "pb-settings-{prefix}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn field(
        path: &'static [&'static str],
        key: &'static str,
        kind: FieldKind,
        value: f64,
        original: f64,
    ) -> Field {
        Field {
            label: key,
            toml_path: path,
            key,
            kind,
            min: 0.0,
            max: 1000.0,
            step: 1.0,
            default: original,
            value,
            original,
            help: "help",
            readout: Readout::Multiplier,
        }
    }

    #[test]
    fn preserves_comments_and_targets_nested_tables() {
        let root = temp_dir("preserve");
        let config = root.join("config.toml");
        fs::write(
            &config,
            "# hello\n[search]\n# blend\nfrecency_weight = 250.0\n[search.frecency]\nlocation_weight = 1.0\n[other]\nkeep = true\n",
        )
        .unwrap();

        let changed = save_changed_fields(
            &config,
            [field(
                &["search", "frecency"],
                "location_weight",
                FieldKind::Float,
                2.5,
                1.0,
            )],
        )
        .unwrap();
        let saved = fs::read_to_string(&config).unwrap();

        assert_eq!(changed, 1);
        assert!(saved.contains("# hello"));
        assert!(saved.contains("# blend"));
        assert!(saved.contains("frecency_weight = 250.0"));
        assert!(saved.contains("location_weight = 2.5"));
        assert!(saved.contains("[other]\nkeep = true"));
    }

    #[test]
    fn creates_missing_file_and_tables() {
        let root = temp_dir("missing");
        let config = root.join("nested/config.toml");

        save_changed_fields(
            &config,
            [field(
                &["search"],
                "frecency_weight",
                FieldKind::Float,
                300.0,
                250.0,
            )],
        )
        .unwrap();

        let saved = fs::read_to_string(&config).unwrap();
        assert!(saved.contains("[search]"));
        assert!(saved.contains("frecency_weight = 300.0"));
    }

    #[test]
    fn emits_ints_not_floats_and_skips_unchanged() {
        let root = temp_dir("ints");
        let config = root.join("config.toml");
        fs::write(&config, "[search.fuzzy]\nname = 30\n").unwrap();

        let changed = save_changed_fields(
            &config,
            [
                field(&["search", "fuzzy"], "name", FieldKind::Int, 31.0, 30.0),
                field(&["search", "fuzzy"], "command", FieldKind::Int, 8.0, 8.0),
            ],
        )
        .unwrap();
        let saved = fs::read_to_string(&config).unwrap();

        assert_eq!(changed, 1);
        assert!(saved.contains("name = 31"));
        assert!(!saved.contains("31.0"));
        assert!(!saved.contains("command"));
    }

    #[test]
    fn invalid_existing_toml_is_left_untouched() {
        let root = temp_dir("bad");
        let config = root.join("config.toml");
        fs::write(&config, "[search\nnope").unwrap();

        let err = save_changed_fields(
            &config,
            [field(
                &["search"],
                "frecency_weight",
                FieldKind::Float,
                300.0,
                250.0,
            )],
        )
        .unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert_eq!(fs::read_to_string(&config).unwrap(), "[search\nnope");
    }
}
