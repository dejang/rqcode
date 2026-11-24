#[derive(Debug)]
pub struct QueryPlan {
    strategy: ScanStrategy,
    estimated_cost: u32,
}

#[derive(Debug)]
enum ScanStrategy {
    FullScan(String),
    IndexLookup(String),
}

impl QueryPlan {
    pub fn compile(table: &str, indexed: bool) -> Self {
        let strategy = if indexed {
            ScanStrategy::IndexLookup(table.into())
        } else {
            ScanStrategy::FullScan(table.into())
        };
        Self {
            strategy,
            estimated_cost: if indexed { 5 } else { 100 },
        }
    }

    pub fn execute(&self, rows: &[&str]) -> Vec<String> {
        let _ = (&self.strategy, self.estimated_cost);
        rows.iter().map(|r| (*r).to_string()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locks_down_internal_debug_layout() {
        let plan = QueryPlan::compile("users", true);
        assert_eq!(
            format!("{:?}", plan),
            "QueryPlan { strategy: IndexLookup(\"users\"), estimated_cost: 5 }"
        );
    }
}
