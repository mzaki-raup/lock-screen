using SharpHook.Reactive;
using SharpHook;
using System.Windows;
using System.Reactive;
using System.Windows.Media;
using System.Windows.Media.Effects;
using System;
using System.Security.Cryptography;
using System.Threading.Tasks;
using System.Windows.Threading;
using System.Runtime.InteropServices;
using System.Diagnostics;
using Microsoft.Win32;
using System.Windows.Forms;
using System.Collections.Generic;

namespace LockScreenApp
{
    public partial class MainWindow : Window
    {
        private ReactiveGlobalHook hook;
        private readonly string storedPassword;
        private readonly string passwordPromptTitle;
        private bool isPasswordWindowOpen = false;
        private DateTime lastCancelTime = DateTime.MinValue;
        private RegistryKey taskManagerKey;
        private List<Window> additionalMonitorWindows = new List<Window>();

        [DllImport("user32.dll")]
        private static extern int SetWindowsHookEx(int idHook, HookProc lpfn, IntPtr hMod, int dwThreadId);

        [DllImport("user32.dll")]
        private static extern bool UnhookWindowsHookEx(int idHook);

        [DllImport("user32.dll")]
        private static extern int CallNextHookEx(int idHook, int nCode, IntPtr wParam, IntPtr lParam);

        [DllImport("kernel32.dll")]
        private static extern IntPtr GetModuleHandle(string lpModuleName);

        private delegate int HookProc(int nCode, IntPtr wParam, IntPtr lParam);

        private const int WH_KEYBOARD_LL = 13;
        private const int WM_KEYDOWN = 0x0100;
        private const int WM_SYSKEYDOWN = 0x0104;
        private int hookHandle = 0;
        private HookProc hookDelegate;

        public MainWindow()
        {
            try
            {
                InitializeComponent();

                var config = IniHelper.ReadConfig();
                storedPassword = EncryptionHelper.DecryptPassword(config.Password);
                passwordPromptTitle = config.Title;
                ApplyAppearance(config);

                DisableSystemKeys();
                CreateMultiMonitorWindows(config);
                HookEvents();

                Loaded += MainWindow_Loaded;
            }
            catch (CryptographicException ex)
            {
                System.Windows.MessageBox.Show($"Encryption Error: The stored password cannot be decrypted.\n\nThis usually happens when:\n- The password was encrypted on a different computer\n- The Windows user profile has changed\n\nPlease reconfigure the password using SettingsApp.\n\nError: {ex.Message}",
                    "LockScreenApp - Decryption Error", MessageBoxButton.OK, MessageBoxImage.Error);
                System.Windows.Application.Current.Shutdown();
            }
            catch (Exception ex)
            {
                System.Windows.MessageBox.Show($"Initialization Error:\n{ex.Message}\n\nStack Trace:\n{ex.StackTrace}",
                    "LockScreenApp Error", MessageBoxButton.OK, MessageBoxImage.Error);
                System.Windows.Application.Current.Shutdown();
            }
        }

        private void CreateMultiMonitorWindows((string Title, string Password, bool BlurBackground, double Transparency) config)
        {
            var screens = Screen.AllScreens;
            var primaryScreen = Screen.PrimaryScreen;

            foreach (var screen in screens)
            {
                if (screen.Primary)
                {
                    this.Left = screen.Bounds.Left;
                    this.Top = screen.Bounds.Top;
                    this.Width = screen.Bounds.Width;
                    this.Height = screen.Bounds.Height;
                }
                else
                {
                    var monitorWindow = new Window
                    {
                        WindowStyle = WindowStyle.None,
                        AllowsTransparency = true,
                        Topmost = true,
                        ShowInTaskbar = false,
                        Background = System.Windows.Media.Brushes.Transparent,
                        Left = screen.Bounds.Left,
                        Top = screen.Bounds.Top,
                        Width = screen.Bounds.Width,
                        Height = screen.Bounds.Height,
                        WindowState = WindowState.Normal
                    };

                    var grid = new System.Windows.Controls.Grid();
                    double clamped = Math.Max(0.0, Math.Min(1.0, config.Transparency));
                    byte alpha = (byte)(clamped * 255);
                    grid.Background = new SolidColorBrush(Color.FromArgb(alpha, 0, 0, 0));

                    if (config.BlurBackground)
                    {
                        grid.Effect = new BlurEffect { Radius = 10 };
                    }

                    monitorWindow.Content = grid;
                    monitorWindow.Show();
                    additionalMonitorWindows.Add(monitorWindow);
                }
            }
        }

