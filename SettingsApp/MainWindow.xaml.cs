
using System;
using System.Globalization;
using System.Linq;
using System.Media;
using System.Text.RegularExpressions;
using System.Threading.Tasks;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Data;
using System.Windows.Input;
using IniParser;
using IniParser.Model;
using SettingsApp.Helpers;


namespace SettingsApp
{


    public partial class MainWindow : Window
    {
        private static readonly Regex TransparencyRegex = new Regex(@"^(0(\.\d{0,3})?|1(\.0{0,3})?)$", RegexOptions.Compiled);

        public MainWindow()
        {
            InitializeComponent();
            LoadCurrentSettings();
        }

        private void LoadCurrentSettings()
        {
            var (title, _, blurEnabled, transparency) = IniHelper.ReadConfig();
            TitleBox.Text = title;
            TransparencyBox.Text = transparency.ToString(CultureInfo.InvariantCulture);
            BlurCheckBox.IsChecked = blurEnabled;

            // Load voice settings
            var voiceConfig = VoiceConfigHelper.ReadVoiceConfig();
            VoiceEnabledCheckBox.IsChecked = voiceConfig.Enabled;

            // Show test button if voice is enabled
            if (voiceConfig.Enabled)
            {
                TestMicrophoneButton.Visibility = Visibility.Visible;
            }
        }

        private void TransparencyBox_PreviewTextInput(object sender, TextCompositionEventArgs e)
        {
            string proposed = GetProposedText((TextBox)sender, e.Text);
            e.Handled = !IsValidTransparencyInput(proposed);
        }

        private void TransparencyBox_Pasting(object sender, DataObjectPastingEventArgs e)
        {
            if (e.DataObject.GetDataPresent(DataFormats.Text))
            {
                string pastedText = e.DataObject.GetData(DataFormats.Text) as string ?? string.Empty;
                string proposed = GetProposedText((TextBox)sender, pastedText);
                if (!IsValidTransparencyInput(proposed))
                {
                    e.CancelCommand();
                }
            }
            else
            {
                e.CancelCommand();
            }
        }

        private static string GetProposedText(TextBox textBox, string input)
        {
            string text = textBox.Text;
            if (textBox.SelectionLength > 0)
            {
                text = text.Remove(textBox.SelectionStart, textBox.SelectionLength);
            }

            return text.Insert(textBox.CaretIndex, input);
        }

        private static bool IsValidTransparencyInput(string input)
        {
            return string.IsNullOrEmpty(input) || TransparencyRegex.IsMatch(input);
        }

        private void Save_Click(object sender, RoutedEventArgs e)
        {
            string newPassword = NewPasswordBox.Password;
            string newTitle = TitleBox.Text.Trim();
            string transparencyText = TransparencyBox.Text.Trim();
            bool blurEnabled = BlurCheckBox.IsChecked == true;

            if (!double.TryParse(transparencyText, NumberStyles.Float, CultureInfo.InvariantCulture, out double transparency) || transparency < 0.0 || transparency > 1.0)
            {
                MessageBox.Show("Transparency must be a number between 0.0 and 1.0.");
                return;
            }

            if (!string.IsNullOrEmpty(newPassword) && !IsStrongPassword(newPassword))
            {
                MessageBox.Show(
                    "Password must be at least 3 characters and contain only letters and/or digits.");
                return;
            }

            var parser = new FileIniDataParser();
            IniData data = parser.ReadFile("config.ini");

            if (!string.IsNullOrEmpty(newPassword))
            {
                string encryptedPassword = EncryptionHelper.EncryptPassword(newPassword);
                data["LockScreen"]["Password"] = encryptedPassword;
            }

            data["LockScreen"]["Title"] = newTitle;
            data["LockScreen"]["Transparency"] = transparency.ToString(CultureInfo.InvariantCulture);
            data["LockScreen"]["BlurBackground"] = blurEnabled.ToString().ToLower();

            // Save voice recognition setting
            bool voiceEnabled = VoiceEnabledCheckBox.IsChecked == true;
            VoiceConfigHelper.UpdateVoiceEnabled(voiceEnabled);

            parser.WriteFile("config.ini", data);

            SystemSounds.Asterisk?.Play();
            MessageBox.Show("Settings updated successfully!");
            Close();
        }

