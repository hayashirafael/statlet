use std::time::Duration;

const REDRAW_FRAME: Duration = Duration::from_millis(16);
const SAVE_TRAILING: Duration = Duration::from_millis(300);
const SAVE_MAXIMUM: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RedrawRequest {
    pub refresh_fonts: bool,
    pub invalidate_semantic_colors: bool,
}

impl RedrawRequest {
    pub const fn paint() -> Self {
        Self {
            refresh_fonts: false,
            invalidate_semantic_colors: false,
        }
    }

    pub const fn semantic_colors() -> Self {
        Self {
            refresh_fonts: false,
            invalidate_semantic_colors: true,
        }
    }

    pub const fn fonts() -> Self {
        Self {
            refresh_fonts: true,
            invalidate_semantic_colors: true,
        }
    }

    fn merge(&mut self, other: Self) {
        self.refresh_fonts |= other.refresh_fonts;
        self.invalidate_semantic_colors |= other.invalidate_semantic_colors;
    }
}

pub struct RuntimeSchedule<T> {
    redraw: Option<(Duration, RedrawRequest)>,
    pending_save: Option<T>,
    first_unsaved_at: Option<Duration>,
    save_deadline: Option<Duration>,
}

impl<T> Default for RuntimeSchedule<T> {
    fn default() -> Self {
        Self {
            redraw: None,
            pending_save: None,
            first_unsaved_at: None,
            save_deadline: None,
        }
    }
}

impl<T> RuntimeSchedule<T> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn request_redraw(&mut self, now: Duration, request: RedrawRequest) {
        match &mut self.redraw {
            Some((_, pending)) => pending.merge(request),
            None => self.redraw = Some((now + REDRAW_FRAME, request)),
        }
    }

    pub fn request_redraw_now(&mut self, now: Duration, request: RedrawRequest) {
        match &mut self.redraw {
            Some((deadline, pending)) => {
                *deadline = (*deadline).min(now);
                pending.merge(request);
            }
            None => self.redraw = Some((now, request)),
        }
    }

    pub fn redraw_deadline(&self) -> Option<Duration> {
        self.redraw.map(|(deadline, _)| deadline)
    }

    pub fn take_due_redraw(&mut self, now: Duration) -> Option<RedrawRequest> {
        if self
            .redraw_deadline()
            .is_some_and(|deadline| deadline <= now)
        {
            return self.redraw.take().map(|(_, request)| request);
        }
        None
    }

    pub fn queue_save(&mut self, now: Duration, document: T) {
        let first_unsaved_at = *self.first_unsaved_at.get_or_insert(now);
        self.pending_save = Some(document);
        self.save_deadline = Some((now + SAVE_TRAILING).min(first_unsaved_at + SAVE_MAXIMUM));
    }

    pub fn request_save_now(&mut self, now: Duration) {
        if self.pending_save.is_some() {
            self.save_deadline = Some(now);
        }
    }

    pub fn save_deadline(&self) -> Option<Duration> {
        self.save_deadline
    }

    pub fn pending_save(&self) -> Option<&T> {
        self.pending_save.as_ref()
    }

    pub fn next_deadline<I>(
        &self,
        mandatory_deadline: Duration,
        additional_deadlines: I,
    ) -> Duration
    where
        I: IntoIterator<Item = Option<Duration>>,
    {
        std::iter::once(Some(mandatory_deadline))
            .chain(additional_deadlines)
            .chain([self.redraw_deadline(), self.save_deadline()])
            .flatten()
            .min()
            .expect("a mandatory deadline is always supplied")
    }
}

impl<T: Clone + Eq> RuntimeSchedule<T> {
    pub fn due_save(&self, now: Duration) -> Option<T> {
        self.save_deadline
            .filter(|deadline| *deadline <= now)
            .and_then(|_| self.pending_save.clone())
    }

    pub fn finish_save(&mut self, attempted: &T, succeeded: bool) {
        if succeeded && self.pending_save.as_ref() == Some(attempted) {
            self.pending_save = None;
            self.first_unsaved_at = None;
            self.save_deadline = None;
        } else if !succeeded {
            self.save_deadline = None;
        }
    }
}
