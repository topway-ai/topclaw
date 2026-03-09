use crate::providers::{ChatMessage, Provider};
use crate::util::truncate_with_ellipsis;
use anyhow::{Context, Result};
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use uuid::Uuid;

const LCM_DB_DIR: &str = "lcm";
const LCM_DB_FILE: &str = "context.db";
const FRESH_TAIL_COUNT: usize = 24;
const MIN_TAIL_COUNT: usize = 8;
const LEAF_MIN_FANOUT: usize = 8;
const CONDENSED_MIN_FANOUT: usize = 4;
const LEAF_SOURCE_MAX_CHARS: usize = 12_000;
const LEAF_SUMMARY_MAX_CHARS: usize = 2_000;
const CONDENSED_SUMMARY_MAX_CHARS: usize = 2_400;
const APPROX_TOKENS_PER_HISTORY_MESSAGE: usize = 220;

#[derive(Debug, Clone)]
struct StoredMessage {
    id: String,
    ordinal: i64,
    role: String,
    content: String,
    token_estimate: usize,
}

#[derive(Debug, Clone)]
struct SummaryNode {
    id: String,
    depth: usize,
    content: String,
    token_estimate: usize,
    descendant_message_count: usize,
    earliest_ordinal: i64,
    latest_ordinal: i64,
}

#[derive(Debug, Clone)]
pub(crate) struct LosslessConversationOverview {
    pub(crate) session_scope: Option<String>,
    pub(crate) session_key: Option<String>,
    pub(crate) conversation_id: String,
    pub(crate) message_count: usize,
    pub(crate) summary_count: usize,
    pub(crate) latest_activity_at: String,
}

#[derive(Debug, Clone)]
pub(crate) struct LosslessSearchHit {
    pub(crate) session_scope: Option<String>,
    pub(crate) session_key: Option<String>,
    pub(crate) conversation_id: String,
    pub(crate) source_kind: String,
    pub(crate) ordinal_hint: String,
    pub(crate) role: Option<String>,
    pub(crate) excerpt: String,
}

pub(crate) struct LosslessContext {
    db_path: PathBuf,
    conversation_id: String,
    next_ordinal: i64,
    session_scope: Option<String>,
    session_key: Option<String>,
}

impl LosslessContext {
    pub(crate) fn new(workspace_dir: &Path, system_prompt: &str) -> Result<Self> {
        let db_dir = workspace_dir.join(LCM_DB_DIR);
        std::fs::create_dir_all(&db_dir)
            .with_context(|| format!("failed to create {}", db_dir.display()))?;
        let db_path = db_dir.join(LCM_DB_FILE);
        let conn = open_connection(&db_path)?;
        init_schema(&conn)?;

        let conversation_id = format!("conv_{}", Uuid::new_v4());
        let now = Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO conversations (id, created_at) VALUES (?1, ?2)",
            params![conversation_id, now],
        )?;

