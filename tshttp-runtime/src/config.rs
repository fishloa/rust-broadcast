/// Session configuration (defaults per DESIGN.md).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Config {
    /// Start timeout in milliseconds (T6).
    pub start_timeout_ms: u64,
    /// Stall notice threshold in milliseconds (T11, T14, T15).
    pub stall_notice_ms: u64,
    /// Stall reconnect threshold in milliseconds (T12).
    pub stall_reconnect_ms: u64,
    /// Backoff schedule in milliseconds (T9).
    pub backoff_ms: alloc::vec::Vec<u64>,
    /// Backoff cap in milliseconds (T9).
    pub backoff_cap_ms: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            start_timeout_ms: 15_000,
            stall_notice_ms: 5_000,
            stall_reconnect_ms: 30_000,
            backoff_ms: alloc::vec![2_000, 4_000, 8_000],
            backoff_cap_ms: 10_000,
        }
    }
}
