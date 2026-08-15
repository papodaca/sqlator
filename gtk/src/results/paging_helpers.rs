//! Pure helpers for paged scrolling in [`super::grid::ResultsGrid`] (plan U2).
//!
//! Kept widget-free so the threshold math and the de-bounce guard are
//! testable without a GTK main loop (the `EditState` tier-1 precedent).

/// Hard display ceiling for paged query results (KTD-5). Keeping the copy
/// local to GTK avoids a core dependency for a display string; the engine
/// enforces the same 50,000 constant server-side.
pub const ROW_LIMIT_DISPLAY: &str = "50,000 row limit reached";

/// Paging state of the loaded rows; drives the status-line copy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PagedStatus {
    /// More rows are available server-side.
    More,
    /// All rows have been loaded.
    Exhausted,
    /// The 50,000-row engine ceiling was hit (see [`ROW_LIMIT_DISPLAY`]).
    Capped,
}

impl PagedStatus {
    /// Status-line part appended after the row count.
    pub fn status_part(&self) -> String {
        match self {
            Self::More => "more rows available".to_string(),
            Self::Exhausted => "all rows loaded".to_string(),
            Self::Capped => ROW_LIMIT_DISPLAY.to_string(),
        }
    }
}

/// Default trigger distance: the viewport's bottom edge is "near" the content
/// end once it comes within one viewport height (one page) of it (KTD-6).
///
/// `value`, `page_size`, and `upper` are the vadjustment properties. The
/// signal is meaningful only when the content exceeds the viewport
/// (`upper > page_size`); a shorter page is the owning tab's viewport-fill
/// concern (U3), never a near-bottom event. An unallocated viewport
/// (`page_size <= 0`) never fires. The boundary is inclusive.
pub fn near_bottom(value: f64, page_size: f64, upper: f64) -> bool {
    if page_size <= 0.0 || upper <= page_size {
        return false;
    }
    value + page_size >= upper - page_size
}

/// One-shot-per-crossing latch over [`near_bottom`].
///
/// Stationary-at-bottom adjustment notifications fire at most one callback;
/// the latch re-arms when the content grows (a new `upper`) or the view
/// scrolls out of the trigger zone. [`Self::reset`] re-arms unconditionally
/// (grid `clear()` / a new result shape).
#[derive(Debug, Default)]
pub struct NearBottomGuard {
    fired: bool,
    last_upper: f64,
}

impl NearBottomGuard {
    /// Evaluate one adjustment notification; `true` means "fire the callback".
    pub fn evaluate(&mut self, value: f64, page_size: f64, upper: f64) -> bool {
        let grew = upper > self.last_upper;
        self.last_upper = upper;
        if !near_bottom(value, page_size, upper) {
            // Scrolled out of the trigger zone (or content shrank below the
            // viewport): re-arm without firing.
            self.fired = false;
            return false;
        }
        if grew {
            // Content grew (a chunk was appended): re-arm for the new page.
            self.fired = false;
        }
        if self.fired {
            return false;
        }
        self.fired = true;
        true
    }

    /// Unconditionally re-arm (called from grid `clear()` / `begin_columns()`).
    pub fn reset(&mut self) {
        self.fired = false;
        self.last_upper = 0.0;
    }
}

