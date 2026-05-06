//! Application state machine for the voice dictation app.
//!
//! States:
//! - Idle: Waiting for hotkey press
//! - Recording: Audio capture active (PTT or Hands-free mode)
//! - Processing: Audio sent to ASR, waiting for result
//! - Pasting: Text received, pasting to cursor

/// Top-level application state.
#[derive(Debug, Clone, PartialEq)]
pub enum AppState {
    Idle,
    Recording(RecordingMode),
    Processing,
    Pasting,
}

/// Recording mode determines how the hotkey interacts with recording.
#[derive(Debug, Clone, PartialEq)]
pub enum RecordingMode {
    PushToTalk,
    HandsFree,
}

/// Events that drive state transitions.
#[derive(Debug, Clone)]
pub enum AppEvent {
    HotKeyDown,
    HotKeyUp,
    VadSilenceStart,
    VadSilenceEnd,
    SilenceTimeout,
    AsrResult(String),
    AsrError(String),
    PasteComplete,
    CancelEsc,
}

/// Side effects produced by state transitions.
#[derive(Debug, Clone)]
pub enum AppAction {
    StartRecording,
    StopRecording,
    PasteText(String),
    PlayStartSound,
    PlayStopSound,
    PlayErrorSound,
    PlayWarningSound,
    ShowOverlay(String),
    HideOverlay,
    ShowNotification(String),
    /// Cancel recording: stop recording and save partial result to history.
    /// Contains optional partial ASR text (if cancellation occurred during Processing state).
    CancelRecording(Option<String>),
}

