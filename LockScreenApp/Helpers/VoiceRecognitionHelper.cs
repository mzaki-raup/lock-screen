using System;
using System.Linq;
using System.Threading.Tasks;
using Windows.Media.SpeechRecognition;
using Windows.Globalization;

namespace LockScreenApp.Helpers
{
    /// <summary>
    /// Helper class for voice recognition using Windows Speech API
    /// </summary>
    public class VoiceRecognitionHelper : IDisposable
    {
        private SpeechRecognizer? speechRecognizer;
        private bool isInitialized;
        private bool isDisposed;

        /// <summary>
        /// Initialize the speech recognizer with grammar constraint for "unlock" command
        /// </summary>
        public async Task<bool> InitializeAsync()
        {
            if (isInitialized)
                return true;

            try
            {
                // Create speech recognizer using default language
                var defaultLanguage = SpeechRecognizer.SystemSpeechLanguage;
                speechRecognizer = new SpeechRecognizer(defaultLanguage);

                // Create grammar constraint with "unlock" command
                var grammarConstraint = new SpeechRecognitionListConstraint(
                    new[] { "unlock" },
                    "unlockCommand"
                );

                // Add constraint and compile
                speechRecognizer.Constraints.Add(grammarConstraint);
                var compilationResult = await speechRecognizer.CompileConstraintsAsync();

                if (compilationResult.Status != SpeechRecognitionResultStatus.Success)
                {
                    return false;
                }

                // Configure timeouts for better user experience
                speechRecognizer.Timeouts.InitialSilenceTimeout = TimeSpan.FromSeconds(5);
                speechRecognizer.Timeouts.EndSilenceTimeout = TimeSpan.FromSeconds(0.5);

                isInitialized = true;
                return true;
            }
            catch (Exception)
            {
                // Failed to initialize (no microphone, permissions denied, etc.)
                return false;
            }
        }

        /// <summary>
        /// Listen for the "unlock" command
        /// </summary>
        /// <param name="timeout">Maximum time to wait for command</param>
        /// <returns>Success status, recognized command, and confidence score</returns>
        public async Task<(bool Success, string Command, double Confidence)> ListenForCommandAsync(TimeSpan timeout)
        {
            if (!isInitialized || speechRecognizer == null)
            {
                return (false, string.Empty, 0.0);
            }

            try
            {
                // Start recognition with timeout
                var recognitionTask = speechRecognizer.RecognizeAsync().AsTask();
                var timeoutTask = Task.Delay(timeout);

                var completedTask = await Task.WhenAny(recognitionTask, timeoutTask);

                if (completedTask == timeoutTask)
                {
                    // Timeout occurred
                    return (false, string.Empty, 0.0);
                }

                var result = await recognitionTask;

                if (result.Status == SpeechRecognitionResultStatus.Success)
                {
                    string recognizedText = result.Text?.ToLowerInvariant() ?? string.Empty;
                    double confidence = result.Confidence == SpeechRecognitionConfidence.High ? 1.0 :
                                       result.Confidence == SpeechRecognitionConfidence.Medium ? 0.7 :
                                       result.Confidence == SpeechRecognitionConfidence.Low ? 0.4 : 0.0;

                    return (true, recognizedText, confidence);
                }

                // Recognition failed or was rejected
                return (false, string.Empty, 0.0);
            }
            catch (Exception)
            {
                // Recognition error (microphone disconnected, etc.)
                return (false, string.Empty, 0.0);
            }
        }

        /// <summary>
        /// Check if microphone is available on the system
        /// </summary>
        public static async Task<bool> IsMicrophoneAvailableAsync()
        {
            try
            {
                // Try to create a temporary recognizer to test microphone availability
                using var testRecognizer = new SpeechRecognizer();
                await testRecognizer.CompileConstraintsAsync();
                return true;
            }
            catch (Exception)
            {
                return false;
            }
        }

        /// <summary>
        /// Get the current system language for speech recognition
        /// </summary>
        public static string GetSystemLanguage()
        {
            try
            {
                return SpeechRecognizer.SystemSpeechLanguage?.DisplayName ?? "Unknown";
            }
            catch
            {
                return "Unknown";
            }
        }

        public void Dispose()
        {
            if (isDisposed)
                return;

            speechRecognizer?.Dispose();
            speechRecognizer = null;
            isInitialized = false;
            isDisposed = true;
        }
    }
}
