//! Per-user Windows startup registration and one UI/lighting owner per session.

#[cfg(windows)]
mod platform {
    use std::{
        io,
        path::Path,
        ptr,
        sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        },
        thread,
    };
    use windows_sys::{
        core::w,
        Win32::{
            Foundation::{
                CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, ERROR_FILE_NOT_FOUND,
                ERROR_SUCCESS, HANDLE, WAIT_OBJECT_0,
            },
            System::{Registry::*, Threading::*},
        },
    };

    const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    const VALUE_NAME: &str = "opencrate";

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(Some(0)).collect()
    }
    fn check(code: u32) -> io::Result<()> {
        if code == ERROR_SUCCESS {
            Ok(())
        } else {
            Err(io::Error::from_raw_os_error(code as i32))
        }
    }

    fn startup_command(executable: &Path) -> io::Result<String> {
        let path = executable
            .to_str()
            .ok_or_else(|| io::Error::other("Executable path is not valid Unicode"))?;
        if !executable.is_absolute() || path.contains(['"', '\0']) {
            return Err(io::Error::other("Invalid startup executable path"));
        }
        let command = format!("\"{path}\" --startup");
        // Windows Run entries are limited to a 260-character command line.
        if command.encode_utf16().count() > 260 {
            return Err(io::Error::other(
                "Executable path is too long for Windows startup",
            ));
        }
        Ok(command)
    }

    fn read_value(subkey: &str, name: &str) -> io::Result<Option<String>> {
        let (subkey, name) = (wide(subkey), wide(name));
        let mut bytes = 0;
        // SAFETY: Both names are NUL-terminated; the first call only queries size.
        let result = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                subkey.as_ptr(),
                name.as_ptr(),
                RRF_RT_REG_SZ,
                ptr::null_mut(),
                ptr::null_mut(),
                &mut bytes,
            )
        };
        if result == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        check(result)?;
        let mut buffer = vec![0u16; (bytes as usize).div_ceil(2) + 1];
        let mut capacity = (buffer.len() * 2) as u32;
        // SAFETY: The writable buffer is sized in bytes as requested by Win32.
        check(unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                subkey.as_ptr(),
                name.as_ptr(),
                RRF_RT_REG_SZ,
                ptr::null_mut(),
                buffer.as_mut_ptr().cast(),
                &mut capacity,
            )
        })?;
        let length = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
        String::from_utf16(&buffer[..length])
            .map(Some)
            .map_err(io::Error::other)
    }

    struct Key(HKEY);
    impl Drop for Key {
        fn drop(&mut self) {
            unsafe {
                RegCloseKey(self.0);
            }
        }
    }

    fn write_value(subkey: &str, name: &str, command: Option<&str>) -> io::Result<()> {
        let (subkey, name) = (wide(subkey), wide(name));
        let mut key = ptr::null_mut();
        // SAFETY: All strings are NUL-terminated and the handle out-pointer lives
        // through the call. Only this user's named registration is modified.
        let result = unsafe {
            if command.is_some() {
                RegCreateKeyExW(
                    HKEY_CURRENT_USER,
                    subkey.as_ptr(),
                    0,
                    ptr::null(),
                    REG_OPTION_NON_VOLATILE,
                    KEY_SET_VALUE,
                    ptr::null(),
                    &mut key,
                    ptr::null_mut(),
                )
            } else {
                RegOpenKeyExW(
                    HKEY_CURRENT_USER,
                    subkey.as_ptr(),
                    0,
                    KEY_SET_VALUE,
                    &mut key,
                )
            }
        };
        if result == ERROR_FILE_NOT_FOUND && command.is_none() {
            return Ok(());
        }
        check(result)?;
        let key = Key(key);
        if let Some(command) = command {
            let value = wide(command);
            check(unsafe {
                RegSetValueExW(
                    key.0,
                    name.as_ptr(),
                    0,
                    REG_SZ,
                    value.as_ptr().cast(),
                    (value.len() * 2) as u32,
                )
            })
        } else {
            let result = unsafe { RegDeleteValueW(key.0, name.as_ptr()) };
            if result == ERROR_FILE_NOT_FOUND {
                Ok(())
            } else {
                check(result)
            }
        }
    }

    pub fn autostart_enabled() -> io::Result<bool> {
        Ok(read_value(RUN_KEY, VALUE_NAME)?.is_some())
    }

    pub fn set_autostart(enabled: bool) -> io::Result<()> {
        let command = if enabled {
            Some(startup_command(&std::env::current_exe()?)?)
        } else {
            None
        };
        write_value(RUN_KEY, VALUE_NAME, command.as_deref())?;
        if read_value(RUN_KEY, VALUE_NAME)? != command {
            return Err(io::Error::other(
                "Windows startup change could not be verified",
            ));
        }
        Ok(())
    }

    struct Handle(HANDLE);
    // SAFETY: Kernel event/mutex handles support cross-thread operations. The
    // handle is closed once, after its owner/last Arc and listener are gone.
    unsafe impl Send for Handle {}
    unsafe impl Sync for Handle {}
    impl Drop for Handle {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }

    struct Shared {
        event: Handle,
        stop: AtomicBool,
        show: AtomicBool,
    }

    pub struct Instance {
        _mutex: Handle,
        shared: Arc<Shared>,
        listener: Option<thread::JoinHandle<()>>,
    }

    impl Instance {
        /// Returns None after notifying an existing instance. Startup invocations
        /// stay quiet; manual invocations reveal the already running window.
        pub fn claim(from_startup: bool) -> io::Result<Option<Self>> {
            let mutex = unsafe { CreateMutexW(ptr::null(), 0, w!("Local\\opencrate.ui.v1")) };
            if mutex.is_null() {
                return Err(io::Error::last_os_error());
            }
            let existing = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
            let mutex = Handle(mutex);
            let event = unsafe { CreateEventW(ptr::null(), 0, 0, w!("Local\\opencrate.show.v1")) };
            if event.is_null() {
                return Err(io::Error::last_os_error());
            }
            let event = Handle(event);
            if existing {
                if !from_startup && unsafe { SetEvent(event.0) } == 0 {
                    return Err(io::Error::last_os_error());
                }
                return Ok(None);
            }
            Ok(Some(Self {
                _mutex: mutex,
                shared: Arc::new(Shared {
                    event,
                    stop: AtomicBool::new(false),
                    show: AtomicBool::new(false),
                }),
                listener: None,
            }))
        }

        pub fn listen(&mut self, wake: impl Fn() + Send + 'static) -> io::Result<()> {
            let shared = self.shared.clone();
            self.listener = Some(
                thread::Builder::new()
                    .name("opencrate-activation".into())
                    .spawn(move || loop {
                        let result = unsafe { WaitForSingleObject(shared.event.0, INFINITE) };
                        if shared.stop.load(Ordering::Acquire) || result != WAIT_OBJECT_0 {
                            break;
                        }
                        shared.show.store(true, Ordering::Release);
                        wake();
                    })?,
            );
            Ok(())
        }

        pub fn take_show_request(&self) -> bool {
            self.shared.show.swap(false, Ordering::AcqRel)
        }
    }

    impl Drop for Instance {
        fn drop(&mut self) {
            self.shared.stop.store(true, Ordering::Release);
            unsafe {
                SetEvent(self.shared.event.0);
            }
            if let Some(listener) = self.listener.take() {
                let _ = listener.join();
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn startup_command_quotes_spaces_and_unicode() {
            assert_eq!(
                startup_command(Path::new("C:\\Apps\\Test User\\Caf\u{00e9}\\opencrate.exe"))
                    .unwrap(),
                "\"C:\\Apps\\Test User\\Caf\u{00e9}\\opencrate.exe\" --startup"
            );
            assert!(startup_command(Path::new("relative.exe")).is_err());
        }

        #[test]
        fn registry_roundtrip_uses_an_isolated_non_startup_key() {
            let subkey = format!(r"Software\opencrate\Test-{}", std::process::id());
            let value = "StartupTest";
            let command = startup_command(Path::new(r"C:\Test User\opencrate.exe")).unwrap();
            write_value(&subkey, value, Some(&command)).unwrap();
            assert_eq!(
                read_value(&subkey, value).unwrap().as_deref(),
                Some(command.as_str())
            );
            write_value(&subkey, value, None).unwrap();
            assert_eq!(read_value(&subkey, value).unwrap(), None);
            write_value(&subkey, value, None).unwrap();
            check(unsafe { RegDeleteKeyW(HKEY_CURRENT_USER, wide(&subkey).as_ptr()) }).unwrap();
        }
    }
}

#[cfg(windows)]
pub use platform::*;

#[cfg(not(windows))]
mod platform {
    use std::io;
    pub fn autostart_enabled() -> io::Result<bool> {
        Ok(false)
    }
    pub fn set_autostart(_: bool) -> io::Result<()> {
        Err(io::Error::other(
            "Startup registration is available on Windows",
        ))
    }
    pub struct Instance;
    impl Instance {
        pub fn claim(_: bool) -> io::Result<Option<Self>> {
            Ok(Some(Self))
        }
        pub fn listen(&mut self, _: impl Fn() + Send + 'static) -> io::Result<()> {
            Ok(())
        }
        pub fn take_show_request(&self) -> bool {
            false
        }
    }
}
#[cfg(not(windows))]
pub use platform::*;
