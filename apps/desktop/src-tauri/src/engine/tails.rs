use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use tauri::async_runtime::JoinHandle;

/// Open live streams, such as a log tail, by id, so the webview can close one.
/// Cheap to clone; every clone is the same set.
#[derive(Clone, Default)]
pub struct Tails(Arc<Open>);

#[derive(Default)]
struct Open {
    next: AtomicU32,
    tasks: Mutex<HashMap<u32, JoinHandle<()>>>,
}

impl Tails {
    pub fn next_id(&self) -> u32 {
        self.0.next.fetch_add(1, Ordering::Relaxed)
    }

    pub fn insert(&self, id: u32, task: JoinHandle<()>) {
        self.tasks().insert(id, task);
    }

    /// Forgets a stream that ended by itself.
    pub fn finished(&self, id: u32) {
        self.tasks().remove(&id);
    }

    pub fn stop(&self, id: u32) {
        if let Some(task) = self.tasks().remove(&id) {
            task.abort();
        }
    }

    fn tasks(&self) -> std::sync::MutexGuard<'_, HashMap<u32, JoinHandle<()>>> {
        self.0.tasks.lock().expect("tails lock")
    }
}
