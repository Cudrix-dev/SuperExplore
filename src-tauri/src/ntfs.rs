use crate::index::{Entry, Index, Volume};
use rayon::prelude::*;
use rustc_hash::FxHashMap;
use std::{
    ffi::{c_void, OsStr, OsString},
    io,
    mem::size_of,
    os::windows::ffi::{OsStrExt, OsStringExt},
    path::PathBuf,
    ptr,
    sync::{Arc, RwLock},
    thread,
    time::{Duration, Instant},
};
use windows::{
    core::PCWSTR,
    Win32::{
        Foundation::{
            CloseHandle, ERROR_HANDLE_EOF, ERROR_MORE_DATA, ERROR_NO_MORE_FILES, GENERIC_READ,
            HANDLE,
        },
        Storage::FileSystem::{
            CreateFileW, FindFirstVolumeW, FindNextVolumeW, FindVolumeClose, GetDriveTypeW,
            GetVolumeInformationW, GetVolumePathNamesForVolumeNameW, FILE_ATTRIBUTE_DIRECTORY,
            FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
            OPEN_EXISTING,
        },
        System::{
            Ioctl::{
                FSCTL_ENUM_USN_DATA, FSCTL_QUERY_USN_JOURNAL, FSCTL_READ_USN_JOURNAL,
                MFT_ENUM_DATA_V0, READ_USN_JOURNAL_DATA_V0, USN_JOURNAL_DATA_V0,
                USN_REASON_FILE_CREATE, USN_REASON_FILE_DELETE, USN_REASON_HARD_LINK_CHANGE,
                USN_REASON_RENAME_NEW_NAME,
            },
            IO::DeviceIoControl,
        },
    },
};

const DRIVE_REMOVABLE: u32 = 2;
const DRIVE_FIXED: u32 = 3;
const DRIVE_CDROM: u32 = 5;
const DRIVE_RAMDISK: u32 = 6;
const BUFFER_SIZE: usize = 1024 * 1024;
const POLL_INTERVAL: Duration = Duration::from_millis(700);
const DISCOVERY_INTERVAL: Duration = Duration::from_secs(15);
const RESCAN_INTERVAL: Duration = Duration::from_secs(60);

struct Handle(HANDLE);

// This wrapper owns the kernel handle. It is moved to the indexing worker and
// calls to DeviceIoControl on it are serialized by that worker.
unsafe impl Send for Handle {}

impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

struct VolumeSearch(HANDLE);

impl Drop for VolumeSearch {
    fn drop(&mut self) {
        unsafe {
            let _ = FindVolumeClose(self.0);
        }
    }
}

#[derive(Clone)]
struct Location {
    guid: String,
    root: PathBuf,
}

fn wide(value: impl AsRef<OsStr>) -> Vec<u16> {
    value.as_ref().encode_wide().chain([0]).collect()
}

fn utf16_z(data: &[u16]) -> &[u16] {
    &data[..data
        .iter()
        .position(|&unit| unit == 0)
        .unwrap_or(data.len())]
}

fn discover() -> io::Result<Vec<Location>> {
    let mut name = [0u16; 128];
    let search = VolumeSearch(unsafe { FindFirstVolumeW(&mut name) }.map_err(win32_error)?);
    let mut locations = Vec::new();
    loop {
        let name_ptr = PCWSTR(name.as_ptr());
        let drive_type = unsafe { GetDriveTypeW(name_ptr) };
        let mut fs_name = [0u16; 32];
        if matches!(
            drive_type,
            DRIVE_FIXED | DRIVE_REMOVABLE | DRIVE_CDROM | DRIVE_RAMDISK
        ) && unsafe {
            GetVolumeInformationW(name_ptr, None, None, None, None, Some(&mut fs_name))
        }
        .is_ok()
            && utf16_z(&fs_name) == [b'N' as u16, b'T' as u16, b'F' as u16, b'S' as u16]
        {
            let guid = String::from_utf16_lossy(utf16_z(&name));
            let root = mount_path(name_ptr).unwrap_or_else(|| PathBuf::from(&guid));
            locations.push(Location { guid, root });
        }

        name.fill(0);
        if let Err(error) = unsafe { FindNextVolumeW(search.0, &mut name) } {
            let error = win32_error(error);
            return if error.raw_os_error() == Some(ERROR_NO_MORE_FILES.0 as i32) {
                Ok(locations)
            } else {
                Err(error)
            };
        }
    }
}

