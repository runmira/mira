//! Score-and-select memory retrieval.
//!
//! Replaces the historical "dump every MIRA.md file, verbatim, up to the
//! byte cap" injection with a scored + budgeted selection: entries are
//! ranked against the current turn's query text (recent conversation),
//! recency-weighted, and packed into a token budget with a floor that
//! always keeps the N most-recent episodic entries regardless of score.
//!
//! The scorer is BM25-lite over word tokens — deliberately no embeddings
//! or FTS index. It runs in-process on every round with O(entries × query
//! terms) work; on realistic memory sizes (hundreds of bullets) this is
//! well under a millisecond and buys us relevance-weighted context without
//! any new dependency or index-maintenance surface.
//!
//! The alternative "dump everything" path is preserved: callers that pass
//! `None` for a query get the old shape verbatim, which is what boot-time
//! and any test that doesn't want to reason about ranking should use.

use crate::episodic::EpisodicEntry;

/// Sensible token-budget default. Roughly 1500 tokens ≈ 6 KB of English
/// prose — enough for ~30 medium bullets, small enough that the model's
/// cache still hits on the fixed system prefix. Callers can override via
/// [`MemoryQuery::token_budget`] or the runtime config.
pub const DEFAULT_TOKEN_BUDGET: usize = 1500;

/// The floor of episodic entries kept regardless of score — the freshest
/// cross-session signal is almost always load-bearing, and demoting it
/// because the query text happens to share few terms is worse than
/// demoting a stale MIRA.md bullet.
pub const RECENT_EPISODIC_FLOOR: usize = 5;

/// Approximate token count for `s`. The 4-chars-per-token heuristic is
/// what OpenAI docs suggest for English; close enough for a soft budget.
pub fn approx_tokens(s: &str) -> usize {
    (s.chars().count() + 3) / 4
}

/// Where a candidate came from. Used both for provenance labelling in
/// the rendered block and for a small "curated wins over drift" score
/// bump: MIRA.md bullets are hand-written by the user, so we prefer
/// them slightly over auto-extracted episodic entries at the same
/// relevance level.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CandidateSource {
    /// `~/.mira/MIRA.md` — user-global, hand-curated.
    UserMd,
    /// `<cwd>/.mira/MIRA.md` — project-scoped, hand-curated.
    ProjectMd,
    /// `<cwd>/.mira/episodic.jsonl` — agent-written across sessions.
    Episodic,
}

impl CandidateSource {
    /// Extra multiplicative score bump given to curated (MIRA.md)
    /// entries over machine-written episodic entries at the same
    /// relevance. Small — ~15% — so a genuinely more-relevant episodic
    /// entry can still outrank an off-topic MIRA.md bullet.
    fn curated_bump(self) -> f32 {
        match self {
            Self::UserMd | Self::ProjectMd => 1.15,
            Self::Episodic => 1.0,
        }
    }
}

/// One entry considered for retrieval. `text` is the display form
/// (single line for bullets; the episodic entry's `text` field);
/// `timestamp` and `source` drive recency + provenance boosts.
#[derive(Clone, Debug)]
pub struct RetrievalCandidate {
    pub text: String,
    pub source: CandidateSource,
    /// Unix seconds. `None` for MIRA.md bullets (we don't track per-
    /// bullet mtime); recency scoring skips those entries.
    pub timestamp: Option<u64>,
}

impl RetrievalCandidate {
    pub fn new_md(text: impl Into<String>, source: CandidateSource) -> Self {
        debug_assert!(
            matches!(source, CandidateSource::UserMd | CandidateSource::ProjectMd),
            "new_md is for MIRA.md sources; use new_episodic for episodic entries"
        );
        Self {
            text: text.into(),
            source,
            timestamp: None,
        }
    }

    pub fn new_episodic(entry: &EpisodicEntry) -> Self {
        Self {
            text: entry.text.clone(),
            source: CandidateSource::Episodic,
            timestamp: Some(entry.timestamp),
        }
    }
}

