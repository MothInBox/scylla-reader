use crate::models::{Book, BookStatus, Chapter};
use curl::easy::{Easy, List};
use extism::{CurrentPlugin, Function, Manifest, Plugin, UserData, Val, ValType, Wasm};
use scylla_plugin_api::{ChapterOutput, PluginSchema, ScrapeInput, ScrapeOutput};
use std::sync::Mutex;

pub struct ScraperRegistry {
    plugins: Vec<(String, std::path::PathBuf)>,
    plugin_cache: Mutex<Vec<(String, Plugin)>>,
}

impl Default for ScraperRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ScraperRegistry {
    pub fn new() -> Self {
        let plugin_dir = crate::plugin_config::plugins_dir();

        let mut plugins = Vec::new();

        if let Ok(entries) = std::fs::read_dir(&plugin_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("wasm") {
                    continue;
                }
                if let Some(domain) = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .and_then(|s| s.strip_prefix("plugin-"))
                {
                    crate::settings::log(
                        crate::settings::LogLevel::Debug,
                        "SCRAPE",
                        &format!("Discovered plugin: {} -> {}", domain, path.display()),
                    );

                    let schema = Self::discover_schema(&path);
                    crate::plugin_config::PluginConfig::init_config_file(
                        domain,
                        &schema.fields,
                        schema.accepts_cookies,
                    );

                    plugins.push((domain.to_string(), path));
                }
            }
        } else {
            crate::settings::log(
                crate::settings::LogLevel::Debug,
                "SCRAPE",
                &format!("Plugin dir not found: {}", plugin_dir.display()),
            );
        }