        private bool IsStrongPassword(string password)
        {
            return password.Length >= 3 && password.All(char.IsLetterOrDigit);
        }

        private void Cancel_Click(object sender, RoutedEventArgs e)
        {
            Close();
        }

        private void VoiceEnabled_Changed(object sender, RoutedEventArgs e)
        {
            bool isEnabled = VoiceEnabledCheckBox.IsChecked == true;

            // Show/hide test button based on voice enabled status
            TestMicrophoneButton.Visibility = isEnabled ? Visibility.Visible : Visibility.Collapsed;

            // Hide status text when toggling
            VoiceTestStatusText.Visibility = Visibility.Collapsed;
        }

        private async void TestMicrophone_Click(object sender, RoutedEventArgs e)
        {
            // Disable button during test
            TestMicrophoneButton.IsEnabled = false;
            VoiceTestStatusText.Visibility = Visibility.Visible;
            VoiceTestStatusText.Foreground = System.Windows.Media.Brushes.DarkBlue;
            VoiceTestStatusText.Text = "Checking microphone...";

            try
            {
                // Check if microphone is available
                bool micAvailable = await VoiceRecognitionHelper.IsMicrophoneAvailableAsync();

                if (!micAvailable)
                {
                    VoiceTestStatusText.Foreground = System.Windows.Media.Brushes.Red;
                    VoiceTestStatusText.Text = "Microphone not available. Check Windows Settings > Privacy > Microphone.";
                    TestMicrophoneButton.IsEnabled = true;
                    return;
                }

                // Initialize voice helper
                using var voiceHelper = new VoiceRecognitionHelper();
                bool initialized = await voiceHelper.InitializeAsync();

                if (!initialized)
                {
                    VoiceTestStatusText.Foreground = System.Windows.Media.Brushes.Red;
                    VoiceTestStatusText.Text = "Failed to initialize voice recognition.";
                    TestMicrophoneButton.IsEnabled = true;
                    return;
                }

                // Prompt user to speak
                VoiceTestStatusText.Foreground = System.Windows.Media.Brushes.Green;
                VoiceTestStatusText.Text = "Listening... Please say 'unlock'";
                SystemSounds.Beep?.Play();

                // Listen for command
                var (success, command) = await voiceHelper.ListenForCommandAsync(TimeSpan.FromSeconds(10));

                if (success && command == "unlock")
                {
                    VoiceTestStatusText.Foreground = System.Windows.Media.Brushes.Green;
                    VoiceTestStatusText.Text = $"Success! Recognized: '{command}'";
                    SystemSounds.Exclamation?.Play();
                }
                else if (success)
                {
                    VoiceTestStatusText.Foreground = System.Windows.Media.Brushes.Orange;
                    VoiceTestStatusText.Text = $"Recognized '{command}', but expected 'unlock'. Try again.";
                }
                else
                {
                    VoiceTestStatusText.Foreground = System.Windows.Media.Brushes.Orange;
                    VoiceTestStatusText.Text = "No command detected. Try speaking louder or closer to microphone.";
                }
            }
            catch (Exception ex)
            {
                VoiceTestStatusText.Foreground = System.Windows.Media.Brushes.Red;
                VoiceTestStatusText.Text = $"Error: {ex.Message}";
            }
            finally
            {
                TestMicrophoneButton.IsEnabled = true;
            }
        }


    }
    
    public class BoolToVisibilityConverter : IValueConverter
    {
        public object Convert(object value, Type targetType, object parameter, CultureInfo culture)
        {
            return (bool)value ? Visibility.Visible : Visibility.Collapsed;
        }

        public object ConvertBack(object value, Type targetType, object parameter, CultureInfo culture)
        {
            throw new NotImplementedException();
        }
    }
}