        let mut this = Self {
            db_path,
            conversation_id,
            next_ordinal: 0,
            session_scope: None,
            session_key: None,
        };
        this.record_raw_message(&ChatMessage::system(system_prompt))?;
        Ok(this)
    }

    pub(crate) fn for_session(
        workspace_dir: &Path,
        session_scope: &str,
        session_key: &str,
        system_prompt: &str,
    ) -> Result<Self> {
        let db_dir = workspace_dir.join(LCM_DB_DIR);
        std::fs::create_dir_all(&db_dir)
            .with_context(|| format!("failed to create {}", db_dir.display()))?;
        let db_path = db_dir.join(LCM_DB_FILE);
        let conn = open_connection(&db_path)?;
        init_schema(&conn)?;

        let conversation_id = match conn
            .query_row(
                "SELECT conversation_id
                 FROM session_bindings
                 WHERE session_scope = ?1 AND session_key = ?2",
                params![session_scope, session_key],
                |row| row.get::<_, String>(0),
            )
            .optional()?
        {
            Some(existing) => existing,
            None => {
                let generated = format!(
                    "sess_{}_{}",
                    normalize_session_scope(session_scope),
                    hash_session_key(session_key)
                );
                ensure_conversation_record(&conn, &generated)?;
                conn.execute(
                    "INSERT OR REPLACE INTO session_bindings (
                        session_scope, session_key, conversation_id, updated_at
                     ) VALUES (?1, ?2, ?3, ?4)",
                    params![
                        session_scope,
                        session_key,
                        generated,
                        Utc::now().to_rfc3339()
                    ],
                )?;
                generated
            }
        };

        ensure_conversation_record(&conn, &conversation_id)?;
        let next_ordinal = next_ordinal_for_conversation(&conn, &conversation_id)?;
        let mut this = Self {
            db_path,
            conversation_id,
            next_ordinal,
            session_scope: Some(session_scope.to_string()),
            session_key: Some(session_key.to_string()),
        };
        if next_ordinal == 0 {
            this.record_raw_message(&ChatMessage::system(system_prompt))?;
        }
        Ok(this)
    }

    pub(crate) fn record_raw_message(&mut self, message: &ChatMessage) -> Result<()> {
        if is_lossless_summary_message(message) {
            return Ok(());
        }

        let conn = open_connection(&self.db_path)?;
        let id = format!("msg_{}", Uuid::new_v4());
        let now = Utc::now().to_rfc3339();
        let token_estimate = estimate_tokens(&message.content);
        conn.execute(
            "INSERT INTO messages (
                id, conversation_id, ordinal, role, content, token_estimate, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                id,
                self.conversation_id,
                self.next_ordinal,
                message.role,
                message.content,
                token_estimate as i64,
                now
            ],
        )?;
        self.next_ordinal += 1;
        Ok(())
    }

    pub(crate) fn record_raw_messages(&mut self, messages: &[ChatMessage]) -> Result<()> {
        for message in messages {
            self.record_raw_message(message)?;
        }
        Ok(())
    }

    pub(crate) fn reset(&mut self, system_prompt: &str) -> Result<()> {
        let conn = open_connection(&self.db_path)?;
        conn.execute(
            "DELETE FROM summary_sources
             WHERE summary_id IN (SELECT id FROM summaries WHERE conversation_id = ?1)",
            params![self.conversation_id],
        )?;
        conn.execute(
            "DELETE FROM summaries WHERE conversation_id = ?1",
            params![self.conversation_id],
        )?;
        conn.execute(
            "DELETE FROM messages WHERE conversation_id = ?1",
            params![self.conversation_id],
        )?;
        conn.execute(
            "DELETE FROM conversations WHERE id = ?1",
            params![self.conversation_id],
        )?;

        self.conversation_id = format!("conv_{}", Uuid::new_v4());
        self.next_ordinal = 0;
        let now = Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO conversations (id, created_at) VALUES (?1, ?2)",
            params![self.conversation_id, now],
        )?;
        if let (Some(scope), Some(key)) = (&self.session_scope, &self.session_key) {
            conn.execute(
                "INSERT OR REPLACE INTO session_bindings (
                    session_scope, session_key, conversation_id, updated_at
                 ) VALUES (?1, ?2, ?3, ?4)",
                params![scope, key, self.conversation_id, Utc::now().to_rfc3339()],
            )?;
        }
        self.record_raw_message(&ChatMessage::system(system_prompt))?;
        Ok(())
    }

    pub(crate) fn has_non_system_messages(&self) -> Result<bool> {
        let conn = open_connection(&self.db_path)?;
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM messages
             WHERE conversation_id = ?1 AND role != 'system'",
            params![self.conversation_id.clone()],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    pub(crate) fn rollback_latest_raw_message(
        &mut self,
        role: &str,
        content: &str,
    ) -> Result<bool> {
        let conn = open_connection(&self.db_path)?;
        let candidate = conn
            .query_row(
                "SELECT id, ordinal
                 FROM messages
                 WHERE conversation_id = ?1
                 ORDER BY ordinal DESC
                 LIMIT 1",
                params![self.conversation_id.clone()],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
            )
            .optional()?;
        let Some((message_id, ordinal)) = candidate else {
            return Ok(false);
        };

        let matches = conn.query_row(
            "SELECT COUNT(*)
             FROM messages
             WHERE id = ?1 AND role = ?2 AND content = ?3",
            params![message_id, role, content],
            |row| row.get::<_, i64>(0),
        )?;
        if matches == 0 {
            return Ok(false);
        }

        conn.execute("DELETE FROM messages WHERE id = ?1", params![message_id])?;
        self.next_ordinal = ordinal;
        Ok(true)
    }

    pub(crate) async fn rebuild_active_history(
        &mut self,
        provider: &dyn Provider,
        model: &str,
        system_prompt: &str,
        max_history_messages: usize,
    ) -> Result<Vec<ChatMessage>> {
        self.ensure_leaf_summaries(provider, model).await?;
        self.ensure_condensed_summaries(provider, model).await?;
        self.build_active_history(system_prompt, max_history_messages)
    }

    #[cfg(test)]
    fn summary_depth_counts(&self) -> Result<Vec<(usize, usize)>> {
        let conn = open_connection(&self.db_path)?;
        let mut stmt = conn.prepare(
            "SELECT depth, COUNT(*) FROM summaries
             WHERE conversation_id = ?1
             GROUP BY depth
             ORDER BY depth",
        )?;
        let rows = stmt.query_map(params![self.conversation_id.clone()], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .context("failed to load summary depth counts")
    }

    #[cfg(test)]
    fn raw_message_count(&self) -> Result<usize> {
        let conn = open_connection(&self.db_path)?;
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM messages WHERE conversation_id = ?1",
            params![self.conversation_id.clone()],
            |row| row.get(0),
        )?;
        usize::try_from(count).context("message count cannot be negative")
    }

    async fn ensure_leaf_summaries(&mut self, provider: &dyn Provider, model: &str) -> Result<()> {
        let raw_messages = self.list_non_system_messages()?;
        if raw_messages.len() <= FRESH_TAIL_COUNT {
            return Ok(());
        }

        let protected_from = raw_messages.len().saturating_sub(FRESH_TAIL_COUNT);
        let protected_ordinal = raw_messages[protected_from].ordinal;
        let uncovered = self.list_uncovered_messages_before(protected_ordinal)?;

        for chunk in uncovered.chunks(LEAF_MIN_FANOUT) {
            if chunk.len() < LEAF_MIN_FANOUT {
                break;
            }
            let transcript = build_message_transcript(chunk, LEAF_SOURCE_MAX_CHARS);
            let summary = summarize_leaf(provider, model, &transcript).await?;
            self.insert_summary("leaf", 0, &summary, chunk, &[])?;
        }

        Ok(())
    }

    async fn ensure_condensed_summaries(
        &mut self,
        provider: &dyn Provider,
        model: &str,
    ) -> Result<()> {
        let mut depth = 0_usize;
        loop {
            let roots = self.list_root_summaries_at_depth(depth)?;
            if roots.len() < CONDENSED_MIN_FANOUT {
                break;
            }

            let mut created_any = false;
            for chunk in roots.chunks(CONDENSED_MIN_FANOUT) {
                if chunk.len() < CONDENSED_MIN_FANOUT {
                    break;
                }
                let transcript = build_summary_transcript(chunk, CONDENSED_SUMMARY_MAX_CHARS);
                let summary = summarize_condensed(provider, model, depth + 1, &transcript).await?;
                self.insert_summary("condensed", depth + 1, &summary, &[], chunk)?;
                created_any = true;
            }

            if !created_any {
                break;
            }
            depth += 1;
        }

        Ok(())
    }

    fn build_active_history(
        &self,
        system_prompt: &str,
        max_history_messages: usize,
    ) -> Result<Vec<ChatMessage>> {
        let mut history = vec![ChatMessage::system(system_prompt)];
        let mut root_summaries = self.list_root_summaries()?;
        let mut fresh_tail = self.list_fresh_tail_messages(FRESH_TAIL_COUNT)?;
        let max_token_budget = max_history_messages
            .max(MIN_TAIL_COUNT)
            .saturating_mul(APPROX_TOKENS_PER_HISTORY_MESSAGE);
        let system_tokens = estimate_tokens(system_prompt);

        let mut summary_messages = root_summaries
            .iter()
            .map(render_summary_message)
            .collect::<Vec<_>>();
        let mut total_tokens = system_tokens
            + root_summaries
                .iter()
                .map(|summary| summary.token_estimate)
                .sum::<usize>()
            + fresh_tail
                .iter()
                .map(|message| message.token_estimate)
                .sum::<usize>();

        while total_tokens > max_token_budget && !summary_messages.is_empty() {
            let dropped = root_summaries.remove(0).token_estimate;
            summary_messages.remove(0);
            total_tokens = total_tokens.saturating_sub(dropped);
        }

        while total_tokens > max_token_budget && fresh_tail.len() > MIN_TAIL_COUNT {
            let dropped = fresh_tail.remove(0).token_estimate;
            total_tokens = total_tokens.saturating_sub(dropped);
        }

        history.extend(summary_messages);
        history.extend(fresh_tail.into_iter().map(|message| ChatMessage {
            role: message.role,
            content: message.content,
        }));
        Ok(history)
    }

    fn list_non_system_messages(&self) -> Result<Vec<StoredMessage>> {
        let conn = open_connection(&self.db_path)?;
        let mut stmt = conn.prepare(
            "SELECT id, ordinal, role, content, token_estimate
             FROM messages
             WHERE conversation_id = ?1 AND role != 'system'
             ORDER BY ordinal",
        )?;
        read_messages(&mut stmt, params![self.conversation_id.clone()])
    }

    fn list_uncovered_messages_before(&self, ordinal: i64) -> Result<Vec<StoredMessage>> {
        let conn = open_connection(&self.db_path)?;
        let mut stmt = conn.prepare(
            "SELECT m.id, m.ordinal, m.role, m.content, m.token_estimate
             FROM messages m
             WHERE m.conversation_id = ?1
               AND m.role != 'system'
               AND m.ordinal < ?2
               AND NOT EXISTS (
                   SELECT 1
                   FROM summary_sources ss
                   WHERE ss.source_kind = 'message' AND ss.source_id = m.id
               )
             ORDER BY m.ordinal",
        )?;
        read_messages(&mut stmt, params![self.conversation_id.clone(), ordinal])
    }

    fn list_root_summaries_at_depth(&self, depth: usize) -> Result<Vec<SummaryNode>> {
        let conn = open_connection(&self.db_path)?;
        let mut stmt = conn.prepare(
            "SELECT s.id, s.depth, s.content, s.token_estimate, s.descendant_message_count,
                    s.earliest_ordinal, s.latest_ordinal
             FROM summaries s
             WHERE s.conversation_id = ?1
               AND s.depth = ?2
               AND NOT EXISTS (
                   SELECT 1 FROM summary_sources ss
                   WHERE ss.source_kind = 'summary' AND ss.source_id = s.id
               )
             ORDER BY s.earliest_ordinal",
        )?;
        read_summaries(
            &mut stmt,
            params![self.conversation_id.clone(), depth as i64],
        )
    }

    fn list_root_summaries(&self) -> Result<Vec<SummaryNode>> {
        let conn = open_connection(&self.db_path)?;
        let mut stmt = conn.prepare(
            "SELECT s.id, s.depth, s.content, s.token_estimate, s.descendant_message_count,
                    s.earliest_ordinal, s.latest_ordinal
             FROM summaries s
             WHERE s.conversation_id = ?1
               AND NOT EXISTS (
                   SELECT 1 FROM summary_sources ss
                   WHERE ss.source_kind = 'summary' AND ss.source_id = s.id
               )
             ORDER BY s.earliest_ordinal",
        )?;
        read_summaries(&mut stmt, params![self.conversation_id.clone()])
    }

    fn list_fresh_tail_messages(&self, count: usize) -> Result<Vec<StoredMessage>> {
        let conn = open_connection(&self.db_path)?;
        let mut stmt = conn.prepare(
            "SELECT id, ordinal, role, content, token_estimate
             FROM messages
             WHERE conversation_id = ?1 AND role != 'system'
             ORDER BY ordinal DESC
             LIMIT ?2",
        )?;
        let mut messages = read_messages(
            &mut stmt,
            params![self.conversation_id.clone(), count as i64],
        )?;
        messages.reverse();
        Ok(messages)
    }

    fn insert_summary(
        &self,
        kind: &str,
        depth: usize,
        content: &str,
        raw_messages: &[StoredMessage],
        parent_summaries: &[SummaryNode],
    ) -> Result<()> {
        let conn = open_connection(&self.db_path)?;

        if !raw_messages.is_empty() {
            let first_message_id = &raw_messages[0].id;
            let existing_id: Option<String> = conn
                .query_row(
                    "SELECT summary_id
                     FROM summary_sources
                     WHERE source_kind = 'message' AND source_id = ?1
                     LIMIT 1",
                    params![first_message_id],
                    |row| row.get(0),
                )
                .optional()?;
            if existing_id.is_some() {
                return Ok(());
            }
        }

        let summary_id = format!("sum_{}", Uuid::new_v4());
        let now = Utc::now().to_rfc3339();
        let descendant_count = if raw_messages.is_empty() {
            parent_summaries
                .iter()
                .map(|summary| summary.descendant_message_count)
                .sum()
        } else {
            raw_messages.len()
        };
        let earliest_ordinal = raw_messages
            .first()
            .map(|message| message.ordinal)
            .or_else(|| {
                parent_summaries
                    .first()
                    .map(|summary| summary.earliest_ordinal)
            })
            .context("summary insertion requires source messages or parent summaries")?;
        let latest_ordinal = raw_messages
            .last()
            .map(|message| message.ordinal)
            .or_else(|| {
                parent_summaries
                    .last()
                    .map(|summary| summary.latest_ordinal)
            })
            .context("summary insertion requires source messages or parent summaries")?;

        conn.execute(
            "INSERT INTO summaries (
                id, conversation_id, depth, kind, content, token_estimate,
                descendant_message_count, earliest_ordinal, latest_ordinal, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                summary_id,
                self.conversation_id,
                depth as i64,
                kind,
                content,
                estimate_tokens(content) as i64,
                descendant_count as i64,
                earliest_ordinal,
                latest_ordinal,
                now
            ],
        )?;

        for (index, message) in raw_messages.iter().enumerate() {
            conn.execute(
                "INSERT INTO summary_sources (summary_id, source_kind, source_id, source_ordinal)
                 VALUES (?1, 'message', ?2, ?3)",
                params![summary_id, message.id, index as i64],
            )?;
        }

        for (index, summary) in parent_summaries.iter().enumerate() {
            conn.execute(
                "INSERT INTO summary_sources (summary_id, source_kind, source_id, source_ordinal)
                 VALUES (?1, 'summary', ?2, ?3)",
                params![summary_id, summary.id, index as i64],
            )?;
        }

        Ok(())
    }
}

