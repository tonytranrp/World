//! Every registered config, its status, and type-erased access by name, so the
//! developer panel can list, edit and save configs without knowing their types.

use std::any::TypeId;
use std::io::Write;
use std::path::{Path, PathBuf};

use bevy::prelude::*;

use super::parse::{ConfigError, parse_config, to_pretty_ron};
use super::{Config, ConfigAsset};

/// Where the live value of a config came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfigOrigin {
    /// The file has not loaded (yet, or ever): the type's safe defaults are live.
    Defaults,
    /// The file on disk.
    File,
    /// Applied by hand from the developer panel and not saved.
    Edited,
}

/// The status of one config, for logs, tests and the developer panel.
#[derive(Clone, Debug)]
pub struct ConfigEntry {
    pub name: &'static str,
    /// Asset path, relative to the asset root.
    pub path: String,
    pub origin: ConfigOrigin,
    /// Values applied so far: the first load, each reload and each hand edit.
    pub applied: u32,
    /// Loads or edits that were rejected.
    pub failures: u32,
    /// Why the most recent rejection happened. Cleared by the next success.
    pub last_error: Option<String>,
    type_id: TypeId,
    ops: ConfigOps,
}

#[derive(Clone, Copy, Debug)]
struct ConfigOps {
    current_text: fn(&World) -> String,
    apply_text: fn(&mut World, &str) -> Result<(), ConfigError>,
}

/// Why saving a config failed.
#[derive(Debug)]
pub enum SaveError {
    UnknownConfig(String),
    /// Saving needs a file asset root, which `WorldReloadPlugin` records.
    NoAssetRoot,
    Invalid(ConfigError),
    Io(std::io::Error),
}

impl std::fmt::Display for SaveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownConfig(name) => write!(f, "no config is named {name:?}"),
            Self::NoAssetRoot => f.write_str("the asset root folder is unknown"),
            Self::Invalid(e) => write!(f, "not saved: {e}"),
            Self::Io(e) => write!(f, "could not write the file: {e}"),
        }
    }
}

impl std::error::Error for SaveError {}

#[derive(Resource, Default, Debug)]
pub struct ConfigRegistry {
    entries: Vec<ConfigEntry>,
    /// Folder that asset paths are relative to, when assets come from disk.
    root: Option<PathBuf>,
}

impl ConfigRegistry {
    pub fn entries(&self) -> &[ConfigEntry] {
        &self.entries
    }

    pub fn get(&self, name: &str) -> Option<&ConfigEntry> {
        self.entries.iter().find(|e| e.name == name)
    }

    pub fn set_root(&mut self, root: impl Into<PathBuf>) {
        self.root = Some(root.into());
    }

    /// The file on disk behind a config, when the asset root is known.
    pub fn file_path(&self, name: &str) -> Option<PathBuf> {
        Some(self.root.as_ref()?.join(&self.get(name)?.path))
    }

    /// Adds `T`. Returns `false` when `T` is already registered.
    ///
    /// # Panics
    /// When a different type already uses the same name or the same file.
    pub(super) fn register<T: ConfigAsset>(&mut self) -> bool {
        let type_id = TypeId::of::<T>();
        if self.entries.iter().any(|e| e.type_id == type_id) {
            return false;
        }
        if let Some(clash) = self
            .entries
            .iter()
            .find(|e| e.name == T::NAME || e.path == T::PATH)
        {
            panic!(
                "config {:?} ({}) clashes with config {:?} ({}): names and files must be unique",
                T::NAME,
                T::PATH,
                clash.name,
                clash.path
            );
        }
        self.entries.push(ConfigEntry {
            name: T::NAME,
            path: T::PATH.to_owned(),
            origin: ConfigOrigin::Defaults,
            applied: 0,
            failures: 0,
            last_error: None,
            type_id,
            ops: ConfigOps {
                current_text: current_text::<T>,
                apply_text: apply_text::<T>,
            },
        });
        true
    }

    fn entry_mut(&mut self, name: &str) -> &mut ConfigEntry {
        self.entries
            .iter_mut()
            .find(|e| e.name == name)
            .expect("configs are registered before they are applied")
    }

