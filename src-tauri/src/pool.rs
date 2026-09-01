//! 多账号负载均衡调度：轮询（round-robin）+ 连续 429 冷却退避。
//!
//! - 每个请求从游标处轮询选号，实现负载均摊
//! - 单次 429：请求立即故障转移到下一账号，但账号不冷却（可能是瞬时抖动）
//! - 连续 429 达到阈值：账号进入冷却，冷却期内调度直接跳过；
//!   冷却时长逐次翻倍（60s → 120s → 240s …，封顶 30min）
//! - 任一请求成功即清零计数

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

/// 连续 429 次数达到该阈值 → 账号进入冷却
pub const CONSECUTIVE_429_THRESHOLD: u32 = 2;
/// 首次冷却时长（秒）
pub const COOLDOWN_BASE_SECS: u64 = 60;
/// 冷却上限（秒）
pub const COOLDOWN_MAX_SECS: u64 = 30 * 60;

/// 第 N 次冷却的时长（秒）：base × 2^(N-1)，封顶 MAX
pub fn cooldown_for(cooldown_count: u32) -> u64 {
    let shift = cooldown_count.saturating_sub(1).min(20);
    COOLDOWN_BASE_SECS
        .saturating_mul(1u64 << shift)
        .min(COOLDOWN_MAX_SECS)
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// 单账号运行态（仅内存，不落盘——限流状态是瞬时的）
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct AccountState {
    pub consecutive_429: u32,
    /// 冷却截止时间（epoch 毫秒；0 = 未冷却）
    pub cooldown_until_ms: i64,
    /// 累计进入冷却次数（决定翻倍）
    pub cooldown_count: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_used_ms: Option<i64>,
}

#[derive(Default)]
struct PoolInner {
    cursor: AtomicUsize,
    states: Mutex<HashMap<String, AccountState>>,
}

/// Clone 共享同一份运行态（AppState 克隆后仍指向同一池）。
#[derive(Clone, Default)]
pub struct AccountPool {
    inner: std::sync::Arc<PoolInner>,
}

impl AccountPool {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, AccountState>> {
        self.inner.states.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// 轮询排序候选账号；冷却中的账号被剔除。
    /// 每调用一次游标前移一位，实现请求级负载均摊。
    pub fn order_candidates(&self, ids: &[String]) -> Vec<String> {
        let now = now_ms();
        let usable: Vec<String> = {
            let map = self.lock();
            ids.iter()
                .filter(|id| {
                    map.get(*id)
                        .is_none_or(|s| s.cooldown_until_ms <= now)
                })
                .cloned()
                .collect()
        };
        if usable.is_empty() {
            return Vec::new();
        }
        let n = usable.len();
        let start = self.inner.cursor.fetch_add(1, Ordering::Relaxed) % n;
        (0..n)
            .map(|i| usable[(start + i) % n].clone())
            .collect()
    }

    /// 请求成功：清零 429 计数与冷却
    pub fn on_success(&self, id: &str) {
        let mut map = self.lock();
        let s = map.entry(id.to_string()).or_default();
        s.consecutive_429 = 0;
        s.cooldown_until_ms = 0;
        s.cooldown_count = 0;
        s.last_error = None;
        s.last_used_ms = Some(now_ms());
    }

    /// 收到 429：计数 +1；达到阈值进入冷却（时长指数翻倍），返回冷却秒数
    pub fn on_429(&self, id: &str) -> Option<u64> {
        let mut map = self.lock();
        let s = map.entry(id.to_string()).or_default();
        s.consecutive_429 += 1;
        if s.consecutive_429 < CONSECUTIVE_429_THRESHOLD {
            return None;
        }
        s.cooldown_count += 1;
        let secs = cooldown_for(s.cooldown_count);
        s.cooldown_until_ms = now_ms() + (secs as i64) * 1000;
        Some(secs)
    }

    /// 其他错误（网络/5xx/刷新失败）：仅记录，不影响调度
    pub fn on_error(&self, id: &str, err: &str) {
        let mut map = self.lock();
        let s = map.entry(id.to_string()).or_default();
        s.last_error = Some(err.to_string());
    }

    pub fn snapshot(&self) -> HashMap<String, AccountState> {
        self.lock().clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn threshold_and_reset() {
        let pool = AccountPool::default();
        assert_eq!(pool.on_429("a"), None, "单次 429 不应触发冷却");
        assert_eq!(pool.on_429("a"), Some(60), "连续第 2 次应冷却 60s");
        pool.on_success("a");
        assert_eq!(
            pool.on_429("a"),
            None,
            "成功后计数清零，单次 429 不应再触发"
        );
        let s = pool.snapshot().get("a").unwrap().clone();
        assert_eq!(s.consecutive_429, 1);
        assert_eq!(s.cooldown_until_ms, 0);
    }

    #[test]
    fn cooldown_doubles_and_caps() {
        assert_eq!(cooldown_for(1), 60);
        assert_eq!(cooldown_for(2), 120);
        assert_eq!(cooldown_for(3), 240);
        assert_eq!(cooldown_for(5), 960);
        assert_eq!(cooldown_for(6), COOLDOWN_MAX_SECS, "60×32 已超封顶");
        assert_eq!(cooldown_for(20), COOLDOWN_MAX_SECS, "封顶 30min");
        assert_eq!(cooldown_for(u32::MAX), COOLDOWN_MAX_SECS);
    }

    #[test]
    fn post_cooldown_429_recools_with_backoff() {
        let pool = AccountPool::default();
        pool.on_429("a");
        assert_eq!(pool.on_429("a"), Some(60));
        // 模拟冷却结束（无成功请求）：状态保留，再次 429 应按翻倍时长重新冷却
        assert_eq!(pool.on_429("a"), Some(120), "冷却翻倍");
    }

    #[test]
    fn round_robin_rotates() {
        let pool = AccountPool::default();
        let ids = vec!["a".to_string(), "b".to_string()];
        let o1 = pool.order_candidates(&ids);
        let o2 = pool.order_candidates(&ids);
        let o3 = pool.order_candidates(&ids);
        assert_eq!(o1, vec!["a".to_string(), "b".to_string()]);
        assert_eq!(o2, vec!["b".to_string(), "a".to_string()]);
        assert_eq!(o3, vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn cooled_account_skipped() {
        let pool = AccountPool::default();
        let ids = vec!["a".to_string(), "b".to_string()];
        pool.on_429("a");
        pool.on_429("a"); // a 进入冷却
        let order = pool.order_candidates(&ids);
        assert_eq!(order, vec!["b".to_string()], "冷却中的 a 应被剔除");
        // 全部冷却 → 空列表
        pool.on_429("b");
        pool.on_429("b");
        assert!(pool.order_candidates(&ids).is_empty());
    }

    #[test]
    fn error_recorded_only() {
        let pool = AccountPool::default();
        let ids = vec!["a".to_string()];
        pool.on_error("a", "网络错误");
        assert_eq!(
            pool.order_candidates(&ids),
            vec!["a".to_string()],
            "普通错误不应剔除账号"
        );
        assert_eq!(
            pool.snapshot().get("a").unwrap().last_error.as_deref(),
            Some("网络错误")
        );
    }
}
