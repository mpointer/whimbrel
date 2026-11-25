//! Ask retrieval bounded by date, app and person (screen 5, memory search and Ask).
//!
//! With no bound these call [`crate::gather_and_assemble_context`] and
//! [`crate::gather_and_assemble_context_for_window`] themselves, so a question with no filter
//! retrieves exactly what it did before, in the same order, into the same prompt. With a bound,
//! every retrieval source is bounded in its own query, before its limit (`memory::find` has the per-kind
//! rules). Meaning search under a narrow bound is exact when the bound selects few enough rows to score
//! (`memory::find::EXACT_SEMANTIC_MAX_ROWS`); past that it reads the nearest neighbours of everything and
//! filters after, so the prompt can then hold fewer meaning matches than exist. The background
//! items Ask adds (recent automation runs and episodes, threads waiting on someone) are included
//! only when they pass the bound. Client exclusion is applied after, by the same
//! [`crate::ContextFilter`] as before: a bound narrows what is read, and never loosens exclusion.
//!
//! Where the bounded background items differ from the unbounded ones: automation runs inside a
//! date range leave out the daily brief and end of day recap runs (as `routine_runs_in_window`
//! always has); episodes come newest by when they ended; waiting threads come newest first, not by
//! priority.

use memory::find::{ReadError, ScopeCtx};

use crate::{assemble_filtered, thread_with_matching_email, unscoped, ContextFilter, ContextItem};

/// [`crate::gather_and_assemble_context`] bounded by `scope`.
///
/// A read that fails is an error, never a thinner prompt: a transient database error on one source would
/// otherwise answer "nothing was found" from the rest. With no bound this is the unbounded call itself and
/// cannot fail.
#[allow(clippy::too_many_arguments)]
pub fn gather_and_assemble_context_scoped(
    store: &memory::Store,
    query: &str,
    now_unix_ms: i64,
    context_limit: usize,
    include_transcripts: bool,
    include_email: bool,
    filter: &ContextFilter,
    scope: &ScopeCtx<'_>,
) -> Result<Vec<ContextItem>, ReadError> {
    if scope.is_empty() {
        return Ok(crate::gather_and_assemble_context(store, query, now_unix_ms, context_limit, include_transcripts, include_email, filter));
    }
    let mut keyword_hits: Vec<(ContextItem, i64)> = Vec::new();
    keyword_hits.extend(store.scoped_capture_hits(query, false, scope, context_limit)?.into_iter().map(|c| {
        let ts = c.ts_unix_ms;
        (ContextItem::Capture(c), ts)
    }));
    if include_transcripts {
        keyword_hits.extend(store.scoped_transcript_hits(query, false, scope, context_limit)?.into_iter().map(|(t, ts)| (ContextItem::Transcript(t), ts)));
        keyword_hits.extend(store.scoped_snapshot_hits(query, false, scope, context_limit)?.into_iter().map(|(h, ts)| (ContextItem::Snapshot(h), ts)));
    }
    keyword_hits.extend(store.scoped_mcp_item_hits(query, false, scope, context_limit)?.into_iter().map(|a| {
        let ts = a.ts_unix_ms;
        (ContextItem::McpItem(a), ts)
    }));
    if include_email {
        keyword_hits.extend(store.scoped_email_hits(query, scope, context_limit)?.into_iter().map(|hit| {
            let ts = hit.ts_unix_ms;
            (ContextItem::Thread(thread_with_matching_email(hit)), ts)
        }));
    }

    let mut semantic_hits: Vec<(ContextItem, i64)> = Vec::new();
    semantic_hits.extend(store.scoped_capture_hits(query, true, scope, context_limit)?.into_iter().map(|c| {
        let ts = c.ts_unix_ms;
        (ContextItem::Capture(c), ts)
    }));
    if include_transcripts {
        semantic_hits.extend(store.scoped_transcript_hits(query, true, scope, context_limit)?.into_iter().map(|(t, ts)| (ContextItem::Transcript(t), ts)));
        semantic_hits.extend(store.scoped_snapshot_hits(query, true, scope, context_limit)?.into_iter().map(|(h, ts)| (ContextItem::Snapshot(h), ts)));
    }
    semantic_hits.extend(store.scoped_mcp_item_hits(query, true, scope, context_limit)?.into_iter().map(|a| {
        let ts = a.ts_unix_ms;
        (ContextItem::McpItem(a), ts)
    }));

    let routine_runs = store
        .scoped_routine_runs(scope, context_limit)?
        .into_iter()
        .map(|r| {
            let ts = r.ran_at_unix_ms;
            (ContextItem::RoutineRun(r), ts)
        })
        .collect();
    let episodes = store.scoped_episodes(scope, 10)?;
    let threads = store.scoped_waiting_threads(scope, 10)?;
    Ok(assemble_filtered(query, now_unix_ms, keyword_hits, semantic_hits, routine_runs, episodes, threads, filter, &unscoped))
}

