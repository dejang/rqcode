use std::collections::HashMap;

/// A candidate pair of unit indices whose exact Jaccard similarity met the Stage 2B threshold.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct CandidatePair {
    pub(crate) left: usize,
    pub(crate) right: usize,
    pub(crate) jaccard: f64,
}

/// Exact prefix-filtered all-pairs Jaccard join (Bayardo et al. WWW 2007 / `SourcererCC` ICSE 2016).
///
/// Returns every pair `(left, right)` with `left < right` and
/// `jaccard(units[left], units[right]) + eps >= threshold`, with zero false negatives relative to
/// `threshold` and sub-quadratic runtime by indexing only each unit's rarest prefix.
#[must_use]
pub(crate) fn find_jaccard_candidates(
    units: &[&[u64]],
    threshold: f64,
    eps: f64,
) -> Vec<CandidatePair> {
    if units.len() < 2 || !threshold.is_finite() || threshold <= 0.0 || threshold > 1.0 {
        return Vec::new();
    }
    let eps = if eps.is_finite() && eps >= 0.0 {
        eps
    } else {
        0.0
    };

    // 1. Global document frequency of each 64-bit fingerprint.
    let mut df: HashMap<u64, usize> = HashMap::new();
    for unit in units {
        for &h in *unit {
            *df.entry(h).or_insert(0) += 1;
        }
    }

    // 2. Order every unit's fingerprints by ascending global frequency `(df[h], h)` (rarest first)
    //    and sort units by ascending length so we only probe already-indexed smaller/equal units.
    let mut ordered_units: Vec<(usize, Vec<(usize, u64)>)> = units
        .iter()
        .enumerate()
        .filter(|(_, u)| !u.is_empty())
        .map(|(orig_idx, hashes)| {
            let mut ranked: Vec<(usize, u64)> = hashes
                .iter()
                .map(|&h| (df.get(&h).copied().unwrap_or(0), h))
                .collect();
            ranked.sort_unstable();
            (orig_idx, ranked)
        })
        .collect();
    ordered_units.sort_unstable_by(|(idx_a, a), (idx_b, b)| {
        a.len().cmp(&b.len()).then_with(|| idx_a.cmp(idx_b))
    });

    // 3. Sweep in ascending length order, probing only rare prefixes of smaller/equal units.
    let mut inverted: HashMap<u64, Vec<usize>> = HashMap::new();
    let mut seen_stamp: Vec<u32> = vec![0; ordered_units.len()];
    let mut candidate_slots: Vec<usize> = Vec::new();
    let mut results: Vec<CandidatePair> = Vec::new();

    for cur in 0..ordered_units.len() {
        let (orig_cur, cur_ranked) = &ordered_units[cur];
        let cur_len = cur_ranked.len();
        let min_other_len = ceil_threshold_count(cur_len, threshold * count_to_f64(cur_len), eps);
        let cur_prefix = cur_len
            .saturating_sub(min_other_len)
            .saturating_add(1)
            .clamp(1, cur_len);

        candidate_slots.clear();
        let stamp = u32::try_from(cur + 1).unwrap_or(u32::MAX);

        for &(_, hash) in &cur_ranked[..cur_prefix] {
            if let Some(postings) = inverted.get(&hash) {
                for &other in postings {
                    if ordered_units[other].1.len() >= min_other_len && seen_stamp[other] != stamp {
                        seen_stamp[other] = stamp;
                        candidate_slots.push(other);
                    }
                }
            }
        }

        for &other in &candidate_slots {
            let (orig_other, other_ranked) = &ordered_units[other];
            let other_len = other_ranked.len();
            let min_overlap = ceil_threshold_count(
                cur_len.min(other_len),
                (threshold / (1.0 + threshold)) * count_to_f64(cur_len + other_len),
                eps,
            );

            // Early-stopping sorted intersection.
            let (mut i, mut j, mut common) = (0usize, 0usize, 0usize);
            while i < cur_len && j < other_len {
                if common + (cur_len - i).min(other_len - j) < min_overlap {
                    common = 0;
                    break;
                }
                match cur_ranked[i].cmp(&other_ranked[j]) {
                    std::cmp::Ordering::Equal => {
                        common += 1;
                        i += 1;
                        j += 1;
                    }
                    std::cmp::Ordering::Less => i += 1,
                    std::cmp::Ordering::Greater => j += 1,
                }
            }

            if common >= min_overlap {
                let jaccard = count_to_f64(common) / count_to_f64(cur_len + other_len - common);
                if jaccard + eps >= threshold {
                    results.push(CandidatePair {
                        left: (*orig_cur).min(*orig_other),
                        right: (*orig_cur).max(*orig_other),
                        jaccard,
                    });
                }
            }
        }

        // Index only `cur`'s rare prefix for subsequent larger/equal units.
        for &(_, hash) in &cur_ranked[..cur_prefix] {
            inverted.entry(hash).or_default().push(cur);
        }
    }

    results.sort_unstable_by(|a, b| a.left.cmp(&b.left).then_with(|| a.right.cmp(&b.right)));
    results
}

