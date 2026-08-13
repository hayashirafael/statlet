use std::ffi::{c_char, c_void, CString};
use std::ptr;

use statlet::system_usage::{GpuReading, GpuSampleOutcome};

type CfTypeRef = *const c_void;
type CfStringRef = *const c_void;
type CfDictionaryRef = *const c_void;
type CfAllocatorRef = *const c_void;
type CfTypeId = usize;
type IoObject = u32;

const KERN_SUCCESS: i32 = 0;
const CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;
const CF_NUMBER_SINT64_TYPE: i32 = 4;
const CF_NUMBER_DOUBLE_TYPE: i32 = 13;

#[link(name = "IOKit", kind = "framework")]
extern "C" {
    fn IOServiceMatching(name: *const c_char) -> *mut c_void;
    fn IOServiceGetMatchingServices(
        main_port: u32,
        matching: *const c_void,
        existing: *mut IoObject,
    ) -> i32;
    fn IOIteratorNext(iterator: IoObject) -> IoObject;
    fn IOObjectRelease(object: IoObject) -> i32;
    fn IORegistryEntryCreateCFProperty(
        entry: IoObject,
        key: CfStringRef,
        allocator: CfAllocatorRef,
        options: u32,
    ) -> CfTypeRef;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFStringCreateWithCString(
        allocator: CfAllocatorRef,
        c_str: *const c_char,
        encoding: u32,
    ) -> CfStringRef;
    fn CFGetTypeID(value: CfTypeRef) -> CfTypeId;
    fn CFDictionaryGetTypeID() -> CfTypeId;
    fn CFNumberGetTypeID() -> CfTypeId;
    fn CFDictionaryGetValue(dictionary: CfDictionaryRef, key: *const c_void) -> *const c_void;
    fn CFNumberGetValue(number: CfTypeRef, number_type: i32, value: *mut c_void) -> bool;
    fn CFRelease(value: CfTypeRef);
}

struct CfKey(CfStringRef);

impl CfKey {
    fn new(value: &str) -> Self {
        let value = CString::new(value).expect("static Core Foundation key");
        let key = unsafe {
            CFStringCreateWithCString(ptr::null(), value.as_ptr(), CF_STRING_ENCODING_UTF8)
        };
        assert!(!key.is_null(), "create static Core Foundation key");
        Self(key)
    }
}

impl Drop for CfKey {
    fn drop(&mut self) {
        unsafe { CFRelease(self.0) };
    }
}

struct GpuKeys {
    performance_statistics: CfKey,
    device_utilization: CfKey,
    renderer_utilization: CfKey,
    tiler_utilization: CfKey,
    in_use_system_memory: CfKey,
    allocated_system_memory: CfKey,
}

impl GpuKeys {
    fn new() -> Self {
        Self {
            performance_statistics: CfKey::new("PerformanceStatistics"),
            device_utilization: CfKey::new("Device Utilization %"),
            renderer_utilization: CfKey::new("Renderer Utilization %"),
            tiler_utilization: CfKey::new("Tiler Utilization %"),
            in_use_system_memory: CfKey::new("In use system memory"),
            allocated_system_memory: CfKey::new("Alloc system memory"),
        }
    }
}

pub struct MacGpuSampler {
    service: IoObject,
    keys: GpuKeys,
}

impl MacGpuSampler {
    pub fn new() -> Self {
        Self {
            service: 0,
            keys: GpuKeys::new(),
        }
    }

    pub fn sample(&mut self) -> GpuSampleOutcome {
        if self.service == 0 && !self.discover_service() {
            return GpuSampleOutcome::Unavailable;
        }
        let first = self.read_current_service();
        if !matches!(first, GpuSampleOutcome::Failed) {
            return first;
        }

        self.release_service();
        if !self.discover_service() {
            return GpuSampleOutcome::Failed;
        }
        self.read_current_service()
    }