pub(crate) fn inspect_store(
    workspace_dir: &Path,
    limit: usize,
) -> Result<Vec<LosslessConversationOverview>> {
    let db_path = workspace_dir.join(LCM_DB_DIR).join(LCM_DB_FILE);
    if !db_path.exists() {
        return Ok(Vec::new());
    }
    let conn = open_connection(&db_path)?;
    init_schema(&conn)?;
    let mut stmt = conn.prepare(
        "SELECT sb.session_scope,
                sb.session_key,
                c.id,
                COALESCE(msg.message_count, 0),
                COALESCE(sum.summary_count, 0),
                COALESCE(msg.latest_message_at, c.created_at)
         FROM conversations c
         LEFT JOIN session_bindings sb ON sb.conversation_id = c.id
         LEFT JOIN (
             SELECT conversation_id,
                    COUNT(*) AS message_count,
                    MAX(created_at) AS latest_message_at
             FROM messages
             GROUP BY conversation_id
         ) msg ON msg.conversation_id = c.id
         LEFT JOIN (
             SELECT conversation_id, COUNT(*) AS summary_count
             FROM summaries
             GROUP BY conversation_id
         ) sum ON sum.conversation_id = c.id
         ORDER BY COALESCE(msg.latest_message_at, c.created_at) DESC
         LIMIT ?1",
    )?;
    let rows = stmt.query_map(params![limit.max(1) as i64], |row| {
        Ok(LosslessConversationOverview {
            session_scope: row.get(0)?,
            session_key: row.get(1)?,
            conversation_id: row.get(2)?,
            message_count: row.get(3)?,
            summary_count: row.get(4)?,
            latest_activity_at: row.get(5)?,
        })
    })?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .context("failed to inspect lossless context store")
}

