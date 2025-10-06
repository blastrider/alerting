use std::collections::VecDeque;
use std::time::Instant;

pub struct LeakyBucket {
    window: std::time::Duration,
    max: usize,
    samples: VecDeque<Instant>,
}

impl LeakyBucket {
    pub fn new(max: usize, window: std::time::Duration) -> Self {
        Self {
            window,
            max,
            samples: VecDeque::with_capacity(max.max(1)),
        }
    }

    pub fn try_acquire(&mut self, now: Instant) -> bool {
        while let Some(front) = self.samples.front() {
            if now.duration_since(*front) > self.window {
                self.samples.pop_front();
            } else {
                break;
            }
        }
        if self.samples.len() >= self.max {
            return false;
        }
        self.samples.push_back(now);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::LeakyBucket;
    use std::time::{Duration, Instant};

    #[test]
    fn leaky_bucket_respects_capacity() {
        let mut bucket = LeakyBucket::new(2, Duration::from_secs(5));
        let now = Instant::now();
        assert!(bucket.try_acquire(now));
        assert!(bucket.try_acquire(now));
        assert!(!bucket.try_acquire(now));
    }

    #[test]
    fn leaky_bucket_drains_over_time() {
        let mut bucket = LeakyBucket::new(1, Duration::from_secs(1));
        let now = Instant::now();
        assert!(bucket.try_acquire(now));
        assert!(!bucket.try_acquire(now));
        let later = now + Duration::from_secs(2);
        assert!(bucket.try_acquire(later));
    }
}
