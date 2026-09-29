use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub(crate) fn now_millis() -> i64 {
    millis(SystemTime::now())
}

pub(crate) fn millis(time: SystemTime) -> i64 {
    time.duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as i64)
}

pub(crate) fn from_millis(millis: i64) -> SystemTime {
    UNIX_EPOCH + Duration::from_millis(millis.max(0) as u64)
}
