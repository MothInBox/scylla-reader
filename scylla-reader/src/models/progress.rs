use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Serialize, Deserialize)]
pub struct Progress {
    pub current: u32,
    pub total: u32,
}
impl Progress {
    pub fn percentage(&self) -> f32 {
        if self.total == 0 { return 0.0; }
        (self.current as f32 / self.total as f32) * 100.0
    }
}
