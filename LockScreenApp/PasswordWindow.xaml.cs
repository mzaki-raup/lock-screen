using System;
using System.Threading.Tasks;
using System.Windows;
using System.Windows.Input;
using LockScreenApp.Helpers;
using Windows.Security.Credentials.UI;

namespace LockScreenApp
{
    public partial class PasswordWindow : Window
    {
        private readonly string correctPassword;
        private readonly MouseButtonEventHandler outsideClickHandler;
        private bool allowClose;

        // Windows Hello fields
        private bool isWindowsHelloAvailable;
        private bool isWindowsHelloInProgress;
        private bool isPasswordModeEnabled;

        // Voice Recognition fields
        private VoiceRecognitionHelper? voiceHelper;
        private bool isVoiceListening;

        public bool IsUnlocked { get; private set; }
        public bool IsCancelled { get; private set; }

        public PasswordWindow(string password, string title)
        {
            InitializeComponent();
            correctPassword = password;
            Title = title;
            outsideClickHandler = HandleClickOutside;
        }

        private async void PasswordWindow_Loaded(object sender, RoutedEventArgs e)
        {
            Mouse.Capture(this, CaptureMode.SubTree);
            AddHandler(Mouse.PreviewMouseDownOutsideCapturedElementEvent, outsideClickHandler, true);
            await InitializeWindowsHelloAsync();

            if (isWindowsHelloAvailable)
            {
                await AttemptWindowsHelloUnlockAsync();
            }
            else
            {
                ShowPasswordMode();
            }
        }

        private async void RetryWindowsHelloButton_Click(object sender, RoutedEventArgs e)
        {
            await AttemptWindowsHelloUnlockAsync();
        }

        private void UsePasswordButton_Click(object sender, RoutedEventArgs e)
        {
            ShowPasswordMode();
        }

        private void Unlock_Click(object sender, RoutedEventArgs e)
        {
            AttemptPasswordUnlock();
        }

        private void Cancel_Click(object sender, RoutedEventArgs e)
        {
            IsCancelled = true;
            allowClose = true;
            Close();
        }

        private void AttemptPasswordUnlock()
        {
            if (PasswordBox.Password == correctPassword)
            {
                IsUnlocked = true;
                allowClose = true;
                Close();
            }
            else
            {
                SoundHelper.PlayInvalidPasswordSound();
                MessageBox.Show("Incorrect password", "Error", MessageBoxButton.OK, MessageBoxImage.None);
            }
        }

        private async Task AttemptWindowsHelloUnlockAsync()
        {
            if (!isWindowsHelloAvailable || isWindowsHelloInProgress || isPasswordModeEnabled)
            {
                return;
            }

            isWindowsHelloInProgress = true;
            StatusText.Text = "Waiting for Windows Hello...";
            RetryWindowsHelloButton.Visibility = Visibility.Collapsed;

            try
            {
                var result = await UserConsentVerifier.RequestVerificationAsync("Authenticate to unlock");

                switch (result)
                {
                    case UserConsentVerificationResult.Verified:
                        IsUnlocked = true;
                        allowClose = true;
                        Close();
                        break;
                    case UserConsentVerificationResult.RetriesExhausted:
                    case UserConsentVerificationResult.Canceled:
                        StatusText.Text = "Windows Hello cancelled or failed. Try again or use password.";
                        RetryWindowsHelloButton.Visibility = Visibility.Visible;
                        break;
                    case UserConsentVerificationResult.DeviceBusy:
                        StatusText.Text = "Windows Hello device is busy. Try again.";
                        RetryWindowsHelloButton.Visibility = Visibility.Visible;
                        break;
                    default:
                        StatusText.Text = "Windows Hello unavailable. Use password.";
                        ShowPasswordMode();
                        break;
                }
            }
            catch (Exception ex)
            {
                StatusText.Text = $"Windows Hello error: {ex.Message}";
                RetryWindowsHelloButton.Visibility = Visibility.Visible;
            }
            finally
            {
                isWindowsHelloInProgress = false;
            }
        }

        private async Task InitializeWindowsHelloAsync()
        {
            StatusText.Text = "Checking Windows Hello...";
            RetryWindowsHelloButton.Visibility = Visibility.Collapsed;

            try
            {
                var availability = await UserConsentVerifier.CheckAvailabilityAsync();
                isWindowsHelloAvailable = availability == UserConsentVerifierAvailability.Available;
            }
            catch
            {
                isWindowsHelloAvailable = false;
            }

            StatusText.Text = isWindowsHelloAvailable
                ? "Waiting for Windows Hello..."
                : "Windows Hello not available. Use password.";
        }

