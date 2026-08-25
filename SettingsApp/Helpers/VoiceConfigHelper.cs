using IniParser;
using IniParser.Model;
using System;
using System.IO;

namespace SettingsApp.Helpers
{
    /// <summary>
    /// Helper class for managing voice recognition configuration
    /// </summary>
    public static class VoiceConfigHelper
    {
        private const string ConfigPath = "config.ini";
        private const string SectionName = "VoiceRecognition";

        /// <summary>
        /// Voice recognition configuration settings
        /// </summary>
        public class VoiceConfig
        {
            public bool Enabled { get; set; }
            public string UnlockCommand { get; set; } = "unlock";
            public double ConfidenceThreshold { get; set; } = 0.7;
            public int ListeningTimeout { get; set; } = 10;
        }

        /// <summary>
        /// Read voice recognition configuration from config.ini
        /// </summary>
        public static VoiceConfig ReadVoiceConfig()
        {
            var config = new VoiceConfig();

            try
            {
                if (!File.Exists(ConfigPath))
                    return config;

                var parser = new FileIniDataParser();
                IniData data = parser.ReadFile(ConfigPath);

                if (data.Sections.ContainsSection(SectionName))
                {
                    var section = data[SectionName];

                    if (bool.TryParse(section["Enabled"], out bool enabled))
                        config.Enabled = enabled;

                    if (!string.IsNullOrWhiteSpace(section["UnlockCommand"]))
                        config.UnlockCommand = section["UnlockCommand"];

                    if (double.TryParse(section["ConfidenceThreshold"], out double threshold))
                        config.ConfidenceThreshold = threshold;

                    if (int.TryParse(section["ListeningTimeout"], out int timeout))
                        config.ListeningTimeout = timeout;
                }
            }
            catch
            {
                // Return default config if reading fails
            }

            return config;
        }

        /// <summary>
        /// Update voice recognition enabled status in config.ini
        /// </summary>
        public static void UpdateVoiceEnabled(bool enabled)
        {
            try
            {
                EnsureVoiceConfigExists();

                var parser = new FileIniDataParser();
                IniData data = parser.ReadFile(ConfigPath);

                if (!data.Sections.ContainsSection(SectionName))
                {
                    data.Sections.AddSection(SectionName);
                }

                data[SectionName]["Enabled"] = enabled.ToString().ToLower();

                parser.WriteFile(ConfigPath, data);
            }
            catch
            {
                // Silently fail - config update is non-critical
            }
        }

        /// <summary>
        /// Check if voice recognition is enabled
        /// </summary>
        public static bool IsVoiceEnabled()
        {
            var config = ReadVoiceConfig();
            return config.Enabled;
        }

        /// <summary>
        /// Ensure voice recognition section exists in config.ini with default values
        /// </summary>
        private static void EnsureVoiceConfigExists()
        {
            try
            {
                if (!File.Exists(ConfigPath))
                    return;

                var parser = new FileIniDataParser();
                IniData data = parser.ReadFile(ConfigPath);

                if (!data.Sections.ContainsSection(SectionName))
                {
                    data.Sections.AddSection(SectionName);
                    data[SectionName]["Enabled"] = "false";
                    data[SectionName]["UnlockCommand"] = "unlock";
                    data[SectionName]["ConfidenceThreshold"] = "0.7";
                    data[SectionName]["ListeningTimeout"] = "10";

                    parser.WriteFile(ConfigPath, data);
                }
            }
            catch
            {
                // Silently fail - will use defaults
            }
        }
    }
}
