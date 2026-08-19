//! Pure paging state machine for the query tab (KTD-7 / U3).
//!
//! Widget glue stays in [`super::QueryTab`]; this struct is unit-tested like
//! [`crate::results::EditState`].

use sqlator_core::db::{PagedQueryOutcome, PAGED_ROW_CEILING};
use sqlator_core::models::SortSpec;

/// First page and every subsequent chunk (KTD-4).
pub(crate) const PAGE_SIZE: usize = 500;

/// Query-tab paging state. No GTK types.
#[derive(Debug, Clone, Default)]
pub(crate) struct QueryPaging {
    sql: String,
    sort: Vec<SortSpec>,
    offset: usize,
    has_more: bool,
    capped: bool,
    in_flight: bool,
    /// Wrappable paged run (vs passthrough). Unknown until page one completes.
    paged: bool,
    /// Page one has finished; near-bottom may fire. Reset on `begin`.
    armed: bool,
}

impl QueryPaging {
    pub(crate) fn begin(&mut self, sql: impl Into<String>, sort: Vec<SortSpec>) {
        self.sql = sql.into();
        self.sort = sort;
        self.offset = 0;
        self.has_more = false;
        self.capped = false;
        self.in_flight = false;
        self.paged = false;
        self.armed = false;
    }

    #[allow(dead_code)]
    pub(crate) fn reset(&mut self) {
        *self = Self::default();
    }

    pub(crate) fn sort_changed(&mut self, sort: Vec<SortSpec>) {
        if self.in_flight {
            // In-flight work is dropped by the tab generation guard; keep
            // this snapshot stable until that happens.
            return;
        }
        let sql = self.sql.clone();
        self.begin(sql, sort);
    }

    pub(crate) fn page_started(&mut self) {
        self.in_flight = true;
    }

    pub(crate) fn page_completed(&mut self, outcome: PagedQueryOutcome) {
        self.in_flight = false;
        self.armed = true;
        self.paged = outcome.paged;
        if !outcome.paged {
            self.has_more = false;
            self.capped = false;
            return;
        }
        self.offset = self.offset.saturating_add(outcome.row_count);
        self.capped = outcome.capped || self.offset >= PAGED_ROW_CEILING;
        self.has_more = outcome.has_more && !self.capped;
    }

    /// A failed page fetch returns to ready-with-more so a later scroll retries.
    /// Page-one failure stays unarmed (no retry-on-scroll of a broken first run).
    pub(crate) fn page_failed(&mut self) {
        self.in_flight = false;
        if self.armed && self.paged {
            self.has_more = true;
        }
    }

    pub(crate) fn should_trigger(&self) -> bool {
        self.paged && self.armed && self.has_more && !self.capped && !self.in_flight
    }

    /// After a chunk, keep fetching while the grid still fits the viewport.
    pub(crate) fn should_fill_viewport(&self, content_fits: bool) -> bool {
        content_fits && self.should_trigger()
    }

    pub(crate) fn sql(&self) -> &str {
        &self.sql
    }

    pub(crate) fn sort(&self) -> &[SortSpec] {
        &self.sort
    }

    pub(crate) fn offset(&self) -> usize {
        self.offset
    }

    pub(crate) fn is_paged(&self) -> bool {
        self.paged
    }

    pub(crate) fn has_more(&self) -> bool {
        self.has_more
    }

    pub(crate) fn capped(&self) -> bool {
        self.capped
    }

