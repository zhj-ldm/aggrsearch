//! 本地 embedding 重排（对齐 SearchPIN 的 `_embedding_rerank`）
//!
//! 默认模型：EmbeddingGemma-300M（ONNX int8，约 309MB），首次使用自动下载并缓存。
//! 文档表示与 SearchPIN 一致：`title + " " + snippet`，query 与文档一起 embed，
//! 用余弦相似度作为 `_rerank_score`。模型不可用时由调用方回退到词法重排。

use fastembed::{EmbeddingModel, TextEmbedding, TextInitOptions};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

/// EmbeddingGemma 官方检索提示模板（只对 Gemma 系模型加，避免污染其他模型）
const GEMMA_QUERY_PREFIX: &str = "task: search result | query: ";
const GEMMA_DOC_PREFIX: &str = "title: ";
const GEMMA_DOC_SEP: &str = " | text: ";

struct ModelSpec {
    model: EmbeddingModel,
    name: &'static str,
    prompt: bool,
    max_length: usize,
}

struct Loaded {
    model: Mutex<TextEmbedding>,
    name: &'static str,
    prompt: bool,
}

static LOADED: OnceLock<Result<Loaded, String>> = OnceLock::new();

/// embedding 重排是否启用（`AGGRSEARCH_EMBED=off|0|false|none` 可关闭）
pub fn enabled() -> bool {
    !matches!(
        std::env::var("AGGRSEARCH_EMBED")
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase()
            .as_str(),
        "off" | "0" | "false" | "none" | "no"
    )
}

fn model_spec() -> Result<ModelSpec, String> {
    let raw = std::env::var("AGGRSEARCH_EMBED_MODEL").unwrap_or_default();
    let key = raw.trim().to_ascii_lowercase();
    match key.as_str() {
        "" | "gemma" | "gemma-int8" | "embeddinggemma-300m" | "embeddinggemma300m" => {
            Ok(ModelSpec {
                model: EmbeddingModel::EmbeddingGemma300MQ,
                name: "embeddinggemma-300m-int8",
                prompt: true,
                max_length: 512,
            })
        }
        "gemma-q4" | "embeddinggemma-300m-q4" | "embeddinggemma300mq4" => Ok(ModelSpec {
            model: EmbeddingModel::EmbeddingGemma300MQ4,
            name: "embeddinggemma-300m-q4",
            prompt: true,
            max_length: 512,
        }),
        // 仅用于测试/CI：体积很小，验证 embedding 链路
        "minilm" | "all-minilm-l6-v2" | "allminilml6v2" => Ok(ModelSpec {
            model: EmbeddingModel::AllMiniLML6V2Q,
            name: "all-minilm-l6-v2-q",
            prompt: false,
            max_length: 512,
        }),
        other => Err(format!(
            "未知 AGGRSEARCH_EMBED_MODEL: {other}（可选: gemma、gemma-q4、minilm）"
        )),
    }
}

fn cache_dir() -> PathBuf {
    if let Ok(p) = std::env::var("AGGRSEARCH_EMBED_CACHE") {
        if !p.trim().is_empty() {
            return PathBuf::from(p);
        }
    }

    #[cfg(windows)]
    {
        if let Ok(local) = std::env::var("LOCALAPPDATA") {
            return PathBuf::from(local).join("aggrsearch").join("models");
        }
    }
    #[cfg(target_os = "macos")]
    {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home)
                .join("Library")
                .join("Caches")
                .join("aggrsearch")
                .join("models");
        }
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        if let Ok(xdg) = std::env::var("XDG_CACHE_HOME") {
            return PathBuf::from(xdg).join("aggrsearch").join("models");
        }
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home)
                .join(".cache")
                .join("aggrsearch")
                .join("models");
        }
    }
    PathBuf::from(".aggrsearch-models")
}

fn intra_threads() -> Option<usize> {
    std::env::var("AGGRSEARCH_EMBED_THREADS")
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .filter(|n| *n > 0)
}

fn load() -> Result<&'static Loaded, String> {
    let result = LOADED.get_or_init(|| {
        let spec = model_spec()?;
        let mut options = TextInitOptions::new(spec.model)
            .with_cache_dir(cache_dir())
            .with_show_download_progress(true)
            .with_max_length(spec.max_length);
        if let Some(threads) = intra_threads() {
            options = options.with_intra_threads(threads);
        }
        let model = TextEmbedding::try_new(options)
            .map_err(|e| format!("加载 embedding 模型 {} 失败: {e}", spec.name))?;
        Ok(Loaded {
            model: Mutex::new(model),
            name: spec.name,
            prompt: spec.prompt,
        })
    });
    result.as_ref().map_err(|e| e.clone())
}

