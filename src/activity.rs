//! One mutating operation owns publication through commit and cleanup.
use core::sync::atomic::{AtomicBool, Ordering};

pub struct OperationGate(AtomicBool);
pub struct Operation<'a>(&'a OperationGate);
pub static OPERATIONS: OperationGate = OperationGate::new();

impl Default for OperationGate {
    fn default() -> Self {
        Self::new()
    }
}
impl OperationGate {
    pub const fn new() -> Self {
        Self(AtomicBool::new(false))
    }
    pub fn try_acquire(&self) -> Option<Operation<'_>> {
        self.0
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| Operation(self))
    }
    pub fn is_active(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}
impl Drop for Operation<'_> {
    fn drop(&mut self) {
        self.0 .0.store(false, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn publishers_are_exclusive_and_failure_releases_the_lease() {
        let gate = OperationGate::new();
        let failed = || -> Result<(), ()> {
            let _lease = gate.try_acquire().unwrap();
            assert!(gate.is_active());
            assert!(gate.try_acquire().is_none());
            Err(())
        };
        assert!(failed().is_err());
        assert!(!gate.is_active());
        assert!(gate.try_acquire().is_some());
    }
}
