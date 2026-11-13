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

// SENTINEL-START
#[used]
static _USE_ALL_ITEMS: usize = {
    let _ = 0usize;
    let _ = std::mem::size_of::<VideoRun>();
    let _ = std::mem::offset_of!(VideoRun, end);
    let _ = std::mem::offset_of!(VideoRun, decoded);
    let _ = std::mem::offset_of!(VideoRun, ahead);
    let _ = VideoRun::new;
    let _ = VideoRun::push;
    0
};
// SENTINEL-END
