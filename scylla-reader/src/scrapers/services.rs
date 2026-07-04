use crate::models::{Book, BookStatus, Chapter, Progress};
use curl::easy::{Easy, List};
use extism::{CurrentPlugin, Function, Manifest, Plugin, UserData, Val, ValType, Wasm};
use scylla_plugin_api::{ChapterOutput, PluginSchema, ScrapeInput, ScrapeOutput};
use std::cell::RefCell;

pub struct ScraperRegistry {
    plugins: Vec<(String, std::path::PathBuf)>,
    plugin_cache: RefCell<Vec<(String, Plugin)>>,
}

impl ScraperRegistry {
    pub fn new() -> Self {
        let plugin_dir = dirs::config_local_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("scylla-reader")
            .join("plugins");

        let mut plugins = Vec::new();

        if let Ok(entries) = std::fs::read_dir(&plugin_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("wasm") {
                    continue;
                }
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    if let Some(domain) = stem.strip_prefix("plugin-") {
                        crate::settings::log(crate::settings::LogLevel::Debug, "SCRAPE", &format!(
                            "Discovered plugin: {} -> {}",
                            domain,
                            path.display()
                        ));

                        let schema = Self::discover_schema(&path);
                        crate::plugin_config::PluginConfig::init_config_file(domain, &schema.fields, schema.accepts_cookies);

                        plugins.push((domain.to_string(), path));
                    }
                }
            }
        } else {
            crate::settings::log(crate::settings::LogLevel::Debug, "SCRAPE", &format!("Plugin dir not found: {}", plugin_dir.display()));
        }

        Self { plugins, plugin_cache: RefCell::new(Vec::new()) }
    }

    fn discover_schema(wasm_path: &std::path::PathBuf) -> PluginSchema {
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
            return PluginSchema { fields: vec![], accepts_cookies: true };
        };
        let Ok(result) = plugin.call::<&[u8], &[u8]>("get_config_schema", b"null") else {
            return PluginSchema { fields: vec![], accepts_cookies: true };
        };
        serde_json::from_slice(&result).unwrap_or(PluginSchema { fields: vec![], accepts_cookies: true })
    }

    fn load_cookies_for_domain(domain: &str) -> Option<String> {
        let path = crate::plugin_config::config_dir().join(format!("{}.json", domain));
        let contents = std::fs::read_to_string(&path).ok()?;
        let json: serde_json::Value = serde_json::from_str(&contents).ok()?;
        let raw = json.get("_cookies")?.as_str()?;
        let parsed: String = raw
            .lines()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .collect::<Vec<_>>()
            .join("; ");
        if parsed.is_empty() {
            crate::settings::log(crate::settings::LogLevel::Debug, "COOKIE", &format!("No cookies for {} (empty after parse)", domain));
            None
        } else {
            crate::settings::log(crate::settings::LogLevel::Debug, "COOKIE", &format!("Loaded {} chars of cookies for {}", parsed.len(), domain));
            Some(parsed)
        }
    }

    fn load_plugin_config(domain: &str) -> Option<String> {
        let path = crate::plugin_config::config_dir().join(format!("{}.json", domain));
        let contents = std::fs::read_to_string(&path).ok()?;
        let mut json: serde_json::Value = serde_json::from_str(&contents).ok()?;
        if let Some(obj) = json.as_object_mut() {
            obj.remove("_schema");
            obj.remove("_cookies");
            obj.remove("_accepts_cookies");
        }
        Some(json.to_string())
    }

    pub async fn scrape_url(
        &self,
        url: &str,
    ) -> Result<Book, Box<dyn std::error::Error + Send + Sync>> {
        let (domain, wasm_path) = self.find_plugin(url)?;
        crate::settings::log(crate::settings::LogLevel::Debug, "SCRAPE", &format!("Using plugin '{}' for: {}", domain, url));

        let cookies = Self::load_cookies_for_domain(domain);
        let config = Self::load_plugin_config(domain);

        let input = ScrapeInput {
            url: url.to_string(),
            cookies,
            config,
        };
        let input_json = serde_json::to_vec(&input)?;

        let output_bytes = self.call_cached_plugin(domain, wasm_path, "scrape_book", &input_json)?;
        let output: ScrapeOutput = serde_json::from_slice(&output_bytes)?;

        Ok(Book {
            title: output.title,
            url: output.url,
            status: BookStatus::Reading,
            progress: Progress {
                current: 0,
                total: output.total_chapters,
            },
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
        let output_bytes = self.call_cached_plugin(domain, wasm_path, "scrape_chapter", &input_json)?;
        let output: ChapterOutput = serde_json::from_slice(&output_bytes)?;
        Ok((output.title, output.content))
    }

    fn call_cached_plugin(
        &self,
        domain: &str,
        wasm_path: &std::path::PathBuf,
        function: &str,
        input: &[u8],
    ) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
        let mut cache = self.plugin_cache.borrow_mut();
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
            let wasm = Wasm::file(wasm_path);
            let manifest = Manifest::new([wasm]).with_allowed_host("*");
            let plugin = Plugin::new(&manifest, [curl_fetch_fn], true)?;
            cache.push((domain.to_string(), plugin));
            &mut cache.last_mut().unwrap().1
        };

        let result = plugin.call::<&[u8], &[u8]>(function, input)?;
        Ok(result.to_vec())
    }

    fn find_plugin(
        &self,
        url: &str,
    ) -> Result<(&str, &std::path::PathBuf), Box<dyn std::error::Error + Send + Sync>> {
        if url.starts_with("template") {
            if let Some((domain, path)) = self.plugins.iter().find(|(d, _)| d == "template") {
                return Ok((domain.as_str(), path));
            }
        }

        self.plugins
            .iter()
            .find(|(domain, _)| url.contains(domain.as_str()))
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

fn host_curl_fetch(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    outputs: &mut [Val],
    _user_data: UserData<()>,
) -> Result<(), extism::Error> {
    let input_str = plugin.memory_get_val::<String>(&inputs[0])?;

    let mut parts = input_str.splitn(2, '\x00');
    let url = parts.next().unwrap_or("").to_string();
    let cookies = parts.next().unwrap_or("").to_string();

    crate::settings::log(crate::settings::LogLevel::Debug, "SCRAPE", &format!("Fetching: {}", url));

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

fn fetch_with_curl(url: &str, cookie_str: &str) -> Result<String, String> {
    let mut data = Vec::new();
    let mut handle = Easy::new();
    let start = std::time::Instant::now();

    handle.url(url).map_err(|e| e.to_string())?;
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
    crate::settings::log(crate::settings::LogLevel::Debug, "SCRAPE", &format!(
        "HTTP {} — {}B — {:?} — {}",
        status,
        data.len(),
        elapsed,
        url,
    ));

    String::from_utf8(data).map_err(|e| e.to_string())
}

#[allow(dead_code)]
fn call_plugin(
    wasm_path: &std::path::PathBuf,
    function: &str,
    input: &[u8],
) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
    let plugin_name = wasm_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown");

    let curl_fetch_fn = Function::new(
        "curl_fetch",
        [ValType::I64],
        [ValType::I64],
        UserData::<()>::default(),
        host_curl_fetch,
    );

    let wasm = Wasm::file(wasm_path);
    let manifest = Manifest::new([wasm]).with_allowed_host("*");
    let mut plugin = Plugin::new(&manifest, [curl_fetch_fn], true)?;

    let result = plugin.call::<&[u8], &[u8]>(function, input)?;
    let bytes = result.to_vec();

    crate::settings::log(crate::settings::LogLevel::Debug, "PLUGIN", &format!(
        "[{}::{}] input={}B output={}B: {}",
        plugin_name,
        function,
        input.len(),
        bytes.len(),
        &String::from_utf8_lossy(&bytes)
            .chars()
            .take(300)
            .collect::<String>()
    ));

    Ok(bytes)
}