/// 启动预热：把模型加载/下载放到服务开始前，失败只告警不退出
pub fn warmup() {
    if !enabled() {
        eprintln!("[embedding] 已通过 AGGRSEARCH_EMBED 关闭，使用词法重排");
        return;
    }
    let started = std::time::Instant::now();
    match load() {
        Ok(loaded) => eprintln!(
            "[embedding] 模型就绪: {} ({:.1}s)",
            loaded.name,
            started.elapsed().as_secs_f64()
        ),
        Err(e) => eprintln!("[embedding] {e}；本次将回退词法重排"),
    }
}

fn query_text(prompt: bool, query: &str) -> String {
    if prompt {
        format!("{GEMMA_QUERY_PREFIX}{}", query.trim())
    } else {
        query.to_string()
    }
}

fn doc_text(prompt: bool, title: &str, snippet: &str) -> String {
    let title = title.trim();
    let snippet = snippet.trim();
    if prompt {
        let title = if title.is_empty() { "none" } else { title };
        let snippet = if snippet.is_empty() { "none" } else { snippet };
        format!("{GEMMA_DOC_PREFIX}{title}{GEMMA_DOC_SEP}{snippet}")
    } else {
        format!("{title} {snippet}").trim().to_string()
    }
}

/// 计算 query 与每条文档的余弦相似度（与 SearchPIN 相同：直接作为 `_rerank_score`）
pub fn score(query: &str, docs: &[(String, String)]) -> Result<Vec<f64>, String> {
    let loaded = load()?;
    if docs.is_empty() {
        return Ok(Vec::new());
    }

    let mut texts = Vec::with_capacity(docs.len() + 1);
    texts.push(query_text(loaded.prompt, query));
    for (title, snippet) in docs {
        texts.push(doc_text(loaded.prompt, title, snippet));
    }

    let mut model = loaded
        .model
        .lock()
        .map_err(|_| "embedding 模型锁已中毒".to_string())?;
    let embeddings = model
        .embed(texts, None)
        .map_err(|e| format!("embedding 推理失败: {e}"))?;
    let query_vec = embeddings
        .first()
        .ok_or_else(|| "embedding 输出为空".to_string())?;

    Ok(embeddings[1..]
        .iter()
        .map(|doc_vec| cosine(query_vec, doc_vec))
        .collect())
}

/// 余弦相似度；零向量返回 0，避免 NaN 污染排序
pub fn cosine(a: &[f32], b: &[f32]) -> f64 {
    let mut dot = 0.0f64;
    let mut norm_a = 0.0f64;
    let mut norm_b = 0.0f64;
    for (x, y) in a.iter().zip(b.iter()) {
        let x = *x as f64;
        let y = *y as f64;
        dot += x * y;
        norm_a += x * x;
        norm_b += y * y;
    }
    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }
    dot / (norm_a.sqrt() * norm_b.sqrt())
}

/// 健康检查里展示的 embedding 状态（不触发模型加载）
pub fn status() -> serde_json::Value {
    let model = model_spec()
        .map(|s| s.name)
        .unwrap_or("unknown")
        .to_string();
    let ready = matches!(LOADED.get(), Some(Ok(_)));
    serde_json::json!({
        "enabled": enabled(),
        "model": model,
        "ready": ready,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cosine_handles_basic_and_zero_vectors() {
        assert!((cosine(&[1.0, 0.0], &[1.0, 0.0]) - 1.0).abs() < 1e-9);
        assert!(cosine(&[1.0, 0.0], &[0.0, 1.0]).abs() < 1e-9);
        assert_eq!(cosine(&[0.0, 0.0], &[1.0, 0.0]), 0.0);
    }

    #[test]
    fn gemma_prompts_follow_model_card() {
        assert_eq!(
            query_text(true, " rust async "),
            "task: search result | query: rust async"
        );
        assert_eq!(
            doc_text(true, "Rust", "async runtime"),
            "title: Rust | text: async runtime"
        );
        assert_eq!(doc_text(true, "", ""), "title: none | text: none");
    }

    #[test]
    fn plain_mode_matches_searchpin_doc_text() {
        assert_eq!(doc_text(false, "Rust", "async"), "Rust async");
        assert_eq!(query_text(false, "rust"), "rust");
    }

    #[test]
    #[ignore = "需要下载 embedding 模型"]
    fn embedding_smoke() {
        let docs = vec![
            (
                "Rust async 编程".to_string(),
                "Tokio 运行时教程".to_string(),
            ),
            ("今日天气".to_string(), "明日多云转晴".to_string()),
        ];
        let scores = score("Rust 异步编程", &docs).expect("embedding 计算失败");
        assert_eq!(scores.len(), 2);
        assert!(scores.iter().all(|s| (-1.0..=1.0).contains(s)));
        assert!(scores[0] > scores[1], "相关文档应排前: {scores:?}");
    }
}
