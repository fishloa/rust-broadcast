pub struct VideoRun {
    end: Option<i64>,
    decoded: i64,
    ahead: Vec<(i64, i64)>,
}

impl VideoRun {
    pub fn new() -> Self {
        Self {
            end: None,
            decoded: i64::MIN,
            ahead: Vec::new(),
        }
    }

    pub fn push(&mut self, pts: i64, dts: i64, dur: i64) -> i64 {
        self.decoded = self.decoded.max(dts);
        let Some(mut end) = self.end else {
            let first = pts + dur;
            self.end = Some(first);
            return first;
        };
        self.ahead.push((pts, pts + dur));
        loop {
            self.ahead.sort_unstable();
            let Some((start, stop)) = self.ahead.first().copied() else {
                break;
            };
            let slot = stop - start;
            if start <= end + slot / 2 || start - slot <= self.decoded {
                end = end.max(stop);
                self.ahead.remove(0);
            } else {
                break;
            }
        }
        self.end = Some(end);
        end
    }
}
