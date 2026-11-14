//! Start menu shortcuts and the icons they show.

use std::path::Path;

use crate::{Error, Result};

/// Create or replace the shortcut `lnk`, starting `target` with the
/// AppUserModelID `aumid` so taskbar pins group with the app's windows.
pub fn create(lnk: &Path, target: &Path, icon: Option<&Path>, aumid: &str) -> Result<()> {
    if let Some(parent) = lnk.parent() {
        crate::fs::create_dir_all(parent)?;
    }
    imp::create(lnk, target, icon, aumid).map_err(|e| Error::Shortcut {
        path: lnk.to_owned(),
        message: e.to_string(),
    })
}

/// Write the main icon of the PE file `exe` to `ico`. Returns false when
/// `exe` has no icon.
pub fn extract_icon(exe: &Path, ico: &Path) -> Result<bool> {
    match imp::icon_bytes(exe) {
        Some(bytes) => crate::fs::write_atomic(ico, &bytes).map(|()| true),
        None => Ok(false),
    }
}

/// An AppUserModelID from parts: at most 128 characters, no spaces.
pub fn aumid(package: &str, name: &str) -> String {
    let id: String = format!("wpk.{package}.{name}")
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect();
    id.chars().take(128).collect()
}

#[cfg(windows)]
mod imp {
    use std::ffi::c_void;
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;

    use windows::Win32::Storage::EnhancedStorage::PKEY_AppUserModel_ID;
    use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
    use windows::Win32::System::Com::{
        CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
        IPersistFile,
    };
    use windows::Win32::System::Variant::VT_LPWSTR;
    use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;
    use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};
    use windows::core::{HSTRING, Interface, PWSTR};
    use windows_sys::Win32::Foundation::FreeLibrary;
    use windows_sys::Win32::System::LibraryLoader::{
        EnumResourceNamesW, FindResourceW, LOAD_LIBRARY_AS_DATAFILE,
        LOAD_LIBRARY_AS_IMAGE_RESOURCE, LoadLibraryExW, LoadResource, LockResource, SizeofResource,
    };

    pub fn create(
        lnk: &Path,
        target: &Path,
        icon: Option<&Path>,
        aumid: &str,
    ) -> windows::core::Result<()> {
        unsafe {
            // Fails harmlessly when this thread already joined another apartment.
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
            link.SetPath(&HSTRING::from(target.as_os_str()))?;
            if let Some(icon) = icon {
                link.SetIconLocation(&HSTRING::from(icon.as_os_str()), 0)?;
            }
            let store: IPropertyStore = link.cast()?;
            // The string stays ours: PROPVARIANT's Drop would CoTaskMemFree it.
            let mut id: Vec<u16> = aumid.encode_utf16().chain(Some(0)).collect();
            let mut value = std::mem::ManuallyDrop::new(PROPVARIANT::default());
            (*value.Anonymous.Anonymous).vt = VT_LPWSTR;
            (*value.Anonymous.Anonymous).Anonymous.pwszVal = PWSTR(id.as_mut_ptr());
            store.SetValue(&PKEY_AppUserModel_ID, &*value)?;
            store.Commit()?;
            let file: IPersistFile = link.cast()?;
            file.Save(&HSTRING::from(lnk.as_os_str()), true)
        }
    }

    const RT_ICON: u16 = 3;
    const RT_GROUP_ICON: u16 = 14;

    /// An `.ico` file built from the first icon group of `exe`.
    pub fn icon_bytes(exe: &Path) -> Option<Vec<u8>> {
        let path: Vec<u16> = exe.as_os_str().encode_wide().chain(Some(0)).collect();
        unsafe {
            let module = LoadLibraryExW(
                path.as_ptr(),
                std::ptr::null_mut(),
                LOAD_LIBRARY_AS_DATAFILE | LOAD_LIBRARY_AS_IMAGE_RESOURCE,
            );
            if module.is_null() {
                return None;
            }
            let result = build_ico(module);
            FreeLibrary(module);
            result
        }
    }

    unsafe fn resource(module: *mut c_void, name: *const u16, kind: u16) -> Option<&'static [u8]> {
        unsafe {
            let found = FindResourceW(module, name, kind as usize as *const u16);
            if found.is_null() {
                return None;
            }
            let loaded = LoadResource(module, found);
            let data = LockResource(loaded) as *const u8;
            let size = SizeofResource(module, found) as usize;
            (!data.is_null()).then(|| std::slice::from_raw_parts(data, size))
        }
    }

    unsafe extern "system" fn first_name(
        _: *mut c_void,
        _: *const u16,
        name: *const u16,
        out: isize,
    ) -> i32 {
        unsafe { *(out as *mut *const u16) = name };
        0
    }

    unsafe fn build_ico(module: *mut c_void) -> Option<Vec<u8>> {
        unsafe {
            let mut name: *const u16 = std::ptr::null();
            EnumResourceNamesW(
                module,
                RT_GROUP_ICON as usize as *const u16,
                Some(first_name),
                &mut name as *mut _ as isize,
            );
            if name.is_null() {
                return None;
            }
            // GRPICONDIR: reserved, type, count; then 14-byte entries whose
            // last field is the RT_ICON id instead of a file offset.
            let group = resource(module, name, RT_GROUP_ICON)?;
            let count = u16::from_le_bytes(group.get(4..6)?.try_into().ok()?) as usize;
            let mut images = Vec::with_capacity(count);
            for i in 0..count {
                let entry = group.get(6 + i * 14..6 + (i + 1) * 14)?;
                let id = u16::from_le_bytes(entry[12..14].try_into().ok()?);
                let image = resource(module, id as usize as *const u16, RT_ICON)?;
                images.push((&entry[..8], image));
            }
            let mut ico = Vec::new();
            ico.extend_from_slice(&[0, 0, 1, 0]);
            ico.extend_from_slice(&(count as u16).to_le_bytes());
            let mut offset = 6 + 16 * count;
            for (head, image) in &images {
                ico.extend_from_slice(head);
                ico.extend_from_slice(&(image.len() as u32).to_le_bytes());
                ico.extend_from_slice(&(offset as u32).to_le_bytes());
                offset += image.len();
            }
            for (_, image) in &images {
                ico.extend_from_slice(image);
            }
            Some(ico)
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use std::path::Path;

    pub fn create(_: &Path, _: &Path, _: Option<&Path>, _: &str) -> std::io::Result<()> {
        Err(std::io::Error::other("shortcuts need Windows"))
    }

    pub fn icon_bytes(_: &Path) -> Option<Vec<u8>> {
        None
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn icon_and_shortcut() {
        let dir = crate::testutil::TempDir::new("lnk");
        let notepad = std::path::PathBuf::from(std::env::var_os("SystemRoot").unwrap())
            .join(r"System32\notepad.exe");
        let ico = dir.join("n.ico");
        assert!(extract_icon(&notepad, &ico).unwrap());
        let bytes = std::fs::read(&ico).unwrap();
        assert_eq!(&bytes[..4], &[0, 0, 1, 0]);
        assert!(bytes.len() > 1000);

        let cmd = notepad.with_file_name("cmd.exe");
        assert!(!extract_icon(&dir.join("missing.exe"), &dir.join("x.ico")).unwrap());

        let lnk = dir.join(r"Sub\Note pad.lnk");
        create(&lnk, &cmd, Some(&ico), &aumid("note pad", "Note pad")).unwrap();
        assert!(lnk.exists());
        assert_eq!(aumid("note pad", "Note pad"), "wpk.note_pad.Note_pad");
    }
}