        private async void ShowPasswordMode()
        {
            if (isPasswordModeEnabled)
                return;

            isPasswordModeEnabled = true;
            PasswordPanel.Visibility = Visibility.Visible;
            PasswordBox.Password = string.Empty;
            PasswordBox.Focus();
            StatusText.Text = "Enter your password or use voice unlock.";
            RetryWindowsHelloButton.Visibility = Visibility.Collapsed;

            // Initialize voice recognition when showing password mode
            await InitializeVoiceRecognitionAsync();
        }

        private async Task InitializeVoiceRecognitionAsync()
        {
            try
            {
                // Check if voice recognition is enabled in config
                var voiceConfig = VoiceConfigHelper.ReadVoiceConfig();
                if (!voiceConfig.Enabled)
                    return;

                // Check if microphone is available
                bool micAvailable = await VoiceRecognitionHelper.IsMicrophoneAvailableAsync();
                if (!micAvailable)
                    return;

                // Initialize voice helper
                voiceHelper = new VoiceRecognitionHelper();
                bool initialized = await voiceHelper.InitializeAsync();

                if (initialized)
                {
                    // Show voice unlock button
                    VoiceUnlockButton.Visibility = Visibility.Visible;
                }
                else
                {
                    voiceHelper?.Dispose();
                    voiceHelper = null;
                }
            }
            catch
            {
                // Silently fail - voice unlock just won't be available
                voiceHelper?.Dispose();
                voiceHelper = null;
            }
        }

        private async void VoiceUnlock_Click(object sender, RoutedEventArgs e)
        {
            if (voiceHelper == null || isVoiceListening)
                return;

            try
            {
                isVoiceListening = true;

                // Update UI
                VoiceUnlockButton.IsEnabled = false;
                VoiceStatusText.Text = "Listening... Say 'unlock'";
                VoiceStatusText.Visibility = Visibility.Visible;
                SoundHelper.PlayVoiceListeningSound();

                // Get voice config for timeout and confidence threshold
                var voiceConfig = VoiceConfigHelper.ReadVoiceConfig();
                var timeout = TimeSpan.FromSeconds(voiceConfig.ListeningTimeout);

                // Listen for command
                var (success, command, confidence) = await voiceHelper.ListenForCommandAsync(timeout);

                // Process result on UI thread
                Dispatcher.Invoke(() =>
                {
                    if (success && command == voiceConfig.UnlockCommand && confidence >= voiceConfig.ConfidenceThreshold)
                    {
                        // Voice unlock successful
                        VoiceStatusText.Text = "Voice recognized!";
                        SoundHelper.PlayVoiceSuccessSound();

                        // Unlock the window
                        IsUnlocked = true;
                        allowClose = true;
                        Close();
                    }
                    else
                    {
                        // Voice unlock failed
                        if (!success)
                        {
                            VoiceStatusText.Text = "No command detected. Try again or use password.";
                        }
                        else
                        {
                            VoiceStatusText.Text = "Command not recognized. Try again or use password.";
                        }

                        SoundHelper.PlayInvalidPasswordSound();

                        // Reset UI after 3 seconds
                        Task.Delay(3000).ContinueWith(_ =>
                        {
                            Dispatcher.Invoke(() =>
                            {
                                VoiceStatusText.Visibility = Visibility.Collapsed;
                                VoiceUnlockButton.IsEnabled = true;
                            });
                        });
                    }
                });
            }
            catch (Exception)
            {
                // Error during voice recognition
                Dispatcher.Invoke(() =>
                {
                    VoiceStatusText.Text = "Voice recognition error. Use password.";
                    VoiceUnlockButton.IsEnabled = true;
                });
            }
            finally
            {
                isVoiceListening = false;
            }
        }

        private void Window_KeyDown(object sender, System.Windows.Input.KeyEventArgs e)
        {
            if (e.Key == System.Windows.Input.Key.Enter || e.Key == System.Windows.Input.Key.Return)
            {
                AttemptPasswordUnlock();
            }
            else if (e.Key == System.Windows.Input.Key.Escape)
            {
                IsCancelled = true;
                allowClose = true;
                Close();
            }
        }

        private void Window_Loaded(object sender, RoutedEventArgs e)
        {
            // Handled in PasswordWindow_Loaded
        }

        private void HandleClickOutside(object sender, MouseButtonEventArgs e)
        {
            IsCancelled = true;
            allowClose = true;
            Close();
        }

        protected override void OnClosed(EventArgs e)
        {
            RemoveHandler(Mouse.PreviewMouseDownOutsideCapturedElementEvent, outsideClickHandler);
            if (Mouse.Captured == this)
            {
                Mouse.Capture(null);
            }

            // Dispose voice helper
            voiceHelper?.Dispose();
            voiceHelper = null;

            base.OnClosed(e);
        }

        private void Window_Closing(object sender, System.ComponentModel.CancelEventArgs e)
        {
            if (!allowClose)
            {
                e.Cancel = true;
                Activate();
            }
        }
    }
}