        Self {
            plugins,
            plugin_cache: Mutex::new(Vec::new()),
        }
    }

    fn discover_schema(wasm_path: &std::path::PathBuf) -> PluginSchema {
        let schema_path = wasm_path.with_extension("schema.json");
        let wasm_modified = std::fs::metadata(wasm_path).and_then(|m| m.modified()).ok();
        let schema_modified = std::fs::metadata(&schema_path)
            .and_then(|m| m.modified())
            .ok();

        // Use cache if schema is newer than wasm
        if let (Some(wasm_t), Some(schema_t)) = (wasm_modified, schema_modified)
            && schema_t >= wasm_t
            && let Ok(contents) = std::fs::read_to_string(&schema_path)
            && let Ok(schema) = serde_json::from_str::<PluginSchema>(&contents)
        {
            return schema;
        }

        let curl_fetch_fn = Function::new(
            "curl_fetch",
            [ValType::I64],
            [ValType::I64],
            UserData::<()>::default(),
            host_curl_fetch,
        );
        let wasm = Wasm::file(wasm_path);
        let manifest = Manifest::new([wasm]).with_allowed_host("*");
        let Ok(mut plugin) = Plugin::new(&manifest, [curl_fetch_fn], true) else {
            return PluginSchema {
                fields: vec![],
                accepts_cookies: true,
            };
        };
        let Ok(result) = plugin.call::<&[u8], &[u8]>("get_config_schema", b"null") else {
            return PluginSchema {
                fields: vec![],
                accepts_cookies: true,
            };
        };
        let schema: PluginSchema = serde_json::from_slice(result).unwrap_or(PluginSchema {
            fields: vec![],
            accepts_cookies: true,
        });

        // Cache to disk
        if let Ok(json) = serde_json::to_string(&schema) {
            let _ = std::fs::write(&schema_path, json);
        }

        schema
    }
    fn load_cookies_for_domain(domain: &str) -> Option<String> {
        let path = crate::plugin_config::plugin_config_path(domain);
        let contents = std::fs::read_to_string(&path).ok()?;
        let json: serde_json::Value = serde_json::from_str(&contents).ok()?;
        let raw = json.get("_cookies")?.as_str()?;
        let parsed = crate::plugin_config::parse_cookies_str(raw);
        if parsed.is_empty() {
            crate::settings::log(
                crate::settings::LogLevel::Debug,
                "COOKIE",
                &format!("No cookies for {} (empty after parse)", domain),
            );
            None
        } else {
            crate::settings::log(
                crate::settings::LogLevel::Debug,
                "COOKIE",
                &format!("Loaded {} chars of cookies for {}", parsed.len(), domain),
            );
            Some(parsed)
        }
    }

    fn load_plugin_config(domain: &str) -> Option<String> {
        let path = crate::plugin_config::plugin_config_path(domain);
        let contents = std::fs::read_to_string(&path).ok()?;
        let mut json: serde_json::Value = serde_json::from_str(&contents).ok()?;
        if let Some(obj) = json.as_object_mut() {
            obj.remove("_schema");
            obj.remove("_cookies");
            obj.remove("_accepts_cookies");
        }
        serde_json::to_string(&json).ok()
    }

    pub async fn scrape_url(
        &self,
        url: &str,
    ) -> Result<Book, Box<dyn std::error::Error + Send + Sync>> {
        let (domain, wasm_path) = self.find_plugin(url)?;
        crate::settings::log(
            crate::settings::LogLevel::Debug,
            "SCRAPE",
            &format!("Using plugin '{}' for: {}", domain, url),
        );

        let cookies = Self::load_cookies_for_domain(domain);
        let config = Self::load_plugin_config(domain);

        let input = ScrapeInput {
            url: url.to_string(),
            cookies,
            config,
        };
        let input_json = serde_json::to_vec(&input)?;

        let output_bytes =
            self.call_cached_plugin(domain, wasm_path, "scrape_book", &input_json)?;
        let output: ScrapeOutput = serde_json::from_slice(&output_bytes)?;

        if output.title.is_empty() {
            crate::settings::log(
                crate::settings::LogLevel::Debug,
                "SCRAPE",
                &format!("Plugin returned empty title for: {}", url),
            );
            return Err("scrape failed: plugin returned empty title".into());
        }
        if output.url.is_empty() {
            crate::settings::log(
                crate::settings::LogLevel::Debug,
                "SCRAPE",
                &format!("Plugin returned empty URL for: {}", url),
            );
            return Err("scrape failed: plugin returned empty URL".into());
        }
        if output.chapters.is_empty() {
            crate::settings::log(
                crate::settings::LogLevel::Debug,
                "SCRAPE",
                &format!("Plugin returned 0 chapters for: {}", url),
            );
            return Err("scrape failed: plugin returned 0 chapters".into());
        }

        Ok(Book {
            title: output.title,
            url: output.url.clone(),
            status: BookStatus::Reading,
            sessions: Vec::new(),
            active_session_id: None,
            tags: Vec::new(),
            cover_url: output.cover_url,
            description: output.description,
            chapters: output
                .chapters
                .into_iter()
                .map(|c| Chapter {
                    title: c.title,
                    url: c.url,
                    order: c.order,
                })
                .collect(),
        })
    }

    pub async fn scrape_chapter(
        &self,
        url: &str,
    ) -> Result<(String, String), Box<dyn std::error::Error + Send + Sync>> {
        let (domain, wasm_path) = self.find_plugin(url)?;
        let cookies = Self::load_cookies_for_domain(domain);
        let config = Self::load_plugin_config(domain);
        let input_json = serde_json::to_vec(&ScrapeInput {
            url: url.to_string(),
            cookies,
            config,
        })?;
        let output_bytes =
            self.call_cached_plugin(domain, wasm_path, "scrape_chapter", &input_json)?;
        let output: ChapterOutput = serde_json::from_slice(&output_bytes)?;

        if output.title.is_empty() {
            crate::settings::log(
                crate::settings::LogLevel::Debug,
                "SCRAPE",
                &format!("Plugin returned empty chapter title for: {}", url),
            );
            return Err("scrape failed: plugin returned empty chapter title".into());
        }
        if output.content.is_empty() {
            crate::settings::log(
                crate::settings::LogLevel::Debug,
                "SCRAPE",
                &format!("Plugin returned empty chapter content for: {}", url),
            );
            return Err("scrape failed: plugin returned empty chapter content".into());
        }

        Ok((output.title, output.content))
    }

    fn call_cached_plugin(
        &self,
        domain: &str,
        wasm_path: &std::path::PathBuf,
        function: &str,
        input: &[u8],
    ) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
        crate::settings::log(
            crate::settings::LogLevel::Debug,
            "SCRAPE",
            &format!("call_cached_plugin: domain={} fn={}", domain, function),
        );
        let mut cache = self.plugin_cache.lock().unwrap();
        let idx = cache.iter().position(|(d, _)| d == domain);

        let plugin = if let Some(idx) = idx {
            &mut cache[idx].1
        } else {
            let curl_fetch_fn = Function::new(
                "curl_fetch",
                [ValType::I64],
                [ValType::I64],
                UserData::<()>::default(),
                host_curl_fetch,
            );
            let fail_fn = Function::new(
                "scylla_fail",
                [ValType::I64],
                [],
                UserData::<()>::default(),
                host_scylla_fail,
            );
            let wasm = Wasm::file(wasm_path);
            let manifest = Manifest::new([wasm]).with_allowed_host("*");
            let plugin = Plugin::new(&manifest, [curl_fetch_fn, fail_fn], true)?;
            cache.push((domain.to_string(), plugin));
            &mut cache.last_mut().unwrap().1
        };

        let result = plugin.call::<&[u8], &[u8]>(function, input)?;
        Ok(result.to_vec())
    }

    fn extract_host(url: &str) -> String {
        let after_protocol = url.find("://").map(|i| &url[i + 3..]).unwrap_or(url);
        let host = after_protocol
            .find('/')
            .map(|i| &after_protocol[..i])
            .unwrap_or(after_protocol);
        let host = host.find('?').map(|i| &host[..i]).unwrap_or(host);
        let host = host.find('#').map(|i| &host[..i]).unwrap_or(host);
        // Strip userinfo (user:password@)
        if let Some(at) = host.rfind('@') {
            host[at + 1..].to_string()
        } else {
            host.to_string()
        }
    }

    fn find_plugin(
        &self,
        url: &str,
    ) -> Result<(&str, &std::path::PathBuf), Box<dyn std::error::Error + Send + Sync>> {
        if url.starts_with("template")
            && let Some((domain, path)) = self.plugins.iter().find(|(d, _)| d == "template")
        {
            return Ok((domain.as_str(), path));
        }

        let host = Self::extract_host(url);

        let match_domain = |domain: &str| -> bool {
            let domain_has_dot = domain.contains('.');
            if domain_has_dot {
                host == domain || host.ends_with(&format!(".{}", domain))
            } else {
                let segments: Vec<&str> = host.split('.').collect();
                segments
                    .contains(&domain)
                    && segments.last().is_some_and(|last| last != &domain)
            }
        };

        let mut matched: Vec<&(String, std::path::PathBuf)> =
            self.plugins.iter().filter(|(d, _)| match_domain(d)).collect();
        matched.sort_by_key(|b| std::cmp::Reverse(b.0.len()));

        matched
            .first()
            .map(|(domain, path)| (domain.as_str(), path))
            .ok_or_else(|| {
                format!(
                    "No plugin found for: {}\nInstall a plugin to ~/.config/scylla-reader/plugins/",
                    url
                )
                .into()
            })
    }
}

