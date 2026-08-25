using System;
using System.Threading.Tasks;
using Windows.Media.SpeechRecognition;

namespace SettingsApp.Helpers
{
    /// <summary>
    /// Helper class for voice recognition testing in settings
    /// </summary>
    public class VoiceRecognitionHelper : IDisposable
    {
        private SpeechRecognizer? speechRecognizer;
        private bool isDisposed;

        /// <summary>
        /// Initialize the speech recognizer with grammar constraint for "unlock" command
        /// </summary>
        public async Task<bool> InitializeAsync()
        {
            try
            {
                speechRecognizer = new SpeechRecognizer(SpeechRecognizer.SystemSpeechLanguage);

                var grammarConstraint = new SpeechRecognitionListConstraint(
                    new[] { "unlock" },
                    "unlockCommand"
                );

                speechRecognizer.Constraints.Add(grammarConstraint);
                var compilationResult = await speechRecognizer.CompileConstraintsAsync();

                if (compilationResult.Status != SpeechRecognitionResultStatus.Success)
                {
                    return false;
                }

                speechRecognizer.Timeouts.InitialSilenceTimeout = TimeSpan.FromSeconds(5);
                speechRecognizer.Timeouts.EndSilenceTimeout = TimeSpan.FromSeconds(0.5);

                return true;
            }
            catch (Exception)
            {
                return false;
            }
        }

        /// <summary>
        /// Listen for the "unlock" command (for testing)
        /// </summary>
        public async Task<(bool Success, string Command)> ListenForCommandAsync(TimeSpan timeout)
        {
            if (speechRecognizer == null)
            {
                return (false, string.Empty);
            }

            try
            {
                var recognitionTask = speechRecognizer.RecognizeAsync().AsTask();
                var timeoutTask = Task.Delay(timeout);

                var completedTask = await Task.WhenAny(recognitionTask, timeoutTask);

                if (completedTask == timeoutTask)
                {
                    return (false, string.Empty);
                }

                var result = await recognitionTask;

                if (result.Status == SpeechRecognitionResultStatus.Success)
                {
                    string recognizedText = result.Text?.ToLowerInvariant() ?? string.Empty;
                    return (true, recognizedText);
                }

                return (false, string.Empty);
            }
            catch (Exception)
            {
                return (false, string.Empty);
            }
        }

        /// <summary>
        /// Check if microphone is available
        /// </summary>
        public static async Task<bool> IsMicrophoneAvailableAsync()
        {
            try
            {
                using var testRecognizer = new SpeechRecognizer();
                await testRecognizer.CompileConstraintsAsync();
                return true;
            }
            catch (Exception)
            {
                return false;
            }
        }

        public void Dispose()
        {
            if (isDisposed)
                return;

            speechRecognizer?.Dispose();
            speechRecognizer = null;
            isDisposed = true;
        }
    }
}
