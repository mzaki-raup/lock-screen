using System;
using System.Security.Cryptography;
using System.Text;

public static class EncryptionHelper
{
    public static string EncryptPassword(string plainText)
    {
        try
        {
            byte[] encrypted = ProtectedData.Protect(
                Encoding.UTF8.GetBytes(plainText),
                null,
                DataProtectionScope.CurrentUser);
            return Convert.ToBase64String(encrypted);
        }
        catch (CryptographicException)
        {
            return plainText;
        }
    }

    public static string DecryptPassword(string encryptedText)
    {
        try
        {
            byte[] decrypted = ProtectedData.Unprotect(
                Convert.FromBase64String(encryptedText),
                null,
                DataProtectionScope.CurrentUser);
            return Encoding.UTF8.GetString(decrypted);
        }
        catch (CryptographicException)
        {
            return encryptedText;
        }
        catch (FormatException)
        {
            return encryptedText;
        }
    }
}