pub(crate) fn search_store(
    workspace_dir: &Path,
    query: &str,
    limit: usize,
    session_scope: Option<&str>,
    session_key: Option<&str>,
) -> Result<Vec<LosslessSearchHit>> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }

    let db_path = workspace_dir.join(LCM_DB_DIR).join(LCM_DB_FILE);
    if !db_path.exists() {
        return Ok(Vec::new());
    }
    let conn = open_connection(&db_path)?;
    init_schema(&conn)?;
    let pattern = format!("%{}%", escape_like(trimmed));
    let limit = limit.max(1);

    let mut hits = Vec::new();
    {
        let mut stmt = conn.prepare(
            "SELECT sb.session_scope,
                    sb.session_key,
                    m.conversation_id,
                    'message',
                    CAST(m.ordinal AS TEXT),
                    m.role,
                    m.content
             FROM messages m
             LEFT JOIN session_bindings sb ON sb.conversation_id = m.conversation_id
             WHERE m.content LIKE ?1 ESCAPE '\\'
               AND (?2 IS NULL OR sb.session_scope = ?2)
               AND (?3 IS NULL OR sb.session_key = ?3)
             ORDER BY m.created_at DESC
             LIMIT ?4",
        )?;
        let rows = stmt.query_map(
            params![pattern, session_scope, session_key, limit as i64],
            |row| {
                Ok(LosslessSearchHit {
                    session_scope: row.get(0)?,
                    session_key: row.get(1)?,
                    conversation_id: row.get(2)?,
                    source_kind: row.get(3)?,
                    ordinal_hint: row.get(4)?,
                    role: row.get(5)?,
                    excerpt: truncate_with_ellipsis(&row.get::<_, String>(6)?, 240),
                })
            },
        )?;
        hits.extend(rows.collect::<rusqlite::Result<Vec<_>>>()?);
    }

    if hits.len() < limit {
        let remaining = (limit - hits.len()).max(1);
        let mut stmt = conn.prepare(
            "SELECT sb.session_scope,
                    sb.session_key,
                    s.conversation_id,
                    'summary',
                    printf('%d-%d', s.earliest_ordinal, s.latest_ordinal),
                    NULL,
                    s.content
             FROM summaries s
             LEFT JOIN session_bindings sb ON sb.conversation_id = s.conversation_id
             WHERE s.content LIKE ?1 ESCAPE '\\'
               AND (?2 IS NULL OR sb.session_scope = ?2)
               AND (?3 IS NULL OR sb.session_key = ?3)
             ORDER BY s.created_at DESC
             LIMIT ?4",
        )?;
        let rows = stmt.query_map(
            params![pattern, session_scope, session_key, remaining as i64],
            |row| {
                Ok(LosslessSearchHit {
                    session_scope: row.get(0)?,
                    session_key: row.get(1)?,
                    conversation_id: row.get(2)?,
                    source_kind: row.get(3)?,
                    ordinal_hint: row.get(4)?,
                    role: row.get(5)?,
                    excerpt: truncate_with_ellipsis(&row.get::<_, String>(6)?, 240),
                })
            },
        )?;
        hits.extend(rows.collect::<rusqlite::Result<Vec<_>>>()?);
    }

    Ok(hits.into_iter().take(limit).collect())
}

