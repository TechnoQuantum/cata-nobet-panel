//! Windows-specific helpers: single instance, sign-in startup, message boxes.

#[cfg(windows)]
mod imp {
    use windows::core::{HSTRING, PCWSTR};
    use windows::Win32::Foundation::{GetLastError, BOOL, ERROR_ALREADY_EXISTS, HWND};
    use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_ABANDONED, WAIT_OBJECT_0};
    use windows::Win32::Security::Authorization::{ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1};
    use windows::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
    use windows::Win32::System::Threading::{CreateMutexW, OpenMutexW, ReleaseMutex, WaitForSingleObject, SYNCHRONIZATION_SYNCHRONIZE};
    use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_ICONINFORMATION, MB_OK};
    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE};
    use winreg::RegKey;

    const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    const VALUE: &str = "NobetPanel";

    /// Returns false if another instance already runs. Two instances would
    /// compete for the panel's single BLE connection.
    pub fn single_instance() -> bool {
        unsafe {
            // Test runs with a separate data folder may run next to the real app.
            let suffix = if std::env::var_os("NOBETPANEL_DATA_DIR").is_some() { ".Test" } else { "" };
            let name = HSTRING::from(format!("Local\\NobetPanel.SingleInstance{suffix}"));
            match CreateMutexW(None, BOOL(1), PCWSTR(name.as_ptr())) {
                // The handle stays open for the life of the process.
                Ok(_handle) => GetLastError() != ERROR_ALREADY_EXISTS,
                Err(_) => true,
            }
        }
    }

    /// Mutex in the Global namespace that both the service (session 0,
    /// LocalSystem) and the window in the user's session can open.
    fn global_mutex(name: &str, initial_owner: bool) -> Option<HANDLE> {
        unsafe {
            let mut sd = PSECURITY_DESCRIPTOR::default();
            // Full access for Everyone.
            let sddl = HSTRING::from("D:(A;;GA;;;WD)");
            ConvertStringSecurityDescriptorToSecurityDescriptorW(PCWSTR(sddl.as_ptr()), SDDL_REVISION_1, &mut sd, None).ok()?;
            let sa = SECURITY_ATTRIBUTES { nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32, lpSecurityDescriptor: sd.0, bInheritHandle: BOOL(0) };
            let suffix = if test_mode() { ".Test" } else { "" };
            let full = HSTRING::from(format!("Global\\{name}{suffix}"));
            CreateMutexW(Some(&sa), BOOL(initial_owner as i32), PCWSTR(full.as_ptr())).ok()
        }
    }

    /// Cross-process lock (Bluetooth access, data file writes). Released on drop.
    /// Must be dropped on the thread that acquired it.
    pub struct NamedLock(Option<HANDLE>);

    impl NamedLock {
        /// Waits up to `timeout`. On failure the returned lock holds nothing;
        /// check `held()` where exclusive access matters.
        pub fn acquire(name: &str, timeout: std::time::Duration) -> NamedLock {
            let Some(h) = global_mutex(name, false) else { return NamedLock(None) };
            let r = unsafe { WaitForSingleObject(h, timeout.as_millis().min(u32::MAX as u128) as u32) };
            if r == WAIT_OBJECT_0 || r == WAIT_ABANDONED {
                NamedLock(Some(h))
            } else {
                unsafe {
                    let _ = CloseHandle(h);
                }
                NamedLock(None)
            }
        }
        pub fn held(&self) -> bool {
            self.0.is_some()
        }
    }

    impl Drop for NamedLock {
        fn drop(&mut self) {
            if let Some(h) = self.0.take() {
                unsafe {
                    let _ = ReleaseMutex(h);
                    let _ = CloseHandle(h);
                }
            }
        }
    }

    const SERVICE_MARK: &str = "NobetPanel.ServiceRunning";

    /// Held by the running service for its whole life.
    pub fn mark_service_running() -> Option<HANDLE> {
        global_mutex(SERVICE_MARK, true)
    }

    /// True while the background service is running (it then owns automatic sending).
    pub fn service_running() -> bool {
        let suffix = if test_mode() { ".Test" } else { "" };
        let name = HSTRING::from(format!("Global\\{SERVICE_MARK}{suffix}"));
        unsafe {
            match OpenMutexW(SYNCHRONIZATION_SYNCHRONIZE, BOOL(0), PCWSTR(name.as_ptr())) {
                Ok(h) => {
                    let _ = CloseHandle(h);
                    true
                }
                Err(_) => false,
            }
        }
    }

    pub fn message_box(title: &str, text: &str, error: bool) {
        unsafe {
            let style = MB_OK | if error { MB_ICONERROR } else { MB_ICONINFORMATION };
            MessageBoxW(HWND::default(), &HSTRING::from(text), &HSTRING::from(title), style);
        }
    }

    fn command() -> Result<String, String> {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        Ok(format!("\"{}\" --minimized", exe.display()))
    }

    /// Test runs (separate data folder) must never touch the real startup entry.
    fn test_mode() -> bool {
        std::env::var_os("NOBETPANEL_DATA_DIR").is_some()
    }

    fn registered() -> Option<String> {
        RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(RUN_KEY, KEY_READ).and_then(|k| k.get_value::<String, _>(VALUE)).ok()
    }

    pub fn startup_enabled() -> bool {
        !test_mode() && registered().map(|v| v.contains("--minimized")).unwrap_or(false)
    }

    pub fn set_startup(enable: bool) -> Result<(), String> {
        if test_mode() {
            return Err("Test modunda (NOBETPANEL_DATA_DIR) Windows başlangıç ayarı değiştirilmez.".into());
        }
        let (key, _) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(RUN_KEY).map_err(|e| e.to_string())?;
        let key = RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(RUN_KEY, KEY_WRITE).unwrap_or(key);
        if enable {
            key.set_value(VALUE, &command()?).map_err(|e| e.to_string())
        } else {
            match key.delete_value(VALUE) {
                Ok(()) => Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(e.to_string()),
            }
        }
    }

    /// Repair the startup entry only when it points at an exe that no longer
    /// exists (e.g. the app was moved). A working entry — possibly another
    /// installed copy — is left alone.
    pub fn refresh_startup_path() {
        let Some(cmd) = registered().filter(|_| !test_mode()) else { return };
        let target = cmd.trim().strip_prefix('"').and_then(|s| s.split('"').next()).map(str::to_string);
        if let Some(path) = target {
            if !std::path::Path::new(&path).exists() {
                let _ = set_startup(true);
            }
        }
    }
}

#[cfg(windows)]
pub use imp::*;

pub fn open_in_shell(path: &std::path::Path) {
    let _ = std::process::Command::new("explorer").arg(path).spawn();
}
