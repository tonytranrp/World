//! Asset sources for hot-reload tests.
//!
//! - [`MemoryAssets`]: an in-memory asset root with a hand-driven watcher.
//!   Writing a file and announcing the change are one call, with no file
//!   system, no debounce and no timing dependence. Use it by default.
//! - [`DiskAssets`]: a real temporary folder watched by Bevy's own file
//!   watcher (needs the `file_watcher` feature), for end-to-end tests of the
//!   real path including the 300 ms debounce.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use bevy::app::App;
use bevy::asset::io::memory::{Dir, MemoryAssetReader};
use bevy::asset::io::{AssetSourceBuilder, AssetSourceEvent, AssetSourceId, AssetWatcher};
use bevy::asset::{AssetApp, AssetPlugin};

/// A watcher that watches nothing: changes are announced by [`MemoryAssets`].
struct HandDrivenWatcher;

impl AssetWatcher for HandDrivenWatcher {}

type EventSender = async_channel::Sender<AssetSourceEvent>;

/// An in-memory default asset source. Clone it freely; clones share the files.
#[derive(Clone)]
pub struct MemoryAssets {
    root: Dir,
    sender: Arc<Mutex<Option<EventSender>>>,
}

impl MemoryAssets {
    /// Replaces the default asset source of `app` and adds `AssetPlugin` with
    /// watching on. Call before anything else adds `AssetPlugin`.
    pub fn install(app: &mut App) -> Self {
        let assets = Self {
            root: Dir::new(PathBuf::new()),
            sender: Arc::default(),
        };
        let (root, slot) = (assets.root.clone(), assets.sender.clone());
        app.register_asset_source(
            AssetSourceId::Default,
            AssetSourceBuilder::new(move || Box::new(MemoryAssetReader { root: root.clone() }))
                .with_watcher(move |sender| {
                    *slot.lock().unwrap() = Some(sender);
                    Some(Box::new(HandDrivenWatcher))
                }),
        );
        app.add_plugins(AssetPlugin {
            watch_for_changes_override: Some(true),
            ..Default::default()
        });
        assets
    }

    /// Stores a file without announcing it, as if it existed before startup.
    pub fn put(&self, path: &str, text: &str) {
        self.root.insert_asset_text(Path::new(path), text);
    }

    /// Changes a file and announces the change, as a save in an editor would.
    pub fn write(&self, path: &str, text: &str) {
        self.put(path, text);
        self.announce(AssetSourceEvent::ModifiedAsset(PathBuf::from(path)));
    }

    /// Deletes a file and announces it.
    pub fn remove(&self, path: &str) {
        self.root.remove_asset(Path::new(path));
        self.announce(AssetSourceEvent::RemovedAsset(PathBuf::from(path)));
    }

    fn announce(&self, event: AssetSourceEvent) {
        let sender = self.sender.lock().unwrap();
        let sender = sender
            .as_ref()
            .expect("the watcher starts when the app is built; step the app once first");
        sender.send_blocking(event).expect("asset server stopped");
    }
}

/// A fresh temporary folder used as the asset root, deleted on drop.
pub struct DiskAssets {
    root: PathBuf,
}

impl DiskAssets {
    /// Creates the folder. Pass [`DiskAssets::plugin`] to the app.
    pub fn new(label: &str) -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let root = std::env::temp_dir().join(format!(
            "world_assets_{label}_{}_{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("could not create a temporary asset folder");
        // macOS's temp folder (/var/...) is a symlink to /private/var/...; the file
        // watcher reports resolved paths, and Bevy 0.20.1 panics when they do not
        // start with the root it was given. Resolve it first. Not on Windows, where
        // canonical paths gain a `\\?\` prefix that the watcher's paths lack.
        #[cfg(unix)]
        let root = root.canonicalize().expect("the folder was just created");
        Self { root }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// An `AssetPlugin` reading from this folder with Bevy's file watcher on.
    pub fn plugin(&self) -> AssetPlugin {
        AssetPlugin {
            file_path: self.root.to_string_lossy().into_owned(),
            watch_for_changes_override: Some(true),
            ..Default::default()
        }
    }

    /// Writes a file, creating its folders.
    pub fn write(&self, path: &str, text: &str) {
        let file = self.root.join(path);
        if let Some(parent) = file.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(file, text).unwrap();
    }

    pub fn read(&self, path: &str) -> String {
        std::fs::read_to_string(self.root.join(path)).unwrap()
    }

    pub fn remove(&self, path: &str) {
        std::fs::remove_file(self.root.join(path)).unwrap();
    }
}

impl Drop for DiskAssets {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