pub(crate) fn host_curl_fetch(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    outputs: &mut [Val],
    _user_data: UserData<()>,
) -> Result<(), extism::Error> {
    let input_str = plugin.memory_get_val::<String>(&inputs[0])?;

    let mut parts = input_str.splitn(2, '\x00');
    let url = parts.next().unwrap_or("").to_string();
    let cookies = parts.next().unwrap_or("").to_string();

    crate::settings::log(
        crate::settings::LogLevel::Debug,
        "SCRAPE",
        &format!("Fetching: {}", url),
    );

    if !cookies.is_empty() {
        let has_equals = cookies.contains('=');
        let has_cf = cookies.contains("cf_clearance=");
        crate::settings::log(
            crate::settings::LogLevel::Debug,
            "COOKIE",
            &format!(
                "len={} has_=={} has_cf_clearance={}",
                cookies.len(),
                has_equals,
                has_cf,
            ),
        );
    }

    let result = fetch_with_curl(&url, &cookies).unwrap_or_default();

    let mem = plugin.memory_new(result.as_bytes())?;
    outputs[0] = plugin.memory_to_val(mem);
    Ok(())
}

pub(crate) fn host_scylla_fail(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    _outputs: &mut [Val],
    _user_data: UserData<()>,
) -> Result<(), extism::Error> {
    let msg = plugin.memory_get_val::<String>(&inputs[0])?;
    crate::settings::log(
        crate::settings::LogLevel::Debug,
        "PLUGIN",
        &format!("Plugin called fail(): {}", msg),
    );
    Err(extism::Error::msg(msg))
}