fn open_connection(path: &Path) -> Result<Connection> {
    Connection::open(path).with_context(|| format!("failed to open {}", path.display()))
}

fn init_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS conversations (
            id TEXT PRIMARY KEY,
            created_at TEXT NOT NULL
         );
         CREATE TABLE IF NOT EXISTS messages (
            id TEXT PRIMARY KEY,
            conversation_id TEXT NOT NULL,
            ordinal INTEGER NOT NULL,
            role TEXT NOT NULL,
            content TEXT NOT NULL,
            token_estimate INTEGER NOT NULL,
            created_at TEXT NOT NULL
         );
         CREATE UNIQUE INDEX IF NOT EXISTS idx_lcm_messages_conv_ord
            ON messages(conversation_id, ordinal);
         CREATE TABLE IF NOT EXISTS summaries (
            id TEXT PRIMARY KEY,
            conversation_id TEXT NOT NULL,
            depth INTEGER NOT NULL,
            kind TEXT NOT NULL,
            content TEXT NOT NULL,
            token_estimate INTEGER NOT NULL,
            descendant_message_count INTEGER NOT NULL,
            earliest_ordinal INTEGER NOT NULL,
            latest_ordinal INTEGER NOT NULL,
            created_at TEXT NOT NULL
         );
         CREATE INDEX IF NOT EXISTS idx_lcm_summaries_conv_depth
            ON summaries(conversation_id, depth, earliest_ordinal);
         CREATE TABLE IF NOT EXISTS session_bindings (
            session_scope TEXT NOT NULL,
            session_key TEXT NOT NULL,
            conversation_id TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            PRIMARY KEY(session_scope, session_key)
         );
         CREATE INDEX IF NOT EXISTS idx_lcm_session_bindings_conversation
            ON session_bindings(conversation_id);
         CREATE TABLE IF NOT EXISTS summary_sources (
            summary_id TEXT NOT NULL,
            source_kind TEXT NOT NULL,
            source_id TEXT NOT NULL,
            source_ordinal INTEGER NOT NULL,
            PRIMARY KEY(summary_id, source_kind, source_id)
         );
         CREATE INDEX IF NOT EXISTS idx_lcm_summary_sources_source
            ON summary_sources(source_kind, source_id);",
    )?;
    Ok(())
}

