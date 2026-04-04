use super::MemoryConfig;

pub(crate) fn default_embedding_provider() -> String {
    "none".into()
}

pub(crate) fn default_hygiene_enabled() -> bool {
    true
}

pub(crate) fn default_archive_after_days() -> u32 {
    7
}

pub(crate) fn default_purge_after_days() -> u32 {
    30
}

pub(crate) fn default_conversation_retention_days() -> u32 {
    30
}

pub(crate) fn default_embedding_model() -> String {
    "text-embedding-3-small".into()
}

pub(crate) fn default_embedding_dims() -> usize {
    1536
}

pub(crate) fn default_vector_weight() -> f64 {
    0.7
}

pub(crate) fn default_keyword_weight() -> f64 {
    0.3
}

pub(crate) fn default_min_relevance_score() -> f64 {
    0.4
}

pub(crate) fn default_cache_size() -> usize {
    10_000
}

pub(crate) fn default_chunk_size() -> usize {
    512
}

pub(crate) fn default_response_cache_ttl() -> u32 {
    60
}

pub(crate) fn default_response_cache_max() -> usize {
    5_000
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            backend: "sqlite".into(),
            auto_save: true,
            hygiene_enabled: default_hygiene_enabled(),
            archive_after_days: default_archive_after_days(),
            purge_after_days: default_purge_after_days(),
            conversation_retention_days: default_conversation_retention_days(),
            embedding_provider: default_embedding_provider(),
            embedding_model: default_embedding_model(),
            embedding_dimensions: default_embedding_dims(),
            vector_weight: default_vector_weight(),
            keyword_weight: default_keyword_weight(),
            min_relevance_score: default_min_relevance_score(),
            embedding_cache_size: default_cache_size(),
            chunk_max_tokens: default_chunk_size(),
            response_cache_enabled: false,
            response_cache_ttl_minutes: default_response_cache_ttl(),
            response_cache_max_entries: default_response_cache_max(),
            snapshot_enabled: false,
            snapshot_on_hygiene: false,
            auto_hydrate: true,
            sqlite_open_timeout_secs: None,
            qdrant: super::QdrantConfig::default(),
        }
    }
}