/// [`crate::gather_and_assemble_context_for_window`] (with an empty query) for the window
/// `[start_unix_ms, end_unix_ms)` bounded by `scope`. The date filter wins: when `scope` has a start or an
/// end, retrieval reads the filter's range and the question's window is ignored; with none, the window
/// is used, narrowed by the app and person bounds. Fails, as above, when a read fails.
#[allow(clippy::too_many_arguments)]
pub fn gather_and_assemble_context_for_window_scoped(
    store: &memory::Store,
    now_unix_ms: i64,
    start_unix_ms: i64,
    end_unix_ms: i64,
    context_limit: usize,
    include_transcripts: bool,
    filter: &ContextFilter,
    scope: &ScopeCtx<'_>,
) -> Result<Vec<ContextItem>, ReadError> {
    if scope.is_empty() {
        return Ok(crate::gather_and_assemble_context_for_window_with_added(store, "", now_unix_ms, start_unix_ms, end_unix_ms, context_limit, include_transcripts, filter));
    }
    let eff = scope.or_window(start_unix_ms, end_unix_ms);
    let (start, end) = (eff.filter().start_unix_ms.unwrap_or(start_unix_ms), eff.filter().end_unix_ms.unwrap_or(end_unix_ms));
    if start >= end {
        return Ok(Vec::new());
    }
    let keyword_hits: Vec<(ContextItem, i64)> = store
        .scoped_timeline(&eff)?
        .into_iter()
        .rev() // newest first, as the unbounded window does, so the day's earliest hour does not win the budget
        .map(|c| {
            let ts = c.ts_unix_ms;
            (ContextItem::Capture(c), ts)
        })
        .collect();
    let mut semantic_hits: Vec<(ContextItem, i64)> = if include_transcripts {
        store.scoped_transcripts_in_window(&eff)?.into_iter().rev().map(|(t, ts)| (ContextItem::Transcript(t), ts)).collect()
    } else {
        Vec::new()
    };
    // Added items in the window, newest first; none under an app or a person bound.
    if eff.filter().allows(memory::find::FindKind::McpItem) {
        semantic_hits.extend(store.mcp_items_in_window(start, end, context_limit)?.into_iter().map(|a| {
            let ts = a.ts_unix_ms;
            (ContextItem::McpItem(a), ts)
        }));
    }
    let routine_runs = store
        .scoped_routine_runs(&eff, context_limit)?
        .into_iter()
        .map(|r| {
            let ts = r.ran_at_unix_ms;
            (ContextItem::RoutineRun(r), ts)
        })
        .collect();
    let episodes = store.scoped_episodes(&eff, 10)?;
    let threads = store.scoped_waiting_threads(&eff, 10)?;
    Ok(assemble_filtered("", now_unix_ms, keyword_hits, semantic_hits, routine_runs, episodes, threads, filter, &unscoped))
}
