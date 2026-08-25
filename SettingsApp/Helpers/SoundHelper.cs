using System;
using System.Media;

namespace SettingsApp.Helpers
{
    public static class SoundHelper
    {
        public static void PlaySuccessSound()
        {
            try
            {
                SystemSounds.Asterisk?.Play();
            }
            catch
            {
            }
        }
    }
}
