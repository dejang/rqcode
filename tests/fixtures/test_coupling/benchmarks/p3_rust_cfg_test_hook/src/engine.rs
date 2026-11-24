pub struct SyncEngine;

impl SyncEngine {
    pub fn should_skip_network() -> bool {
        if cfg!(test) {
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::SyncEngine;

    #[test]
    fn skips_network() {
        assert!(SyncEngine::should_skip_network());
    }
}
