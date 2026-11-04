//! 每张图独立的阶段计时；TINYPNG_PROFILE=1 时输出，不使用全局可变状态。
use std::{cell::RefCell, time::Instant};

pub(crate) struct Timing {
    label: &'static str,
    started: Instant,
    stages: Option<RefCell<Vec<(String, f64)>>>,
}
impl Timing {
    pub fn new(label: &'static str) -> Self {
        Self {
            label,
            started: Instant::now(),
            stages: std::env::var_os("TINYPNG_PROFILE")
                .filter(|value| value == "1")
                .map(|_| RefCell::new(Vec::new())),
        }
    }
    pub fn run<T>(&self, label: impl Into<String>, action: impl FnOnce() -> T) -> T {
        let Some(stages) = &self.stages else {
            return action();
        };
        let start = Instant::now();
        let result = action();
        stages
            .borrow_mut()
            .push((label.into(), start.elapsed().as_secs_f64() * 1000.0));
        result
    }
}
impl Drop for Timing {
    fn drop(&mut self) {
        if let Some(stages) = &self.stages {
            let mut output = format!(
                "[tinypng-profile {}] total {:.2} ms",
                self.label,
                self.started.elapsed().as_secs_f64() * 1000.0
            );
            for (label, ms) in stages.borrow().iter() {
                output.push_str(&format!("\n  {label}: {ms:.2} ms"));
            }
            eprintln!("{output}");
        }
    }
}
