//! Voice unlock, through WinRT's `SpeechRecognizer`.
//!
//! # Read this before changing anything here
//!
//! Windows speech recognition is **speaker-independent**. It recognises the
//! word, not the person. Anyone standing at the machine who says the command
//! out loud gets in, and so does a recording, and so does somebody in the next
//! room if the microphone is good enough. This is a convenience that bypasses
//! the password, not a second factor, and no amount of tuning makes it one.
//!
//! It is therefore off by default and stays off by default. The study (§7)
//! records the reasoning.
//!
//! # The confidence bug this corrects
//!
//! `PasswordWindow.xaml.cs:235` admits a match when
//! `confidence >= voiceConfig.ConfidenceThreshold`, and
//! `VoiceRecognitionHelper.cs:92-94` maps WinRT's `Medium` to exactly `0.7` --
//! which is also the default threshold `IniHelper` writes. So on a default
//! install a *medium*-confidence guess at the word unlocks the screen.
//!
//! The thresholds are kept as floats for compatibility with configs the .NET
//! app wrote, but they are converted to an ordered level before anything is
//! compared: `SpeechRecognitionConfidence` is numbered `High = 0 … Rejected =
//! 3`, so comparing the raw values would silently invert the test.

use std::sync::mpsc;
use std::time::Duration;

use windows::Foundation::TimeSpan;
use windows::Media::SpeechRecognition::{
    SpeechRecognitionConfidence, SpeechRecognitionListConstraint, SpeechRecognitionResultStatus, SpeechRecognizer,
};
use windows::core::HSTRING;
use windows_collections::IIterable;

/// How sure the recogniser was, ordered worst to best so comparison is
/// meaningful. WinRT numbers these the other way round.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Confidence {
    Rejected,
    Low,
    Medium,
    High,
}

impl Confidence {
    fn from_winrt(value: SpeechRecognitionConfidence) -> Self {
        match value {
            SpeechRecognitionConfidence::High => Confidence::High,
            SpeechRecognitionConfidence::Medium => Confidence::Medium,
            SpeechRecognitionConfidence::Low => Confidence::Low,
            // Rejected, and anything added later: the safe end.
            _ => Confidence::Rejected,
        }
    }
}

/// The lowest confidence a stored threshold admits.
///
/// Mirrors the .NET float mapping exactly -- `High` 1.0, `Medium` 0.7, `Low`
/// 0.4 -- under the old `confidence >= threshold` test, so a config carried
/// over behaves as it used to. The *default* is what changed: `config.rs` now
/// writes `1.00`, which is `High` only.
pub fn required_level(threshold: f64) -> Confidence {
    if threshold > 0.7 {
        Confidence::High
    } else if threshold > 0.4 {
        Confidence::Medium
    } else if threshold > 0.0 {
        Confidence::Low
    } else {
        // A threshold of zero would admit a rejection, which cannot be what
        // anybody means. Floor it at Low.
        Confidence::Low
    }
}

/// What a listen attempt produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The expected command, clearly enough. Unlock.
    Accepted,
    /// Something was heard, but not the command, or not clearly enough.
    /// Carries what was heard so the prompt can say so -- never logged.
    Rejected { heard: String, confidence: Confidence },
    /// Nothing was said before the timeout.
    Silence,
    /// The recogniser could not be used: no microphone, permission refused, or
    /// speech is not installed for this language.
    Unavailable,
}

impl Outcome {
    /// The line to show under the prompt.
    pub fn message(&self) -> String {
        match self {
            Outcome::Accepted => "Recognised.".to_owned(),
            Outcome::Rejected { heard, .. } if heard.is_empty() => {
                "That was not the unlock command. Try again, or use your password.".to_owned()
            }
            Outcome::Rejected { heard, .. } => {
                format!("Heard \u{201c}{heard}\u{201d}, which is not the unlock command.")
            }
            Outcome::Silence => "Nothing heard. Try again, or use your password.".to_owned(),
            Outcome::Unavailable => "Voice unlock is not available on this machine.".to_owned(),
        }
    }
}

