use std::time::Duration;

use statlet::runtime_schedule::{RedrawRequest, RuntimeSchedule};

fn millis(value: u64) -> Duration {
    Duration::from_millis(value)
}

#[test]
fn burst_keeps_the_first_frame_deadline_and_latest_save_document() {
    let mut schedule = RuntimeSchedule::new();

    schedule.request_redraw(millis(0), RedrawRequest::paint());
    schedule.queue_save(millis(0), "first");
    schedule.request_redraw(millis(5), RedrawRequest::paint());
    schedule.queue_save(millis(5), "second");
    schedule.request_redraw(millis(10), RedrawRequest::semantic_colors());
    schedule.queue_save(millis(10), "latest");

    assert_eq!(schedule.redraw_deadline(), Some(millis(16)));
    assert_eq!(schedule.save_deadline(), Some(millis(310)));
    assert_eq!(schedule.take_due_redraw(millis(15)), None);
    assert_eq!(
        schedule.take_due_redraw(millis(16)),
        Some(RedrawRequest::semantic_colors())
    );
    assert_eq!(schedule.take_due_redraw(millis(16)), None);
    assert_eq!(schedule.due_save(millis(309)), None);
    assert_eq!(schedule.due_save(millis(310)), Some("latest"));
}

#[test]
fn continuous_edits_flush_no_later_than_two_seconds_after_the_first_change() {
    let mut schedule = RuntimeSchedule::new();

    schedule.queue_save(millis(0), 0);
    for time in [250, 500, 750, 1_000, 1_250, 1_500, 1_750, 1_990] {
        schedule.queue_save(millis(time), time);
    }

    assert_eq!(schedule.save_deadline(), Some(millis(2_000)));
    assert_eq!(schedule.due_save(millis(1_999)), None);
    assert_eq!(schedule.due_save(millis(2_000)), Some(1_990));
}

#[test]
fn failed_attempt_and_later_change_never_retry_the_stale_snapshot() {
    let mut schedule = RuntimeSchedule::new();

    schedule.queue_save(millis(0), "old");
    let attempted = schedule.due_save(millis(300)).unwrap();
    schedule.finish_save(&attempted, false);
    schedule.queue_save(millis(400), "latest");
    schedule.request_save_now(millis(400));

    assert_eq!(schedule.due_save(millis(400)), Some("latest"));
    schedule.finish_save(&attempted, true);
    assert_eq!(schedule.due_save(millis(400)), Some("latest"));
}

#[test]
fn successful_save_clears_only_the_document_that_was_attempted() {
    let mut schedule = RuntimeSchedule::new();

    schedule.queue_save(millis(0), "document");
    let attempted = schedule.due_save(millis(300)).unwrap();
    schedule.finish_save(&attempted, true);

    assert_eq!(schedule.pending_save(), None);
    assert_eq!(schedule.save_deadline(), None);
}

#[test]
fn next_deadline_is_the_earliest_of_metrics_disk_redraw_and_save() {
    let mut schedule = RuntimeSchedule::new();
    schedule.request_redraw(millis(100), RedrawRequest::paint());
    schedule.queue_save(millis(0), "document");

    assert_eq!(
        schedule.next_deadline(millis(500), None, Some(millis(250))),
        millis(116)
    );

    schedule.take_due_redraw(millis(116));
    assert_eq!(
        schedule.next_deadline(millis(500), None, Some(millis(250))),
        millis(250)
    );
}

#[test]
fn visible_system_usage_deadline_preempts_a_slower_indicator_interval() {
    let schedule = RuntimeSchedule::<()>::new();

    assert_eq!(
        schedule.next_deadline(Duration::from_secs(60), Some(Duration::from_secs(2)), None,),
        Duration::from_secs(2)
    );
}

#[test]
fn redraw_requests_accumulate_font_and_semantic_invalidation_without_extra_frames() {
    let mut schedule = RuntimeSchedule::<()>::new();

    schedule.request_redraw(millis(0), RedrawRequest::semantic_colors());
    schedule.request_redraw(millis(4), RedrawRequest::fonts());

    assert_eq!(
        schedule.take_due_redraw(millis(16)),
        Some(RedrawRequest {
            refresh_fonts: true,
            invalidate_semantic_colors: true,
        })
    );
}

#[test]
fn an_immediate_metrics_redraw_consumes_an_older_frame_request_without_delay() {
    let mut schedule = RuntimeSchedule::<()>::new();
    schedule.request_redraw(millis(0), RedrawRequest::semantic_colors());

    schedule.request_redraw_now(millis(5), RedrawRequest::paint());

    assert_eq!(schedule.redraw_deadline(), Some(millis(5)));
    assert_eq!(
        schedule.take_due_redraw(millis(5)),
        Some(RedrawRequest::semantic_colors())
    );
    assert_eq!(schedule.take_due_redraw(millis(16)), None);
}