fn mount_path(volume: PCWSTR) -> Option<PathBuf> {
    let mut paths = vec![0; 512];
    let mut needed = 0;
    loop {
        let result =
            unsafe { GetVolumePathNamesForVolumeNameW(volume, Some(&mut paths), &mut needed) };
        if let Err(error) = result {
            if win32_error(error).raw_os_error() != Some(ERROR_MORE_DATA.0 as i32)
                || needed as usize <= paths.len()
            {
                return None;
            }
            paths.resize(needed as usize, 0);
        } else {
            // Prefer a drive letter, then the shortest mounted directory.
            return paths
                .split(|&unit| unit == 0)
                .take_while(|path| !path.is_empty())
                .min_by(|a, b| a.len().cmp(&b.len()).then_with(|| a.cmp(b)))
                .map(|path| PathBuf::from(OsString::from_wide(path)));
        }
    }
}

fn open(location: &Location) -> io::Result<Handle> {
    // CreateFileW expects a volume GUID without its trailing backslash.
    let volume = wide(location.guid.trim_end_matches('\\'));
    unsafe {
        CreateFileW(
            PCWSTR(volume.as_ptr()),
            GENERIC_READ.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            None,
            OPEN_EXISTING,
            FILE_FLAGS_AND_ATTRIBUTES(0),
            None,
        )
    }
    .map(Handle)
    .map_err(win32_error)
}

fn win32_error(error: windows::core::Error) -> io::Error {
    let code = error.code().0 as u32;
    if code >> 16 == 0x8007 {
        io::Error::from_raw_os_error((code & 0xffff) as i32)
    } else {
        io::Error::other(error)
    }
}

fn ioctl<T>(handle: &Handle, code: u32, input: Option<&T>, out: &mut [u8]) -> io::Result<usize> {
    let mut returned = 0;
    unsafe {
        DeviceIoControl(
            handle.0,
            code,
            input.map(|input| ptr::from_ref(input).cast::<c_void>()),
            if input.is_some() {
                size_of::<T>() as u32
            } else {
                0
            },
            Some(out.as_mut_ptr().cast()),
            out.len() as u32,
            Some(&mut returned),
            None,
        )
    }
    .map_err(win32_error)?;
    Ok(returned as usize)
}

fn invalid_record() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "无效的 NTFS USN 记录")
}

fn u16_at(data: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(data[offset..offset + 2].try_into().unwrap())
}

fn u32_at(data: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(data[offset..offset + 4].try_into().unwrap())
}

fn u64_at(data: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(data[offset..offset + 8].try_into().unwrap())
}

struct Record<'a>(&'a [u8]);

impl Record<'_> {
    fn id(&self) -> u64 {
        u64_at(self.0, 8)
    }

    fn reason(&self) -> u32 {
        u32_at(self.0, 40)
    }

    fn entry(&self) -> io::Result<Entry> {
        let length = u16_at(self.0, 56) as usize;
        let offset = u16_at(self.0, 58) as usize;
        if offset < 60 || offset % 2 != 0 || length % 2 != 0 || length == 0 {
            return Err(invalid_record());
        }
        let name = self
            .0
            .get(offset..offset + length)
            .ok_or_else(invalid_record)?;
        let name: String = char::decode_utf16(name.chunks_exact(2).map(|pair| u16_at(pair, 0)))
            .map(|character| character.unwrap_or(char::REPLACEMENT_CHARACTER))
            .collect();
        Ok(Entry::new(
            u64_at(self.0, 16),
            name,
            u32_at(self.0, 52) & FILE_ATTRIBUTE_DIRECTORY.0 != 0,
        ))
    }
}