/// Build a recogniser that listens for exactly one phrase.
///
/// `patience` becomes the initial-silence timeout, which is what the config's
/// `ListeningTimeout` actually controls: how long to wait for someone to start
/// speaking. WinRT enforces its own bounds, so it is clamped to something it
/// will accept rather than passed through and silently rejected.
fn recognizer(command: &str, patience: Duration) -> windows::core::Result<SpeechRecognizer> {
    let recognizer = SpeechRecognizer::new()?;

    // A list constraint rather than free dictation: the recogniser only ever
    // has one phrase to match, which is both far more reliable and means the
    // microphone's contents are never transcribed into anything.
    let phrases: IIterable<HSTRING> = vec![HSTRING::from(command)].into();
    let constraint = SpeechRecognitionListConstraint::CreateWithTag(&phrases, &HSTRING::from("unlockCommand"))?;
    recognizer.Constraints()?.Append(&constraint)?;

    let compiled = recognizer.CompileConstraintsAsync()?.join()?;
    if compiled.Status()? != SpeechRecognitionResultStatus::Success {
        return Err(windows::core::Error::from_hresult(windows::core::HRESULT(-1)));
    }

    // WinRT counts in 100-nanosecond ticks.
    let ticks = |d: Duration| TimeSpan { Duration: (d.as_nanos() / 100) as i64 };
    let timeouts = recognizer.Timeouts()?;
    // Both are best-effort: WinRT rejects values outside its own range, and a
    // rejected timeout just leaves the default in place rather than failing the
    // whole attempt.
    let _ = timeouts.SetInitialSilenceTimeout(ticks(patience.clamp(PATIENCE_MIN, PATIENCE_MAX)));
    let _ = timeouts.SetEndSilenceTimeout(ticks(Duration::from_millis(500)));

    Ok(recognizer)
}

/// Bounds WinRT will accept for the initial-silence timeout.
const PATIENCE_MIN: Duration = Duration::from_secs(1);
const PATIENCE_MAX: Duration = Duration::from_secs(10);

/// Clamp a configured `ListeningTimeout` in seconds to what WinRT allows.
pub fn patience(seconds: u32) -> Duration {
    Duration::from_secs(u64::from(seconds)).clamp(PATIENCE_MIN, PATIENCE_MAX)
}

/// Whether voice unlock can be offered at all.
///
/// Compiling a constraint is the only honest probe: a machine can have a
/// microphone and still refuse, because Windows privacy settings gate access
/// per-application and speech may not be installed for the system language.
///
/// Not called from the unlock prompt, on purpose. Probing takes about a second,
/// and stalling the prompt that long to decide whether to draw a button is a
/// bad trade when the button can simply report [`Outcome::Unavailable`] if it
/// is pressed. Settings mode wants it for its "Test microphone" button, which
/// is where a one-second wait is expected.
#[allow(dead_code, reason = "used by settings mode in phase 05")]
pub fn is_available(command: &str) -> bool {
    recognizer(command, PATIENCE_MIN).is_ok()
}

/// Listen once, blocking until the recogniser answers.
///
/// There is no manual deadline here on purpose. `RecognizeAsync` honours the
/// silence timeouts set above and returns on its own, so a hand-rolled poll
/// loop would only add a second, worse timeout -- and the first draft of it
/// reached for private API to do so.
fn listen_blocking(command: &str, required: Confidence, patience: Duration) -> Outcome {
    let Ok(recognizer) = recognizer(command, patience) else {
        return Outcome::Unavailable;
    };

    let Ok(result) = recognizer.RecognizeAsync().and_then(|op| op.join()) else {
        // A cancelled or failed recognition is indistinguishable from nothing
        // being said, and both mean the same thing to the user.
        return Outcome::Silence;
    };

    match result.Status() {
        Ok(SpeechRecognitionResultStatus::Success) => {}
        // TimeoutExceeded and the rest all mean "nothing usable was said".
        Ok(_) | Err(_) => return Outcome::Silence,
    }

    let heard = result.Text().map(|t| t.to_string_lossy()).unwrap_or_default().to_lowercase();
    let confidence = result.Confidence().map(Confidence::from_winrt).unwrap_or(Confidence::Rejected);

    // Both tests, every time: the right phrase *and* clearly enough. The .NET
    // code checks both too; what it got wrong was where the bar sat.
    if heard == command.to_lowercase() && confidence >= required {
        Outcome::Accepted
    } else {
        Outcome::Rejected { heard, confidence }
    }
}