    fn discover_service(&mut self) -> bool {
        let matching = unsafe { IOServiceMatching(c"AGXAccelerator".as_ptr()) };
        if matching.is_null() {
            return false;
        }
        let mut iterator = 0;
        let result = unsafe { IOServiceGetMatchingServices(0, matching, &mut iterator) };
        if result != KERN_SUCCESS || iterator == 0 {
            return false;
        }
        self.service = unsafe { IOIteratorNext(iterator) };
        unsafe { IOObjectRelease(iterator) };
        self.service != 0
    }

    fn read_current_service(&self) -> GpuSampleOutcome {
        self.read_performance_statistics_with(|| unsafe {
            IORegistryEntryCreateCFProperty(
                self.service,
                self.keys.performance_statistics.0,
                ptr::null(),
                0,
            )
        })
    }

    fn read_performance_statistics_with<F>(&self, read_property: F) -> GpuSampleOutcome
    where
        F: FnOnce() -> CfTypeRef,
    {
        let dictionary = read_property();
        if dictionary.is_null() {
            return GpuSampleOutcome::Unavailable;
        }
        let outcome = unsafe { self.read_dictionary(dictionary) };
        unsafe { CFRelease(dictionary) };
        outcome
    }

    unsafe fn read_dictionary(&self, dictionary: CfDictionaryRef) -> GpuSampleOutcome {
        if CFGetTypeID(dictionary) != CFDictionaryGetTypeID() {
            return GpuSampleOutcome::Unavailable;
        }
        let Some(utilization) = dictionary_number(dictionary, self.keys.device_utilization.0)
        else {
            return GpuSampleOutcome::Unavailable;
        };
        let reading = GpuReading::normalized(
            utilization,
            dictionary_number(dictionary, self.keys.renderer_utilization.0),
            dictionary_number(dictionary, self.keys.tiler_utilization.0),
            dictionary_u64(dictionary, self.keys.in_use_system_memory.0),
            dictionary_u64(dictionary, self.keys.allocated_system_memory.0),
            None,
        );
        reading
            .map(GpuSampleOutcome::Available)
            .unwrap_or(GpuSampleOutcome::Unavailable)
    }

    fn release_service(&mut self) {
        if self.service != 0 {
            unsafe { IOObjectRelease(self.service) };
            self.service = 0;
        }
    }
}

impl Default for MacGpuSampler {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for MacGpuSampler {
    fn drop(&mut self) {
        self.release_service();
    }
}

unsafe fn dictionary_number(dictionary: CfDictionaryRef, key: CfStringRef) -> Option<f64> {
    let value = CFDictionaryGetValue(dictionary, key);
    if value.is_null() || CFGetTypeID(value) != CFNumberGetTypeID() {
        return None;
    }
    let mut number = 0.0_f64;
    CFNumberGetValue(
        value,
        CF_NUMBER_DOUBLE_TYPE,
        &mut number as *mut f64 as *mut c_void,
    )
    .then_some(number)
}

unsafe fn dictionary_u64(dictionary: CfDictionaryRef, key: CfStringRef) -> Option<u64> {
    let value = CFDictionaryGetValue(dictionary, key);
    if value.is_null() || CFGetTypeID(value) != CFNumberGetTypeID() {
        return None;
    }
    let mut number = 0_i64;
    let converted = CFNumberGetValue(
        value,
        CF_NUMBER_SINT64_TYPE,
        &mut number as *mut i64 as *mut c_void,
    );
    (converted && number >= 0).then_some(number as u64)
}

#[cfg(test)]
mod tests {
    use std::ptr;

    use super::MacGpuSampler;
    use statlet::system_usage::GpuSampleOutcome;

    #[test]
    fn iokit_gpu_smoke_is_safe_and_best_effort() {
        let mut sampler = MacGpuSampler::new();

        if let GpuSampleOutcome::Available(reading) = sampler.sample() {
            assert!((0.0..=100.0).contains(&reading.utilization_percent));
        }
    }

    #[test]
    fn missing_performance_statistics_is_an_unavailable_capability() {
        let sampler = MacGpuSampler::new();

        assert_eq!(
            sampler.read_performance_statistics_with(ptr::null),
            GpuSampleOutcome::Unavailable
        );
    }
}
