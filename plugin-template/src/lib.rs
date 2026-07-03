use extism_pdk::*;
use scylla_plugin_api::{ChapterOutput, ConfigField, PluginChapter, PluginSchema, ScrapeInput, ScrapeOutput};

#[link(wasm_import_module = "wasi_snapshot_preview1")]
extern "C" {
    fn clock_time_get(clock_id: u32, precision: u64, time: *mut u64) -> u32;
}

fn get_wasi_nanoseconds() -> u64 {
    let mut time: u64 = 0;
    unsafe {
        if clock_time_get(0, 1, &mut time) == 0 {
            time
        } else {
            1337
        }
    }
}

struct SimpleRng {
    state: u64,
}

impl SimpleRng {
    fn new(seed: u64) -> Self {
        Self {
            state: if seed == 0 { 42 } else { seed },
        }
    }

    fn next_u32(&mut self) -> u32 {
        let old_state = self.state;
        self.state = old_state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let xorshifted = (((old_state >> 18) ^ old_state) >> 27) as u32;
        let rot = (old_state >> 59) as u32;
        (xorshifted >> rot) | (xorshifted << ((!rot).wrapping_add(1) & 31))
    }

    fn choose<'a>(&mut self, words: &'a [&'a str]) -> &'a str {
        let idx = (self.next_u32() as usize) % words.len();
        words[idx]
    }
}

const LOREM_WORDS: &[&str] = &[
    "lorem",
    "ipsum",
    "dolor",
    "sit",
    "amet",
    "consectetur",
    "adipiscing",
    "elit",
    "sed",
    "do",
    "eiusmod",
    "tempor",
    "incididunt",
    "ut",
    "labore",
    "et",
    "dolore",
    "magna",
    "aliqua",
    "ut",
    "enim",
    "ad",
    "minim",
    "veniam",
    "quis",
    "nostrud",
    "exercitation",
    "ullamco",
    "laboris",
    "nisi",
    "ut",
    "aliquip",
    "ex",
    "ea",
    "commodo",
    "consequat",
    "duis",
    "aute",
    "irure",
    "dolor",
    "in",
    "reprehenderit",
    "in",
    "voluptate",
    "velit",
    "esse",
    "cillum",
    "dolore",
    "eu",
    "fugiat",
    "nulla",
    "pariatur",
    "excepteur",
    "sint",
    "occaecat",
    "cupidatat",
    "non",
    "proident",
    "sunt",
    "in",
    "culpa",
    "qui",
    "officia",
    "deserunt",
    "mollit",
    "anim",
    "id",
    "est",
    "laborum",
];

fn generate_sentence(rng: &mut SimpleRng) -> String {
    let word_count = 5 + (rng.next_u32() % 10) as usize;
    let mut sentence = Vec::new();

    for i in 0..word_count {
        let mut word = rng.choose(LOREM_WORDS).to_string();
        if i == 0 {
            let mut chars = word.chars();
            if let Some(first) = chars.next() {
                word = first.to_uppercase().collect::<String>() + chars.as_str();
            }
        }
        sentence.push(word);
    }
    sentence.join(" ") + "."
}

fn generate_paragraphs(rng: &mut SimpleRng, paragraph_count: usize) -> String {
    let mut paragraphs = Vec::new();
    for _ in 0..paragraph_count {
        let sentence_count = 3 + (rng.next_u32() % 4) as usize;
        let mut sentences = Vec::new();
        for _ in 0..sentence_count {
            sentences.push(generate_sentence(rng));
        }
        paragraphs.push(sentences.join(" "));
    }
    paragraphs.join("\n\n")
}

fn seed_from_str(s: &str) -> u64 {
    let mut hash: u64 = 5381;
    for byte in s.bytes() {
        hash = (hash.wrapping_shl(5))
            .wrapping_add(hash)
            .wrapping_add(byte as u64);
    }
    hash
}

fn config_value(config: &Option<String>, key: &str, default: &str) -> String {
    config
        .as_deref()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
        .and_then(|v| v.get(key).and_then(|v| v.as_str().map(String::from)))
        .unwrap_or_else(|| default.to_string())
}

// ENTRY POINTS. Here is where data is passed from the plugin to scylla-reader, everything above is
// can be done however you want to do it, as long as these two plugin_fn exist and return the data
// as you intend and as specified.

#[plugin_fn]
pub fn get_config_schema(Json(()): Json<()>) -> FnResult<Json<PluginSchema>> {
    Ok(Json(PluginSchema {
        fields: vec![
            ConfigField {
                key: "chapter_count".into(),
                label: "Chapter count".into(),
                field_type: "number".into(),
                default: "5".into(),
            },
            ConfigField {
                key: "cover_source".into(),
                label: "Cover image URL".into(),
                field_type: "string".into(),
                default: "https://picsum.photos/400/600".into(),
            },
        ],
        accepts_cookies: false,
    }))
}

// Called when the book is initially added to the library (i : template : ctrl + s)
#[plugin_fn]
pub fn scrape_book(Json(input): Json<ScrapeInput>) -> FnResult<Json<ScrapeOutput>> {
    let title = "Inifine Scroll (Template)".to_string();
    let cover_url = Some(config_value(&input.config, "cover_source", "https://picsum.photos/400/600"));
    let description = Some(
        "An infinite scroll containing choas (this is the template plugin included with scylla-reader)."
            .to_string(),
    );

    let count: usize = config_value(&input.config, "chapter_count", "5")
        .parse()
        .unwrap_or(5);
    let mut chapters = Vec::new();
    for i in 1..=count {
        chapters.push(PluginChapter {
            title: format!("Chapter {}: Mutable Horizons", i),
            url: format!("{}#chapter-{}", input.url, i),
            order: i as u32,
        });
    }

    Ok(Json(ScrapeOutput {
        title,
        url: input.url,
        cover_url,
        description,
        total_chapters: chapters.len() as u32,
        chapters,
    }))
}

//Called when the reader is trying to access a new chapter and needs its content. Happens on initial
//Enter on selected book in library or when accessing < > chapter. This one in particular returns
//something new every time it is called (random)
#[plugin_fn]
pub fn scrape_chapter(Json(input): Json<ScrapeInput>) -> FnResult<Json<ChapterOutput>> {
    let base_seed = seed_from_str(&input.url);
    let live_timestamp = get_wasi_nanoseconds();

    let dynamic_seed = base_seed.wrapping_add(live_timestamp);
    let mut rng = SimpleRng::new(dynamic_seed);

    let chapter_num = input.url.split("#chapter-").last().unwrap_or("1");
    let title = format!("Chapter {}: Mutable Horizons", chapter_num);
    let paragraph_count = 4 + (rng.next_u32() % 5) as usize;
    let mut content = format!(
        "[Generated live at host runtime timestamp: {} ns]\n\n",
        live_timestamp
    );
    content.push_str(&generate_paragraphs(&mut rng, paragraph_count));

    Ok(Json(ChapterOutput { title, content }))
}
