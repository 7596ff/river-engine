//! Port allocation for llama-server instances

use river_core::RiverError;
use std::collections::HashSet;

/// Allocates ports from a configured range
pub struct PortAllocator {
    range_start: u16,
    range_end: u16,
    allocated: HashSet<u16>,
}

impl PortAllocator {
    pub fn new(range_start: u16, range_end: u16) -> Self {
        Self {
            range_start,
            range_end,
            allocated: HashSet::new(),
        }
    }

    /// Allocate the next available port
    pub fn next(&mut self) -> Result<u16, RiverError> {
        for port in self.range_start..=self.range_end {
            if !self.allocated.contains(&port) {
                self.allocated.insert(port);
                return Ok(port);
            }
        }
        Err(RiverError::orchestrator(format!(
            "No available ports in range {}-{}",
            self.range_start, self.range_end
        )))
    }

    /// Release a port back to the pool
    pub fn release(&mut self, port: u16) {
        self.allocated.remove(&port);
    }

    pub fn allocated_count(&self) -> usize {
        self.allocated.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_port_allocation() {
        let mut allocator = PortAllocator::new(8080, 8082);
        assert_eq!(allocator.next().unwrap(), 8080);
        assert_eq!(allocator.next().unwrap(), 8081);
        assert_eq!(allocator.next().unwrap(), 8082);
        assert!(allocator.next().is_err());
    }

    #[test]
    fn test_port_release() {
        let mut allocator = PortAllocator::new(8080, 8080);
        let port = allocator.next().unwrap();
        assert!(allocator.next().is_err());
        allocator.release(port);
        assert_eq!(allocator.next().unwrap(), 8080);
    }

    #[test]
    fn test_allocated_count() {
        let mut allocator = PortAllocator::new(8080, 8085);
        assert_eq!(allocator.allocated_count(), 0);
        allocator.next().unwrap();
        allocator.next().unwrap();
        assert_eq!(allocator.allocated_count(), 2);
    }
}
