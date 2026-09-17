use scylla_core::types::*;

pub struct LibraryManager {
    pub backends: Vec<Box<dyn StorageBackend>>,
    pub active_filter: LibraryFilter,
}

impl LibraryManager {
    pub fn new(backends: Vec<Box<dyn StorageBackend>>) -> Self {
        Self {
            backends,
            active_filter: LibraryFilter::All,
        }
    }

    pub fn backend_names(&self) -> Vec<String> {
        self.backends.iter().map(|b| b.name().to_string()).collect()
    }

    pub fn get_backend(&self, name: &str) -> Option<&dyn StorageBackend> {
        self.backends
            .iter()
            .find(|b| b.name() == name)
            .map(|b| b.as_ref())
    }

    pub fn active_backend(&self) -> Option<&dyn StorageBackend> {
        match &self.active_filter {
            LibraryFilter::Backend(name) => self.get_backend(name),
            _ => None,
        }
    }

    /// The backend used for persistence: the explicitly active one if set,
    /// otherwise the first configured backend.
    pub fn primary_backend(&self) -> Option<&dyn StorageBackend> {
        self.active_backend()
            .or_else(|| self.backends.first().map(|b| b.as_ref()))
    }

    /// Set the active backend by name, or clear it (back to `All`) when `None`.
    pub fn set_active_backend(&mut self, name: Option<String>) {
        self.active_filter = match name {
            Some(name) => LibraryFilter::Backend(name),
            None => LibraryFilter::All,
        };
    }
}
