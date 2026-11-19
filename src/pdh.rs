//! Performance counters (PDH): disks, GPU engines, thermal zones, memory lists.

use crate::util::{from_wide_ptr, wide};
use std::collections::HashMap;
use windows_sys::Win32::System::Performance::{
    PdhAddEnglishCounterW, PdhCloseQuery, PdhCollectQueryData, PdhGetFormattedCounterArrayW, PdhOpenQueryW,
    PDH_FMT_COUNTERVALUE_ITEM_W,
};

const PDH_FMT_DOUBLE: u32 = 0x0000_0200;
const PDH_FMT_NOCAP100: u32 = 0x0000_8000;
const PDH_MORE_DATA: u32 = 0x8000_07D2;

pub struct Pdh {
    query: isize,
    counters: HashMap<&'static str, isize>,
}

unsafe impl Send for Pdh {}

impl Pdh {
    pub fn new() -> Option<Self> {
        let mut q: isize = 0;
        let r = unsafe { PdhOpenQueryW(std::ptr::null(), 0, &mut q) };
        if r != 0 {
            return None;
        }
        Some(Self { query: q, counters: HashMap::new() })
    }

    pub fn add(&mut self, key: &'static str, path: &str) -> bool {
        let w = wide(path);
        let mut c: isize = 0;
        let r = unsafe { PdhAddEnglishCounterW(self.query, w.as_ptr(), 0, &mut c) };
        if r == 0 {
            self.counters.insert(key, c);
            true
        } else {
            false
        }
    }

    pub fn has(&self, key: &str) -> bool {
        self.counters.contains_key(key)
    }

    pub fn collect(&mut self) {
        unsafe {
            PdhCollectQueryData(self.query);
        }
    }

    /// All instances of a wildcard counter as (instance, value).
    pub fn array(&self, key: &str) -> Vec<(String, f64)> {
        let Some(&c) = self.counters.get(key) else { return vec![] };
        let mut out = vec![];
        unsafe {
            let mut size = 0u32;
            let mut count = 0u32;
            let r = PdhGetFormattedCounterArrayW(c, PDH_FMT_DOUBLE | PDH_FMT_NOCAP100, &mut size, &mut count, std::ptr::null_mut());
            if r != PDH_MORE_DATA || size == 0 {
                return out;
            }
            let mut buf: Vec<u64> = vec![0; size as usize / 8 + 8];
            let r = PdhGetFormattedCounterArrayW(
                c,
                PDH_FMT_DOUBLE | PDH_FMT_NOCAP100,
                &mut size,
                &mut count,
                buf.as_mut_ptr() as *mut PDH_FMT_COUNTERVALUE_ITEM_W,
            );
            if r != 0 {
                return out;
            }
            let items = buf.as_ptr() as *const PDH_FMT_COUNTERVALUE_ITEM_W;
            for i in 0..count as usize {
                let it = &*items.add(i);
                if it.FmtValue.CStatus != 0 && it.FmtValue.CStatus != 1 {
                    continue;
                }
                let name = from_wide_ptr(it.szName);
                out.push((name, it.FmtValue.Anonymous.doubleValue));
            }
        }
        out
    }

    pub fn single(&self, key: &str) -> Option<f64> {
        self.array(key).into_iter().next().map(|(_, v)| v)
    }
}

impl Drop for Pdh {
    fn drop(&mut self) {
        unsafe {
            PdhCloseQuery(self.query);
        }
    }
}
