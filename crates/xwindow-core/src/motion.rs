//! Bounded cursor coalescing shared by window adapters. The caller supplies
//! a rectangle where movement has no observable effect other than position.

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CursorMove {
    pub x: f64,
    pub y: f64,
    pub device_id: i32,
}

#[derive(Default)]
pub struct CursorMoves {
    region: Option<[f64; 4]>,
    latest: Option<CursorMove>,
}

impl CursorMoves {
    /// Replaces the quiet rectangle, retaining any unread position.
    pub fn region(&mut self, left: f64, top: f64, right: f64, bottom: f64) {
        self.region = ([left, top, right, bottom].iter().all(|v| v.is_finite())
            && left < right
            && top < bottom)
            .then_some([left, top, right, bottom]);
    }

    /// True when the move can be retained instead of delivered. A move out
    /// of the rectangle disables it and supersedes the retained position.
    pub fn hold(&mut self, x: f64, y: f64, device_id: i32) -> bool {
        if let Some([left, top, right, bottom]) = self.region
            && x >= left
            && x < right
            && y >= top
            && y < bottom
        {
            self.latest = Some(CursorMove { x, y, device_id });
            true
        } else {
            self.region = None;
            self.latest = None;
            false
        }
    }

    /// Whether an interior position is pending under an active quiet policy.
    /// Desktop adapters can suppress duplicate X/Y valuators when raw input is disabled.
    pub fn holding_position(&self) -> bool {
        self.region.is_some() && self.latest.is_some()
    }

    /// Before other input or application work: disable the rectangle and
    /// consume its latest position. Never queues an event or allocates.
    pub fn take(&mut self) -> Option<CursorMove> {
        self.region = None;
        self.latest.take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thousands_of_moves_retain_only_the_latest_and_take_disables() {
        let mut moves = CursorMoves::default();
        moves.region(0.0, 0.0, 100.0, 100.0);
        for i in 0..10_000 {
            assert!(moves.hold((i % 99) as f64, 23.5, 4));
        }
        assert_eq!(
            moves.take(),
            Some(CursorMove {
                x: 0.0,
                y: 23.5,
                device_id: 4
            })
        );
        assert_eq!(moves.take(), None);
        assert!(!moves.hold(50.0, 50.0, 4));
    }

    #[test]
    fn crossing_each_edge_resumes_delivery() {
        for (x, y) in [(-0.1, 50.0), (100.0, 50.0), (50.0, -0.1), (50.0, 100.0)] {
            let mut moves = CursorMoves::default();
            moves.region(0.0, 0.0, 100.0, 100.0);
            assert!(moves.hold(0.0, 0.0, 1));
            assert!(!moves.hold(x, y, 1));
            assert_eq!(moves.take(), None); // the delivered move is newer
        }
    }

    #[test]
    fn invalid_regions_disable_without_losing_pending_position() {
        for bounds in [
            [0.0; 4],
            [1.0, 0.0, 0.0, 1.0],
            [0.0, 0.0, f64::NAN, 1.0],
            [0.0, 0.0, f64::INFINITY, 1.0],
        ] {
            let mut moves = CursorMoves::default();
            moves.region(0.0, 0.0, 100.0, 100.0);
            assert!(moves.hold(20.25, 30.5, 7));
            moves.region(bounds[0], bounds[1], bounds[2], bounds[3]);
            assert_eq!(
                moves.take(),
                Some(CursorMove {
                    x: 20.25,
                    y: 30.5,
                    device_id: 7
                })
            );
            assert!(!moves.hold(20.25, 30.5, 7));
        }
    }
}
