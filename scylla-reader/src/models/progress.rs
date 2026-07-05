//! Progress tracking — current/total chapters, reading status.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Serialize, Deserialize)]
pub struct Progress {
    pub current: u32,
    pub total: u32,
}
impl Progress {
    pub fn percentage(&self) -> f32 {
        if self.total == 0 {
            return 0.0;
        }
        (self.current as f32 / self.total as f32) * 100.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_percentage_zero_total() {
        let p = Progress {
            current: 50,
            total: 0,
        };
        assert_eq!(p.percentage(), 0.0);
    }

    #[test]
    fn test_percentage_half() {
        let p = Progress {
            current: 50,
            total: 100,
        };
        assert!((p.percentage() - 50.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_percentage_complete() {
        let p = Progress {
            current: 100,
            total: 100,
        };
        assert!((p.percentage() - 100.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_percentage_zero_current() {
        let p = Progress {
            current: 0,
            total: 100,
        };
        assert!((p.percentage() - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_percentage_over_total() {
        let p = Progress {
            current: 150,
            total: 100,
        };
        assert!((p.percentage() - 150.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_percentage_mid_range() {
        let p = Progress {
            current: 1,
            total: 3,
        };
        let expected = 100.0 / 3.0;
        assert!((p.percentage() - expected).abs() < 1e-5);
    }
}
