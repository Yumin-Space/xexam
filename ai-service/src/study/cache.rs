//! The record-identity cache: which chat a saved record belongs to (record key, kind, time).
//!
//! Keyed by (user id, user stamp, record id): a user whose id was reused by another account (another stamp) never reads
//! the old entry. At most [`MAX_ENTRIES`] entries (the oldest is dropped), each valid for [`TTL`]. A `notFound` answer
//! is never stored (the caller simply does not call `put`). It only saves backend calls for the 2-second polling of
//! `GET /api/ai/study/saved?record=`; it holds no text of a question or an answer.
use crate::study::wire::RecordIdent;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Entries kept.
pub const MAX_ENTRIES: usize = 1_000;
/// How long an entry is used.
pub const TTL: Duration = Duration::from_secs(10 * 60);

type Key = (i64, String, i64);

struct Entry {
    rec: RecordIdent,
    at: Instant,
    seq: u64,
}

#[derive(Default)]
struct Inner {
    next: u64,
    map: HashMap<Key, Entry>,
}

/// The cache. Cheap to lock; never held across an await.
#[derive(Default)]
pub struct RecordCache {
    inner: Mutex<Inner>,
}

impl RecordCache {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The cached identity of a record, unless it expired.
    pub fn get(&self, user: i64, stamp: &str, record: i64) -> Option<RecordIdent> {
        self.get_at(user, stamp, record, Instant::now())
    }

    /// Stores (or replaces) the identity of a record.
    pub fn put(&self, user: i64, stamp: &str, rec: &RecordIdent) {
        self.put_at(user, stamp, rec, Instant::now());
    }

    /// Number of entries (tests).
    pub fn len(&self) -> usize {
        self.lock().map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn get_at(&self, user: i64, stamp: &str, record: i64, now: Instant) -> Option<RecordIdent> {
        let key = (user, stamp.to_string(), record);
        let mut inner = self.lock();
        match inner.map.get(&key) {
            Some(e) if now.saturating_duration_since(e.at) < TTL => Some(e.rec.clone()),
            Some(_) => {
                inner.map.remove(&key);
                None
            }
            None => None,
        }
    }

    fn put_at(&self, user: i64, stamp: &str, rec: &RecordIdent, now: Instant) {
        let key = (user, stamp.to_string(), rec.id);
        let mut inner = self.lock();
        inner.next += 1;
        let seq = inner.next;
        inner.map.insert(
            key,
            Entry {
                rec: rec.clone(),
                at: now,
                seq,
            },
        );
        while inner.map.len() > MAX_ENTRIES {
            let oldest = inner
                .map
                .iter()
                .min_by_key(|(_, e)| e.seq)
                .map(|(k, _)| k.clone());
            match oldest {
                Some(k) => {
                    inner.map.remove(&k);
                }
                None => break,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::study::summary::RecordKind;

    fn rec(id: i64, at: &str) -> RecordIdent {
        RecordIdent {
            id,
            at: at.to_string(),
            kind: RecordKind::Practice,
            key: Some("0123456789abcdef0123456789abcdef".to_string()),
            started_at: Some(1_700_000_000),
        }
    }

    #[test]
    fn put_get_replace_and_stamp() {
        let c = RecordCache::default();
        assert!(c.get(1, "s", 5).is_none());
        c.put(1, "s", &rec(5, "2026-01-02 03:04"));
        assert_eq!(
            c.get(1, "s", 5).map(|r| r.at),
            Some("2026-01-02 03:04".into())
        );
        // another stamp (a reused user id) or another user never reads it
        assert!(c.get(1, "other", 5).is_none());
        assert!(c.get(2, "s", 5).is_none());
        // a put replaces: the new time wins
        c.put(1, "s", &rec(5, "2026-02-02 03:04"));
        assert_eq!(
            c.get(1, "s", 5).map(|r| r.at),
            Some("2026-02-02 03:04".into())
        );
        assert_eq!(c.len(), 1);
    }

    #[test]
    fn expires_after_the_ttl() {
        let c = RecordCache::default();
        let t0 = Instant::now();
        c.put_at(1, "s", &rec(5, "2026-01-02 03:04"), t0);
        assert!(c
            .get_at(1, "s", 5, t0 + TTL - Duration::from_secs(1))
            .is_some());
        assert!(c.get_at(1, "s", 5, t0 + TTL).is_none());
        assert!(c.is_empty(), "an expired entry is removed when it is read");
    }

    #[test]
    fn keeps_at_most_a_thousand_and_drops_the_oldest() {
        let c = RecordCache::default();
        for id in 1..=(MAX_ENTRIES as i64 + 5) {
            c.put(1, "s", &rec(id, "2026-01-02 03:04"));
        }
        assert_eq!(c.len(), MAX_ENTRIES);
        for id in 1..=5 {
            assert!(c.get(1, "s", id).is_none(), "{id}");
        }
        assert!(c.get(1, "s", 6).is_some());
        assert!(c.get(1, "s", MAX_ENTRIES as i64 + 5).is_some());
    }
}
