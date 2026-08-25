using System;
using System.Security.Cryptography;
using System.Text;

public static class EncryptionHelper
{
    public static string EncryptPassword(string plainText)
    {
        byte[] encrypted = ProtectedData.Protect(
            Encoding.UTF8.GetBytes(plainText),
            null,
            DataProtectionScope.CurrentUser);
        return Convert.ToBase64String(encrypted);
    }

    public static string DecryptPassword(string encryptedText)
    {
        byte[] decrypted = ProtectedData.Unprotect(
            Convert.FromBase64String(encryptedText),
            null,
            DataProtectionScope.CurrentUser);
        return Encoding.UTF8.GetString(decrypted);
    }
}