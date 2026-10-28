//! Reply Priorities: the five raw TypeSafe judgments per thread and the formula that
//! turns them into one ordering number. See
//! docs/superpowers/specs/2026-10-05-reply-priorities-design.md.

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PriorityJudgments {
    pub asks_reply: f64,
    /// 0.0 (no time pressure) to 3.0 (needs a response now).
    pub urgency: f64,
    pub person: f64,
    pub bulk: f64,
    pub automated: f64,
}

// Calibration, tuned against the labeled spike. Change only after re-validating on
// fresh labels (spec "Ship gate"); every constant lives here.
const URGENCY_FLOOR: f64 = 0.4;
const URGENCY_MAX_LEVEL: f64 = 3.0;
const PERSON_BOOST: f64 = 0.15;

impl PriorityJudgments {
    /// `asks_reply × (1 − bulk) × urgency factor + person boost`, times `(1 − automated)`.
    pub fn priority(&self) -> f64 {
        let urgency_factor = URGENCY_FLOOR
            + (1.0 - URGENCY_FLOOR) * (self.urgency / URGENCY_MAX_LEVEL).clamp(0.0, 1.0);
        let base = self.asks_reply * (1.0 - self.bulk) * urgency_factor + PERSON_BOOST * self.person;
        base * (1.0 - self.automated)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn j(asks_reply: f64, urgency: f64, person: f64, bulk: f64, automated: f64) -> PriorityJudgments {
        PriorityJudgments { asks_reply, urgency, person, bulk, automated }
    }

    #[test]
    fn a_personal_ask_outranks_an_automated_notification() {
        // Real shapes from the 30-thread labeled spike: the AlphaSights confirmation
        // versus a "New sign-in to your account" alert that reads as urgent.
        let personal = j(0.99, 2.20, 0.69, 0.09, 0.03).priority();
        let sign_in = j(0.92, 2.72, 0.02, 0.38, 0.97).priority();
        assert!(personal > 0.8, "personal ask scored {personal}");
        assert!(sign_in < 0.05, "automated alert scored {sign_in}");
    }

    #[test]
    fn bulk_mail_scores_near_zero() {
        let redfin = j(0.81, 0.99, 0.02, 0.89, 0.77).priority();
        assert!(redfin < 0.05, "bulk scored {redfin}");
    }

    #[test]
    fn a_real_reply_with_moderate_automated_score_survives() {
        // AVTSupport: a support-system mail that still needs the user's reply.
        let support = j(0.97, 0.82, 0.07, 0.50, 0.56).priority();
        assert!(support > 0.1, "support reply scored {support}");
    }

    #[test]
    fn urgency_is_clamped_to_its_scale() {
        assert_eq!(j(1.0, 5.0, 0.0, 0.0, 0.0).priority(), j(1.0, 3.0, 0.0, 0.0, 0.0).priority());
        assert_eq!(j(1.0, -2.0, 0.0, 0.0, 0.0).priority(), j(1.0, 0.0, 0.0, 0.0, 0.0).priority());
    }

    #[test]
    fn all_zero_judgments_score_zero() {
        assert_eq!(j(0.0, 0.0, 0.0, 0.0, 0.0).priority(), 0.0);
    }
}
