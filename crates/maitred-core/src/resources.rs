/// An amount of the two budgeted resources: CPU in millicores, memory in megabytes (ARCHITECTURE.md,
/// Inside `decide`: the sampler converts its fractions before writing).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Resources {
    pub cpu_millicores: u64,
    pub mem_mb: u64,
}

impl Resources {
    #[must_use]
    pub const fn new(cpu_millicores: u64, mem_mb: u64) -> Self {
        Self {
            cpu_millicores,
            mem_mb,
        }
    }

    /// The larger of each dimension.
    #[must_use]
    pub fn max(self, other: Self) -> Self {
        Self::new(
            self.cpu_millicores.max(other.cpu_millicores),
            self.mem_mb.max(other.mem_mb),
        )
    }

    /// The smaller of each dimension.
    #[must_use]
    pub fn min(self, other: Self) -> Self {
        Self::new(
            self.cpu_millicores.min(other.cpu_millicores),
            self.mem_mb.min(other.mem_mb),
        )
    }

    /// What is left of `self` once `used` is taken off, never below zero in either dimension.
    #[must_use]
    pub fn saturating_sub(self, used: Self) -> Self {
        Self::new(
            self.cpu_millicores.saturating_sub(used.cpu_millicores),
            self.mem_mb.saturating_sub(used.mem_mb),
        )
    }
}