/// A query for [`select_top_k`]. `context` is the free-text query
/// (usually the last user message plus a slice of the recent
/// assistant/tool turns concatenated together).
pub struct MemoryQuery {
    pub context: String,
    /// Token budget for the total rendered block. `None` → uses
    /// [`DEFAULT_TOKEN_BUDGET`].
    pub token_budget: Option<usize>,
    /// Wall-clock "now" for recency weighting. Injectable so tests
    /// can pin a deterministic clock instead of drifting with wall time.
    pub now_secs: u64,
}

impl MemoryQuery {
    pub fn budget(&self) -> usize {
        self.token_budget.unwrap_or(DEFAULT_TOKEN_BUDGET)
    }
}

/// Parse a MIRA.md file into one candidate per Markdown bullet.
///
/// Bullets can be multi-line (continuation lines indented). We fold
/// continuation lines back into the parent bullet so scoring sees the
/// whole thought, not a lonely `- ` line. Non-bullet paragraphs (the
/// file's header prose, if any) are collected as one candidate at the
/// end so they aren't silently dropped.
pub fn parse_md_bullets(content: &str, source: CandidateSource) -> Vec<RetrievalCandidate> {
    let mut out: Vec<RetrievalCandidate> = Vec::new();
    let mut current: Option<String> = None;
    let mut prose: String = String::new();

    for line in content.lines() {
        let trimmed_start = line.trim_start();
        // A new bullet — flush the previous one.
        if trimmed_start.starts_with("- ") || trimmed_start.starts_with("* ") {
            if let Some(prev) = current.take() {
                push_if_nonempty(&mut out, prev, source);
            }
            current = Some(trimmed_start[2..].trim().to_string());
            continue;
        }
        // Continuation of the current bullet (indented).
        if current.is_some() && (line.starts_with("  ") || line.starts_with('\t')) {
            let piece = trimmed_start.trim();
            if let Some(buf) = current.as_mut() {
                if !buf.is_empty() && !piece.is_empty() {
                    buf.push(' ');
                }
                buf.push_str(piece);
            }
            continue;
        }
        // Blank line closes the current bullet without opening a new one.
        if trimmed_start.is_empty() {
            if let Some(prev) = current.take() {
                push_if_nonempty(&mut out, prev, source);
            }
            continue;
        }
        // Everything else is prose — collect for the tail candidate.
        if !prose.is_empty() {
            prose.push(' ');
        }
        prose.push_str(trimmed_start.trim());
    }

    if let Some(prev) = current.take() {
        push_if_nonempty(&mut out, prev, source);
    }
    let prose = prose.trim().to_string();
    if !prose.is_empty() {
        out.push(RetrievalCandidate::new_md(prose, source));
    }

    out
}

fn push_if_nonempty(out: &mut Vec<RetrievalCandidate>, text: String, source: CandidateSource) {
    let trimmed = text.trim().to_string();
    if !trimmed.is_empty() {
        out.push(RetrievalCandidate::new_md(trimmed, source));
    }
}

