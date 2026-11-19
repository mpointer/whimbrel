//! The audio level meter's value: a decaying peak per source, from the peak the capture code already
//! computes for each packet. Pure and dependency-free (std only), so it compiles and tests on its own.
//!
//! One `u64` per source holds the peak and the time it was stored (`peak` in the low 16 bits, unix ms
//! above), so the capture thread writes one atomic per packet and a reader needs no lock. The level is
//! read, drawn and thrown away: nothing here is stored anywhere.

/// A peak falls linearly to zero over this long, so a 4 Hz reader sees a smooth meter rather than the
/// last 10 ms packet.
pub const DECAY_MS: i64 = 400;

/// Full scale for a 16-bit sample.
const FULL_SCALE: f32 = 32_767.0;

pub fn pack(peak: u16, at_ms: i64) -> u64 {
    ((at_ms.max(0) as u64) << 16) | peak as u64
}

pub fn unpack(value: u64) -> (u16, i64) {
    ((value & 0xFFFF) as u16, (value >> 16) as i64)
}

/// The peak `peak` stored at `at_ms` has fallen to by `now_ms`.
pub fn decayed(peak: u16, at_ms: i64, now_ms: i64) -> u16 {
    let age = (now_ms - at_ms).clamp(0, DECAY_MS);
    (peak as i64 * (DECAY_MS - age) / DECAY_MS) as u16
}

/// The value to store after a packet with `packet_peak` arrives at `now_ms`: the louder of the packet and
/// what the last value has decayed to.
pub fn after_packet(previous: u64, packet_peak: i16, now_ms: i64) -> u64 {
    let (peak, at) = unpack(previous);
    let held = decayed(peak, at, now_ms);
    pack(held.max(packet_peak.unsigned_abs().min(32_767)), now_ms)
}

/// The meter's level, 0 to 1, for a stored value read at `now_ms`. Square-rooted, because speech sits well
/// below full scale and a straight scale would leave the meter nearly flat.
pub fn level(value: u64, now_ms: i64) -> f32 {
    let (peak, at) = unpack(value);
    (decayed(peak, at, now_ms) as f32 / FULL_SCALE).sqrt().min(1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_value_packs_and_unpacks() {
        let v = pack(12_345, 1_780_000_000_123);
        assert_eq!(unpack(v), (12_345, 1_780_000_000_123));
    }

    #[test]
    fn a_peak_falls_linearly_to_zero() {
        assert_eq!(decayed(10_000, 1_000, 1_000), 10_000);
        assert_eq!(decayed(10_000, 1_000, 1_200), 5_000);
        assert_eq!(decayed(10_000, 1_000, 1_000 + DECAY_MS), 0);
        assert_eq!(decayed(10_000, 1_000, 99_000), 0);
        assert_eq!(decayed(10_000, 1_000, 500), 10_000, "a clock that moved back does not raise it");
    }

    #[test]
    fn a_packet_keeps_the_louder_of_itself_and_the_decayed_peak() {
        let first = after_packet(0, 8_000, 1_000);
        assert_eq!(unpack(first), (8_000, 1_000));
        let quieter = after_packet(first, 1_000, 1_100);
        assert_eq!(unpack(quieter).0, 6_000, "the old peak, decayed a quarter, still beats the packet");
        let louder = after_packet(quieter, -20_000, 1_110);
        assert_eq!(unpack(louder), (20_000, 1_110), "the sign does not matter");
        assert_eq!(unpack(after_packet(0, i16::MIN, 5)).0, 32_767);
    }

    #[test]
    fn the_level_is_zero_with_no_audio_and_one_at_full_scale() {
        assert_eq!(level(0, 1_000), 0.0);
        assert_eq!(level(pack(32_767, 1_000), 1_000), 1.0);
        let quiet = level(pack(3_277, 1_000), 1_000);
        assert!(quiet > 0.3 && quiet < 0.32, "a tenth of full scale reads about a third: {quiet}");
    }

    #[test]
    fn a_silent_source_reads_zero_once_its_last_peak_has_decayed() {
        let v = pack(20_000, 1_000);
        assert!(level(v, 1_100) > 0.0);
        assert_eq!(level(v, 1_000 + DECAY_MS), 0.0);
    }
}
