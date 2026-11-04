//! 文件边界的资源准入；独立于格式、画质策略和 UI 队列。
use std::sync::{Condvar, Mutex, OnceLock};

pub(crate) const MAX_MEMORY_INPUT_BYTES: u64 = 128 * 1024 * 1024;
const MIB: u64 = 1024 * 1024;

/// SSIMULACRA2 同时持有 RGB/XYB、多个模糊/方差平面；还需解码和编码缓冲。
/// 192 B/像素是保守规划值，并非分配器强制上限。无损路径不做感知测量。
pub(crate) fn estimate(width: u32, height: u32, input: u64, smart: bool) -> Result<u64, String> {
    if width == 0 || height == 0 || input > MAX_MEMORY_INPUT_BYTES {
        return Err("该输入不适合常规内存路径，应选择扫描线或原文件路径".into());
    }
    u64::from(width)
        .checked_mul(u64::from(height))
        .and_then(|pixels| pixels.checked_mul(if smart { 192 } else { 48 }))
        .and_then(|pixels| {
            input
                .checked_mul(3)
                .and_then(|bytes| pixels.checked_add(bytes))
        })
        .and_then(|bytes| bytes.checked_add(32 * MIB))
        .ok_or_else(|| "图片尺寸过大，无法安全处理".into())
}

fn budget(total: Option<u64>) -> u64 {
    total.map_or(512 * MIB, |bytes| (bytes / 4).clamp(128 * MIB, 4096 * MIB))
}

pub(crate) fn shared() -> &'static Budget {
    static SHARED: OnceLock<Budget> = OnceLock::new();
    SHARED.get_or_init(|| Budget::new(budget(physical_memory())))
}

pub(crate) struct Budget {
    limit: u64,
    used: Mutex<u64>,
    available: Condvar,
}

impl Budget {
    pub(crate) fn limit(&self) -> u64 {
        self.limit
    }
    pub(crate) fn new(limit: u64) -> Self {
        Self {
            limit,
            used: Mutex::new(0),
            available: Condvar::new(),
        }
    }

    pub(crate) fn acquire(&self, bytes: u64) -> Result<Reservation<'_>, String> {
        if bytes > self.limit {
            return Err(format!(
                "图片预计需要 {} MiB 工作内存，超过当前 {} MiB 安全预算；请缩小尺寸，或改用无损优化",
                bytes.div_ceil(MIB),
                self.limit / MIB
            ));
        }
        let mut used = self.used.lock().unwrap_or_else(|error| error.into_inner());
        while bytes > self.limit - *used {
            used = self
                .available
                .wait(used)
                .unwrap_or_else(|error| error.into_inner());
        }
        *used += bytes;
        Ok(Reservation {
            budget: self,
            bytes,
        })
    }
}

pub(crate) struct Reservation<'a> {
    budget: &'a Budget,
    bytes: u64,
}
impl Drop for Reservation<'_> {
    fn drop(&mut self) {
        let mut used = self
            .budget
            .used
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        *used -= self.bytes;
        self.budget.available.notify_all();
    }
}

#[cfg(target_os = "macos")]
fn physical_memory() -> Option<u64> {
    Some(objc2_foundation::NSProcessInfo::processInfo().physicalMemory())
}

#[cfg(target_os = "windows")]
fn physical_memory() -> Option<u64> {
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    // SAFETY: MEMORYSTATUSEX 为 POD，dwLength 按 API 约定设置；指针有效且只用于本次调用。
    let mut status: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
    status.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
    (unsafe { GlobalMemoryStatusEx(&mut status) } != 0).then_some(status.ullTotalPhys)
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn physical_memory() -> Option<u64> {
    let text = std::fs::read_to_string("/proc/meminfo").ok()?;
    let kb = text
        .lines()
        .find_map(|line| line.strip_prefix("MemTotal:"))?
        .split_whitespace()
        .next()?
        .parse::<u64>()
        .ok()?;
    kb.checked_mul(1024)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_files_do_not_hide_large_pixel_allocations() {
        let small = estimate(100, 100, 1024, true).unwrap();
        let large = estimate(20_000, 20_000, 1024, true).unwrap();
        assert!(large > small * 100);
        assert!(Budget::new(512 * MIB).acquire(large).is_err());
        assert!(estimate(u32::MAX, u32::MAX, 1, true).is_err());
        assert!(estimate(0, 1, 1, true).is_err());
        assert!(estimate(1, 1, MAX_MEMORY_INPUT_BYTES + 1, false).is_err());
    }

    #[test]
    fn budget_scales_with_ram_and_has_a_conservative_fallback() {
        assert_eq!(budget(None), 512 * MIB);
        assert_eq!(budget(Some(4 * 1024 * MIB)), 1024 * MIB);
        assert_eq!(budget(Some(64 * 1024 * MIB)), 4096 * MIB);
        assert!(
            estimate(1000, 1000, 1024, false).unwrap() < estimate(1000, 1000, 1024, true).unwrap()
        );
    }

    #[test]
    fn reservations_serialize_large_jobs_and_release_on_error() {
        let budget = Budget::new(10);
        let held = budget.acquire(7).unwrap();
        std::thread::scope(|scope| {
            let (ready, observed) = std::sync::mpsc::channel();
            let budget = &budget;
            let worker = scope.spawn(move || {
                ready.send(()).unwrap();
                let _reservation = budget.acquire(7).unwrap();
                assert_eq!(*budget.used.lock().unwrap(), 7);
            });
            observed.recv().unwrap();
            assert_eq!(*budget.used.lock().unwrap(), 7);
            drop(held);
            worker.join().unwrap();
        });
        assert_eq!(*budget.used.lock().unwrap(), 0);
        let failure = || -> Result<(), ()> {
            let _reservation = budget.acquire(10).unwrap();
            Err(())
        };
        assert!(failure().is_err());
        assert!(budget.acquire(10).is_ok());
    }
}
