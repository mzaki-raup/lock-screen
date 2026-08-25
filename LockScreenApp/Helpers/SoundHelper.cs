using System;
using System.Media;

public static class SoundHelper
{
    public static void PlayAppStartSound(bool waitForCompletion = false)
    {
        PlaySafe(SystemSounds.Asterisk, waitForCompletion);
    }

    public static void PlayInvalidPasswordSound(bool waitForCompletion = false)
    {
        PlaySafe(SystemSounds.Hand, waitForCompletion);
    }

    public static void PlayUnlockSuccessSound(bool waitForCompletion = false)
    {
        PlaySafe(SystemSounds.Exclamation, waitForCompletion);
    }

    public static void PlayVoiceListeningSound(bool waitForCompletion = false)
    {
        PlaySafe(SystemSounds.Beep, waitForCompletion);
    }

    public static void PlayVoiceSuccessSound(bool waitForCompletion = false)
    {
        PlaySafe(SystemSounds.Exclamation, waitForCompletion);
    }

    private static void PlaySafe(SystemSound sound, bool wait)
    {
        try
        {
            sound?.Play();
            if (wait)
            {
                System.Threading.Thread.Sleep(500);
            }
        }
        catch
        {
        }
    }
}
