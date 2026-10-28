//! Is this machine in the call, or only near one? With several machines in one room, a
//! calendar event on a machine that is not in the call used to auto-record whatever its
//! microphone overheard and file it under the event's name. Ported from the Mac's #203
//! fix: the auto-start countdown runs only while another app holds the microphone (every
//! Zoom/Teams/Meet call does), and a calendar recording with no call audio is retitled.
//! Pure: the shell observes the microphone and the transcript, these functions decide.

/// What a calendar recording with no call audio is called instead of the event's name.
pub const ROOM_AUDIO_PREFIX: &str = "Room audio only · ";

/// When a calendar prompt's auto-start countdown starts counting: never while no other app
/// holds the microphone (`None`: the prompt waits), otherwise at the later of the prompt
/// appearing and the microphone first being seen busy, so a call that was already running
/// does not start the countdown before the user could see the prompt.
pub fn countdown_start(prompted_unix_ms: i64, mic_busy_since_unix_ms: Option<i64>) -> Option<i64> {
    mic_busy_since_unix_ms.map(|busy_since| busy_since.max(prompted_unix_ms))
}

/// The title for a finished recording, when it should change: a calendar-triggered
/// meeting whose transcript has lines and every line is the local speaker (`me_label`)
/// heard nothing from the call, so it is room audio, not the event. `None` otherwise, and
/// for a title already marked, so re-running the pipeline never stacks prefixes.
pub fn room_audio_title(is_calendar: bool, title: &str, speaker_labels: &[&str], me_label: &str) -> Option<String> {
    let only_the_local_speaker = !speaker_labels.is_empty() && speaker_labels.iter().all(|label| *label == me_label);
    (is_calendar && !title.is_empty() && !title.starts_with(ROOM_AUDIO_PREFIX) && only_the_local_speaker)
        .then(|| format!("{ROOM_AUDIO_PREFIX}{title}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_countdown_waits_while_no_other_app_has_the_microphone() {
        assert_eq!(countdown_start(10_000, None), None);
    }

    #[test]
    fn a_call_that_begins_while_the_prompt_shows_starts_the_countdown_then() {
        assert_eq!(countdown_start(10_000, Some(14_000)), Some(14_000));
    }

    #[test]
    fn a_call_already_running_when_the_prompt_appears_counts_from_the_prompt() {
        assert_eq!(countdown_start(10_000, Some(2_000)), Some(10_000));
    }

    #[test]
    fn a_calendar_recording_with_only_the_local_speaker_is_room_audio() {
        assert_eq!(
            room_audio_title(true, "Daily Win Room", &["Me", "Me"], "Me").as_deref(),
            Some("Room audio only · Daily Win Room")
        );
    }

    #[test]
    fn any_line_from_the_call_means_it_was_a_real_recording() {
        assert_eq!(room_audio_title(true, "Standup", &["Me", "Speaker 1"], "Me"), None);
        assert_eq!(room_audio_title(true, "Standup", &["Others"], "Me"), None);
    }

    #[test]
    fn manual_detected_and_empty_recordings_keep_their_titles() {
        assert_eq!(room_audio_title(false, "Zoom", &["Me"], "Me"), None, "not a calendar meeting");
        assert_eq!(room_audio_title(true, "Standup", &[], "Me"), None, "no transcript lines");
        assert_eq!(room_audio_title(true, "", &["Me"], "Me"), None, "no title to prefix");
    }

    #[test]
    fn rerunning_the_pipeline_does_not_stack_prefixes() {
        assert_eq!(room_audio_title(true, "Room audio only · Standup", &["Me"], "Me"), None);
    }
}
