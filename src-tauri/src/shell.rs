use std::{os::windows::ffi::OsStrExt, path::Path};
use windows::{
    core::{Interface, Result, BSTR},
    Win32::{
        System::{
            Com::{
                CoCreateInstance, CoInitializeEx, CoUninitialize, IDispatch, IServiceProvider,
                CLSCTX_LOCAL_SERVER, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE,
            },
            Variant::VARIANT,
        },
        UI::{
            Shell::{
                IShellBrowser, IShellDispatch2, IShellFolderViewDual, IShellWindows,
                SID_STopLevelBrowser, ShellWindows, SVGIO_BACKGROUND, SWC_DESKTOP,
                SWFO_NEEDDISPATCH,
            },
            WindowsAndMessaging::SW_SHOWNORMAL,
        },
    },
};

struct Apartment;

impl Apartment {
    fn initialize() -> Result<Self> {
        unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE).ok()? };
        Ok(Self)
    }
}

impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe { CoUninitialize() }
    }
}

pub fn open(path: &Path) -> Result<()> {
    let _apartment = Apartment::initialize()?;
    // Run the launch operation in the interactive Explorer session.
    unsafe {
        let windows: IShellWindows = CoCreateInstance(&ShellWindows, None, CLSCTX_LOCAL_SERVER)?;
        let desktop = windows.FindWindowSW(
            &VARIANT::default(),
            &VARIANT::default(),
            SWC_DESKTOP,
            &mut 0,
            SWFO_NEEDDISPATCH,
        )?;
        let services: IServiceProvider = desktop.cast()?;
        let browser: IShellBrowser = services.QueryService(&SID_STopLevelBrowser)?;
        let view = browser.QueryActiveShellView()?;
        // Explorer exposes the background as IDispatch. Query its dual interface
        // after obtaining the automation object from the view.
        let background: IDispatch = view.GetItemObject(SVGIO_BACKGROUND)?;
        let folder: IShellFolderViewDual = background.cast()?;
        let shell: IShellDispatch2 = folder.Application()?.cast()?;
        let file = BSTR::from_wide(&path.as_os_str().encode_wide().collect::<Vec<_>>());
        shell.ShellExecute(
            &file,
            &VARIANT::default(),
            &VARIANT::default(),
            &VARIANT::from("open"),
            &VARIANT::from(SW_SHOWNORMAL.0),
        )
    }
}