    pub(crate) fn in_flight(&self) -> bool {
        self.in_flight
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paged_ok(row_count: usize, has_more: bool, capped: bool) -> PagedQueryOutcome {
        PagedQueryOutcome {
            row_count,
            has_more,
            capped,
            paged: true,
        }
    }

    fn passthrough() -> PagedQueryOutcome {
        PagedQueryOutcome::passthrough()
    }

    fn spec(col: &str, desc: bool) -> SortSpec {
        SortSpec {
            column: col.to_string(),
            desc,
        }
    }

    #[test]
    fn begin_resets_offset_sort_and_blocks_trigger_until_page_one() {
        let mut p = QueryPaging::default();
        p.begin("SELECT 1", vec![spec("id", false)]);
        p.page_started();
        p.page_completed(paged_ok(500, true, false));
        assert!(p.should_trigger());

        p.begin("SELECT 2", vec![]);
        assert_eq!(p.sql(), "SELECT 2");
        assert!(p.sort().is_empty());
        assert_eq!(p.offset(), 0);
        assert!(!p.should_trigger());
        assert!(!p.in_flight());
    }

    #[test]
    fn should_trigger_false_while_in_flight_exhausted_capped_or_unpaged() {
        let mut p = QueryPaging::default();
        // Unpaged default.
        assert!(!p.should_trigger());

        p.begin("SELECT * FROM t", vec![]);
        p.page_started();
        assert!(!p.should_trigger());
        p.page_completed(paged_ok(500, true, false));
        assert!(p.should_trigger());

        p.page_started();
        assert!(!p.should_trigger());
        p.page_completed(paged_ok(250, false, false));
        assert!(!p.should_trigger()); // exhausted

        p.begin("SELECT * FROM t", vec![]);
        p.page_started();
        p.page_completed(paged_ok(500, true, true));
        assert!(!p.should_trigger()); // capped

        p.begin("INSERT INTO t VALUES (1)", vec![]);
        p.page_started();
        p.page_completed(passthrough());
        assert!(!p.should_trigger()); // passthrough
        assert!(!p.is_paged());
    }

    #[test]
    fn page_completed_advances_offset_and_flags() {
        let mut p = QueryPaging::default();
        p.begin("SELECT * FROM t", vec![]);
        p.page_started();
        p.page_completed(paged_ok(500, true, false));
        assert_eq!(p.offset(), 500);
        assert!(p.has_more());
        assert!(!p.capped());

        p.page_started();
        p.page_completed(paged_ok(500, false, false));
        assert_eq!(p.offset(), 1000);
        assert!(!p.has_more());

        p.begin("SELECT * FROM t", vec![]);
        p.page_started();
        p.page_completed(paged_ok(200, true, true));
        assert!(p.capped());
        assert!(!p.has_more());
        assert!(!p.should_trigger());
    }

    #[test]
    fn failed_page_fetch_returns_to_ready_with_more() {
        let mut p = QueryPaging::default();
        p.begin("SELECT * FROM t", vec![]);
        p.page_started();
        p.page_completed(paged_ok(500, true, false));
        p.page_started();
        p.page_failed();
        assert!(!p.in_flight());
        assert!(p.should_trigger());
        assert_eq!(p.offset(), 500);
    }

    #[test]
    fn failed_page_one_does_not_arm_trigger() {
        let mut p = QueryPaging::default();
        p.begin("SELECT * FROM t", vec![]);
        p.page_started();
        p.page_failed();
        assert!(!p.should_trigger());
    }

    #[test]
    fn sort_change_resets_offset_and_blocks_until_page_one() {
        let mut p = QueryPaging::default();
        p.begin("SELECT * FROM t", vec![]);
        p.page_started();
        p.page_completed(paged_ok(500, true, false));
        assert_eq!(p.offset(), 500);

        p.sort_changed(vec![spec("name", true)]);
        assert_eq!(p.sort()[0].column, "name");
        assert!(p.sort()[0].desc);
        assert_eq!(p.sql(), "SELECT * FROM t");
        assert_eq!(p.offset(), 0);
        assert!(!p.should_trigger());
    }

    #[test]
    fn sort_change_ignored_while_in_flight() {
        let mut p = QueryPaging::default();
        p.begin("SELECT * FROM t", vec![]);
        p.page_started();
        p.sort_changed(vec![spec("id", false)]);
        assert!(p.sort().is_empty());
        assert!(p.in_flight());
    }

    #[test]
    fn trigger_after_reset_waits_for_page_one() {
        let mut p = QueryPaging::default();
        p.begin("SELECT * FROM t", vec![]);
        p.page_started();
        p.page_completed(paged_ok(500, true, false));
        p.reset();
        assert!(!p.should_trigger());
        p.begin("SELECT * FROM t", vec![]);
        assert!(!p.should_trigger());
        p.page_started();
        p.page_completed(paged_ok(500, true, false));
        assert!(p.should_trigger());
    }

    #[test]
    fn viewport_fill_requests_another_page_then_stops() {
        let mut p = QueryPaging::default();
        p.begin("SELECT * FROM t", vec![]);
        p.page_started();
        p.page_completed(paged_ok(50, true, false));
        assert!(p.should_fill_viewport(true));
        assert!(!p.should_fill_viewport(false));

        p.page_started();
        p.page_completed(paged_ok(50, false, false));
        assert!(!p.should_fill_viewport(true));
    }
}