fn ensure_conversation_record(conn: &Connection, conversation_id: &str) -> Result<()> {
    let existing: Option<String> = conn
        .query_row(
            "SELECT id FROM conversations WHERE id = ?1",
            params![conversation_id],
            |row| row.get(0),
        )
        .optional()?;
    if existing.is_none() {
        conn.execute(
            "INSERT INTO conversations (id, created_at) VALUES (?1, ?2)",
            params![conversation_id, Utc::now().to_rfc3339()],
        )?;
    }
    Ok(())
}

fn next_ordinal_for_conversation(conn: &Connection, conversation_id: &str) -> Result<i64> {
    let max_ordinal: Option<i64> = conn.query_row(
        "SELECT MAX(ordinal) FROM messages WHERE conversation_id = ?1",
        params![conversation_id],
        |row| row.get(0),
    )?;
    Ok(max_ordinal.map_or(0, |value| value + 1))
}

fn read_messages<P: rusqlite::Params>(
    stmt: &mut rusqlite::Statement<'_>,
    params: P,
) -> Result<Vec<StoredMessage>> {
    let rows = stmt.query_map(params, |row| {
        Ok(StoredMessage {
            id: row.get(0)?,
            ordinal: row.get(1)?,
            role: row.get(2)?,
            content: row.get(3)?,
            token_estimate: row.get(4)?,
        })
    })?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .context("failed to read messages")
}

fn read_summaries<P: rusqlite::Params>(
    stmt: &mut rusqlite::Statement<'_>,
    params: P,
) -> Result<Vec<SummaryNode>> {
    let rows = stmt.query_map(params, |row| {
        Ok(SummaryNode {
            id: row.get(0)?,
            depth: row.get(1)?,
            content: row.get(2)?,
            token_estimate: row.get(3)?,
            descendant_message_count: row.get(4)?,
            earliest_ordinal: row.get(5)?,
            latest_ordinal: row.get(6)?,
        })
    })?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .context("failed to read summaries")
}