fn fetch_with_curl(url: &str, cookie_str: &str) -> Result<String, String> {
    let mut data = Vec::new();
    let mut handle = Easy::new();
    let start = std::time::Instant::now();

    handle.url(url).map_err(|e| e.to_string())?;
    handle
        .timeout(std::time::Duration::from_secs(30))
        .map_err(|e| e.to_string())?;
    handle
        .useragent("Mozilla/5.0 (X11; Linux x86_64; rv:151.0) Gecko/20100101 Firefox/151.0")
        .map_err(|e| e.to_string())?;
    if !cookie_str.is_empty() {
        handle.cookie(cookie_str).map_err(|e| e.to_string())?;
    }

    let mut list = List::new();
    list.append("Accept: text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8")
        .map_err(|e| e.to_string())?;
    handle.http_headers(list).map_err(|e| e.to_string())?;
    handle.follow_location(true).map_err(|e| e.to_string())?;

    {
        let mut transfer = handle.transfer();
        transfer
            .write_function(|new_data| {
                data.extend_from_slice(new_data);
                Ok(new_data.len())
            })
            .map_err(|e| e.to_string())?;
        transfer.perform().map_err(|e| e.to_string())?;
    }

    let elapsed = start.elapsed();
    let status = handle.response_code().unwrap_or(0);
    crate::settings::log(
        crate::settings::LogLevel::Debug,
        "SCRAPE",
        &format!(
            "HTTP {} — {}B — {:?} — {}",
            status,
            data.len(),
            elapsed,
            url,
        ),
    );

    String::from_utf8(data).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_empty_registry_no_plugins() {
        let registry = ScraperRegistry {
            plugins: vec![],
            plugin_cache: Mutex::new(vec![]),
        };
        assert!(registry.plugins.is_empty());
        let result = registry.find_plugin("https://example.com/page");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("No plugin found"));
    }

    #[test]
    fn test_find_plugin_matches_exact_domain() {
        let registry = ScraperRegistry {
            plugins: vec![("example.com".into(), PathBuf::from("/p/example.wasm"))],
            plugin_cache: Mutex::new(vec![]),
        };
        let (domain, path) = registry.find_plugin("https://example.com/page").unwrap();
        assert_eq!(domain, "example.com");
        assert_eq!(path, &PathBuf::from("/p/example.wasm"));
    }

    #[test]
    fn test_find_plugin_matches_subdomain_segment() {
        let registry = ScraperRegistry {
            plugins: vec![("blog".into(), PathBuf::from("/p/blog.wasm"))],
            plugin_cache: Mutex::new(vec![]),
        };
        let (domain, _) = registry
            .find_plugin("https://blog.example.com/post")
            .unwrap();
        assert_eq!(domain, "blog");
    }

    #[test]
    fn test_find_plugin_longest_match_wins() {
        let registry = ScraperRegistry {
            plugins: vec![
                ("com".into(), PathBuf::from("/p/com.wasm")),
                ("example.com".into(), PathBuf::from("/p/example.wasm")),
            ],
            plugin_cache: Mutex::new(vec![]),
        };
        let (domain, _) = registry.find_plugin("https://example.com/page").unwrap();
        assert_eq!(domain, "example.com");
    }

    #[test]
    fn test_find_plugin_tld_does_not_match() {
        let registry = ScraperRegistry {
            plugins: vec![("com".into(), PathBuf::from("/p/com.wasm"))],
            plugin_cache: Mutex::new(vec![]),
        };
        let err = registry.find_plugin("https://example.com/page").unwrap_err();
        assert!(err.to_string().contains("No plugin found for:"));
    }

    #[test]
    fn test_find_plugin_no_match() {
        let registry = ScraperRegistry {
            plugins: vec![("example.com".into(), PathBuf::from("/p/example.wasm"))],
            plugin_cache: Mutex::new(vec![]),
        };
        let err = registry.find_plugin("https://other.com/page").unwrap_err();
        assert!(err.to_string().contains("No plugin found for:"));
    }

    #[test]
    fn test_find_plugin_template_without_template_plugin() {
        let registry = ScraperRegistry {
            plugins: vec![("other".into(), PathBuf::from("/p/other.wasm"))],
            plugin_cache: Mutex::new(vec![]),
        };
        let result = registry.find_plugin("template://something");
        assert!(result.is_err());
    }

    #[test]
    fn test_find_plugin_template_with_template_plugin() {
        let registry = ScraperRegistry {
            plugins: vec![
                ("template".into(), PathBuf::from("/p/template.wasm")),
                ("other".into(), PathBuf::from("/p/other.wasm")),
            ],
            plugin_cache: Mutex::new(vec![]),
        };
        let (domain, path) = registry.find_plugin("template://something").unwrap();
        assert_eq!(domain, "template");
        assert_eq!(path, &PathBuf::from("/p/template.wasm"));
    }

    #[test]
    fn test_find_plugin_matches_segment_in_host() {
        let registry = ScraperRegistry {
            plugins: vec![("royalroad".into(), PathBuf::from("/p/rr.wasm"))],
            plugin_cache: Mutex::new(vec![]),
        };
        let (domain, _) = registry
            .find_plugin("https://www.royalroad.com/fiction/123")
            .unwrap();
        assert_eq!(domain, "royalroad");
    }

    #[test]
    fn test_find_plugin_case_sensitive_matching() {
        let registry = ScraperRegistry {
            plugins: vec![("Example".into(), PathBuf::from("/p/example.wasm"))],
            plugin_cache: Mutex::new(vec![]),
        };
        let (domain, _) = registry.find_plugin("https://Example.com/page").unwrap();
        assert_eq!(domain, "Example");
    }

    #[test]
    fn test_find_plugin_case_sensitive_no_match() {
        let registry = ScraperRegistry {
            plugins: vec![("Example".into(), PathBuf::from("/p/example.wasm"))],
            plugin_cache: Mutex::new(vec![]),
        };
        let result = registry.find_plugin("https://example.com/page");
        assert!(result.is_err());
    }

    #[test]
    fn test_plugin_list_contains_no_wasm() {
        let registry = ScraperRegistry {
            plugins: vec![("plugin".into(), PathBuf::from("/p/plugin.wasm"))],
            plugin_cache: Mutex::new(vec![]),
        };
        assert_eq!(registry.plugins.len(), 1);
        assert_eq!(registry.plugins[0].0, "plugin");
    }
}