    pub(super) fn record_applied(&mut self, name: &str, origin: ConfigOrigin) {
        let entry = self.entry_mut(name);
        entry.origin = origin;
        entry.applied += 1;
        entry.last_error = None;
    }

    pub(super) fn record_failure(&mut self, name: &str, error: String) {
        let entry = self.entry_mut(name);
        entry.failures += 1;
        entry.last_error = Some(error);
    }

    /// The live value of a config as pretty RON.
    pub fn current_text(world: &World, name: &str) -> Option<String> {
        let ops = world.resource::<Self>().get(name)?.ops;
        Some((ops.current_text)(world))
    }

    /// Parses, validates and applies `text` to the live config, without
    /// touching the file. A rejected text changes nothing but the status.
    pub fn apply_text(world: &mut World, name: &str, text: &str) -> Result<(), ConfigError> {
        let ops = world
            .resource::<Self>()
            .get(name)
            .ok_or_else(|| ConfigError::UnknownConfig(name.to_owned()))?
            .ops;
        let result = (ops.apply_text)(world, text);
        let mut registry = world.resource_mut::<Self>();
        match &result {
            Ok(()) => registry.record_applied(name, ConfigOrigin::Edited),
            Err(e) => registry.record_failure(name, e.to_string()),
        }
        result
    }

    /// Applies `text`, then writes it to the config's file. Invalid text is
    /// never written. The write is atomic and flushed to disk: a temporary file
    /// is renamed over the old one, so a crash cannot leave half a file. The
    /// live value then matches the file, so the status says so straight away;
    /// the watcher's reload of the same text confirms it. Returns the file written.
    pub fn save_text(world: &mut World, name: &str, text: &str) -> Result<PathBuf, SaveError> {
        let path = {
            let registry = world.resource::<Self>();
            registry
                .get(name)
                .ok_or_else(|| SaveError::UnknownConfig(name.to_owned()))?;
            registry.file_path(name).ok_or(SaveError::NoAssetRoot)?
        };
        Self::apply_text(world, name, text).map_err(SaveError::Invalid)?;
        write_atomically(&path, text).map_err(SaveError::Io)?;
        world.resource_mut::<Self>().entry_mut(name).origin = ConfigOrigin::File;
        Ok(path)
    }
}

fn current_text<T: ConfigAsset>(world: &World) -> String {
    to_pretty_ron(&world.resource::<Config<T>>().0)
}

fn apply_text<T: ConfigAsset>(world: &mut World, text: &str) -> Result<(), ConfigError> {
    let value = parse_config::<T>(text)?;
    world.resource_mut::<Config<T>>().0 = value;
    Ok(())
}

fn write_atomically(path: &Path, text: &str) -> std::io::Result<()> {
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(".tmp");
    let temporary = PathBuf::from(temporary);
    let result =
        write_and_flush(&temporary, text).and_then(|()| rename_with_retries(&temporary, path));
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

fn write_and_flush(path: &Path, text: &str) -> std::io::Result<()> {
    let mut file = std::fs::File::create(path)?;
    file.write_all(text.as_bytes())?;
    file.sync_all()
}

/// On Windows a virus scanner or indexer can hold the target for a moment,
/// which makes the rename fail with "access denied"; a short retry fixes it.
fn rename_with_retries(from: &Path, to: &Path) -> std::io::Result<()> {
    let mut attempt = 0;
    loop {
        match std::fs::rename(from, to) {
            Err(e) if attempt < 5 && e.kind() == std::io::ErrorKind::PermissionDenied => {
                attempt += 1;
                std::thread::sleep(std::time::Duration::from_millis(20 * attempt));
            }
            result => return result,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_write_replaces_and_leaves_no_temporary_file() {
        let dir = std::env::temp_dir().join(format!("world_reload_atomic_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("a.ron");
        std::fs::write(&file, "old").unwrap();
        write_atomically(&file, "new").unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "new");
        assert!(!dir.join("a.ron.tmp").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