fn estimate_tokens(content: &str) -> usize {
    content.chars().count().div_ceil(4).max(1)
}

fn normalize_session_scope(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect()
}

fn hash_session_key(value: &str) -> String {
    let digest = Sha256::digest(value.as_bytes());
    hex::encode(digest)
}

fn escape_like(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

fn is_lossless_summary_message(message: &ChatMessage) -> bool {
    message.role == "assistant" && message.content.starts_with("<lossless_summary ")
}

fn build_message_transcript(messages: &[StoredMessage], max_chars: usize) -> String {
    let mut transcript = String::new();
    for message in messages {
        let _ = std::fmt::Write::write_fmt(
            &mut transcript,
            format_args!(
                "[ord:{} role:{}]\n{}\n\n",
                message.ordinal,
                message.role,
                message.content.trim()
            ),
        );
    }
    truncate_with_ellipsis(&transcript, max_chars)
}

fn build_summary_transcript(summaries: &[SummaryNode], max_chars: usize) -> String {
    let mut transcript = String::new();
    for summary in summaries {
        let _ = std::fmt::Write::write_fmt(
            &mut transcript,
            format_args!(
                "[summary id:{} depth:{} descendants:{} ord:{}-{}]\n{}\n\n",
                summary.id,
                summary.depth,
                summary.descendant_message_count,
                summary.earliest_ordinal,
                summary.latest_ordinal,
                summary.content.trim()
            ),
        );
    }
    truncate_with_ellipsis(&transcript, max_chars)
}

fn render_summary_message(summary: &SummaryNode) -> ChatMessage {
    ChatMessage::assistant(format!(
        "<lossless_summary id=\"{}\" depth=\"{}\" descendants=\"{}\" ord=\"{}-{}\">\n{}\n</lossless_summary>",
        summary.id,
        summary.depth,
        summary.descendant_message_count,
        summary.earliest_ordinal,
        summary.latest_ordinal,
        summary.content.trim()
    ))
}

async fn summarize_leaf(provider: &dyn Provider, model: &str, transcript: &str) -> Result<String> {
    let prompt = "You are a lossless context compactor. Summarize a raw conversation chunk for future turns. Preserve: user preferences, explicit decisions, constraints, file or config changes, unresolved work, important results, failures, and follow-up obligations. Use concise bullet points. Do not invent details.";
    summarize_with_fallback(provider, model, prompt, transcript, LEAF_SUMMARY_MAX_CHARS).await
}

async fn summarize_condensed(
    provider: &dyn Provider,
    model: &str,
    depth: usize,
    transcript: &str,
) -> Result<String> {
    let prompt = if depth <= 1 {
        "You are condensing existing conversation summaries into a higher-level context node. Deduplicate repeated details, preserve chronology, and keep durable decisions, constraints, unresolved tasks, and user preferences. Output concise bullet points only."
    } else {
        "You are condensing older context into durable long-term guidance. Keep only stable facts, recurring preferences, important architectural decisions, unresolved issues, and lessons that still matter. Output concise bullet points only."
    };
    summarize_with_fallback(
        provider,
        model,
        prompt,
        transcript,
        CONDENSED_SUMMARY_MAX_CHARS,
    )
    .await
}

async fn summarize_with_fallback(
    provider: &dyn Provider,
    model: &str,
    system_prompt: &str,
    transcript: &str,
    max_chars: usize,
) -> Result<String> {
    let user_prompt = format!(
        "Summarize this context while preserving important recoverable detail. Keep the result short and structured.\n\n{}",
        transcript
    );
    let summary = provider
        .chat_with_system(Some(system_prompt), &user_prompt, model, 0.2)
        .await
        .unwrap_or_else(|_| transcript.to_string());
    Ok(truncate_with_ellipsis(summary.trim(), max_chars))
}

#[cfg(test)]
mod tests {
    use super::{inspect_store, search_store, LosslessContext};
    use crate::providers::{ChatMessage, Provider};
    use async_trait::async_trait;
    use tempfile::tempdir;

    struct StubProvider;

    #[async_trait]
    impl Provider for StubProvider {
        async fn chat_with_system(
            &self,
            _system_prompt: Option<&str>,
            message: &str,
            _model: &str,
            _temperature: f64,
        ) -> anyhow::Result<String> {
            Ok(format!(
                "- summary\n- digest: {}",
                &message.chars().take(80).collect::<String>()
            ))
        }
    }

    #[tokio::test]
    async fn rebuild_creates_leaf_summaries_and_preserves_raw_messages() {
        let temp = tempdir().unwrap();
        let mut lcm = LosslessContext::new(temp.path(), "system prompt").unwrap();

        for idx in 0..40 {
            lcm.record_raw_message(&ChatMessage::user(format!("user message {idx}")))
                .unwrap();
            lcm.record_raw_message(&ChatMessage::assistant(format!("assistant reply {idx}")))
                .unwrap();
        }

        let active = lcm
            .rebuild_active_history(&StubProvider, "test-model", "system prompt", 20)
            .await
            .unwrap();

        assert!(active[0].role == "system");
        assert!(active
            .iter()
            .any(|message| message.content.starts_with("<lossless_summary ")));
        assert_eq!(lcm.raw_message_count().unwrap(), 81);
    }

    #[tokio::test]
    async fn rebuild_condenses_leaf_summaries_into_higher_depth_nodes() {
        let temp = tempdir().unwrap();
        let mut lcm = LosslessContext::new(temp.path(), "system prompt").unwrap();

        for idx in 0..80 {
            lcm.record_raw_message(&ChatMessage::user(format!("user message {idx}")))
                .unwrap();
            lcm.record_raw_message(&ChatMessage::assistant(format!("assistant reply {idx}")))
                .unwrap();
        }

        let _ = lcm
            .rebuild_active_history(&StubProvider, "test-model", "system prompt", 24)
            .await
            .unwrap();

        let counts = lcm.summary_depth_counts().unwrap();
        assert!(counts
            .iter()
            .any(|(depth, count)| *depth == 0 && *count >= 4));
        assert!(counts
            .iter()
            .any(|(depth, count)| *depth >= 1 && *count >= 1));
    }

    #[tokio::test]
    async fn reset_starts_a_fresh_conversation() {
        let temp = tempdir().unwrap();
        let mut lcm = LosslessContext::new(temp.path(), "system prompt").unwrap();
        let original_conversation = lcm.conversation_id.clone();

        lcm.record_raw_message(&ChatMessage::user("first")).unwrap();
        lcm.reset("system prompt").unwrap();

        assert_ne!(lcm.conversation_id, original_conversation);
        assert_eq!(lcm.raw_message_count().unwrap(), 1);
    }

    #[test]
    fn for_session_reuses_binding_and_restores_next_ordinal() {
        let temp = tempdir().unwrap();
        let mut first =
            LosslessContext::for_session(temp.path(), "channel", "sender-1", "system prompt")
                .unwrap();
        first
            .record_raw_message(&ChatMessage::user("first user turn"))
            .unwrap();
        let conversation_id = first.conversation_id.clone();

        let mut second =
            LosslessContext::for_session(temp.path(), "channel", "sender-1", "system prompt")
                .unwrap();
        assert_eq!(second.conversation_id, conversation_id);
        second
            .record_raw_message(&ChatMessage::assistant("first assistant turn"))
            .unwrap();

        assert_eq!(second.raw_message_count().unwrap(), 3);
    }

    #[test]
    fn for_session_uses_distinct_conversation_ids_for_distinct_session_keys() {
        let temp = tempdir().unwrap();
        let first =
            LosslessContext::for_session(temp.path(), "channel", "sender-a", "system prompt")
                .unwrap();
        let second =
            LosslessContext::for_session(temp.path(), "channel", "sender-b", "system prompt")
                .unwrap();

        assert_ne!(first.conversation_id, second.conversation_id);
    }

    #[test]
    fn rollback_latest_raw_message_only_removes_matching_latest_turn() {
        let temp = tempdir().unwrap();
        let mut lcm =
            LosslessContext::for_session(temp.path(), "channel", "sender-2", "system prompt")
                .unwrap();
        lcm.record_raw_message(&ChatMessage::user("first")).unwrap();
        lcm.record_raw_message(&ChatMessage::assistant("reply"))
            .unwrap();

        assert!(!lcm.rollback_latest_raw_message("user", "first").unwrap());
        assert!(lcm
            .rollback_latest_raw_message("assistant", "reply")
            .unwrap());
        assert_eq!(lcm.raw_message_count().unwrap(), 2);
    }

    #[test]
    fn inspect_and_search_store_include_session_metadata() {
        let temp = tempdir().unwrap();
        let mut lcm =
            LosslessContext::for_session(temp.path(), "gateway_ws", "session-1", "system prompt")
                .unwrap();
        lcm.record_raw_message(&ChatMessage::user("search needle"))
            .unwrap();
        lcm.record_raw_message(&ChatMessage::assistant("reply"))
            .unwrap();

        let sessions = inspect_store(temp.path(), 10).unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].session_scope.as_deref(), Some("gateway_ws"));
        assert_eq!(sessions[0].session_key.as_deref(), Some("session-1"));

        let hits = search_store(temp.path(), "needle", 10, Some("gateway_ws"), None).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].session_scope.as_deref(), Some("gateway_ws"));
        assert!(hits[0].excerpt.contains("needle"));
    }
}