/// Smallest integer `k` in `1..=max_count` satisfying `k >= target - eps`.
/// Computed via integer binary search to remain exact without lossy float-to-int casts.
fn ceil_threshold_count(max_count: usize, target: f64, eps: f64) -> usize {
    let adjusted = target - eps;
    if adjusted <= 1.0 {
        return 1;
    }
    let mut lo = 1usize;
    let mut hi = max_count.max(1);
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if count_to_f64(mid) >= adjusted {
            hi = mid;
        } else {
            lo = mid + 1;
        }
    }
    lo
}

fn count_to_f64(n: usize) -> f64 {
    f64::from(u32::try_from(n).unwrap_or(u32::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_matches_brute_force(slices: &[&[u64]], threshold: f64) {
        let eps = 1e-12;
        let fast = find_jaccard_candidates(slices, threshold, eps);
        let mut brute = Vec::new();
        for i in 0..slices.len() {
            for j in (i + 1)..slices.len() {
                if slices[i].is_empty() || slices[j].is_empty() {
                    continue;
                }
                let inter = slices[i]
                    .iter()
                    .filter(|h| slices[j].binary_search(h).is_ok())
                    .count();
                let union = slices[i].len() + slices[j].len() - inter;
                let jaccard = f64::from(u32::try_from(inter).unwrap_or(u32::MAX))
                    / f64::from(u32::try_from(union).unwrap_or(u32::MAX));
                if jaccard + eps >= threshold {
                    brute.push(CandidatePair {
                        left: i,
                        right: j,
                        jaccard,
                    });
                }
            }
        }
        assert_eq!(
            fast.len(),
            brute.len(),
            "mismatch at threshold {threshold}: fast={fast:?}, brute={brute:?}"
        );
        for (f, b) in fast.iter().zip(brute.iter()) {
            assert_eq!((f.left, f.right), (b.left, b.right));
            assert!((f.jaccard - b.jaccard).abs() < 1e-12);
        }
    }

    #[test]
    fn rejects_invalid_or_nan_thresholds() {
        let a = [1u64, 2, 3];
        let b = [1u64, 2, 3];
        let slices: [&[u64]; 2] = [&a, &b];
        for bad in [f64::NAN, f64::from(0), f64::from(2)] {
            assert_eq!(
                find_jaccard_candidates(&slices, bad, f64::EPSILON),
                Vec::new()
            );
        }
    }

    #[test]
    fn prefix_filter_matches_brute_force_exactly_across_thresholds() {
        let mut raw_sets: Vec<Vec<u64>> = Vec::with_capacity(40);
        let mut set_buf: Vec<u64> = Vec::with_capacity(24);
        let mut state: u64 = 0x1234_5678_9abc_def0;
        let mut next_u64 = || {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            state
        };

        for idx in 0..40usize {
            let size = 4 + (idx % 18);
            set_buf.clear();
            let base = (idx / 4) as u64 * 100;
            for _ in 0..size {
                set_buf.push(base + (next_u64() % 24));
            }
            set_buf.sort_unstable();
            set_buf.dedup();
            raw_sets.push(set_buf.clone());
        }

        let slices: Vec<&[u64]> = raw_sets.iter().map(Vec::as_slice).collect();
        assert_matches_brute_force(&slices, 0.25);
        assert_matches_brute_force(&slices, 0.40);
        assert_matches_brute_force(&slices, 0.50);
        assert_matches_brute_force(&slices, 0.60);
        assert_matches_brute_force(&slices, 0.75);
        assert_matches_brute_force(&slices, 0.90);
        assert_matches_brute_force(&slices, 1.0);
    }
}
