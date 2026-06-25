use crate::models::{Book, BookStatus, Chapter, Progress};
use curl::easy::{Easy, List};
use extism::{CurrentPlugin, Function, Manifest, Plugin, UserData, Val, ValType, Wasm};
use scylla_plugin_api::{ChapterOutput, ScrapeInput, ScrapeOutput};

pub struct ScraperRegistry {
    plugins: Vec<(String, std::path::PathBuf)>, // (domain, wasm_path)
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
                        crate::settings::log_debug(&format!(
                            "Discovered plugin: {} -> {}",
                            domain,
                            path.display()
                        ));

                        // Check/create cookie file before moving path
                        let store = crate::cookie_store::CookieStore::for_domain(domain);
                        let cookie_path = store.path();
                        if !cookie_path.exists() {
                            std::fs::create_dir_all(cookie_path.parent().unwrap()).ok();
                            std::fs::write(cookie_path, "# Paste cookies for this domain here\n")
                                .ok();
                        }

                        plugins.push((domain.to_string(), path));
                    }
                }
            }
        } else {
            crate::settings::log_debug(&format!("Plugin dir not found: {}", plugin_dir.display()));
        }

        Self { plugins }
    }

    pub async fn scrape_url(
        &self,
        url: &str,
    ) -> Result<Book, Box<dyn std::error::Error + Send + Sync>> {
        let (domain, wasm_path) = self.find_plugin(url)?;
        crate::settings::log_debug(&format!("Using plugin '{}' for: {}", domain, url));

        let cookies = crate::cookie_store::CookieStore::for_domain(domain)
            .load()
            .ok();

        let input = ScrapeInput {
            url: url.to_string(),
            cookies,
        };
        let input_json = serde_json::to_vec(&input)?;

        let output_bytes = call_plugin(wasm_path, "scrape_book", &input_json)?;
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
        let cookies = crate::cookie_store::CookieStore::for_domain(domain)
            .load()
            .ok();
        let input_json = serde_json::to_vec(&ScrapeInput {
            url: url.to_string(),
            cookies,
        })?;
        let output_bytes = call_plugin(wasm_path, "scrape_chapter", &input_json)?;
        let output: ChapterOutput = serde_json::from_slice(&output_bytes)?;
        Ok((output.title, output.content))
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

// ── curl host function ────────────────────────────────────────────────────────

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

    let result = fetch_with_curl(&url, &cookies).unwrap_or_default();

    let mem = plugin.memory_new(result.as_bytes())?;
    outputs[0] = plugin.memory_to_val(mem);
    Ok(())
}

fn fetch_with_curl(url: &str, cookie_str: &str) -> Result<String, String> {
    let mut data = Vec::new();
    let mut handle = Easy::new();

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
    list.append("Referer: https://www.scribblehub.com/")
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

    String::from_utf8(data).map_err(|e| e.to_string())
}

// ── Plugin runner ─────────────────────────────────────────────────────────────

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

    crate::settings::log_debug(&format!(
        "[{}::{}] returned {} bytes: {}",
        plugin_name,
        function,
        bytes.len(),
        &String::from_utf8_lossy(&bytes)
            .chars()
            .take(300)
            .collect::<String>()
    ));

    Ok(bytes)
}
