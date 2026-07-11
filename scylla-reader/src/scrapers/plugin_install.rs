use crate::scrapers::services::{host_curl_fetch, host_scylla_fail};
use extism::{Function, Manifest, Plugin, UserData, ValType, Wasm};
use scylla_plugin_api::PluginSchema;

pub fn install_plugin(repo_url: &str) -> Result<(String, String), String> {
    let (owner, repo) = parse_github_url(repo_url)?;

    let release_url = format!(
        "https://api.github.com/repos/{}/{}/releases/latest",
        owner, repo
    );
    let client = reqwest::blocking::Client::builder()
        .user_agent("scylla-reader/0.2")
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?;

    let resp = client
        .get(&release_url)
        .header("Accept", "application/json")
        .send()
        .map_err(|e| format!("Failed to fetch release: {}", e))?
        .error_for_status()
        .map_err(|e| format!("GitHub API error: {}", e))?;

    let release: serde_json::Value = resp
        .json()
        .map_err(|e| format!("Failed to parse release: {}", e))?;

    let assets = release["assets"]
        .as_array()
        .ok_or_else(|| "No assets found in release".to_string())?;

    let mut installed = Vec::new();
    for asset in assets {
        let name = asset["name"]
            .as_str()
            .ok_or_else(|| "Invalid asset name".to_string())?;
        if !name.ends_with(".wasm") {
            continue;
        }
        let download_url = asset["browser_download_url"]
            .as_str()
            .ok_or_else(|| "Missing download URL".to_string())?;

        let stem = name.strip_suffix(".wasm").unwrap_or(name);
        let domain = stem.strip_prefix("plugin-").unwrap_or(stem);

        let wasm_data = client
            .get(download_url)
            .send()
            .map_err(|e| format!("Failed to download {}: {}", name, e))?
            .bytes()
            .map_err(|e| format!("Failed to read {}: {}", name, e))?;

        let plugin_dir = crate::plugin_config::plugins_dir();
        std::fs::create_dir_all(&plugin_dir).map_err(|e| e.to_string())?;

        let wasm_path = plugin_dir.join(name);
        std::fs::write(&wasm_path, &wasm_data).map_err(|e| e.to_string())?;

        let schema = discover_schema_from_bytes(&wasm_data, &wasm_path);
        crate::plugin_config::PluginConfig::init_config_file(domain, &schema.fields, schema.accepts_cookies);

        crate::settings::log(
            crate::settings::LogLevel::Debug,
            "PLUGIN",
            &format!("Installed plugin: {} from {}", domain, name),
        );

        installed.push((domain.to_string(), wasm_path.to_string_lossy().to_string()));
    }

    if installed.is_empty() {
        return Err("No .wasm files found in release".to_string());
    }

    Ok(installed[0].clone())
}

fn parse_github_url(url: &str) -> Result<(String, String), String> {
    let url = url.trim().trim_end_matches(".git").trim_end_matches('/');
    let path = if let Some(rest) = url.strip_prefix("https://github.com/") {
        rest
    } else if let Some(rest) = url.strip_prefix("git@github.com:") {
        rest
    } else if let Some(rest) = url.strip_prefix("github.com/") {
        rest
    } else {
        url
    };

    let parts: Vec<&str> = path.split('/').collect();
    if parts.len() < 2 || parts[0].is_empty() || parts[1].is_empty() {
        return Err(format!(
            "Invalid GitHub URL: {}. Expected https://github.com/owner/repo",
            url
        ));
    }
    Ok((parts[0].to_string(), parts[1].to_string()))
}

fn discover_schema_from_bytes(wasm_data: &[u8], wasm_path: &std::path::Path) -> PluginSchema {
    let schema_path = wasm_path.with_extension("schema.json");
    if let Ok(contents) = std::fs::read_to_string(&schema_path)
        && let Ok(schema) = serde_json::from_str(&contents)
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
    let fail_fn = Function::new(
        "scylla_fail",
        [ValType::I64],
        [],
        UserData::<()>::default(),
        host_scylla_fail,
    );
    let wasm = Wasm::data(wasm_data.to_vec());
    let manifest = Manifest::new([wasm]).with_allowed_host("*");
    let Ok(mut plugin) = Plugin::new(&manifest, [curl_fetch_fn, fail_fn], true) else {
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

    if let Ok(json) = serde_json::to_string(&schema) {
        let _ = std::fs::write(&schema_path, json);
    }

    schema
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_github_url_https() {
        let (owner, repo) = parse_github_url("https://github.com/owner/repo").unwrap();
        assert_eq!(owner, "owner");
        assert_eq!(repo, "repo");
    }

    #[test]
    fn test_parse_github_url_with_trailing_slash() {
        let (owner, repo) = parse_github_url("https://github.com/owner/repo/").unwrap();
        assert_eq!(owner, "owner");
        assert_eq!(repo, "repo");
    }

    #[test]
    fn test_parse_github_url_with_dot_git() {
        let (owner, repo) = parse_github_url("https://github.com/owner/repo.git").unwrap();
        assert_eq!(owner, "owner");
        assert_eq!(repo, "repo");
    }

    #[test]
    fn test_parse_github_url_ssh() {
        let (owner, repo) = parse_github_url("git@github.com:owner/repo").unwrap();
        assert_eq!(owner, "owner");
        assert_eq!(repo, "repo");
    }

    #[test]
    fn test_parse_github_url_shorthand() {
        let (owner, repo) = parse_github_url("owner/repo").unwrap();
        assert_eq!(owner, "owner");
        assert_eq!(repo, "repo");
    }

    #[test]
    fn test_parse_github_url_invalid() {
        let result = parse_github_url("not-a-url");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_github_url_empty() {
        let result = parse_github_url("");
        assert!(result.is_err());
    }
}
