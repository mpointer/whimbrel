//! When to compact the database (`memory::Store::compact`): only when it gives back a real
//! amount of disk, and never when the disk can't hold the rewrite. `VACUUM` writes a new copy of
//! the database before dropping the old one, and in WAL mode the compacted pages also pass
//! through the WAL, so it needs about twice the file's size in free space.

use memory::DbSpace;

/// Free space inside the file below which compacting isn't worth holding the store for.
pub const MIN_FREE_BYTES: i64 = 256 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompactDecision {
    Run,
    /// Less than a quarter of the file, or under `MIN_FREE_BYTES`, is free.
    NotWorthIt,
    /// The volume has less free space than the rewrite needs (twice the file).
    NotEnoughDisk { need_bytes: i64 },
}

/// `forced` (the user asked) skips the worth-it rule, never the disk rule.
pub fn decide(space: &DbSpace, volume_free_bytes: i64, forced: bool) -> CompactDecision {
    let file = space.file_bytes();
    if volume_free_bytes < 2 * file {
        return CompactDecision::NotEnoughDisk { need_bytes: 2 * file };
    }
    if forced {
        return CompactDecision::Run;
    }
    let free = space.free_bytes();
    if free * 4 >= file && free >= MIN_FREE_BYTES {
        CompactDecision::Run
    } else {
        CompactDecision::NotWorthIt
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GB: i64 = 1024 * 1024 * 1024;
    const MB: i64 = 1024 * 1024;

    /// A file of `file` bytes with `free` of them free, in 4 KB pages.
    fn space(file: i64, free: i64) -> DbSpace {
        DbSpace { page_size: 4096, page_count: file / 4096, freelist_count: free / 4096 }
    }

    #[test]
    fn forced_runs_when_disk_fits() {
        assert_eq!(decide(&space(3 * GB, 0), 10 * GB, true), CompactDecision::Run);
    }

    #[test]
    fn forced_refused_without_disk() {
        assert_eq!(
            decide(&space(3 * GB, GB), GB, true),
            CompactDecision::NotEnoughDisk { need_bytes: 6 * GB }
        );
    }

    #[test]
    fn needs_twice_the_file() {
        // WAL mode: a temporary copy plus the compacted pages in the WAL.
        assert_eq!(
            decide(&space(3 * GB, GB), 5 * GB, true),
            CompactDecision::NotEnoughDisk { need_bytes: 6 * GB }
        );
        assert_eq!(decide(&space(3 * GB, GB), 6 * GB, true), CompactDecision::Run);
    }

    #[test]
    fn small_free_space_not_worth_it() {
        assert_eq!(decide(&space(3 * GB, 100 * MB), 10 * GB, false), CompactDecision::NotWorthIt);
    }

    #[test]
    fn quarter_free_runs() {
        assert_eq!(decide(&space(3 * GB, GB), 10 * GB, false), CompactDecision::Run);
    }

    #[test]
    fn large_but_under_quarter_not_worth_it() {
        assert_eq!(decide(&space(3 * GB, 300 * MB), 10 * GB, false), CompactDecision::NotWorthIt);
    }

    #[test]
    fn a_quarter_of_a_small_file_is_still_not_worth_it() {
        assert_eq!(decide(&space(400 * MB, 200 * MB), 10 * GB, false), CompactDecision::NotWorthIt);
    }
}
