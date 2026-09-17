#[derive(Clone)]
pub struct ServerSettingsState {
    pub connected: bool,
    pub port: u16,
    pub max_workers: u8,
    pub rate_limit: u64,
    pub plugins: Vec<String>,
    pub loading: bool,
    pub error: Option<String>,
}

impl ServerSettingsState {
    pub fn new() -> Self {
        Self {
            connected: false,
            port: 8080,
            max_workers: 4,
            rate_limit: 2,
            plugins: vec![],
            loading: false,
            error: None,
        }
    }

    pub async fn refresh_from_server(&mut self, backend: &dyn scylla_core::types::StorageBackend) {
        self.loading = true;
        match backend.get_server_settings().await {
            Ok(v) => {
                self.port = v.get("port").and_then(|x| x.as_u64()).unwrap_or(8080) as u16;
                self.max_workers = v.get("max_workers").and_then(|x| x.as_u64()).unwrap_or(4) as u8;
                self.rate_limit = v.get("rate_limit").and_then(|x| x.as_u64()).unwrap_or(2);
                self.plugins = v
                    .get("plugins")
                    .and_then(|x| x.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|p| p.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default();
                self.error = None;
                self.connected = true;
            }
            Err(e) => {
                self.error = Some(e);
                self.connected = false;
            }
        }
        self.loading = false;
    }
}

impl Default for ServerSettingsState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::*;

    #[test]
    fn test_refresh_from_server_sets_port_and_connected() {
        let mock = MockBackend::new("mock");
        *mock.server_settings.lock().unwrap() = Some(serde_json::json!({
            "port": 9090,
            "max_workers": 8,
            "rate_limit": 5,
            "plugins": ["p1"],
        }));
        let mut state = ServerSettingsState::new();
        crate::storage::client::block_on(state.refresh_from_server(&mock));
        assert!(state.connected);
        assert_eq!(state.port, 9090);
        assert_eq!(state.max_workers, 8);
        assert_eq!(state.rate_limit, 5);
        assert_eq!(state.plugins, vec!["p1"]);
        assert!(state.error.is_none());
    }

    #[test]
    fn test_refresh_from_server_error_sets_connected_false() {
        let mock = MockBackend::new("mock");
        let mut state = ServerSettingsState::new();
        crate::storage::client::block_on(state.refresh_from_server(&mock));
        assert!(!state.connected);
        assert!(state.error.is_some());
    }
}