/// Pure state transition function.
///
/// Given the current state and an event, returns the new state and a list
/// of side-effect actions that the caller should execute.
///
/// Transition table:
/// - Idle + HotKeyDown → Recording(mode), [StartRecording, PlayStartSound, ShowOverlay("Recording...")]
/// - Recording(PTT) + HotKeyUp → Processing, [StopRecording, PlayStopSound, ShowOverlay("Processing...")]
/// - Recording(HandsFree) + HotKeyDown → Processing, [StopRecording, PlayStopSound, ShowOverlay("Processing...")]
/// - Recording(HandsFree) + SilenceTimeout → Processing, [StopRecording, PlayStopSound, ShowOverlay("Processing...")]
/// - Recording(HandsFree) + VadSilenceStart → Recording(HandsFree), [ShowOverlay("Silence detected... auto-stopping soon")]
/// - Recording(HandsFree) + VadSilenceEnd → Recording(HandsFree), [ShowOverlay("Recording...")]
/// - Recording(_) + CancelEsc → Idle, [StopRecording, HideOverlay]
/// - Processing + AsrResult(text) → Pasting, [PasteText(text), ShowOverlay("Pasting...")]
/// - Processing + AsrError(msg) → Idle, [HideOverlay, ShowNotification(msg)]
/// - Pasting + PasteComplete → Idle, [HideOverlay, SaveHistory(...)]
pub fn handle_event(
    state: AppState,
    event: AppEvent,
    mode: RecordingMode,
) -> (AppState, Vec<AppAction>) {
    match (state.clone(), event) {
        // Idle + HotKeyDown → start recording
        (AppState::Idle, AppEvent::HotKeyDown) => (
            AppState::Recording(mode.clone()),
            vec![
                AppAction::StartRecording,
                AppAction::PlayStartSound,
                AppAction::ShowOverlay("Recording...".to_string()),
            ],
        ),

        // PTT: HotKeyUp stops recording
        (AppState::Recording(RecordingMode::PushToTalk), AppEvent::HotKeyUp) => (
            AppState::Processing,
            vec![
                AppAction::StopRecording,
                AppAction::PlayStopSound,
                AppAction::ShowOverlay("Processing...".to_string()),
            ],
        ),

        // HandsFree: second HotKeyDown stops recording
        (AppState::Recording(RecordingMode::HandsFree), AppEvent::HotKeyDown) => (
            AppState::Processing,
            vec![
                AppAction::StopRecording,
                AppAction::PlayStopSound,
                AppAction::ShowOverlay("Processing...".to_string()),
            ],
        ),

        // HandsFree: silence timeout stops recording
        (AppState::Recording(RecordingMode::HandsFree), AppEvent::SilenceTimeout) => (
            AppState::Processing,
            vec![
                AppAction::StopRecording,
                AppAction::PlayStopSound,
                AppAction::ShowOverlay("Processing...".to_string()),
            ],
        ),

        // HandsFree: VAD silence start
        (AppState::Recording(RecordingMode::HandsFree), AppEvent::VadSilenceStart) => (
            AppState::Recording(RecordingMode::HandsFree),
            vec![
                AppAction::PlayWarningSound,
                AppAction::ShowOverlay(
                    "Silence detected... auto-stopping soon".to_string(),
                ),
            ],
        ),

        // HandsFree: VAD silence end (speech resumed)
        (AppState::Recording(RecordingMode::HandsFree), AppEvent::VadSilenceEnd) => (
            AppState::Recording(RecordingMode::HandsFree),
            vec![AppAction::ShowOverlay("Recording...".to_string())],
        ),

        // Any recording mode: cancel
        (AppState::Recording(_), AppEvent::CancelEsc) => (
            AppState::Idle,
            vec![AppAction::CancelRecording(None), AppAction::HideOverlay],
        ),

        // Processing: cancel (abort ASR, save nothing since we don't have the result yet)
        (AppState::Processing, AppEvent::CancelEsc) => (
            AppState::Idle,
            vec![AppAction::CancelRecording(None), AppAction::HideOverlay],
        ),

        // Processing: ASR success
        (AppState::Processing, AppEvent::AsrResult(text)) => (
            AppState::Pasting,
            vec![
                AppAction::PasteText(text.clone()),
                AppAction::ShowOverlay("Pasting...".to_string()),
            ],
        ),

        // Processing: ASR failure
        (AppState::Processing, AppEvent::AsrError(msg)) => (
            AppState::Idle,
            vec![AppAction::PlayErrorSound, AppAction::HideOverlay, AppAction::ShowNotification(msg)],
        ),

        // Pasting: complete → idle
        (AppState::Pasting, AppEvent::PasteComplete) => {
            (
                AppState::Idle,
                vec![AppAction::HideOverlay],
            )
        }

        // All other combinations: no state change, no actions
        _ => (state, vec![]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_hotkey_down_starts_recording_ptt() {
        let (new_state, actions) = handle_event(
            AppState::Idle,
            AppEvent::HotKeyDown,
            RecordingMode::PushToTalk,
        );
        assert_eq!(new_state, AppState::Recording(RecordingMode::PushToTalk));
        assert!(actions
            .iter()
            .any(|a| matches!(a, AppAction::StartRecording)));
        assert!(actions
            .iter()
            .any(|a| matches!(a, AppAction::PlayStartSound)));
    }

    #[test]
    fn idle_hotkey_down_starts_recording_handsfree() {
        let (new_state, actions) = handle_event(
            AppState::Idle,
            AppEvent::HotKeyDown,
            RecordingMode::HandsFree,
        );
        assert_eq!(new_state, AppState::Recording(RecordingMode::HandsFree));
        assert!(actions
            .iter()
            .any(|a| matches!(a, AppAction::StartRecording)));
    }

    #[test]
    fn ptt_hotkey_up_stops_recording() {
        let (new_state, actions) = handle_event(
            AppState::Recording(RecordingMode::PushToTalk),
            AppEvent::HotKeyUp,
            RecordingMode::PushToTalk,
        );
        assert_eq!(new_state, AppState::Processing);
        assert!(actions
            .iter()
            .any(|a| matches!(a, AppAction::StopRecording)));
        assert!(actions.iter().any(|a| matches!(a, AppAction::PlayStopSound)));
    }

    #[test]
    fn handsfree_hotkey_down_stops_recording() {
        let (new_state, actions) = handle_event(
            AppState::Recording(RecordingMode::HandsFree),
            AppEvent::HotKeyDown,
            RecordingMode::HandsFree,
        );
        assert_eq!(new_state, AppState::Processing);
        assert!(actions
            .iter()
            .any(|a| matches!(a, AppAction::StopRecording)));
    }

    #[test]
    fn handsfree_silence_timeout_stops_recording() {
        let (new_state, _actions) = handle_event(
            AppState::Recording(RecordingMode::HandsFree),
            AppEvent::SilenceTimeout,
            RecordingMode::HandsFree,
        );
        assert_eq!(new_state, AppState::Processing);
    }

    #[test]
    fn handsfree_vad_silence_start_shows_warning() {
        let (new_state, actions) = handle_event(
            AppState::Recording(RecordingMode::HandsFree),
            AppEvent::VadSilenceStart,
            RecordingMode::HandsFree,
        );
        assert_eq!(new_state, AppState::Recording(RecordingMode::HandsFree));
        assert!(actions
            .iter()
            .any(|a| matches!(a, AppAction::ShowOverlay(_))));
    }

    #[test]
    fn handsfree_vad_silence_end_resumes_overlay() {
        let (new_state, actions) = handle_event(
            AppState::Recording(RecordingMode::HandsFree),
            AppEvent::VadSilenceEnd,
            RecordingMode::HandsFree,
        );
        assert_eq!(new_state, AppState::Recording(RecordingMode::HandsFree));
        assert!(actions
            .iter()
            .any(|a| matches!(a, AppAction::ShowOverlay(_))));
    }

    #[test]
    fn cancel_esc_during_recording() {
        let (new_state, actions) = handle_event(
            AppState::Recording(RecordingMode::PushToTalk),
            AppEvent::CancelEsc,
            RecordingMode::PushToTalk,
        );
        assert_eq!(new_state, AppState::Idle);
        assert!(actions
            .iter()
            .any(|a| matches!(a, AppAction::CancelRecording(_))));
        assert!(actions.iter().any(|a| matches!(a, AppAction::HideOverlay)));
    }

    #[test]
    fn asr_result_triggers_pasting() {
        let (new_state, actions) = handle_event(
            AppState::Processing,
            AppEvent::AsrResult("hello world".to_string()),
            RecordingMode::PushToTalk,
        );
        assert_eq!(new_state, AppState::Pasting);
        assert!(actions
            .iter()
            .any(|a| matches!(a, AppAction::PasteText(_))));
    }

    #[test]
    fn asr_error_returns_to_idle() {
        let (new_state, actions) = handle_event(
            AppState::Processing,
            AppEvent::AsrError("timeout".to_string()),
            RecordingMode::PushToTalk,
        );
        assert_eq!(new_state, AppState::Idle);
        assert!(actions.iter().any(|a| matches!(a, AppAction::PlayErrorSound)));
        assert!(actions.iter().any(|a| matches!(a, AppAction::HideOverlay)));
        assert!(actions
            .iter()
            .any(|a| matches!(a, AppAction::ShowNotification(_))));
    }

    #[test]
    fn paste_complete_returns_to_idle() {
        let (new_state, actions) = handle_event(
            AppState::Pasting,
            AppEvent::PasteComplete,
            RecordingMode::PushToTalk,
        );
        assert_eq!(new_state, AppState::Idle);
        assert!(actions.iter().any(|a| matches!(a, AppAction::HideOverlay)));
    }

    #[test]
    fn ignored_events_produce_no_change() {
        // HotKeyUp in Idle — ignored
        let (s, a) = handle_event(AppState::Idle, AppEvent::HotKeyUp, RecordingMode::PushToTalk);
        assert_eq!(s, AppState::Idle);
        assert!(a.is_empty());

        // AsrResult in Idle — ignored
        let (s, a) = handle_event(
            AppState::Idle,
            AppEvent::AsrResult("x".to_string()),
            RecordingMode::HandsFree,
        );
        assert_eq!(s, AppState::Idle);
        assert!(a.is_empty());

        // CancelEsc in Processing — now handled (cancels ASR)
        let (s, a) = handle_event(
            AppState::Processing,
            AppEvent::CancelEsc,
            RecordingMode::PushToTalk,
        );
        assert_eq!(s, AppState::Idle);
        assert!(!a.is_empty());
    }
}
