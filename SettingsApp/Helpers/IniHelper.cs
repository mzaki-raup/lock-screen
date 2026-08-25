using IniParser;
using IniParser.Model;
using System.IO;

public static class IniHelper
{
    private const string ConfigPath = "config.ini";

    public static (string Title, string Password, bool BlurBackground, double Transparency) ReadConfig()
    {
        EnsureConfigExists();
        var parser = new FileIniDataParser();
        IniData data = parser.ReadFile(ConfigPath);

        bool blur = bool.TryParse(data["LockScreen"]["BlurBackground"], out var blurResult) && blurResult;
        double transparency = double.TryParse(data["LockScreen"]["Transparency"], out var transResult) ? transResult : 0.5;

        return (data["LockScreen"]["Title"], data["LockScreen"]["Password"], blur, transparency);
    }

    public static void UpdatePassword(string encryptedPassword)
    {
        var parser = new FileIniDataParser();
        IniData data = parser.ReadFile(ConfigPath);
        data["LockScreen"]["Password"] = encryptedPassword;
        parser.WriteFile(ConfigPath, data);
    }

    private static void EnsureConfigExists()
    {
        if (!File.Exists(ConfigPath))
        {
            var parser = new FileIniDataParser();
            var data = new IniData();

            string defaultTitle = "Enter Password to Unlock";
            string defaultPassword = "1234";
            string encryptedPassword = EncryptionHelper.EncryptPassword(defaultPassword);

            data["LockScreen"]["Title"] = defaultTitle;
            data["LockScreen"]["Password"] = encryptedPassword;
            data["LockScreen"]["BlurBackground"] = "false";
            data["LockScreen"]["Transparency"] = "0.5";

            // Add VoiceRecognition section with defaults
            data["VoiceRecognition"]["Enabled"] = "false";
            data["VoiceRecognition"]["UnlockCommand"] = "unlock";
            data["VoiceRecognition"]["ConfidenceThreshold"] = "0.7";
            data["VoiceRecognition"]["ListeningTimeout"] = "10";

            parser.WriteFile(ConfigPath, data);

            System.Windows.MessageBox.Show($"Config file created.\nDefault password: {defaultPassword}",
                                           "Info",
                                           System.Windows.MessageBoxButton.OK,
                                           System.Windows.MessageBoxImage.Information);
        }
    }
}