        private void DisableSystemKeys()
        {
            try
            {
                taskManagerKey = Registry.CurrentUser.CreateSubKey(@"Software\Microsoft\Windows\CurrentVersion\Policies\System");
                taskManagerKey.SetValue("DisableTaskMgr", 1, RegistryValueKind.DWord);
            }
            catch { }

            hookDelegate = new HookProc(KeyboardHookProc);
            IntPtr hInstance = GetModuleHandle(null);
            hookHandle = SetWindowsHookEx(WH_KEYBOARD_LL, hookDelegate, hInstance, 0);
        }

        private int KeyboardHookProc(int nCode, IntPtr wParam, IntPtr lParam)
        {
            if (nCode >= 0 && (wParam == (IntPtr)WM_KEYDOWN || wParam == (IntPtr)WM_SYSKEYDOWN))
            {
                int vkCode = Marshal.ReadInt32(lParam);
                bool ctrlPressed = (System.Windows.Input.Keyboard.Modifiers & System.Windows.Input.ModifierKeys.Control) != 0;
                bool altPressed = (System.Windows.Input.Keyboard.Modifiers & System.Windows.Input.ModifierKeys.Alt) != 0;
                bool shiftPressed = (System.Windows.Input.Keyboard.Modifiers & System.Windows.Input.ModifierKeys.Shift) != 0;
                bool winPressed = (System.Windows.Input.Keyboard.Modifiers & System.Windows.Input.ModifierKeys.Windows) != 0;

                if ((vkCode == 0x5B || vkCode == 0x5C) ||
                    (altPressed && vkCode == 0x09) ||
                    (altPressed && vkCode == 0x73) ||
                    (ctrlPressed && altPressed && vkCode == 0x2E) ||
                    (ctrlPressed && shiftPressed && vkCode == 0x1B))
                {
                    return 1;
                }
            }

            return CallNextHookEx(hookHandle, nCode, wParam, lParam);
        }

        private void EnableSystemKeys()
        {
            try
            {
                if (taskManagerKey != null)
                {
                    taskManagerKey.DeleteValue("DisableTaskMgr", false);
                    taskManagerKey.Close();
                }
            }
            catch { }

            if (hookHandle != 0)
            {
                UnhookWindowsHookEx(hookHandle);
                hookHandle = 0;
            }
        }

        private void ApplyAppearance((string Title, string Password, bool BlurBackground, double Transparency) config)
        {
            double clamped = Math.Max(0.0, Math.Min(1.0, config.Transparency));
            byte alpha = (byte)(clamped * 255);
            OverlayGrid.Background = new SolidColorBrush(Color.FromArgb(alpha, 0, 0, 0));

            if (config.BlurBackground)
            {
                OverlayGrid.Effect = new BlurEffect { Radius = 10 };
            }
            else
            {
                OverlayGrid.Effect = null;
            }
        }

        private void HookEvents()
        {
            hook = new ReactiveGlobalHook();

            hook.KeyPressed.Subscribe(Observer.Create<KeyboardHookEventArgs>(e =>
            {
                Dispatcher.BeginInvoke(new Action(ShowPasswordForm));
            }));

            hook.MouseClicked.Subscribe(Observer.Create<MouseHookEventArgs>(e =>
            {
                Dispatcher.BeginInvoke(new Action(ShowPasswordForm));
            }));

            hook.RunAsync();
        }

        private void ShowPasswordForm()
        {
            if (isPasswordWindowOpen)
                return;

            if ((DateTime.Now - lastCancelTime).TotalMilliseconds < 500)
                return;

            isPasswordWindowOpen = true;

            var passwordWindow = new PasswordWindow(storedPassword, passwordPromptTitle)
            {
                Owner = this,
                WindowStartupLocation = WindowStartupLocation.CenterOwner,
                Topmost = true
            };
            passwordWindow.ShowDialog();

            isPasswordWindowOpen = false;

            if (passwordWindow.IsUnlocked)
            {
                SoundHelper.PlayUnlockSuccessSound();
                EnableSystemKeys();
                CloseAdditionalMonitorWindows();
                hook?.Dispose();
                System.Windows.Application.Current.Shutdown();
            }
            else if (passwordWindow.IsCancelled)
            {
                lastCancelTime = DateTime.Now;
            }
        }

        private void CloseAdditionalMonitorWindows()
        {
            foreach (var window in additionalMonitorWindows)
            {
                window.Close();
            }
            additionalMonitorWindows.Clear();
        }

        private void MainWindow_Loaded(object sender, RoutedEventArgs e)
        {
            Loaded -= MainWindow_Loaded;
            SoundHelper.PlayAppStartSound();
        }

        protected override void OnClosed(EventArgs e)
        {
            EnableSystemKeys();
            CloseAdditionalMonitorWindows();
            hook?.Dispose();
            base.OnClosed(e);
        }
    }
}