fn records(data: &[u8], mut visit: impl FnMut(Record<'_>) -> io::Result<()>) -> io::Result<u64> {
    if data.len() < 8 {
        return Err(invalid_record());
    }
    let next = u64_at(data, 0);
    let mut rest = &data[8..];
    while !rest.is_empty() {
        if rest.len() < 8 {
            return Err(invalid_record());
        }
        let length = u32_at(rest, 0) as usize;
        // NTFS holds 64-bit FRNs. These V0 controls return USN_RECORD_V2.
        if length < 60 || length > rest.len() || u16_at(rest, 4) != 2 {
            return Err(invalid_record());
        }
        visit(Record(&rest[..length]))?;
        rest = &rest[length..];
    }
    Ok(next)
}

struct Journal {
    id: u64,
    cursor: i64,
}

impl Journal {
    fn from_tail(handle: &Handle) -> io::Result<Self> {
        let mut output = [0; size_of::<USN_JOURNAL_DATA_V0>()];
        let length = ioctl::<()>(handle, FSCTL_QUERY_USN_JOURNAL, None, &mut output)?;
        if length < output.len() {
            return Err(invalid_record());
        }
        Ok(Self {
            id: u64_at(&output, 0),
            cursor: u64_at(&output, 16) as i64,
        })
    }

    fn read(&mut self, handle: &Handle, buffer: &mut [u8]) -> io::Result<usize> {
        let request = READ_USN_JOURNAL_DATA_V0 {
            StartUsn: self.cursor,
            ReasonMask: u32::MAX,
            UsnJournalID: self.id,
            ..Default::default()
        };
        ioctl(handle, FSCTL_READ_USN_JOURNAL, Some(&request), buffer)
    }

    fn apply(&mut self, data: &[u8], volume: &mut Volume) -> io::Result<()> {
        self.cursor = records(data, |record| {
            let id = record.id();
            let reason = record.reason();
            if reason & USN_REASON_FILE_DELETE != 0 {
                volume.entries.remove(&id);
            } else if reason
                & (USN_REASON_FILE_CREATE
                    | USN_REASON_RENAME_NEW_NAME
                    | USN_REASON_HARD_LINK_CHANGE)
                != 0
            {
                volume.entries.insert(id, record.entry()?);
            }
            Ok(())
        })? as i64;
        Ok(())
    }
}

struct Scan {
    volume: Volume,
    handle: Handle,
    journal: Option<Journal>,
}

fn scan(location: &Location, buffer: &mut [u8]) -> io::Result<Scan> {
    let handle = open(location)?;
    let mut journal = Journal::from_tail(&handle).ok();
    let mut volume = Volume {
        root: location.root.clone(),
        entries: Default::default(),
    };
    let mut request = MFT_ENUM_DATA_V0 {
        HighUsn: i64::MAX,
        ..Default::default()
    };
    loop {
        let length = match ioctl(&handle, FSCTL_ENUM_USN_DATA, Some(&request), buffer) {
            Ok(length) => length,
            Err(error) if error.raw_os_error() == Some(ERROR_HANDLE_EOF.0 as i32) => break,
            Err(error) => return Err(error),
        };
        let next = records(&buffer[..length], |record| {
            volume.entries.insert(record.id(), record.entry()?);
            Ok(())
        })?;
        if next <= request.StartFileReferenceNumber {
            return Err(invalid_record());
        }
        request.StartFileReferenceNumber = next;
    }

    // Replay changes made while the MFT was being enumerated before exposing it.
    if let Some(journal) = &mut journal {
        let end = Journal::from_tail(&handle)?;
        if journal.id != end.id {
            return Err(io::Error::other("扫描期间 USN 日志已重建"));
        }
        while journal.cursor < end.cursor {
            let previous = journal.cursor;
            let length = journal.read(&handle, buffer)?;
            journal.apply(&buffer[..length], &mut volume)?;
            if journal.cursor <= previous {
                return Err(invalid_record());
            }
        }
    }
    Ok(Scan {
        volume,
        handle,
        journal,
    })
}

struct Watch {
    data: Arc<RwLock<Volume>>,
    handle: Handle,
    journal: Option<Journal>,
    last_scan: Instant,
}

impl Watch {
    fn publish(index: &Index, id: String, scan: Scan) -> Self {
        Self {
            data: index.set_volume(id, scan.volume),
            handle: scan.handle,
            journal: scan.journal,
            last_scan: Instant::now(),
        }
    }

    fn poll(&mut self, index: &Index, buffer: &mut [u8]) -> io::Result<()> {
        let Some(journal) = &mut self.journal else {
            return Ok(());
        };
        // Bound work per tick so activity on one volume cannot starve another.
        for _ in 0..8 {
            let length = journal.read(&self.handle, buffer)?;
            journal.apply(&buffer[..length], &mut self.data.write().unwrap())?;
            if length == 8 {
                break;
            }
            index.changed();
        }
        Ok(())
    }
}

pub fn monitor(index: Index) {
    let mut locations = discover().unwrap_or_else(|error| {
        index.set_warning(Some(format!("无法枚举 NTFS 卷 · {error}")));
        Vec::new()
    });
    let initial: Vec<_> = locations
        .par_iter()
        .map(|location| (location, scan(location, &mut vec![0; BUFFER_SIZE])))
        .collect();
    let mut watches = FxHashMap::default();
    let mut errors = FxHashMap::default();
    for (location, result) in initial {
        match result {
            Ok(scan) => {
                let watch = Watch::publish(&index, location.guid.clone(), scan);
                watches.insert(location.guid.clone(), watch);
            }
            Err(error) => {
                errors.insert(location.guid.clone(), error.to_string());
            }
        }
    }
    update_warning(&index, &locations, &watches, &errors);
    index.ready();

    let mut last_discovery = Instant::now();
    let mut buffer = vec![0; BUFFER_SIZE];
    loop {
        thread::sleep(POLL_INTERVAL);
        let rediscover = last_discovery.elapsed() >= DISCOVERY_INTERVAL;
        if rediscover {
            last_discovery = Instant::now();
            if let Ok(current) = discover() {
                locations = current;
                let present = |id: &str| locations.iter().any(|location| location.guid == id);
                watches.retain(|id, _| present(id));
                errors.retain(|id, _| present(id));
                index.retain_volumes(present);
            }
        }

        for location in &locations {
            let replace = if let Some(watch) = watches.get_mut(&location.guid) {
                let mut volume = watch.data.write().unwrap();
                if volume.root != location.root {
                    volume.root.clone_from(&location.root);
                    index.changed();
                }
                drop(volume);
                watch.poll(&index, &mut buffer).is_err()
                    || (watch.journal.is_none() && watch.last_scan.elapsed() >= RESCAN_INTERVAL)
            } else {
                rediscover
            };
            if replace {
                match scan(location, &mut buffer) {
                    Ok(scan) => {
                        let watch = Watch::publish(&index, location.guid.clone(), scan);
                        watches.insert(location.guid.clone(), watch);
                        errors.remove(&location.guid);
                    }
                    Err(error) => {
                        watches.remove(&location.guid);
                        index.retain_volumes(|id| id != location.guid);
                        errors.insert(location.guid.clone(), error.to_string());
                    }
                }
            }
        }
        update_warning(&index, &locations, &watches, &errors);
    }
}

fn update_warning(
    index: &Index,
    locations: &[Location],
    watches: &FxHashMap<String, Watch>,
    errors: &FxHashMap<String, String>,
) {
    let mut warnings = Vec::new();
    if locations.is_empty() {
        warnings.push("未检测到可用的本地 NTFS 卷".into());
    }
    for location in locations {
        if let Some(error) = errors.get(&location.guid) {
            warnings.push(format!("{} 索引失败 · {error}", location.root.display()));
        } else if watches
            .get(&location.guid)
            .is_some_and(|watch| watch.journal.is_none())
        {
            warnings.push(format!(
                "{} 的 USN 日志不可用，每分钟重新扫描",
                location.root.display()
            ));
        }
    }
    index.set_warning((!warnings.is_empty()).then(|| warnings.join("；")));
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::System::Ioctl::USN_REASON_RENAME_OLD_NAME;

    fn journal_batch(cursor: u64, reason: u32, name: &str) -> Vec<u8> {
        let wide: Vec<_> = name.encode_utf16().flat_map(u16::to_le_bytes).collect();
        let length = (60 + wide.len()).next_multiple_of(8);
        let mut data = vec![0; 8 + length];
        data[..8].copy_from_slice(&cursor.to_le_bytes());
        let record = &mut data[8..];
        record[..4].copy_from_slice(&(length as u32).to_le_bytes());
        record[4..6].copy_from_slice(&2u16.to_le_bytes());
        record[8..16].copy_from_slice(&81u64.to_le_bytes());
        record[16..24].copy_from_slice(&5u64.to_le_bytes());
        record[40..44].copy_from_slice(&reason.to_le_bytes());
        record[56..58].copy_from_slice(&(wide.len() as u16).to_le_bytes());
        record[58..60].copy_from_slice(&60u16.to_le_bytes());
        record[60..60 + wide.len()].copy_from_slice(&wide);
        data
    }

    #[test]
    fn journal_applies_create_rename_delete_and_rejects_truncated_records() {
        let mut journal = Journal { id: 1, cursor: 0 };
        let mut volume = Volume {
            root: PathBuf::from(r"X:\"),
            entries: Default::default(),
        };
        journal
            .apply(
                &journal_batch(10, USN_REASON_FILE_CREATE, "原始.txt"),
                &mut volume,
            )
            .unwrap();
        assert_eq!(&*volume.entries[&81].name, "原始.txt");
        journal
            .apply(
                &journal_batch(20, USN_REASON_RENAME_OLD_NAME, "原始.txt"),
                &mut volume,
            )
            .unwrap();
        assert!(volume.entries.contains_key(&81));
        journal
            .apply(
                &journal_batch(30, USN_REASON_RENAME_NEW_NAME, "更新.txt"),
                &mut volume,
            )
            .unwrap();
        assert_eq!(&*volume.entries[&81].name, "更新.txt");
        let mut deletion = journal_batch(40, USN_REASON_FILE_DELETE, "更新.txt");
        assert!(journal
            .apply(&deletion[..deletion.len() - 1], &mut volume)
            .is_err());
        assert_eq!(journal.cursor, 30);
        assert!(volume.entries.contains_key(&81));
        journal.apply(&deletion, &mut volume).unwrap();
        assert_eq!(journal.cursor, 40);
        assert!(!volume.entries.contains_key(&81));
        deletion[12] = 3;
        assert!(journal.apply(&deletion, &mut volume).is_err());
    }
}