/// Listen on a worker thread, answering over a channel.
///
/// `RecognizeAsync` blocks and the prompt owns a message loop, so this cannot
/// run on the UI thread -- the same split the Hello request uses.
pub fn listen_on_worker(command: &str, required: Confidence, patience: Duration) -> mpsc::Receiver<Outcome> {
    let (tx, rx) = mpsc::channel();
    let command = command.to_owned();
    std::thread::spawn(move || {
        // A send into a dropped channel is fine: it just means the prompt
        // closed while the recogniser was still listening.
        let _ = tx.send(listen_blocking(&command, required, patience));
    });
    rx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn confidence_is_ordered_best_last_unlike_winrt() {
        assert!(Confidence::High > Confidence::Medium);
        assert!(Confidence::Medium > Confidence::Low);
        assert!(Confidence::Low > Confidence::Rejected);
        // WinRT numbers them the other way round, so a raw comparison of the
        // underlying values would invert the gate. Pinned so that a future
        // "simplification" back to the raw enum fails here.
        assert_eq!(SpeechRecognitionConfidence::High.0, 0);
        assert_eq!(SpeechRecognitionConfidence::Rejected.0, 3);
        assert_eq!(Confidence::from_winrt(SpeechRecognitionConfidence::High), Confidence::High);
        assert_eq!(Confidence::from_winrt(SpeechRecognitionConfidence::Rejected), Confidence::Rejected);
        // An unknown future value must land on the safe end.
        assert_eq!(Confidence::from_winrt(SpeechRecognitionConfidence(99)), Confidence::Rejected);
    }

    #[test]
    fn the_dotnet_default_threshold_admits_a_medium_guess() {
        // This is the bug. IniHelper writes ConfidenceThreshold=0.7 and
        // VoiceRecognitionHelper maps Medium to exactly 0.7, so on a default
        // install `confidence >= threshold` is true for a medium-confidence
        // guess at the word. Asserted rather than described, so nobody restores
        // 0.7 as a default without this failing.
        assert_eq!(required_level(0.7), Confidence::Medium);
        assert!(Confidence::Medium >= required_level(0.7), "a medium guess passes the old default");
    }

    #[test]
    fn the_new_default_admits_only_high_confidence() {
        let shipped = lockscreen_core::config::VoiceConfig::default().confidence_threshold;
        assert_eq!(shipped, 1.0, "the shipped default must be strict");
        assert_eq!(required_level(shipped), Confidence::High);
        assert!(Confidence::Medium < required_level(shipped), "a medium guess no longer passes");
        assert!(Confidence::High >= required_level(shipped));
    }

    #[test]
    fn thresholds_map_the_same_way_the_dotnet_floats_did() {
        // Compatibility: a config carried over behaves as it used to.
        assert_eq!(required_level(1.0), Confidence::High);
        assert_eq!(required_level(0.8), Confidence::High);
        assert_eq!(required_level(0.7), Confidence::Medium);
        assert_eq!(required_level(0.5), Confidence::Medium);
        assert_eq!(required_level(0.4), Confidence::Low);
        assert_eq!(required_level(0.1), Confidence::Low);
        // Never Rejected, whatever the file says.
        assert_eq!(required_level(0.0), Confidence::Low);
        assert_eq!(required_level(-1.0), Confidence::Low);
    }

    #[test]
    fn a_rejection_can_never_satisfy_any_threshold() {
        for threshold in [0.0, 0.1, 0.4, 0.7, 1.0] {
            assert!(Confidence::Rejected < required_level(threshold), "Rejected must not pass threshold {threshold}");
        }
    }

    #[test]
    fn the_configured_timeout_is_clamped_to_what_winrt_accepts() {
        assert_eq!(patience(10), Duration::from_secs(10));
        assert_eq!(patience(5), Duration::from_secs(5));
        // The .NET default.
        assert_eq!(patience(10), PATIENCE_MAX);
        // Nonsense in the file must not produce a timeout WinRT rejects, which
        // would silently leave its own default in place instead.
        assert_eq!(patience(0), PATIENCE_MIN);
        assert_eq!(patience(9999), PATIENCE_MAX);
    }

    #[test]
    fn every_outcome_explains_itself_without_leaking_anything_odd() {
        let outcomes = [
            Outcome::Accepted,
            Outcome::Silence,
            Outcome::Unavailable,
            Outcome::Rejected { heard: String::new(), confidence: Confidence::Low },
            Outcome::Rejected { heard: "unlog".into(), confidence: Confidence::Medium },
        ];
        for outcome in &outcomes {
            let message = outcome.message();
            assert!(!message.is_empty(), "{outcome:?}");
            assert!(message.ends_with('.'), "{outcome:?}: {message}");
        }
        // What was heard is shown back so a near-miss is obvious, but an empty
        // result must not produce empty quotes.
        assert!(!outcomes[3].message().contains("\u{201c}\u{201d}"));
        assert!(outcomes[4].message().contains("unlog"));
    }
}