/// Render the paging-aware status text: "<loaded> rows · <state part>".
/// Column counts are appended by the grid (they live in the widget).
pub fn paged_status_text(loaded: usize, status: PagedStatus) -> String {
    format!("{loaded} rows · {}", status.status_part())
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: f64 = 100.0;

    #[test]
    fn near_bottom_triggers_within_one_page_of_end() {
        // Content is 10 pages tall; scrolled so the bottom edge is 0.5 pages
        // from the end — inside the one-page trigger distance.
        let upper = 10.0 * PAGE;
        assert!(near_bottom(8.5 * PAGE, PAGE, upper));
    }

    #[test]
    fn near_bottom_does_not_fire_mid_list() {
        let upper = 10.0 * PAGE;
        assert!(!near_bottom(4.0 * PAGE, PAGE, upper));
    }

    #[test]
    fn near_bottom_not_meaningful_when_content_shorter_than_viewport() {
        // Content fits in the viewport: the tab fills it via its own logic
        // (U3); the scroll signal must stay silent.
        assert!(!near_bottom(0.0, PAGE, PAGE));
        assert!(!near_bottom(0.0, PAGE, 0.5 * PAGE));
        // Boundary: upper exactly equal to page_size is "fits", not near-bottom.
        assert!(!near_bottom(0.0, PAGE, PAGE));
    }

    #[test]
    fn near_bottom_boundary_is_inclusive_at_trigger_distance() {
        let upper = 10.0 * PAGE;
        // Bottom edge exactly one page from the content end fires (inclusive).
        assert!(near_bottom(8.0 * PAGE, PAGE, upper));
        // Any epsilon short of the trigger distance does not.
        assert!(!near_bottom(8.0 * PAGE - 0.5, PAGE, upper));
    }

    #[test]
    fn near_bottom_fires_from_top_when_content_is_under_two_pages() {
        // 1.5 pages of content, user has not scrolled: the bottom edge is
        // already within one page of the end. The boundary `upper > page_size`
        // is exclusive (fits-viewport is the tab's concern), anything over it
        // is eligible.
        assert!(near_bottom(0.0, PAGE, PAGE + 1.0));
        assert!(near_bottom(0.0, PAGE, 1.5 * PAGE));
        assert!(!near_bottom(0.0, PAGE, PAGE));
    }

    #[test]
    fn near_bottom_false_when_viewport_unallocated() {
        // Before realization the adjustment reports page_size 0 — never fire.
        assert!(!near_bottom(0.0, 0.0, 0.0));
        assert!(!near_bottom(0.0, 0.0, 1000.0));
    }

    fn parked_at_bottom(guard: &mut NearBottomGuard, upper: f64) -> bool {
        guard.evaluate(9.0 * PAGE, PAGE, upper)
    }

    #[test]
    fn guard_fires_once_when_stationary_at_bottom() {
        let upper = 10.0 * PAGE;
        let mut guard = NearBottomGuard::default();
        assert!(parked_at_bottom(&mut guard, upper));
        // Repeated notifications with unchanged adjustment do not re-fire.
        for _ in 0..5 {
            assert!(!parked_at_bottom(&mut guard, upper));
        }
    }

    #[test]
    fn guard_rearms_when_content_grows() {
        let upper = 10.0 * PAGE;
        let mut guard = NearBottomGuard::default();
        assert!(parked_at_bottom(&mut guard, upper));
        assert!(!parked_at_bottom(&mut guard, upper));
        // A page was appended: the latch must allow exactly one more fire.
        let grown = 11.0 * PAGE;
        assert!(parked_at_bottom(&mut guard, grown));
        assert!(!parked_at_bottom(&mut guard, grown));
    }

    #[test]
    fn guard_rearms_after_scrolling_away_and_back() {
        let upper = 10.0 * PAGE;
        let mut guard = NearBottomGuard::default();
        assert!(parked_at_bottom(&mut guard, upper));
        // Scroll out of the trigger zone — re-arms without firing.
        assert!(!guard.evaluate(4.0 * PAGE, PAGE, upper));
        assert!(!guard.evaluate(4.0 * PAGE, PAGE, upper));
        // Back at the bottom: fires once more, then latches again.
        assert!(parked_at_bottom(&mut guard, upper));
        assert!(!parked_at_bottom(&mut guard, upper));
    }

    #[test]
    fn guard_reset_rearms() {
        let upper = 10.0 * PAGE;
        let mut guard = NearBottomGuard::default();
        assert!(parked_at_bottom(&mut guard, upper));
        guard.reset();
        // New run with identical content size must fire again.
        assert!(parked_at_bottom(&mut guard, upper));
    }

    #[test]
    fn guard_never_fires_when_content_fits_viewport() {
        let mut guard = NearBottomGuard::default();
        for _ in 0..3 {
            assert!(!guard.evaluate(0.0, PAGE, 0.5 * PAGE));
        }
    }

    #[test]
    fn guard_shrink_to_fit_rearms_via_scroll_away() {
        // What clear() does to the adjustment: content shrinks below the
        // viewport, which reads as "scrolled away" and so re-arms the latch;
        // an explicit reset() is the belt-and-braces for synchronous refills.
        let upper = 10.0 * PAGE;
        let mut guard = NearBottomGuard::default();
        assert!(parked_at_bottom(&mut guard, upper));
        assert!(!guard.evaluate(0.0, PAGE, PAGE));
        // Refilled to the same upper: fires again without an explicit reset.
        assert!(parked_at_bottom(&mut guard, upper));
    }

    #[test]
    fn paged_status_text_renders_more_state() {
        assert_eq!(
            paged_status_text(500, PagedStatus::More),
            "500 rows · more rows available"
        );
    }

    #[test]
    fn paged_status_text_renders_exhausted_state() {
        assert_eq!(
            paged_status_text(1_250, PagedStatus::Exhausted),
            "1250 rows · all rows loaded"
        );
    }

    #[test]
    fn paged_status_text_renders_capped_state_with_exact_limit_message() {
        let text = paged_status_text(50_000, PagedStatus::Capped);
        assert_eq!(text, "50000 rows · 50,000 row limit reached");
        // R5: the ceiling message wording is pinned exactly.
        assert_eq!(PagedStatus::Capped.status_part(), ROW_LIMIT_DISPLAY);
    }
}
