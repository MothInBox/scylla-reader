use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct ConfigField {
    pub key: String,
    pub label: String,
    pub field_type: String,
    pub default: String,
}

#[derive(Serialize, Deserialize)]
pub struct PluginSchema {
    pub fields: Vec<ConfigField>,
    pub accepts_cookies: bool,
}

#[derive(Serialize, Deserialize)]
pub struct ScrapeInput {
    pub url: String,
    pub cookies: Option<String>,
    pub config: Option<String>,
}


#[derive(Serialize, Deserialize)]
pub struct ScrapeOutput {
    pub title: String,
    pub url: String,
    pub cover_url: Option<String>,
    pub description: Option<String>,
    pub total_chapters: u32,
    pub chapters: Vec<PluginChapter>,
}


#[derive(Serialize, Deserialize)]
pub struct ChapterOutput {
    pub title: String,
    pub content: String,
}

#[derive(Serialize, Deserialize)]
pub struct PluginChapter {
    pub title: String,
    pub url: String,
    pub order: u32,
}