/// Score and select the top candidates that fit within
/// `query.budget()`. Returns candidates in the order they should render
/// (highest-scoring first, but the [`RECENT_EPISODIC_FLOOR`] set is
/// inserted first regardless of score).
pub fn select_top_k(
    candidates: Vec<RetrievalCandidate>,
    query: &MemoryQuery,
) -> Vec<RetrievalCandidate> {
    if candidates.is_empty() {
        return Vec::new();
    }
    let budget = query.budget();

    // Pre-tokenize query and every candidate once. Cheap; keeps the
    // BM25 loop tight.
    let q_terms = tokenize(&query.context);
    let doc_terms: Vec<Vec<String>> = candidates.iter().map(|c| tokenize(&c.text)).collect();

    // Corpus stats for BM25's IDF: how many documents contain each
    // query term at least once.
    let n_docs = candidates.len();
    let avg_len: f32 = if n_docs == 0 {
        0.0
    } else {
        doc_terms.iter().map(|t| t.len()).sum::<usize>() as f32 / n_docs as f32
    };

    // Compute IDF only for query terms — the only ones that contribute.
    let mut idf: std::collections::HashMap<&str, f32> =
        std::collections::HashMap::with_capacity(q_terms.len());
    for term in &q_terms {
        if idf.contains_key(term.as_str()) {
            continue;
        }
        let mut df = 0usize;
        for terms in &doc_terms {
            if terms.iter().any(|t| t == term) {
                df += 1;
            }
        }
        // BM25's smoothed IDF. Clamped at 0 so a query term that
        // appears in every document doesn't push scores negative.
        let n = n_docs as f32;
        let df = df as f32;
        let raw = ((n - df + 0.5) / (df + 0.5) + 1.0).ln();
        idf.insert(term.as_str(), raw.max(0.0));
    }

    // Score each candidate.
    const K1: f32 = 1.2;
    const B: f32 = 0.75;
    let mut scored: Vec<(usize, f32)> = candidates
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let terms = &doc_terms[i];
            let doc_len = terms.len() as f32;
            let mut base = 0.0f32;
            for term in &q_terms {
                let tf = terms.iter().filter(|t| *t == term).count() as f32;
                if tf <= 0.0 {
                    continue;
                }
                let idf_w = *idf.get(term.as_str()).unwrap_or(&0.0);
                let denom = tf + K1 * (1.0 - B + B * doc_len / avg_len.max(1.0));
                base += idf_w * tf * (K1 + 1.0) / denom;
            }
            // Recency multiplier for episodic entries.
            let recency = recency_multiplier(c.timestamp, query.now_secs);
            // Curated bump.
            let curated = c.source.curated_bump();
            // Small floor so BM25 zero doesn't zero the whole score —
            // recency + curated still nudge tie-breakers.
            let score = (base + 0.001) * recency * curated;
            (i, score)
        })
        .collect();

    // Sort by score descending. Stable so ties preserve source order.
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    // Always-include: the N most-recent episodic entries by timestamp,
    // taken *before* score-based selection so their spots are guaranteed.
    let mut recent_episodic_idx: Vec<(usize, u64)> = candidates
        .iter()
        .enumerate()
        .filter_map(|(i, c)| match c.source {
            CandidateSource::Episodic => c.timestamp.map(|ts| (i, ts)),
            _ => None,
        })
        .collect();
    recent_episodic_idx.sort_by(|a, b| b.1.cmp(&a.1));
    let mut must_include: std::collections::HashSet<usize> = recent_episodic_idx
        .into_iter()
        .take(RECENT_EPISODIC_FLOOR)
        .map(|(i, _)| i)
        .collect();

    // Greedy pack: floor entries first (in recency-desc order), then
    // scored entries until we run out of budget.
    let mut used_tokens = 0usize;
    let mut picked_indices: Vec<usize> = Vec::new();

    // Floor first: iterate scored (so ordering follows the score sort)
    // but only include floor members in this pass.
    for (i, _) in &scored {
        if !must_include.remove(i) {
            continue;
        }
        let cost = approx_tokens(&candidates[*i].text) + 1; // +1 for "- " overhead
        if used_tokens + cost > budget && !picked_indices.is_empty() {
            // Even floor entries respect the budget once at least one
            // is in — otherwise we could pathologically exceed by 100x
            // on tiny budgets.
            break;
        }
        used_tokens += cost;
        picked_indices.push(*i);
    }

    // Then scored fill.
    for (i, score) in &scored {
        if picked_indices.contains(i) {
            continue;
        }
        // Skip zero-signal entries — including "just floor" boosts only.
        if *score <= 0.001 {
            // Actually keep them if the budget is otherwise wide open;
            // this avoids dropping every entry when there's no query
            // overlap at all (e.g., first turn of a session).
            if used_tokens > budget / 2 {
                continue;
            }
        }
        let cost = approx_tokens(&candidates[*i].text) + 1;
        if used_tokens + cost > budget {
            continue;
        }
        used_tokens += cost;
        picked_indices.push(*i);
    }

    // Re-sort the final selection by original source order + score so
    // the rendered block reads coherently (user before project before
    // episodic; within each group, higher-scoring first).
    let mut final_out: Vec<RetrievalCandidate> = picked_indices
        .into_iter()
        .map(|i| candidates[i].clone())
        .collect();
    final_out.sort_by_key(|c| source_order(c.source));
    final_out
}

