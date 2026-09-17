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

    pub fn cycle_filter_to_next_backend(&mut self) {
        let names = self.backend_names();
        let next = match &self.active_filter {
            LibraryFilter::All => names.first().cloned().map(LibraryFilter::Backend),
            LibraryFilter::Backend(current) => {
                if let Some(pos) = names.iter().position(|n| n == current) {
                    if pos + 1 < names.len() {
                        Some(LibraryFilter::Backend(names[pos + 1].clone()))
                    } else {
                        Some(LibraryFilter::All)
                    }
                } else {
                    names.first().cloned().map(LibraryFilter::Backend)
                }
            }
            other => names
                .first()
                .cloned()
                .map(LibraryFilter::Backend)
                .or_else(|| Some(other.clone())),
        };
        if let Some(f) = next {
            self.active_filter = f;
        }
    }
}