fn source_order(s: CandidateSource) -> u8 {
    match s {
        CandidateSource::UserMd => 0,
        CandidateSource::ProjectMd => 1,
        CandidateSource::Episodic => 2,
    }
}

/// Exponential-decay recency boost. Half-life ≈ 30 days; a fresh entry
/// scores ~2.0, a month-old one ~1.5, three months ~1.125, then trails
/// off. Entries without a timestamp get a flat 1.0 (no boost, no penalty).
fn recency_multiplier(ts: Option<u64>, now: u64) -> f32 {
    let Some(ts) = ts else {
        return 1.0;
    };
    let age_secs = now.saturating_sub(ts) as f32;
    let age_days = age_secs / 86_400.0;
    let half_life_days = 30.0;
    let decay = (0.5f32).powf(age_days / half_life_days);
    1.0 + decay
}

/// Lowercase, split on non-alphanumeric, drop empties + one-character
/// tokens. Deliberately no stemming or stopword list — the extra
/// dependency (and language assumptions) isn't worth the marginal
/// accuracy on short queries.
fn tokenize(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut buf = String::new();
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            for l in c.to_lowercase() {
                buf.push(l);
            }
        } else {
            if buf.len() > 1 {
                out.push(std::mem::take(&mut buf));
            } else {
                buf.clear();
            }
        }
    }
    if buf.len() > 1 {
        out.push(buf);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::episodic::EpisodicSource;

    fn md(text: &str, source: CandidateSource) -> RetrievalCandidate {
        RetrievalCandidate::new_md(text, source)
    }

    fn ep(text: &str, ts: u64) -> RetrievalCandidate {
        RetrievalCandidate {
            text: text.to_string(),
            source: CandidateSource::Episodic,
            timestamp: Some(ts),
        }
    }

    fn q(context: &str, budget: usize, now: u64) -> MemoryQuery {
        MemoryQuery {
            context: context.into(),
            token_budget: Some(budget),
            now_secs: now,
        }
    }

    #[test]
    fn tokenize_basics() {
        assert_eq!(tokenize("Hello, world!"), vec!["hello", "world"]);
        // One-char tokens dropped; case folded; digits kept.
        assert_eq!(tokenize("a AB c123 x"), vec!["ab", "c123"]);
    }

    #[test]
    fn parse_bullets_single_line() {
        let c = parse_md_bullets(
            "- first bullet\n- second bullet\n",
            CandidateSource::ProjectMd,
        );
        assert_eq!(c.len(), 2);
        assert_eq!(c[0].text, "first bullet");
        assert_eq!(c[1].text, "second bullet");
    }

    #[test]
    fn parse_bullets_folds_continuations() {
        // Continuation lines (indented) fold into the parent bullet so
        // scoring sees the whole thought.
        let c = parse_md_bullets(
            "- first line\n  continuation\n  more\n- second\n",
            CandidateSource::UserMd,
        );
        assert_eq!(c.len(), 2);
        assert_eq!(c[0].text, "first line continuation more");
        assert_eq!(c[1].text, "second");
    }

    #[test]
    fn parse_bullets_captures_trailing_prose() {
        // Non-bullet paragraphs shouldn't be silently dropped.
        let c = parse_md_bullets(
            "Free-form intro paragraph.\n- a bullet\n",
            CandidateSource::UserMd,
        );
        assert_eq!(c.len(), 2);
        assert_eq!(c[0].text, "a bullet");
        assert_eq!(c[1].text, "Free-form intro paragraph.");
    }

    #[test]
    fn scoring_prefers_overlap() {
        // The bullet mentioning "pnpm" should win over the unrelated one.
        let bullets = vec![
            md("we use pnpm not npm", CandidateSource::ProjectMd),
            md("dark mode toggle sits in the header", CandidateSource::ProjectMd),
        ];
        let sel = select_top_k(bullets, &q("pnpm install failed", 1000, 1_700_000_000));
        assert_eq!(sel.len(), 2); // both fit
        assert_eq!(sel[0].text, "we use pnpm not npm");
    }

    #[test]
    fn budget_drops_lowest_scoring() {
        // Two bullets, budget tight enough for only one. The relevant
        // one to the query wins.
        let bullets = vec![
            md("we use pnpm not npm", CandidateSource::ProjectMd),
            md("dark mode toggle in header", CandidateSource::ProjectMd),
        ];
        // "we use pnpm not npm" = 19 chars ≈ 5 tokens + 1 overhead = 6
        // "dark mode toggle in header" = 26 chars ≈ 7 + 1 = 8
        // Budget = 6 → only the first fits.
        let sel = select_top_k(bullets, &q("pnpm install failed", 6, 1_700_000_000));
        assert_eq!(sel.len(), 1);
        assert_eq!(sel[0].text, "we use pnpm not npm");
    }

    #[test]
    fn recency_boosts_episodic_entries() {
        // Two episodic entries, same textual relevance. The newer one
        // wins because of the recency multiplier.
        let now: u64 = 1_700_000_000;
        let one_day = 86_400u64;
        let entries = vec![
            ep("cargo test flake happens sometimes", now - 90 * one_day),
            ep("cargo test flake happens sometimes", now - one_day),
        ];
        // Budget only fits one.
        let cost_each = approx_tokens("cargo test flake happens sometimes") + 1;
        let sel = select_top_k(entries.clone(), &q("cargo test", cost_each, now));
        // Because both have identical text the query scores are equal;
        // the *sort* is stable, but our recency multiplier pulls the
        // newer one to the top. The picked entry's timestamp should be
        // the newer one.
        assert_eq!(sel.len(), 1);
        assert_eq!(sel[0].timestamp, Some(now - one_day));
    }

    #[test]
    fn always_include_recent_episodic_floor() {
        // Five recent episodic entries + one MIRA.md entry that's very
        // relevant. Budget generous — everything should fit and the
        // floor guarantees the episodic entries survive even with zero
        // query overlap.
        let now: u64 = 1_700_000_000;
        let mut candidates: Vec<RetrievalCandidate> = (0..RECENT_EPISODIC_FLOOR as u64)
            .map(|i| ep(&format!("episodic {i}"), now - i * 86_400))
            .collect();
        candidates.push(md(
            "user memory: we use pnpm",
            CandidateSource::UserMd,
        ));
        let sel = select_top_k(candidates.clone(), &q("pnpm build", 500, now));
        // All RECENT_EPISODIC_FLOOR + the MIRA.md entry should be in.
        assert_eq!(sel.len(), RECENT_EPISODIC_FLOOR + 1);
    }

    #[test]
    fn empty_candidates_returns_empty() {
        let sel = select_top_k(Vec::new(), &q("anything", 1000, 0));
        assert!(sel.is_empty());
    }

    #[test]
    fn ordering_puts_user_then_project_then_episodic() {
        let now: u64 = 1_700_000_000;
        let candidates = vec![
            ep("episodic A", now),
            md("project A", CandidateSource::ProjectMd),
            md("user A", CandidateSource::UserMd),
        ];
        let sel = select_top_k(candidates, &q("A", 1000, now));
        assert_eq!(sel[0].source, CandidateSource::UserMd);
        assert_eq!(sel[1].source, CandidateSource::ProjectMd);
        assert_eq!(sel[2].source, CandidateSource::Episodic);
    }

    #[test]
    fn from_episodic_entry_helper() {
        let e = EpisodicEntry {
            text: "hi".into(),
            timestamp: 1234,
            session_id: None,
            source: EpisodicSource::Tool,
        };
        let c = RetrievalCandidate::new_episodic(&e);
        assert_eq!(c.text, "hi");
        assert_eq!(c.source, CandidateSource::Episodic);
        assert_eq!(c.timestamp, Some(1234));
    }
}
